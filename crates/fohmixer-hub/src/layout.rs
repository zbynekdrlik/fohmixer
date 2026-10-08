//! The served layout (S3 design note §5; spec §2.5, D4): `layout.json` in
//! the data folder, checked every 2 s.
//!
//! A changed file is served only when it parses, validates, and binds only
//! configured instances. Otherwise the last good layout stays served (a bad
//! edit never blanks the surface) and the error is reported in
//! `/api/status`. Every accepted content is kept as a dated backup,
//! `layout-backups/layout.json.<UTC time>`, the newest 30. A hub that starts
//! while the file is bad (or gone) serves the newest backup that is still
//! valid — the last good layout survives a restart — and reports the file's
//! error until the file is fixed.
//!
//! The file is the frame (#68, spec D16): the store serves it composed with
//! the Tuner markers the router's marker keeper found
//! (`fohmixer_proto::markers::compose`). A new marker set bumps the revision
//! only when the composition changed (I10); each such composition is kept
//! as `layout-backups/served.json.<UTC time>`, the newest 30.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use fohmixer_proto::client::MarkersStatus;
use fohmixer_proto::layout::Layout;
use fohmixer_proto::markers::{Found, MarkerReport, compose, frame_problems};

/// Backups kept.
pub const MAX_BACKUPS: usize = 30;
/// The backup folder inside the data folder.
pub const BACKUP_DIR: &str = "layout-backups";
/// The backups' name of a served composition (#68).
pub const SERVED: &str = "served.json";

/// What is served and why the file on disk is not (when it is not).
#[derive(Debug, Default)]
struct State {
    rev: u64,
    /// The served layout: the frame composed with the markers.
    layout: Option<Arc<Layout>>,
    /// The accepted file (#68: the frame).
    frame: Option<Arc<Layout>>,
    /// The markers found, and the problems of the last composition.
    markers: Vec<Found>,
    problems: Vec<MarkerReport>,
    /// The accepted file content.
    accepted: Option<Vec<u8>>,
    error: Option<String>,
    /// The file's (modified time, length) at the last check.
    seen: Option<(SystemTime, u64)>,
    /// The backups were tried once (nothing served, the file unusable).
    fallback_tried: bool,
}

/// The layout store.
#[derive(Debug)]
pub struct LayoutStore {
    path: PathBuf,
    backups: PathBuf,
    instances: Vec<String>,
    state: Mutex<State>,
}

/// The backup file name of a content accepted at `now` (UTC,
/// millisecond precision; names sort by time).
pub fn backup_name(file_name: &str, now: SystemTime) -> String {
    let since = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = i64::try_from(since.as_secs()).unwrap_or(i64::MAX);
    let stamp = chrono::DateTime::from_timestamp(secs, since.subsec_nanos())
        .map(|t| t.format("%Y%m%dT%H%M%S%.3fZ").to_string())
        .unwrap_or_else(|| "unknown".to_string());
    format!("{file_name}.{stamp}")
}

/// `fohmixer-hub layout check <layout> <config>` (#21): whether the hub
/// configured by `config` would serve `layout` (it parses, validates and
/// binds only the config's instances). The installer asks the new hub before
/// it stops the running one, so a schema change never leaves the hub without
/// a layout.
pub fn check_files(layout: &Path, config: &Path) -> anyhow::Result<()> {
    use anyhow::Context as _;
    let text = std::fs::read_to_string(config)
        .with_context(|| format!("reading config {}", config.display()))?;
    let dir = config.parent().unwrap_or(Path::new("."));
    let config = crate::config::Config::parse(&text, dir)
        .with_context(|| format!("config {}", config.display()))?;
    let names: Vec<String> = config.instances.iter().map(|i| i.name.clone()).collect();
    let bytes =
        std::fs::read(layout).with_context(|| format!("reading layout {}", layout.display()))?;
    check(&bytes, &names)
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("layout {}: {e}", layout.display()))
}

