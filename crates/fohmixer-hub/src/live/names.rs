//! The layout's report of unresolved names (spec §2.5 D4, I5): the hub
//! resolves every binding of the served layout on its instance and lists the
//! ones that do not resolve — a missing or an ambiguous name — in
//! `/api/status`, so an edit that names a track wrongly shows at once, not
//! only as a red control.
//!
//! A pure state machine like the subscription table: a check of an
//! instance runs when a layout is accepted (for every connected instance)
//! and when an instance connects. Each binding target is asked for its
//! `name` (`get_prop`); a `PathError` answer is an unresolved binding, any
//! other answer means the object is there (it may just have no `name`).
//! A new layout, a new check or a disconnect makes the older answers void.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use fohmixer_proto::client::Unresolved;
use fohmixer_proto::layout::Layout;
use serde_json::{Value, json};

use super::subs::{BATCH_MAX, Outgoing};

/// The distinct binding targets of a layout, per instance, sorted.
pub fn layout_targets(layout: &Layout) -> BTreeMap<String, Vec<String>> {
    let mut targets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for binding in layout.bindings() {
        if let Ok(target) = binding.target() {
            targets
                .entry(binding.instance.clone())
                .or_default()
                .insert(target);
        }
    }
    targets
        .into_iter()
        .map(|(instance, set)| (instance, set.into_iter().collect()))
        .collect()
}

/// Why a `get_prop name` answer says its target does not resolve, if it
/// does not: a path error (missing or ambiguous name), or no answer at all.
fn unresolved(slot: Option<&Value>) -> Option<String> {
    let Some(slot) = slot else {
        return Some("no result".to_string());
    };
    if slot.get("errorType").and_then(Value::as_str) != Some("PathError") {
        return None;
    }
    Some(
        slot.get("error")
            .and_then(Value::as_str)
            .unwrap_or("does not resolve")
            .to_string(),
    )
}

/// A check of one instance in flight.
#[derive(Debug)]
struct Run {
    id: u64,
    /// Requests not answered yet.
    waiting: usize,
    found: Vec<Unresolved>,
}

/// A request of a check: its instance, run and targets (in command order).
#[derive(Debug)]
struct Asked {
    instance: String,
    run: u64,
    targets: Vec<String>,
}

/// The check.
#[derive(Debug, Default)]
pub struct NameCheck {
    targets: BTreeMap<String, Vec<String>>,
    runs: BTreeMap<String, Run>,
    inflight: HashMap<String, Asked>,
    report: BTreeMap<String, Vec<Unresolved>>,
    next: u64,
    outgoing: Vec<Outgoing>,
}

impl NameCheck {
    /// A new layout's targets: every report and check so far is void.
    pub fn set_targets(&mut self, targets: BTreeMap<String, Vec<String>>) {
        self.targets = targets;
        self.runs.clear();
        self.inflight.clear();
        self.report.clear();
    }

    /// Checks every target of `instance` (it is connected) again.
    pub fn start(&mut self, instance: &str) {
        self.next += 1;
        let id = self.next;
        let targets = self.targets.get(instance).cloned().unwrap_or_default();
        // Answers of an older check of the instance still arrive (the
        // script answers every request): the run number voids them.
        self.report.remove(instance);
        if targets.is_empty() {
            self.runs.remove(instance);
            self.report.insert(instance.to_string(), Vec::new());
            return;
        }
        let chunks: Vec<Vec<String>> = targets.chunks(BATCH_MAX).map(<[String]>::to_vec).collect();
        self.runs.insert(
            instance.to_string(),
            Run {
                id,
                waiting: chunks.len(),
                found: Vec::new(),
            },
        );
        for chunk in chunks {
            self.next += 1;
            let uuid = format!("n{}", self.next);
            let commands = chunk
                .iter()
                .map(|target| json!({"target": target, "name": "get_prop", "args": {"prop": "name"}}))
                .collect();
            self.inflight.insert(
                uuid.clone(),
                Asked {
                    instance: instance.to_string(),
                    run: id,
                    targets: chunk,
                },
            );
            self.outgoing.push(Outgoing {
                instance: instance.to_string(),
                uuid,
                commands,
            });
        }
    }

    /// `instance` disconnected: its report and check are void.
    pub fn stop(&mut self, instance: &str) {
        self.runs.remove(instance);
        self.report.remove(instance);
        self.inflight.retain(|_, asked| asked.instance != instance);
    }

