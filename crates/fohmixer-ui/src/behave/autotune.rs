//! AUTOTUNE on the surface (#140 PR 3c; the approved mockup
//! `docs/mockups/autotune-v2.html`, the owner's approval of 2026-10-23):
//! the rail button's look and the singers' panel's model. Pure.
//!
//! Two states only (spec §2.2): AUT on, the hub's rule switches each
//! singer's tracks; AUT off, every singer is on DRY. A 1 s hold switches
//! AUT at its lift (the press is `behave::rail`'s, as the merged STAGE
//! button's); a tap opens the singers' panel and sends nothing. The button
//! is lit in the former toggle's orange while every singer tunes, dark
//! while none does, and mixed (dark, an orange ring, half its light) while
//! some do and some do not (off by hand, a mismatch). With AUT on it wears
//! the `AUT` pill, and its second line says the state and the key:
//! `off · F`, `stopped · F`, `tuning · F`, `no song`, `no key`,
//! `checking · F`, `tuning · F · 1 off`, `tuning · F · 1 !`. A singer whose
//! key does not read back, or a problem in the song map, shows the yellow
//! `!` corner; with no target found the button is red. The panel lists
//! each singer's stream and PA chip, its key and its manual switch, its
//! head the song and the key, its foot the song map's counts and Live's
//! bar. Everything here reads the hub's `AutotuneState`.

use fohmixer_proto::client::{AutotuneState, SingerState, TargetState, TrackState};

use super::hold::{Hold, Lift};
use super::rail::{AUT_BADGE, Aut, Fill, RAIL_HOLD_MS, RailLook, full_text, hold_text};

/// The key in force as the button and the panel name it: `F`, `F#m`, or
/// `—` outside a song or in a keyless one (spec §9).
fn key_text(state: &AutotuneState) -> &str {
    state.key.as_deref().unwrap_or("—")
}

/// Whether a singer's track tunes (its selector on the autotune chain).
fn tunes(track: TrackState) -> bool {
    track == TrackState::Tune
}

/// Whether every track a singer has tunes, and it has one.
fn singer_tunes(singer: &SingerState) -> bool {
    let tracks: Vec<TrackState> = [singer.stream, singer.pa]
        .into_iter()
        .filter(|t| *t != TrackState::None)
        .collect();
    !tracks.is_empty() && tracks.into_iter().all(tunes)
}

/// What the singers' tracks show: every singer tuning (lit), some track
/// tuning but not every singer (mixed), or none tuning (dark).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    On,
    Mixed,
    Off,
}

/// What the singers' tracks show (Live's selectors, as the hub tells).
pub fn shown(state: &AutotuneState) -> Shown {
    let any = state.singers.iter().any(|s| tunes(s.stream) || tunes(s.pa));
    let every = !state.singers.is_empty() && state.singers.iter().all(singer_tunes);
    match (every, any) {
        (true, _) => Shown::On,
        (false, true) => Shown::Mixed,
        (false, false) => Shown::Off,
    }
}

/// Whether the button is ready: none before the hub's state, and while
/// every target still waits (its walk, its value, its Live set offline);
/// `Some(false)` without a found target: no `+AUTOTUNE` track, or none
/// with its rack (red, I5).
pub fn autotune_ready(state: Option<&AutotuneState>) -> Option<bool> {
    let s = state?;
    let found = s
        .targets
        .iter()
        .any(|t| matches!(t.state, TargetState::On | TargetState::Off));
    let waits = s.targets.iter().any(|t| t.state == TargetState::Waiting);
    match (found, waits) {
        (true, _) => Some(true),
        (false, true) => None,
        (false, false) => Some(false),
    }
}

/// The button's `data-state`: `on`, `off`, `mixed`, or `waiting` before
/// the hub's state and while it is not ready either way.
pub fn state_name(state: Option<&AutotuneState>) -> &'static str {
    match state.filter(|s| autotune_ready(Some(*s)).is_some()) {
        None => "waiting",
        Some(s) => match shown(s) {
            Shown::On => "on",
            Shown::Mixed => "mixed",
            Shown::Off => "off",
        },
    }
}

/// The yellow `!` corner: a singer whose key or target is not right (the
/// hub's `key_ok`), or a problem in the song map.
pub fn has_problem(state: Option<&AutotuneState>) -> bool {
    state.is_some_and(|s| s.map_problems > 0 || s.singers.iter().any(|x| !x.key_ok))
}

/// The singers off by hand.
fn off_count(state: &AutotuneState) -> usize {
    state.singers.iter().filter(|s| s.off).count()
}

/// The singers with a mismatch or a target problem (`!`).
fn bad_count(state: &AutotuneState) -> usize {
    state.singers.iter().filter(|s| !s.key_ok).count()
}