/// Why a layout text cannot be served: it does not parse, does not validate,
/// or binds an instance the hub does not have.
pub fn check(text: &[u8], instances: &[String]) -> Result<Layout, String> {
    let layout: Layout =
        serde_json::from_slice(text).map_err(|e| format!("layout does not parse: {e}"))?;
    // A frame (#68): the layout's own rules, and what only the markers make.
    let mut problems: Vec<String> = layout
        .validate()
        .into_iter()
        .chain(frame_problems(&layout))
        .map(|e| e.to_string())
        .collect();
    let named = layout.bindings().into_iter().map(|b| &b.instance);
    for instance in named {
        if !instances.contains(instance) {
            problems.push(format!("unknown instance {instance:?}"));
        }
    }
    problems.dedup();
    if problems.is_empty() {
        Ok(layout)
    } else {
        Err(format!("layout is invalid: {}", problems.join("; ")))
    }
}

impl LayoutStore {
    /// A store for `path`, backups in `backups`, bindings allowed on
    /// `instances`. Nothing is read until the first [`LayoutStore::poll`].
    pub fn new(path: PathBuf, backups: PathBuf, instances: Vec<String>) -> Self {
        Self {
            path,
            backups,
            instances,
            state: Mutex::new(State::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The served revision (0: none) and layout.
    pub fn current(&self) -> (u64, Option<Arc<Layout>>) {
        let state = self.lock();
        (state.rev, state.layout.clone())
    }

    /// Why the file on disk is not served, if it is not.
    pub fn error(&self) -> Option<String> {
        self.lock().error.clone()
    }

    /// The markers the keeper found (#68): served composed with the frame;
    /// `Some(rev)` when the composition changed (I10).
    pub fn set_markers(&self, found: Vec<Found>) -> Option<u64> {
        let mut state = self.lock();
        state.markers = found;
        let before = state.layout.clone();
        serve(&mut state);
        if state.layout == before {
            return None;
        }
        state.rev += 1;
        let rev = state.rev;
        let served = state.layout.clone();
        drop(state);
        tracing::info!(
            rev,
            "the layout's markers changed: a new composition served"
        );
        self.backup_served(served.as_deref());
        Some(rev)
    }

    /// The markers in `/api/status` (#68): how many, and every problem.
    pub fn markers_status(&self) -> MarkersStatus {
        let state = self.lock();
        MarkersStatus {
            found: state.markers.len(),
            problems: state.problems.clone(),
        }
    }

    /// Checks the file; `Some(rev)` when a new layout is served.
    pub fn poll(&self) -> Option<u64> {
        let seen = match std::fs::metadata(&self.path) {
            Ok(meta) => (meta.modified().unwrap_or(UNIX_EPOCH), meta.len()),
            Err(e) => {
                let message = format!("{}: {e}", self.path.display());
                let mut state = self.lock();
                if state.error.as_deref() != Some(message.as_str()) {
                    tracing::warn!(error = %message, "no layout file");
                    state.error = Some(message);
                }
                state.seen = None;
                return self.fallback(&mut state);
            }
        };
        if self.lock().seen == Some(seen) {
            return None;
        }
        let text = match std::fs::read(&self.path) {
            Ok(text) => text,
            Err(e) => {
                let mut state = self.lock();
                state.error = Some(format!("{}: {e}", self.path.display()));
                return self.fallback(&mut state);
            }
        };
        let mut state = self.lock();
        state.seen = Some(seen);
        if state.accepted.as_deref() == Some(text.as_slice()) {
            state.error = None;
            return None;
        }
        match check(&text, &self.instances) {
            Ok(layout) => {
                state.rev += 1;
                state.frame = Some(Arc::new(layout));
                serve(&mut state);
                self.backup_served(state.layout.as_deref());
                state.error = None;
                self.backup(&text);
                state.accepted = Some(text);
                tracing::info!(rev = state.rev, path = %self.path.display(), "layout accepted");
                Some(state.rev)
            }
            Err(error) => {
                tracing::warn!(error = %error, rev = state.rev, "layout rejected: the last good one stays served");
                state.error = Some(error);
                self.fallback(&mut state)
            }
        }
    }

    /// Nothing served yet and the file unusable: serves the newest backup
    /// that is still valid (tried once); `Some(rev)` when it does.
    fn fallback(&self, state: &mut State) -> Option<u64> {
        if state.layout.is_some() || state.fallback_tried {
            return None;
        }
        state.fallback_tried = true;
        let file_name = self.file_name();
        for name in self.list_backups(&file_name).iter().rev() {
            let Ok(text) = std::fs::read(self.backups.join(name)) else {
                continue;
            };
            let Ok(layout) = check(&text, &self.instances) else {
                tracing::warn!(backup = %name, "a layout backup no longer valid: skipped");
                continue;
            };
            state.rev += 1;
            state.frame = Some(Arc::new(layout));
            serve(state);
            state.accepted = Some(text);
            tracing::warn!(
                backup = %name,
                rev = state.rev,
                error = ?state.error,
                "the layout file is not usable: serving its newest good backup"
            );
            return Some(state.rev);
        }
        None
    }

    /// The layout file's own name (the backups' prefix).
    fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "layout.json".to_string())
    }

    /// Keeps `text` as a dated backup of the file (unless the newest backup
    /// already holds it) and prunes the oldest beyond [`MAX_BACKUPS`].
    fn backup(&self, text: &[u8]) {
        self.backup_named(&self.file_name(), text);
    }

    /// Keeps a served composition (#68) as a dated `served.json` backup.
    fn backup_served(&self, served: Option<&Layout>) {
        if let Some(text) = served.and_then(|l| serde_json::to_vec_pretty(l).ok()) {
            self.backup_named(SERVED, &text);
        }
    }

    /// The same for backups named `file_name`.
    fn backup_named(&self, file_name: &str, text: &[u8]) {
        let file_name = file_name.to_string();
        if let Err(e) = std::fs::create_dir_all(&self.backups) {
            tracing::error!(dir = %self.backups.display(), error = %e, "cannot create the layout backup folder");
            return;
        }
        let mut backups = self.list_backups(&file_name);
        if let Some(newest) = backups.last()
            && std::fs::read(self.backups.join(newest)).ok().as_deref() == Some(text)
        {
            return;
        }
        let name = backup_name(&file_name, SystemTime::now());
        if let Err(e) = std::fs::write(self.backups.join(&name), text) {
            tracing::error!(backup = %name, error = %e, "cannot write the layout backup");
            return;
        }
        backups.push(name);
        backups.sort();
        let excess = backups.len().saturating_sub(MAX_BACKUPS);
        for old in &backups[..excess] {
            if let Err(e) = std::fs::remove_file(self.backups.join(old)) {
                tracing::warn!(backup = %old, error = %e, "cannot prune a layout backup");
            }
        }
    }

    /// The backup file names of `file_name`, oldest first.
    fn list_backups(&self, file_name: &str) -> Vec<String> {
        let prefix = format!("{file_name}.");
        let mut names: Vec<String> = std::fs::read_dir(&self.backups)
            .map(|dir| {
                dir.filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.starts_with(&prefix) && n.ends_with('Z'))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// The backups kept now (tests, diagnostics).
    pub fn backups(&self) -> Vec<String> {
        self.list_backups(&self.file_name())
    }

    /// The served compositions kept now (#68; tests, diagnostics).
    pub fn served_backups(&self) -> Vec<String> {
        self.list_backups(SERVED)
    }
}

/// Serves the frame composed with the markers (none without a frame).
fn serve(state: &mut State) {
    let Some(frame) = state.frame.clone() else {
        state.problems = Vec::new();
        return;
    };
    let composed = compose(&frame, &state.markers);
    state.problems = composed.problems;
    state.layout = Some(Arc::new(composed.layout));
}

/// The store of a data folder: `layout` (relative to it) and its backups.
pub fn store_in(data_dir: &Path, layout: &Path, instances: Vec<String>) -> LayoutStore {
    LayoutStore::new(data_dir.join(layout), data_dir.join(BACKUP_DIR), instances)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    fn layout_json(title: &str, instance: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "schema": 2,
            "default_page": "main",
            "pages": [{"id": "main", "title": title, "rail": [
                {"kind": "solo",
                 "binding": {"instance": instance, "anchor": {"kind": "track", "name": "Stems grp#"}}}
            ]}]
        }))
        .unwrap()
    }