    /// An answer; false when `uuid` is not a check's request.
    pub fn on_result(&mut self, instance: &str, uuid: &str, slots: &[Value]) -> bool {
        let Some(asked) = self.inflight.remove(uuid) else {
            return false;
        };
        let current = self
            .runs
            .get_mut(instance)
            .filter(|run| run.id == asked.run && asked.instance == instance);
        let Some(run) = current else {
            return true;
        };
        for (i, target) in asked.targets.iter().enumerate() {
            if let Some(error) = unresolved(slots.get(i)) {
                run.found.push(Unresolved {
                    instance: asked.instance.clone(),
                    target: target.clone(),
                    error,
                });
            }
        }
        run.waiting -= 1;
        if run.waiting == 0 {
            let done = self.runs.remove(instance).expect("present above");
            tracing::info!(
                instance,
                unresolved = done.found.len(),
                "the layout's bindings are checked (the unresolved ones are in /api/status)"
            );
            self.report.insert(asked.instance, done.found);
        }
        true
    }

    /// The requests to send now.
    pub fn drain_outgoing(&mut self) -> Vec<Outgoing> {
        std::mem::take(&mut self.outgoing)
    }

    /// The unresolved bindings of every finished check, by instance and
    /// target.
    pub fn unresolved(&self) -> Vec<Unresolved> {
        self.report.values().flatten().cloned().collect()
    }

    /// Whether the check of `instance` has finished (tests, diagnostics).
    pub fn is_checked(&self, instance: &str) -> bool {
        self.report.contains_key(instance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(band: &[&str], master: &[&str]) -> BTreeMap<String, Vec<String>> {
        let list = |t: &[&str]| t.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        BTreeMap::from([
            ("band".to_string(), list(band)),
            ("master".to_string(), list(master)),
        ])
    }

    fn fine() -> Value {
        json!({"ok": true, "data": "Hand1 #"})
    }

    fn missing(step: &str) -> Value {
        json!({"ok": false, "error": format!("not found: {step}"), "errorType": "PathError"})
    }

    fn unresolved_of(instance: &str, target: &str, error: &str) -> Unresolved {
        Unresolved {
            instance: instance.into(),
            target: target.into(),
            error: error.into(),
        }
    }

    #[test]
    fn a_check_asks_every_target_its_name_and_lists_the_path_errors() {
        let mut check = NameCheck::default();
        check.set_targets(targets(
            &[
                "live_set tracks[name=A]",
                "live_set tracks[name=B]",
                "live_set",
            ],
            &["live_set tracks[name=C]"],
        ));
        check.start("band");
        let out = check.drain_outgoing();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].instance, "band");
        assert_eq!(
            out[0].commands[1],
            json!({"target": "live_set tracks[name=B]", "name": "get_prop", "args": {"prop": "name"}})
        );
        assert!(check.drain_outgoing().is_empty(), "sent once");
        assert!(!check.is_checked("band"));
        // B is missing; `live_set` has no `name` (an error of another kind:
        // the object is there).
        let slots = vec![
            fine(),
            missing("tracks[name=B]"),
            json!({"ok": false, "error": "no such property: name", "errorType": "OpError"}),
        ];
        assert!(check.on_result("band", &out[0].uuid, &slots));
        assert!(
            !check.on_result("band", &out[0].uuid, &slots),
            "answered once"
        );
        assert!(check.is_checked("band"));
        assert!(!check.is_checked("master"), "master was not checked");
        assert_eq!(
            check.unresolved(),
            vec![unresolved_of(
                "band",
                "live_set tracks[name=B]",
                "not found: tracks[name=B]"
            )]
        );
        assert!(!check.on_result("band", "s1", &[]), "not a check's request");
    }