/// The second line with AUT on while Live plays: the state, the key and
/// what keeps a singer from tuning. A mismatch (`1 !`) is named before a
/// singer off by hand (`1 off`): one of them fits the 98 px. With no track
/// tuning while a singer is not right (the hub keeps it DRY for a reason of
/// its own or a key's: PR #150 review I2), `waiting · F`, never `tuning`.
fn playing_line(state: &AutotuneState) -> String {
    let key = key_text(state);
    match (state.song, state.key.as_deref()) {
        (None, _) => return "no song".to_string(),
        (Some(_), None) => return "no key".to_string(),
        (Some(_), Some(_)) => {}
    }
    if state.checking {
        return format!("checking · {key}");
    }
    let none_tunes = shown(state) == Shown::Off;
    match (bad_count(state), off_count(state)) {
        (bad, _) if bad > 0 && none_tunes => format!("waiting · {key}"),
        (0, 0) => format!("tuning · {key}"),
        (0, off) => format!("tuning · {key} · {off} off"),
        (bad, _) => format!("tuning · {key} · {bad} !"),
    }
}

/// The second line: AUT off `off · F`; AUT on, Live stopped (or its
/// transport not heard yet) `stopped · F`, else the playing line.
pub fn second_line(state: &AutotuneState) -> String {
    let key = key_text(state);
    match (state.aut, state.playing) {
        (false, _) => format!("off · {key}"),
        (true, Some(true)) => playing_line(state),
        (true, _) => format!("stopped · {key}"),
    }
}

/// The AUTOTUNE button (mockup v2, screen 1): lit while every singer tunes,
/// mixed while some do, dark else; the pill runs while AUT is on; the
/// second line is [`second_line`], or a press's fill
/// (`behave::rail::fill_of`) shows its own line.
pub fn autotune_look(state: Option<&AutotuneState>, fill: Fill) -> RailLook {
    let Some(s) = state else {
        return RailLook {
            lit: false,
            aut: Aut::Off,
            sub: None,
            low: false,
            mixed: false,
        };
    };
    let shows = shown(s);
    let sub = match fill {
        Fill::Filling => hold_text(s.aut).to_string(),
        Fill::Full => full_text(s.aut).to_string(),
        Fill::None => second_line(s),
    };
    RailLook {
        lit: shows == Shown::On,
        aut: if s.aut { Aut::Run } else { Aut::Off },
        sub: Some(sub),
        low: false,
        mixed: shows == Shown::Mixed,
    }
}

/// Whether a tap opens the singers' panel: only on the state the hub told
/// this connection (`fresh`) and while the page is connected (Review Focus
/// 5: after a reconnect it shows no stale singer); else it is refused. A
/// tap sends nothing either way.
pub fn tap_opens_panel(fresh: Option<&AutotuneState>, connected: bool) -> bool {
    fresh.is_some() && connected
}

/// A chip of a singer's track: its text and whether it is lit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chip {
    pub text: &'static str,
    pub on: bool,
}

/// The chip of a track (`stream TUNE`, `PA DRY`, `stream —`).
fn chip(track: TrackState, stream: bool) -> Chip {
    let text = match (stream, track) {
        (true, TrackState::Tune) => "stream TUNE",
        (true, TrackState::Dry) => "stream DRY",
        (true, TrackState::None) => "stream —",
        (false, TrackState::Tune) => "PA TUNE",
        (false, TrackState::Dry) => "PA DRY",
        (false, TrackState::None) => "PA —",
    };
    Chip {
        text,
        on: tunes(track),
    }
}

/// One singer in the panel (mockup v2, screen 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelRow {
    /// The singer's id (`set_hub` `autotune_singer`); empty for a target
    /// that names no singer.
    pub id: String,
    pub label: String,
    pub stream: Chip,
    pub pa: Chip,
    /// `F ✓`, `—` without a key in force, or `!` when not right.
    pub key: String,
    pub ok: bool,
    /// Off by hand (the row dimmed).
    pub off: bool,
    /// What a tap on its switch does: `OFF` while on, `ON` while off by
    /// hand; none for a singer with no id (no manual off, PR #147 N6).
    pub switch: Option<&'static str>,
    /// The line under a row that is not right.
    pub why: Option<String>,
}

/// A row's key in the panel's keyed list (PR #150 review N2): its id and
/// label (a target that names no singer has no id, so the label tells
/// such rows apart).
pub type RowKey = (String, String);

impl PanelRow {
    /// Its key in the panel's keyed list.
    pub fn key(&self) -> RowKey {
        (self.id.clone(), self.label.clone())
    }
}

/// The panel's rows, one per singer in Live's order.
pub fn panel_rows(state: &AutotuneState) -> Vec<PanelRow> {
    state
        .singers
        .iter()
        .map(|s| {
            // The ✓ only for a key written and read back (PR #150 review
            // M4); one on its way shows the pending mark.
            let key = match (s.key_ok, state.key.as_deref(), s.confirmed) {
                (false, _, _) => "!".to_string(),
                (true, Some(key), true) => format!("{key} ✓"),
                (true, Some(key), false) => format!("{key} …"),
                (true, None, _) => "—".to_string(),
            };
            let why = (!s.key_ok).then(|| {
                let why = s.why.as_deref().unwrap_or("key not confirmed");
                format!("{}: {why} · on DRY, rewrite at the next change", s.label)
            });
            PanelRow {
                id: s.id.clone(),
                label: s.label.clone(),
                stream: chip(s.stream, true),
                pa: chip(s.pa, false),
                key,
                ok: s.key_ok,
                off: s.off,
                switch: (!s.id.is_empty()).then_some(if s.off { "ON" } else { "OFF" }),
                why,
            }
        })
        .collect()
}