    fn store(dir: &Path) -> LayoutStore {
        store_in(
            dir,
            Path::new("layout.json"),
            vec!["band".into(), "master".into()],
        )
    }

    /// Writes the file with a modified time no earlier write had (a coarse
    /// file-system clock could otherwise hide a change of equal length).
    fn write(dir: &Path, text: &[u8]) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static WRITES: AtomicU64 = AtomicU64::new(1);
        let path = dir.join("layout.json");
        std::fs::write(&path, text).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        let offset = Duration::from_secs(WRITES.fetch_add(1, Ordering::Relaxed));
        file.set_modified(SystemTime::now() + offset).unwrap();
    }

    fn title(store: &LayoutStore) -> String {
        store.current().1.unwrap().pages[0].title.clone()
    }

    #[test]
    fn no_file_serves_nothing_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        assert_eq!(store.poll(), None);
        assert_eq!(store.current().0, 0);
        assert!(store.current().1.is_none());
        let error = store.error().unwrap();
        assert!(error.contains("layout.json"), "{error}");
        assert_eq!(store.poll(), None);
    }

    #[test]
    fn a_valid_file_is_served_and_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        write(dir.path(), &layout_json("FOH", "band"));
        assert_eq!(store.poll(), Some(1));
        assert_eq!(title(&store), "FOH");
        assert_eq!(store.error(), None);
        assert_eq!(store.poll(), None, "unchanged: nothing new");
        let backups = store.backups();
        assert_eq!(backups.len(), 1);
        assert!(backups[0].starts_with("layout.json.2"), "{backups:?}");
        assert_eq!(
            std::fs::read(dir.path().join(BACKUP_DIR).join(&backups[0])).unwrap(),
            layout_json("FOH", "band")
        );
    }

    #[test]
    fn a_changed_valid_file_bumps_the_revision() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        write(dir.path(), &layout_json("FOH", "band"));
        store.poll();
        std::thread::sleep(Duration::from_millis(5));
        write(dir.path(), &layout_json("FOH 2", "master"));
        assert_eq!(store.poll(), Some(2));
        assert_eq!(title(&store), "FOH 2");
        assert_eq!(store.backups().len(), 2);
        // The same content touched again is not a change.
        write(dir.path(), &layout_json("FOH 2", "master"));
        assert_eq!(store.poll(), None);
        assert_eq!(store.current().0, 2);
    }

    #[test]
    fn an_invalid_file_keeps_the_last_good_layout() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        write(dir.path(), &layout_json("FOH", "band"));
        store.poll();
        write(dir.path(), b"{\"schema\": 2, \"pages\":");
        assert_eq!(store.poll(), None);
        assert_eq!(store.current().0, 1);
        assert_eq!(title(&store), "FOH");
        assert!(
            store
                .error()
                .unwrap()
                .starts_with("layout does not parse: "),
            "{:?}",
            store.error()
        );
        write(dir.path(), &layout_json("Drums", "drums"));
        assert_eq!(store.poll(), None);
        assert_eq!(
            store.error().unwrap(),
            r#"layout is invalid: unknown instance "drums""#
        );
        assert_eq!(
            store.backups().len(),
            1,
            "rejected content is not backed up"
        );
        // Back to the accepted content: served as it was, error cleared.
        write(dir.path(), &layout_json("FOH", "band"));
        assert_eq!(store.poll(), None);
        assert_eq!(store.error(), None);
        assert_eq!(store.current().0, 1);
        // Deleted: the last good layout stays; the error says why.
        std::fs::remove_file(dir.path().join("layout.json")).unwrap();
        assert_eq!(store.poll(), None);
        assert_eq!(title(&store), "FOH");
        assert!(store.error().is_some());
    }

    #[test]
    fn validation_errors_are_listed() {
        let mut v: serde_json::Value = serde_json::from_slice(&layout_json("X", "band")).unwrap();
        v["schema"] = json!(9);
        let error = check(&serde_json::to_vec(&v).unwrap(), &["band".into()]).unwrap_err();
        assert_eq!(error, "layout is invalid: schema: schema 9 is not 2");
        assert!(check(&layout_json("X", "band"), &["band".into()]).is_ok());
        // TouchOSC's groups to unfold are gone (#58): the hub unfolds the
        // strips' groups itself, and a layout still naming them is refused.
        let mut v: serde_json::Value = serde_json::from_slice(&layout_json("X", "band")).unwrap();
        v["config"] = json!({"unfold": [{"instance": "band", "name": "Kit grp#"}]});
        let error = check(&serde_json::to_vec(&v).unwrap(), &["band".into()]).unwrap_err();
        assert!(error.contains("unknown field `unfold`"), "{error}");
    }

    fn backup_of(dir: &Path, stamp: &str, text: &[u8]) {
        let backups = dir.join(BACKUP_DIR);
        std::fs::create_dir_all(&backups).unwrap();
        std::fs::write(backups.join(format!("layout.json.{stamp}")), text).unwrap();
    }

    #[test]
    fn a_bad_file_at_start_serves_the_newest_good_backup() {
        let dir = tempfile::tempdir().unwrap();
        backup_of(
            dir.path(),
            "20260101T000000.000Z",
            &layout_json("Old", "band"),
        );
        backup_of(
            dir.path(),
            "20260102T000000.000Z",
            &layout_json("New", "band"),
        );
        // The newest binds an instance this hub no longer has: skipped.
        backup_of(
            dir.path(),
            "20260103T000000.000Z",
            &layout_json("Drums", "drums"),
        );
        write(dir.path(), b"{\"schema\": 2, \"pages\":");
        let store = store(dir.path());
        assert_eq!(store.poll(), Some(1));
        assert_eq!(title(&store), "New");
        let error = store.error().unwrap();
        assert!(error.starts_with("layout does not parse: "), "{error}");
        assert_eq!(store.poll(), None, "tried once");
        // The file fixed: served as usual.
        write(dir.path(), &layout_json("Fixed", "band"));
        assert_eq!(store.poll(), Some(2));
        assert_eq!(title(&store), "Fixed");
        assert_eq!(store.error(), None);
    }

    #[test]
    fn a_missing_file_at_start_serves_the_newest_good_backup_too() {
        let dir = tempfile::tempdir().unwrap();
        backup_of(
            dir.path(),
            "20260101T000000.000Z",
            &layout_json("Kept", "band"),
        );
        let store = store(dir.path());
        assert_eq!(store.poll(), Some(1));
        assert_eq!(title(&store), "Kept");
        assert!(store.error().unwrap().contains("layout.json"));
        // The file back with that content: nothing new, the error goes.
        write(dir.path(), &layout_json("Kept", "band"));
        assert_eq!(store.poll(), None);
        assert_eq!(store.error(), None);
        assert_eq!(store.current().0, 1);
        assert_eq!(store.backups().len(), 1);
    }

    #[test]
    fn without_a_good_backup_nothing_is_served() {
        let dir = tempfile::tempdir().unwrap();
        backup_of(dir.path(), "20260101T000000.000Z", b"not json");
        write(dir.path(), b"not json either");
        let store = store(dir.path());
        assert_eq!(store.poll(), None);
        assert!(store.current().1.is_none());
        assert!(store.error().is_some());
    }

    #[test]
    fn more_than_thirty_backups_prune_the_oldest() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let backups = dir.path().join(BACKUP_DIR);
        std::fs::create_dir_all(&backups).unwrap();
        for i in 0..MAX_BACKUPS {
            std::fs::write(
                backups.join(format!("layout.json.2020010{i:02}T000000.000Z")),
                b"old",
            )
            .unwrap();
        }
        std::fs::write(backups.join("other.txt"), b"not a backup").unwrap();
        // Neither is a backup: a name without the time, a time-like name of
        // another file.
        std::fs::write(backups.join("layout.json.bak"), b"not a backup").unwrap();
        std::fs::write(backups.join("zzZ"), b"not a backup").unwrap();
        write(dir.path(), &layout_json("FOH", "band"));
        assert_eq!(store.poll(), Some(1));
        let kept = store.backups();
        assert_eq!(kept.len(), MAX_BACKUPS);
        assert!(!kept.contains(&"layout.json.bak".to_string()));
        assert!(!kept.contains(&"zzZ".to_string()));
        assert!(backups.join("layout.json.bak").exists());
        assert!(backups.join("zzZ").exists());
        assert!(!kept.contains(&"layout.json.202001000T000000.000Z".to_string()));
        assert!(kept.contains(&"layout.json.202001001T000000.000Z".to_string()));
        assert!(kept.last().unwrap().starts_with("layout.json.20"));
        assert!(backups.join("other.txt").exists());
    }

    #[test]
    fn the_newest_backup_is_not_duplicated_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &layout_json("FOH", "band"));
        store(dir.path()).poll();
        // A restarted hub accepts the same file again: no second backup.
        assert_eq!(store(dir.path()).poll(), Some(1));
        let fresh = store(dir.path());
        fresh.poll();
        assert_eq!(fresh.backups().len(), 1);
    }

    #[test]
    fn backup_names_are_utc_milliseconds() {
        let t = UNIX_EPOCH + Duration::from_millis(1_758_707_100_123);
        assert_eq!(
            backup_name("layout.json", t),
            "layout.json.20250924T094500.123Z"
        );
    }

    #[test]
    fn markers_compose_with_the_frame_and_each_new_composition_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let frame = serde_json::to_vec(&json!({
            "schema": 2,
            "default_page": "main",
            "pages": [{"id": "main", "title": "M", "rows": [{"sections": [
                {"kind": "group", "id": "g", "tags": "A"}]}]}]
        }))
        .unwrap();
        // No frame yet: markers wait for one.
        let found = |name: &str| {
            vec![Found {
                instance: "band".into(),
                kind: fohmixer_proto::markers::TrackKind::Track,
                index: 2,
                name: name.into(),
                tuners: 1,
            }]
        };
        assert_eq!(store.set_markers(found(r#""Vox" +G:A"#)), None);
        assert_eq!(store.markers_status().found, 1);
        write(dir.path(), &frame);
        assert_eq!(store.poll(), Some(1));
        let strips = |store: &LayoutStore| store.current().1.unwrap().controls().len();
        assert_eq!(strips(&store), 1, "the markers the store held");
        assert_eq!(store.served_backups().len(), 1);
        // A new composition: a revision and a backup; the same: neither.
        assert_eq!(store.set_markers(found(r#""Other" +G:A"#)), Some(2));
        assert_eq!(store.served_backups().len(), 2);
        assert_eq!(store.set_markers(found(r#""Other" +G:A"#)), None);
        assert_eq!(store.served_backups().len(), 2);
        // A problem is listed.
        assert_eq!(store.set_markers(found(r#""X" +G:A +NOPE"#)), Some(3));
        assert_eq!(store.markers_status().problems.len(), 1);
        // The frame's backups stay the frame's.
        assert_eq!(store.backups().len(), 1);
        // A frame the markers' rules refuse is not served.
        write(
            dir.path(),
            &serde_json::to_vec(&json!({
                "schema": 2,
                "default_page": "main",
                "pages": [{"id": "main", "title": "M"}, {"id": "view-A", "title": "A", "view": true}]
            }))
            .unwrap(),
        );
        assert_eq!(store.poll(), None);
        assert!(store.error().unwrap().contains("a view page"));
        assert_eq!(store.current().0, 3);
    }

    #[test]
    fn an_unwritable_backup_folder_does_not_stop_the_layout() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(BACKUP_DIR), b"a file, not a folder").unwrap();
        let store = store(dir.path());
        write(dir.path(), &layout_json("FOH", "band"));
        assert_eq!(store.poll(), Some(1));
        assert!(store.backups().is_empty());
    }
}