    #[test]
    fn a_long_list_is_checked_in_batches_and_reported_when_all_answered() {
        let mut check = NameCheck::default();
        let many: Vec<String> = (0..(BATCH_MAX + 2))
            .map(|i| format!("live_set tracks {i}"))
            .collect();
        check.set_targets(BTreeMap::from([("band".to_string(), many)]));
        check.start("band");
        let out = check.drain_outgoing();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].commands.len(), BATCH_MAX);
        assert_eq!(out[1].commands.len(), 2);
        assert_ne!(out[0].uuid, out[1].uuid);
        // The second batch was refused (no slots): its targets are listed.
        check.on_result("band", &out[1].uuid, &[]);
        assert!(!check.is_checked("band"), "one batch still waits");
        let all_fine: Vec<Value> = (0..BATCH_MAX).map(|_| fine()).collect();
        check.on_result("band", &out[0].uuid, &all_fine);
        assert!(check.is_checked("band"));
        let listed: Vec<String> = check
            .unresolved()
            .into_iter()
            .map(|u| format!("{} {}", u.target, u.error))
            .collect();
        assert_eq!(listed.len(), 2, "{listed:?}");
        assert!(
            listed.iter().all(|l| l.ends_with(" no result")),
            "{listed:?}"
        );
    }

    #[test]
    fn a_new_check_a_new_layout_or_a_disconnect_voids_older_answers() {
        let mut check = NameCheck::default();
        check.set_targets(targets(
            &["live_set tracks[name=A]"],
            &["live_set tracks[name=C]"],
        ));
        check.start("band");
        let first = check.drain_outgoing();
        check.start("band");
        let second = check.drain_outgoing();
        // The first check's answer is void: the report waits for the second.
        assert!(check.on_result("band", &first[0].uuid, &[missing("tracks[name=A]")]));
        assert!(!check.is_checked("band"));
        check.on_result("band", &second[0].uuid, &[fine()]);
        assert!(check.is_checked("band"));
        assert_eq!(check.unresolved(), vec![]);
        // A disconnect forgets the instance's report and its check in flight.
        check.start("master");
        let master = check.drain_outgoing();
        check.stop("band");
        assert!(!check.is_checked("band"));
        check.stop("master");
        assert!(!check.on_result("master", &master[0].uuid, &[missing("tracks[name=C]")]));
        assert!(!check.is_checked("master"));
        // An answer from another instance than the one asked is void.
        check.start("master");
        let asked = check.drain_outgoing();
        assert!(check.on_result("band", &asked[0].uuid, &[missing("tracks[name=C]")]));
        assert!(!check.is_checked("master"));
        // A new layout voids everything.
        check.start("master");
        let before = check.drain_outgoing();
        check.set_targets(targets(&[], &["live_set tracks[name=D]"]));
        assert!(!check.on_result("master", &before[0].uuid, &[fine()]));
        assert!(!check.is_checked("master"));
    }

    #[test]
    fn an_instance_without_bindings_is_checked_at_once() {
        let mut check = NameCheck::default();
        check.set_targets(targets(&[], &["live_set tracks[name=C]"]));
        check.start("band");
        assert!(check.drain_outgoing().is_empty());
        assert!(check.is_checked("band"));
        check.start("drums");
        assert!(check.is_checked("drums"));
        assert_eq!(check.unresolved(), vec![]);
    }

    #[test]
    fn answers_read_as_resolved_or_not() {
        assert_eq!(unresolved(Some(&fine())), None);
        assert_eq!(
            unresolved(Some(&missing("tracks[name=X]"))),
            Some("not found: tracks[name=X]".into())
        );
        assert_eq!(
            unresolved(Some(&json!({"ok": false, "errorType": "PathError"}))),
            Some("does not resolve".into())
        );
        assert_eq!(
            unresolved(Some(
                &json!({"ok": false, "error": "live error", "errorType": "OpError"})
            )),
            None
        );
        assert_eq!(unresolved(None), Some("no result".into()));
    }

    #[test]
    fn layout_targets_are_distinct_per_instance() {
        let layout: Layout = serde_json::from_value(json!({
            "schema": 1,
            "canvas": {"w": 2360, "h": 1640},
            "pages": [{"id": "main", "title": "FOH", "items": [
                {"kind": "solo", "frame": {"x": 10, "y": 100, "w": 100, "h": 60},
                 "binding": {"instance": "band", "anchor": {"kind": "track", "name": "B"}}},
                {"kind": "solo", "frame": {"x": 10, "y": 200, "w": 100, "h": 60},
                 "binding": {"instance": "band", "anchor": {"kind": "track", "name": "A"}}},
                {"kind": "solo", "frame": {"x": 10, "y": 300, "w": 100, "h": 60},
                 "binding": {"instance": "band", "anchor": {"kind": "track", "name": "B"}}},
                {"kind": "solo", "frame": {"x": 10, "y": 400, "w": 100, "h": 60},
                 "binding": {"instance": "master", "anchor": {"kind": "master"}}}
            ]}]
        }))
        .unwrap();
        assert_eq!(
            layout_targets(&layout),
            BTreeMap::from([
                (
                    "band".to_string(),
                    vec![
                        "live_set tracks[name=A]".to_string(),
                        "live_set tracks[name=B]".to_string()
                    ]
                ),
                (
                    "master".to_string(),
                    vec!["live_set master_track".to_string()]
                ),
            ])
        );
    }
}