/// The `set_hub` `autotune_singer` value a row's switch sends: its singer
/// off by hand, or back on.
pub fn switch_value(row: &PanelRow) -> (String, bool) {
    (row.id.clone(), !row.off)
}

/// The panel's head after its title: the song and the key in force
/// (`song 031 · F`) and, while the song has a key change to come, that key
/// and its clip (`song 031 · F → G at Key {G}`, mockup v2; PR #150 review
/// K1); `no song` outside every song.
pub fn panel_head(state: &AutotuneState) -> String {
    let Some(song) = state.song else {
        return "no song".to_string();
    };
    let now = format!("song {song:03} · {}", key_text(state));
    match &state.change {
        Some(change) => format!("{now} → {} at {}", change.key, change.clip),
        None => now,
    }
}

/// `n` and the word for one or more.
fn counted(n: usize, one: &str, more: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {more}")
    }
}

/// The panel's foot: the song map's counts, then Live's bar and the hold's
/// help (the pill's word is the shared badge's, #141 renames it).
pub fn panel_foot(state: &AutotuneState) -> [String; 2] {
    let map = format!(
        "Song map: {}, {}, {}.",
        counted(state.map_keyed, "song with a key", "songs with a key"),
        counted(state.map_changes, "key change", "key changes"),
        counted(state.map_problems, "problem", "problems"),
    );
    // The first problems in the hub's words (PR #150 review K2, M3: the
    // mockup's `1 problem (088: no key)`), more of them marked.
    let map = match state.map_texts.as_slice() {
        [] => map,
        texts => {
            let more = if state.map_problems > texts.len() {
                "; …"
            } else {
                ""
            };
            let listed = texts.join("; ");
            format!("{} ({listed}{more}).", map.trim_end_matches('.'))
        }
    };
    let bar = state.bar.as_deref().unwrap_or("—");
    let live = format!("Live: {bar}. Hold AUTOTUNE 1 s: {AUT_BADGE} on/off.");
    [map, live]
}

/// The gap between the panel and the column, and the screen's edges (px).
pub const PANEL_GAP: f64 = 8.0;

/// The widest screen that is a phone held upright (page.css's
/// `(max-width: 520px)`): the panel at its full width there.
pub const UPRIGHT_MAX: f64 = 520.0;

/// Where the panel opens from, on the screen: the column's right edge and
/// the AUTOTUNE button's top and bottom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    pub column_right: f64,
    pub button_top: f64,
    pub button_bottom: f64,
}

/// Where the panel goes: its left, its top and the most it may be high
/// (it scrolls past that).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub left: f64,
    pub top: f64,
    pub max_h: f64,
}

/// Where the panel opens for its `height` on a screen `view_w` × `view_h`
/// (PR #150 review N3: placed again at every resize). Beside the column,
/// its top at the button's but inside the screen. On a phone held upright
/// it is the screen's width (the stylesheet) and never covers the button:
/// below it when it fits there or that side is the larger, else above it.
pub fn panel_place(anchor: Anchor, view_w: f64, view_h: f64, height: f64) -> Place {
    if view_w <= UPRIGHT_MAX {
        let below = view_h - anchor.button_bottom - 2.0 * PANEL_GAP;
        let above = anchor.button_top - 2.0 * PANEL_GAP;
        if height <= below || below >= above {
            return Place {
                left: PANEL_GAP,
                top: anchor.button_bottom + PANEL_GAP,
                max_h: below,
            };
        }
        let high = height.min(above);
        return Place {
            left: PANEL_GAP,
            top: anchor.button_top - PANEL_GAP - high,
            max_h: above,
        };
    }
    let top = anchor
        .button_top
        .min(view_h - height - PANEL_GAP)
        .max(PANEL_GAP);
    Place {
        left: anchor.column_right + PANEL_GAP,
        top,
        max_h: view_h - top - PANEL_GAP,
    }
}

/// Whether a down closes the open panel and is taken for that alone (PR
/// #150 review I1): one outside the panel and the AUTOTUNE button, while it
/// is open. It reaches nothing under it (a strip's mute acts on its down).
pub fn dismisses(open: bool, on_button: bool, on_panel: bool) -> bool {
    open && !on_button && !on_panel
}

/// A singer's switch's press (PR #150 review M1): it acts at the lift of a
/// tap (a slide past 10 px, a cancel or a lost capture ends it with nothing
/// done), never at its down; a long press acts at its lift too.
pub fn switch_press() -> Hold {
    Hold::lifted(RAIL_HOLD_MS)
}

/// Whether a switch's lift acts.
pub fn switch_acts(lift: Lift) -> bool {
    matches!(lift, Lift::Tap | Lift::Open)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::client::AutotuneTargetState;
    use fohmixer_proto::client::TrackState::{Dry, None as NoTrack, Tune};

    fn singer(id: &str, stream: TrackState, pa: TrackState) -> SingerState {
        SingerState {
            id: id.to_string(),
            label: format!("Vox {}", id.trim_start_matches("VOX")),
            stream,
            pa,
            off: false,
            key_ok: true,
            why: None,
            confirmed: true,
        }
    }

    fn off(mut s: SingerState) -> SingerState {
        s.off = true;
        s
    }

    fn bad(mut s: SingerState, why: &str) -> SingerState {
        s.key_ok = false;
        s.why = Some(why.to_string());
        s
    }

    /// A state with one found target, AUT, the transport, song 31 in `key`
    /// and these singers.
    fn state(
        aut: bool,
        playing: Option<bool>,
        key: Option<&str>,
        singers: Vec<SingerState>,
    ) -> AutotuneState {
        AutotuneState {
            aut,
            playing,
            targets: vec![AutotuneTargetState {
                label: "Vox 1".into(),
                state: TargetState::Off,
            }],
            song: Some(31),
            key: key.map(str::to_string),
            checking: false,
            singers,
            map_problems: 0,
            map_changes: 0,
            map_keyed: 0,
            bar: None,
            change: None,
            map_texts: Vec::new(),
        }
    }

    fn two_tuning() -> Vec<SingerState> {
        vec![singer("VOX1", Tune, Tune), singer("VOX2", Tune, Tune)]
    }

    fn line(s: &AutotuneState) -> String {
        autotune_look(Some(s), Fill::None).sub.unwrap()
    }

    #[test]
    fn aut_off_reads_off_and_the_key() {
        let s = state(false, Some(true), Some("F"), vec![singer("VOX1", Dry, Dry)]);
        assert_eq!(line(&s), "off · F");
        let look = autotune_look(Some(&s), Fill::None);
        assert_eq!((look.lit, look.mixed, look.aut), (false, false, Aut::Off));
        let mut none = s.clone();
        none.key = None;
        assert_eq!(line(&none), "off · —");
        // AUT off whatever the transport.
        for playing in [None, Some(false)] {
            let mut st = s.clone();
            st.playing = playing;
            assert_eq!(line(&st), "off · F");
        }
    }

    #[test]
    fn aut_on_and_stopped_reads_stopped_and_the_key() {
        let s = state(true, Some(false), Some("F"), vec![singer("VOX1", Dry, Dry)]);
        assert_eq!(line(&s), "stopped · F");
        let look = autotune_look(Some(&s), Fill::None);
        assert_eq!((look.aut, look.lit), (Aut::Run, false));
        // The transport not heard yet: as stopped (the hub decides `stop`).
        let mut unheard = s.clone();
        unheard.playing = None;
        assert_eq!(line(&unheard), "stopped · F");
        let mut keyless = s;
        keyless.key = None;
        assert_eq!(line(&keyless), "stopped · —");
    }

    #[test]
    fn aut_on_and_playing_in_a_keyed_song_reads_tuning_and_is_lit() {
        let s = state(true, Some(true), Some("F"), two_tuning());
        assert_eq!(line(&s), "tuning · F");
        let look = autotune_look(Some(&s), Fill::None);
        assert_eq!(
            look,
            RailLook {
                lit: true,
                aut: Aut::Run,
                sub: Some("tuning · F".into()),
                low: false,
                mixed: false,
            }
        );
        let g = state(true, Some(true), Some("G"), two_tuning());
        assert_eq!(line(&g), "tuning · G");
    }

    #[test]
    fn out_of_every_song_reads_no_song_and_a_keyless_one_no_key() {
        let mut s = state(true, Some(true), None, vec![singer("VOX1", Dry, Dry)]);
        s.song = None;
        assert_eq!(line(&s), "no song");
        s.song = Some(88);
        assert_eq!(line(&s), "no key");
        // A song with a key out of song? None: the song decides first.
        s.song = None;
        s.key = Some("F".into());
        assert_eq!(line(&s), "no song");
    }

    #[test]
    fn a_key_being_checked_reads_checking() {
        let mut s = state(true, Some(true), Some("F"), vec![singer("VOX1", Dry, Dry)]);
        s.checking = true;
        assert_eq!(line(&s), "checking · F");
        // Checking only while playing in a keyed song.
        s.playing = Some(false);
        assert_eq!(line(&s), "stopped · F");
    }

    #[test]
    fn a_singer_off_by_hand_is_counted_and_the_button_mixed() {
        let s = state(
            true,
            Some(true),
            Some("F"),
            vec![singer("VOX1", Tune, Tune), off(singer("VOX2", Dry, Dry))],
        );
        assert_eq!(line(&s), "tuning · F · 1 off");
        let look = autotune_look(Some(&s), Fill::None);
        assert_eq!((look.lit, look.mixed), (false, true));
        let two = state(
            true,
            Some(true),
            Some("F"),
            vec![
                singer("VOX1", Tune, Tune),
                off(singer("VOX2", Dry, Dry)),
                off(singer("VOX3", Dry, Dry)),
            ],
        );
        assert_eq!(line(&two), "tuning · F · 2 off");
    }

    #[test]
    fn a_mismatch_is_counted_before_a_singer_off_and_shows_the_corner() {
        let s = state(
            true,
            Some(true),
            Some("F"),
            vec![
                singer("VOX1", Tune, Tune),
                bad(singer("VOX5", Dry, Tune), "stream Key reads G, wrote F"),
            ],
        );
        assert_eq!(line(&s), "tuning · F · 1 !");
        assert!(has_problem(Some(&s)));
        let look = autotune_look(Some(&s), Fill::None);
        assert_eq!((look.lit, look.mixed), (false, true));
        let both = state(
            true,
            Some(true),
            Some("F"),
            vec![
                bad(singer("VOX5", Dry, Tune), "w"),
                off(singer("VOX3", Dry, Dry)),
                bad(singer("VOX6", Dry, Dry), "w"),
            ],
        );
        assert_eq!(line(&both), "tuning · F · 2 !");
    }

    #[test]
    fn every_track_dry_for_the_hubs_own_reason_reads_waiting_not_tuning() {
        // PR #150 review I2: the fall back not kept, a hold, a rack not
        // found again: no track tunes and a singer is not right.
        let held = bad(
            singer("VOX1", Dry, Dry),
            "PA: its fall back to DRY is not kept yet",
        );
        let s = state(true, Some(true), Some("F"), vec![held.clone()]);
        assert_eq!(line(&s), "waiting · F");
        assert!(has_problem(Some(&s)), "the corner");
        let look = autotune_look(Some(&s), Fill::None);
        assert_eq!((look.lit, look.mixed), (false, false));
        // A track that tunes beside it: tuning, the `!` counted.
        let two = state(
            true,
            Some(true),
            Some("F"),
            vec![held, singer("VOX2", Tune, Tune)],
        );
        assert_eq!(line(&two), "tuning · F · 1 !");
        // Every singer off by hand: the singers' own choice, not waiting.
        let off_all = state(
            true,
            Some(true),
            Some("F"),
            vec![off(singer("VOX1", Dry, Dry))],
        );
        assert_eq!(line(&off_all), "tuning · F · 1 off");
    }

    #[test]
    fn a_key_is_ticked_only_once_written_and_read_back() {
        // PR #150 review M4: a key on its way shows the pending mark.
        let mut pending = singer("VOX1", Dry, Dry);
        pending.confirmed = false;
        let s = state(
            true,
            Some(false),
            Some("F"),
            vec![pending, singer("VOX2", Dry, Dry)],
        );
        let rows = panel_rows(&s);
        assert_eq!(rows[0].key, "F …");
        assert!(rows[0].ok);
        assert_eq!(rows[1].key, "F ✓");
    }

    #[test]
    fn lit_mixed_and_dark_follow_the_singers_tracks() {
        let on = |singers| shown(&state(true, Some(true), Some("F"), singers));
        assert_eq!(on(two_tuning()), Shown::On);
        // A singer without a stream track tunes on its PA alone.
        assert_eq!(
            on(vec![
                singer("VOX1", NoTrack, Tune),
                singer("VOX2", Tune, Tune)
            ]),
            Shown::On
        );
        assert_eq!(
            on(vec![singer("VOX1", Tune, Dry), singer("VOX2", Tune, Tune)]),
            Shown::Mixed
        );
        assert_eq!(
            on(vec![singer("VOX1", Dry, Tune), singer("VOX2", Dry, Dry)]),
            Shown::Mixed
        );
        assert_eq!(
            on(vec![singer("VOX1", Dry, Dry), singer("VOX2", NoTrack, Dry)]),
            Shown::Off
        );
        // A singer with no track never counts as tuning.
        assert_eq!(on(vec![singer("VOX1", NoTrack, NoTrack)]), Shown::Off);
        assert_eq!(
            on(vec![
                singer("VOX1", NoTrack, NoTrack),
                singer("VOX2", Tune, Tune)
            ]),
            Shown::Mixed
        );
        assert_eq!(on(Vec::new()), Shown::Off);
        // `data-state`.
        let names = |singers| state_name(Some(&state(true, Some(true), Some("F"), singers)));
        assert_eq!(names(two_tuning()), "on");
        assert_eq!(
            names(vec![singer("VOX1", Tune, Dry), singer("VOX2", Tune, Tune)]),
            "mixed"
        );
        assert_eq!(names(vec![singer("VOX1", Dry, Dry)]), "off");
        assert_eq!(state_name(None), "waiting");
    }

    #[test]
    fn it_is_waiting_before_the_state_and_red_without_a_found_target() {
        let with = |states: &[TargetState]| {
            let mut s = state(false, None, None, Vec::new());
            s.targets = states
                .iter()
                .map(|t| AutotuneTargetState {
                    label: "x".into(),
                    state: *t,
                })
                .collect();
            s
        };
        use TargetState::{Missing, Off, On, Waiting};
        assert_eq!(autotune_ready(None), None);
        assert_eq!(autotune_ready(Some(&with(&[]))), Some(false));
        assert_eq!(
            autotune_ready(Some(&with(&[Missing, Missing]))),
            Some(false)
        );
        assert_eq!(autotune_ready(Some(&with(&[Waiting, Missing]))), None);
        assert_eq!(autotune_ready(Some(&with(&[Waiting]))), None);
        assert_eq!(autotune_ready(Some(&with(&[On]))), Some(true));
        assert_eq!(autotune_ready(Some(&with(&[Off, Missing]))), Some(true));
        assert_eq!(autotune_ready(Some(&with(&[Waiting, Off]))), Some(true));
        assert_eq!(state_name(Some(&with(&[Waiting]))), "waiting");
        assert_eq!(state_name(Some(&with(&[Missing]))), "off", "red: dark");
        // Before the hub's state: dark, nothing.
        assert_eq!(
            autotune_look(None, Fill::None),
            RailLook {
                lit: false,
                aut: Aut::Off,
                sub: None,
                low: false,
                mixed: false,
            }
        );
        assert_eq!(autotune_look(None, Fill::Full).sub, None);
    }

    #[test]
    fn the_corner_shows_a_bad_key_or_a_map_problem() {
        let mut s = state(true, Some(true), Some("F"), two_tuning());
        assert!(!has_problem(Some(&s)));
        s.map_problems = 1;
        assert!(has_problem(Some(&s)), "a song map problem");
        // Lit with the corner (the mockup's cell 12).
        let look = autotune_look(Some(&s), Fill::None);
        assert!(look.lit);
        s.map_problems = 0;
        s.singers[1].key_ok = false;
        assert!(has_problem(Some(&s)));
        assert!(!has_problem(None));
    }

    #[test]
    fn a_press_shows_the_shared_hold_and_full_lines() {
        let on = state(true, Some(true), Some("F"), two_tuning());
        let off_state = state(false, Some(true), Some("F"), two_tuning());
        let sub = |s: &AutotuneState, fill| autotune_look(Some(s), fill).sub.unwrap();
        assert_eq!(sub(&on, Fill::Filling), hold_text(true));
        assert_eq!(sub(&off_state, Fill::Filling), hold_text(false));
        assert_eq!(sub(&on, Fill::Full), full_text(true));
        assert_eq!(sub(&off_state, Fill::Full), full_text(false));
        // The light and the pill stay.
        let held = autotune_look(Some(&on), Fill::Full);
        assert_eq!((held.lit, held.aut), (true, Aut::Run));
    }

    #[test]
    fn a_tap_opens_the_panel_only_on_this_connections_state_while_online() {
        let s = state(true, Some(true), Some("F"), two_tuning());
        assert!(tap_opens_panel(Some(&s), true));
        assert!(!tap_opens_panel(Some(&s), false), "not connected");
        assert!(!tap_opens_panel(None, true), "no state on this connection");
        assert!(!tap_opens_panel(None, false));
    }

    #[test]
    fn panel_rows_carry_every_chip_the_key_and_the_switch() {
        let s = state(
            true,
            Some(true),
            Some("F"),
            vec![
                singer("VOX1", Tune, Tune),
                off(singer("VOX3", Dry, Dry)),
                singer("VOX4", NoTrack, Tune),
                bad(singer("VOX5", Dry, Tune), "stream Key reads G, wrote F"),
                singer("VOX7", Tune, NoTrack),
            ],
        );
        let rows = panel_rows(&s);
        let chips: Vec<(&str, bool, &str, bool)> = rows
            .iter()
            .map(|r| (r.stream.text, r.stream.on, r.pa.text, r.pa.on))
            .collect();
        assert_eq!(
            chips,
            vec![
                ("stream TUNE", true, "PA TUNE", true),
                ("stream DRY", false, "PA DRY", false),
                ("stream —", false, "PA TUNE", true),
                ("stream DRY", false, "PA TUNE", true),
                ("stream TUNE", true, "PA —", false),
            ]
        );
        assert_eq!(
            rows[0],
            PanelRow {
                id: "VOX1".into(),
                label: "Vox 1".into(),
                stream: Chip {
                    text: "stream TUNE",
                    on: true
                },
                pa: Chip {
                    text: "PA TUNE",
                    on: true
                },
                key: "F ✓".into(),
                ok: true,
                off: false,
                switch: Some("OFF"),
                why: None,
            }
        );
        assert_eq!((rows[1].off, rows[1].switch), (true, Some("ON")));
        assert_eq!(rows[3].key, "!");
        assert!(!rows[3].ok);
        assert_eq!(
            rows[3].why.as_deref(),
            Some("Vox 5: stream Key reads G, wrote F · on DRY, rewrite at the next change")
        );
        assert_eq!(rows[4].why, None);
        // Each row's key: its id and label.
        assert_eq!(rows[1].key(), ("VOX3".to_string(), "Vox 3".to_string()));
        // The switch's value: off by hand, or back on.
        assert_eq!(switch_value(&rows[0]), ("VOX1".to_string(), true));
        assert_eq!(switch_value(&rows[1]), ("VOX3".to_string(), false));
        // Without a key in force, and a bad row with no words of the hub's.
        let mut keyless = state(true, Some(true), None, vec![singer("VOX1", Dry, Dry)]);
        let mut wordless = bad(singer("VOX2", Dry, Dry), "x");
        wordless.why = None;
        keyless.singers.push(wordless);
        let rows = panel_rows(&keyless);
        assert_eq!(rows[0].key, "—");
        assert_eq!(
            rows[1].why.as_deref(),
            Some("Vox 2: key not confirmed · on DRY, rewrite at the next change")
        );
        // A target that names no singer has no manual off.
        let lone = state(true, Some(true), Some("F"), vec![singer("", Dry, Dry)]);
        assert_eq!(panel_rows(&lone)[0].switch, None);
    }

    #[test]
    fn the_panel_head_names_the_coming_change_and_its_clip() {
        // PR #150 review K1, mockup v2's head.
        let mut s = state(true, Some(true), Some("F"), Vec::new());
        s.change = Some(fohmixer_proto::client::ComingChange {
            key: "G".into(),
            clip: "Key {G}".into(),
        });
        assert_eq!(panel_head(&s), "song 031 · F → G at Key {G}");
        s.song = None;
        assert_eq!(panel_head(&s), "no song");
    }

    #[test]
    fn the_panel_foot_names_the_first_problems() {
        // PR #150 review K2, M3.
        let mut s = state(true, Some(true), Some("F"), Vec::new());
        s.map_keyed = 165;
        s.map_changes = 8;
        s.map_problems = 1;
        s.map_texts = vec!["song 088: not ended by SONG END, STOP or AUTOSTOP".into()];
        assert_eq!(
            panel_foot(&s)[0],
            "Song map: 165 songs with a key, 8 key changes, 1 problem (song 088: not ended by SONG END, STOP or AUTOSTOP)."
        );
        // More problems than told: marked.
        s.map_problems = 5;
        s.map_texts = vec!["a".into(), "b".into(), "c".into()];
        assert_eq!(
            panel_foot(&s)[0],
            "Song map: 165 songs with a key, 8 key changes, 5 problems (a; b; c; …)."
        );
        // Exactly as many as told: no mark.
        s.map_problems = 3;
        assert_eq!(
            panel_foot(&s)[0],
            "Song map: 165 songs with a key, 8 key changes, 3 problems (a; b; c)."
        );
    }

    #[test]
    fn the_panel_head_names_the_song_and_the_key() {
        let s = state(true, Some(true), Some("F"), Vec::new());
        assert_eq!(panel_head(&s), "song 031 · F");
        let mut other = state(true, Some(true), Some("F#m"), Vec::new());
        other.song = Some(7);
        assert_eq!(panel_head(&other), "song 007 · F#m");
        other.song = Some(165);
        other.key = None;
        assert_eq!(panel_head(&other), "song 165 · —");
        other.song = None;
        assert_eq!(panel_head(&other), "no song");
    }

    #[test]
    fn the_panel_foot_counts_the_song_map_and_names_the_bar() {
        let mut s = state(true, Some(true), Some("F"), Vec::new());
        s.map_keyed = 165;
        s.map_changes = 8;
        s.map_problems = 1;
        s.bar = Some("F Major".into());
        assert_eq!(
            panel_foot(&s),
            [
                "Song map: 165 songs with a key, 8 key changes, 1 problem.".to_string(),
                "Live: F Major. Hold AUTOTUNE 1 s: AUT on/off.".to_string(),
            ]
        );
        s.map_keyed = 1;
        s.map_changes = 1;
        s.map_problems = 0;
        s.bar = None;
        assert_eq!(
            panel_foot(&s),
            [
                "Song map: 1 song with a key, 1 key change, 0 problems.".to_string(),
                "Live: —. Hold AUTOTUNE 1 s: AUT on/off.".to_string(),
            ]
        );
        s.map_keyed = 0;
        s.map_changes = 2;
        s.map_problems = 3;
        assert_eq!(
            panel_foot(&s)[0],
            "Song map: 0 songs with a key, 2 key changes, 3 problems."
        );
    }

    #[test]
    fn the_panel_opens_beside_the_column_inside_the_screen() {
        assert_eq!((PANEL_GAP, UPRIGHT_MAX), (8.0, 520.0));
        let at = |button_top: f64| Anchor {
            column_right: 663.0,
            button_top,
            button_bottom: button_top + 44.0,
        };
        let place = |left, top, max_h| Place { left, top, max_h };
        // Room below the button: its top at the button's.
        assert_eq!(
            panel_place(at(300.0), 1194.0, 834.0, 400.0),
            place(671.0, 300.0, 526.0)
        );
        // A button low on the screen: the panel up to fit, 8 px above the
        // foot (834 - 400 - 8 = 426).
        assert_eq!(
            panel_place(at(500.0), 1194.0, 834.0, 400.0),
            place(671.0, 426.0, 400.0)
        );
        assert_eq!(
            panel_place(at(426.0), 1194.0, 834.0, 400.0),
            place(671.0, 426.0, 400.0)
        );
        // Taller than the screen: 8 px from its top, scrolling.
        let side = Anchor {
            column_right: 497.0,
            button_top: 120.0,
            button_bottom: 146.0,
        };
        assert_eq!(
            panel_place(side, 844.0, 390.0, 600.0),
            place(505.0, 8.0, 374.0)
        );
        // A button near the top: never above 8 px.
        assert_eq!(
            panel_place(at(2.0), 1194.0, 834.0, 400.0),
            place(671.0, 8.0, 818.0)
        );
        assert_eq!(
            panel_place(at(8.0), 1194.0, 834.0, 400.0),
            place(671.0, 8.0, 818.0)
        );
    }

    #[test]
    fn on_a_phone_upright_the_panel_never_covers_the_button() {
        // PR #150 review N3: the screen's width, below the button when it
        // fits there or that side is the larger, else above it.
        let at = |button_top: f64| Anchor {
            column_right: 270.0,
            button_top,
            button_bottom: button_top + 32.0,
        };
        let place = |top, max_h| Place {
            left: 8.0,
            top,
            max_h,
        };
        // 844 high, the button at 200..232: 596 below, 184 above.
        assert_eq!(
            panel_place(at(200.0), 390.0, 844.0, 300.0),
            place(240.0, 596.0)
        );
        // The button low (600..632): 196 below, 584 above; 300 fits above.
        assert_eq!(
            panel_place(at(600.0), 390.0, 844.0, 300.0),
            place(292.0, 584.0)
        );
        // Fits below exactly: below.
        assert_eq!(
            panel_place(at(600.0), 390.0, 844.0, 196.0),
            place(640.0, 196.0)
        );
        // Fits nowhere: the larger side, scrolling.
        assert_eq!(
            panel_place(at(600.0), 390.0, 844.0, 900.0),
            place(8.0, 584.0)
        );
        assert_eq!(
            panel_place(at(200.0), 390.0, 844.0, 900.0),
            place(240.0, 596.0)
        );
        // Equal sides: below.
        assert_eq!(
            panel_place(at(406.0), 390.0, 844.0, 900.0),
            place(446.0, 390.0)
        );
        // 520 px wide is upright, 521 beside the column.
        assert_eq!(panel_place(at(200.0), 520.0, 844.0, 300.0).left, 8.0);
        assert_eq!(panel_place(at(200.0), 521.0, 844.0, 300.0).left, 278.0);
    }

    #[test]
    fn a_down_outside_the_open_panel_and_the_button_only_closes_it() {
        // PR #150 review I1.
        assert!(dismisses(true, false, false));
        assert!(
            !dismisses(true, true, false),
            "on the button: its tap closes"
        );
        assert!(!dismisses(true, false, true), "on the panel");
        assert!(!dismisses(false, false, false), "closed: nothing to take");
    }

    #[test]
    fn a_singers_switch_acts_at_a_taps_lift_never_at_its_down_or_after_a_slide() {
        // PR #150 review M1.
        let mut tap = switch_press();
        assert!(tap.down(5, true, 0.0));
        assert_eq!(tap.up(5, 100.0), Lift::Tap);
        assert!(switch_acts(Lift::Tap));
        let mut slid = switch_press();
        assert!(slid.down(5, true, 0.0));
        assert!(!slid.moved(5, 0.0, 0.0), "the first move anchors");
        assert!(slid.moved(5, 0.0, 30.0), "a slide past 10 px ends it");
        assert_eq!(slid.up(5, 200.0), Lift::Nothing);
        assert!(!switch_acts(Lift::Nothing));
        let mut long = switch_press();
        assert!(long.down(5, true, 0.0));
        assert_eq!(long.up(5, 1500.0), Lift::Open, "a long press");
        assert!(switch_acts(Lift::Open));
        let mut cancelled = switch_press();
        assert!(cancelled.down(5, true, 0.0));
        cancelled.cancel(5);
        assert_eq!(cancelled.up(5, 100.0), Lift::Nothing);
        assert_eq!(switch_press(), Hold::lifted(RAIL_HOLD_MS));
    }
}
