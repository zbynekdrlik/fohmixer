# Autotune song map (PR 1 of #140) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** the hub reads every Live set's locators and `Sections` clips into a song map: the songs, their spans, their keys and key changes, with problems as values. It serves the map at `GET /api/songmap`, sums it up in `/api/status`, and records each read. It writes nothing to Live.

**Architecture:**

- **A pure parser in the proto crate** (`songmap`): it turns names and times into spans and keys, and keys into Auto-Tune's values. It sits under the mutation gate.
- **A router glue module** (`router/songmap.rs`):
  - It listens to each instance's `live_set cue_points` and `tracks[name=Sections] arrangement_clips`.
  - It reads the names in one call and the times in chunked calls.
  - It reads at a connect, at a list change, at every transport change and when the `Sections` track appears, and keeps the newest map per instance.
- **SimLive** gains cue points and arrangement clips, so a host test drives it end to end.

**Tech Stack:** Rust (fohmixer-proto, fohmixer-hub: axum, tokio, serde), Python (SimLive), nextest host tests.

**Spec:** `docs/superpowers/specs/2026-10-23-autotune-key-design.md` (§2.1, §4, §6, §12's PR 1, Appendix A). Read it first.

## Global Constraints

- **Tier 0:** no local cargo compilation. Push `dev` and read CI. Locally only `cargo fmt --all`, `ruff check .`, the Python tests, `python3 scripts/check_integrity.py` and `python3 scripts/check_version.py`.
- **Public repo:** no song names, singer names, host names or IPs in code, tests, fixtures, commits or tickets.
  - Test fixtures use invented titles (`001 Test Song {C 120 4/4}`).
  - Run the denylist scan before every push: `python3 scripts/denylist_scan.py --denylist <list> --identities scripts/allowed-identities.txt --boundary scripts/denylist-boundary.txt --accepted scripts/denylist-accepted.txt --tree HEAD --commits HEAD`.
- **No writes to Live in this PR.** Every LOM command this PR adds is `get_prop` or `add_listener` / `remove_listener`.
- **Version:** `dev` is already 1.9.0 (> master 1.8.0). No bump.
- **Commits:** the noreply identity, with `(#140)` in the message and `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`. A bug fix in review goes `[red]` before `[green]`.
- **Mutation gate:** every pure function has a direct unit test. No hand-stepped index loops (`.claude/rules/hub-rust.md`). No struct-update base in a new literal.
- **English surface terms (#141):** any text the hub returns for a page, and every record field, is English.

## Review Focus

1. **Locator names with leading or trailing spaces** (8 in the real set). They are trimmed as AbleSet trims them: ` 058 Song {F#/Gb 70}` is song 058 in F#. Test `a_name_is_trimmed_before_it_is_read` (Task 2).
2. **A song directly followed by the next song,** with no SONG END: a problem on the first song, which gets no key. The next song is still a song. Test `a_song_ended_by_the_next_song_is_a_problem_and_keyless` (Task 2).
3. **Two locators at the same time,** or a key clip exactly at a span's start or end: the first is a problem and makes the song keyless. A clip at the span's start applies from the start; a clip at the end belongs to no span (a problem). Tests `two_locators_at_one_time_make_the_song_keyless`, `a_key_clip_at_the_span_start_applies_from_it` and `a_key_clip_at_a_song_end_belongs_to_no_span` (Task 2).
4. **A set without a `Sections` track, or a second track of that name.**
   - None: the map has no key clips and reports `sections: none` (no problem).
   - Two: the name binding takes the first; the report says `sections: first of 2`, since AbleSet takes the first too.
   - Tests `a_set_without_sections_has_spans_and_no_changes` (Task 4) and `the_report_names_a_second_sections_track` (Task 4).
5. **A big set:** 928 cue points and 3508 clips. Times are read only for the cue points that are neither ignored nor sections, and for the clips whose name holds `{`. They are read in calls of at most `songmap::TIME_CHUNK` (200) commands, so no single call holds the script for long. A read never runs twice at once per instance: a trigger during a read marks it dirty and reads again after it. Test `a_trigger_during_a_read_reads_again_once_after_it` (Task 4) and the host test's 450-cue site (Task 5).

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/fohmixer-proto/src/songmap.rs` (create) | Pure types and `build`: cue classification, spans, key clips, problems, `key_at`, report counts |
| `crates/fohmixer-proto/src/songmap/key.rs` (create) | Pure key token grammar, normalization, Auto-Tune and Live values |
| `crates/fohmixer-proto/src/songmap/tests.rs`, `songmap/key_tests.rs` (create) | Unit tests |
| `crates/fohmixer-proto/src/lib.rs` (modify) | `pub mod songmap;` and the module doc line |
| `crates/fohmixer-proto/src/client.rs` (modify) | `SongMapStatus` in `HubStatus` |
| `crates/fohmixer-hub/src/router/songmap.rs` (create) | Glue: subscriptions, triggers, chunked reads, the newest map per instance, the `songmap` record |
| `crates/fohmixer-hub/src/router/songmap/tests.rs` (create) | Router unit tests |
| `crates/fohmixer-hub/src/router.rs` (modify) | Hooks: construct, connected, disconnected, flush, transport, status; `RouterMsg::SongMapRead` |
| `crates/fohmixer-hub/src/routes.rs` (modify) | `GET /api/songmap` |
| `crates/fohmixer-hub/src/events.rs` (modify) | `songmap` warn when it has problems or an error |
| `sim/Live/__init__.py`, `sim/site_builder.py`, `sim/host.py`, `sim/tests/test_songmap.py` (modify, create) | `CuePoint`, `Clip`, `Song.cue_points`, `Track.arrangement_clips`, fixture keys, host lines |
| `crates/fohmixer-hub/tests/songmap.rs` (create), `.config/nextest.toml` (modify) | Host test in `hub-hosts` |
| `.claude/rules/hub-rust.md` (modify) | The song map's design facts |

---

### Task 1: The key token and its values (pure)

**Files:**
- Create: `crates/fohmixer-proto/src/songmap/key.rs`, `crates/fohmixer-proto/src/songmap/key_tests.rs`
- Create (stub for the module tree): `crates/fohmixer-proto/src/songmap.rs` with `#[path = "songmap/key.rs"] pub mod key;` (Task 2 fills the rest)
- Modify: `crates/fohmixer-proto/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub struct Key { pub root: u8, pub minor: bool }` (root 0 = C … 11 = B), deriving `Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize`.
  - `pub enum KeyError { Empty, Letter, Accidental, Suffix, TwoKeys }` (serde lowercase), plus `fn text(&self) -> &'static str` (English).
  - `pub fn parse_token(token: &str) -> Result<Key, KeyError>`
  - `impl Key { pub fn name(&self) -> String; pub fn autotune_key(&self) -> f64; pub fn autotune_scale(&self) -> f64; pub fn live_scale_name(&self) -> &'static str }`
  - `pub fn key_index_of(p: f64) -> u8`, `pub fn scale_index_of(p: f64) -> u8` (read-back, round half up)
  - `pub const SCALE_STEPS: u8 = 28;` `pub const KEY_STEPS: u8 = 11;`

- [ ] **Step 1: Write the failing tests** in `key_tests.rs` (`#[cfg(test)] #[path = "key_tests.rs"] mod tests;` in `key.rs`):

```rust
#[test]
fn every_accepted_root_parses_to_its_index() {
    let cases = [("C",0),("C#",1),("Db",1),("D",2),("D#",3),("Eb",3),("E",4),("F",5),
                 ("F#",6),("Gb",6),("G",7),("G#",8),("Ab",8),("A",9),("A#",10),("Bb",10),("B",11)];
    for (t, k) in cases { assert_eq!(parse_token(t), Ok(Key { root: k, minor: false }), "{t}"); }
}
#[test]
fn a_trailing_m_is_minor() {
    assert_eq!(parse_token("F#m"), Ok(Key { root: 6, minor: true }));
    assert_eq!(parse_token("Am"), Ok(Key { root: 9, minor: true }));
}
#[test]
fn a_double_spelling_of_one_key_is_that_key() {
    assert_eq!(parse_token("F#/Gb"), Ok(Key { root: 6, minor: false }));
    assert_eq!(parse_token("Gb/F#"), Ok(Key { root: 6, minor: false }));
    assert_eq!(parse_token("C#m/Dbm"), Ok(Key { root: 1, minor: true }));
}
#[test]
fn a_double_spelling_of_two_keys_is_refused() {
    assert_eq!(parse_token("F/G"), Err(KeyError::TwoKeys));
    assert_eq!(parse_token("Am/A"), Err(KeyError::TwoKeys));
}
#[test]
fn rejected_spellings_are_problems() {
    for t in ["", "c", "H", "Fis", "Es", "Amin", "Amol", "Amoll", "Amaj", "Adur", "AM", "A m",
              "F♯", "B♭", "E#", "B#", "Cb", "Fb", "Cx", "C##"] {
        assert!(parse_token(t).is_err(), "{t}");
    }
    assert_eq!(parse_token(""), Err(KeyError::Empty));
    assert_eq!(parse_token("H"), Err(KeyError::Letter));
    assert_eq!(parse_token("E#"), Err(KeyError::Accidental));
    assert_eq!(parse_token("Amin"), Err(KeyError::Suffix));
}
#[test]
fn names_are_autotunes_spelling() {
    let names: Vec<String> = (0..12).map(|r| Key { root: r, minor: false }.name()).collect();
    assert_eq!(names, ["C","C#","D","Eb","E","F","F#","G","G#","A","Bb","B"]);
    assert_eq!(Key { root: 6, minor: true }.name(), "F#m");
}
#[test]
fn autotune_values_are_k_over_11_and_s_over_28() {
    assert_eq!(Key { root: 0, minor: false }.autotune_key(), 0.0);
    assert_eq!(Key { root: 11, minor: false }.autotune_key(), 1.0);
    assert!((Key { root: 5, minor: false }.autotune_key() - 5.0 / 11.0).abs() < 1e-12);
    assert_eq!(Key { root: 5, minor: false }.autotune_scale(), 0.0);
    assert!((Key { root: 5, minor: true }.autotune_scale() - 1.0 / 28.0).abs() < 1e-12);
    assert_eq!(Key { root: 5, minor: true }.live_scale_name(), "Minor");
    assert_eq!(Key { root: 5, minor: false }.live_scale_name(), "Major");
}
#[test]
fn read_back_indexes_round_half_up_at_the_boundaries() {
    assert_eq!(key_index_of(0.5), 6);          // (5 + 0.5) / 11 = 0.5 shows F#
    assert_eq!(key_index_of(0.499886), 5);
    assert_eq!(key_index_of(1.0), 11);
    assert_eq!(scale_index_of(0.0), 0);
    assert_eq!(scale_index_of(0.035714), 1);
    assert_eq!(scale_index_of(0.071429), 2);   // Chromatic: a mismatch for the hub
    assert_eq!(scale_index_of(0.017857), 1);   // the boundary (0 + 0.5)/28 shows Minor
}
```

- [ ] **Step 2: Run the local checks.** `cargo fmt --all --check` must pass. The tests run in CI (Tier 0). Expected after the push: the proto tests fail to compile until Step 3.
- [ ] **Step 3: Implement** `parse_token`, `Key` and its methods, and `key_index_of` / `scale_index_of` in `key.rs`.
  - It is a char-by-char grammar over `token.split('/')`, at most 2 parts. Both parts must give the same `Key`, otherwise `TwoKeys`.
  - Accidentals are ASCII `#` and lower-case `b` only.
  - `E#`, `B#`, `Cb` and `Fb` are `Accidental`. A suffix is either nothing or exactly `m`.
  - Index tables are `const` arrays, not arithmetic on chars.
  - `key_index_of(p) = (p * 11.0 + 0.5).floor()` clamped to 0..=11. `scale_index_of` the same with 28.
- [ ] **Step 4: Push the commit with Task 2, or alone.** All `songmap::key` tests pass in CI.
- [ ] **Step 5: Commit** `feat(proto): the song key token and Auto-Tune's values (#140)`

### Task 2: The song map (pure)

**Files:**
- Modify: `crates/fohmixer-proto/src/songmap.rs`
- Create: `crates/fohmixer-proto/src/songmap/tests.rs`

**Interfaces:**
- Consumes: `key::{Key, KeyError, parse_token}`.
- Produces (all `Serialize, Deserialize, Debug, Clone, PartialEq`):
  - `pub struct Cue { pub name: String, pub time: Option<f64> }`: `time` is none for the cues whose time was not read (ignored ones and sections).
  - `pub struct ClipName { pub name: String, pub start: Option<f64> }`
  - `pub enum CueKind { Ignored, Section, SongEnd, Stop, Song { number: u16, description: Option<String> }, Other }`
  - `pub fn classify(name: &str) -> CueKind`
  - `pub fn needs_time(kind: &CueKind) -> bool`: true for `SongEnd`, `Stop`, `Song`, `Other`.
  - `pub fn clip_needs_time(name: &str) -> bool`: the name holds `{`.
  - `pub struct KeyChange { pub time: f64, pub key: Key }`
  - `pub struct Span { pub number: u16, pub start: f64, pub end: Option<f64>, pub key: Option<Key>, pub changes: Vec<KeyChange> }`. `end` is none when the song's entry did not end with SONG END/STOP/AUTOSTOP.
  - `pub enum ProblemKind { DuplicateNumber, SameTime, TwoDescriptions, BadKey(KeyError), NotEnded, ClipOutsideSong, BadClipKey, ClipInKeylessSong }`
  - `pub struct Problem { pub kind: ProblemKind, pub song: Option<u16>, pub time: Option<f64> }`, plus `fn text(&self) -> String` (English, e.g. `song 031: two locators at beat 22766`).
  - `pub struct SongMap { pub spans: Vec<Span>, pub problems: Vec<Problem> }`
  - `pub fn build(cues: &[Cue], clips: &[ClipName]) -> SongMap`
  - `pub fn key_at(map: &SongMap, time: f64) -> Option<(u16, Key)>`
  - `pub struct Counts { pub songs: usize, pub keyed: usize, pub changes: usize, pub problems: usize }` and `pub fn counts(map: &SongMap) -> Counts`
  - `pub const TIME_CHUNK: usize = 200;`

**Rules (spec Appendix A, exactly):**

- **Trimming and the description:**
  - Every name is trimmed first.
  - The description is the text between the first `{` and the last `}` (AbleSet's greedy `/\{(.+)\}/`). A name with more than one `{` is `TwoDescriptions`.
- **What a name is:**
  - **Ignored:** it starts with `*` or `()`, or matches `^\[.+?\]`.
  - **Section:** it starts with `>`.
  - **`SONG END` / `SONGEND` / `STOP` / `AUTOSTOP`:** compared after removing the description, `+FLAG`, `#tag` and `[…]` words. A `+STAY` or `+JUMP` flag on any non-ignored, non-section locator makes it a `Stop`.
  - **Song:** after an optional leading `.` and spaces, three digits then a space (`NNN title`).
  - **Other:** anything else (START, MODE, `…`). It ends the previous entry, but is no song.
- **Spans:** cues sorted by time (stable). Each `Song` runs to the next cue whose kind is not `Ignored`/`Section`. `end` is set only when that cue is `SongEnd`/`Stop`. An entry ended by anything else is `NotEnded`.
- **The song key:** the first word of the description through `parse_token`. The second word, if any, must parse as a number, else `BadKey(KeyError::Suffix)`. No description is no key and no problem. A `BadKey` or `NotEnded` song has `key: None`.
- **Duplicates and same times:**
  - A duplicate number marks every song with that number keyless (`DuplicateNumber`).
  - Two needed cues at the same time: `SameTime` on the song whose span contains or starts at it, which becomes keyless.
- **Key clips:**
  - A clip whose trimmed description is exactly one key token (no second word) is a key clip.
  - One whose description starts like a key but holds more words is `BadClipKey`, not applied. A clip without `{` is no key clip.
  - A key clip belongs to the span with `start <= clip.start < end`. None is `ClipOutsideSong`, and so is a span with `end: None`.
  - A key clip in a keyless span is `ClipInKeylessSong` and is not applied (the span stays keyless).
  - Changes are sorted by time. Two clips at one time in a span: the later one in clip order wins, with `SameTime`.
- **`key_at(map, t)`:** the span with `start <= t < end` and `key: Some`. The key is the last change with `time <= t`, else the song key. `None` outside spans and in keyless spans.

- [ ] **Step 1: Write the failing tests** in `songmap/tests.rs`, with helpers `cue(name, t)` and `clip(name, t)`:

```rust
#[test]
fn a_song_runs_from_its_locator_to_its_song_end() {
    let m = build(&[cue("001 Test {C 120 4/4}", 0.0), cue("SONG END", 100.0), cue("002 Next {Am 90}", 120.0), cue("SONG END", 200.0)], &[]);
    assert_eq!(m.spans.len(), 2);
    assert_eq!((m.spans[0].number, m.spans[0].start, m.spans[0].end), (1, 0.0, Some(100.0)));
    assert_eq!(m.spans[1].key, Some(Key { root: 9, minor: true }));
    assert!(m.problems.is_empty());
}
#[test]
fn sections_and_ignored_cues_do_not_end_a_song() {
    let m = build(&[cue("001 T {D 100}", 0.0), cue("> Ending", 50.0), cue("* TS", 60.0), cue("*MS", 61.0), cue("[x] clyphx", 62.0), cue("SONG END", 90.0)], &[]);
    assert_eq!(m.spans[0].end, Some(90.0));
}
#[test]
fn a_name_is_trimmed_before_it_is_read() {
    let m = build(&[cue("  058 T {F#/Gb 70 4/4}  ", 0.0), cue(" SONG END", 10.0)], &[]);
    assert_eq!(m.spans[0].key, Some(Key { root: 6, minor: false }));
    assert_eq!(m.spans[0].end, Some(10.0));
}
#[test]
fn stop_autostop_and_a_jump_flag_end_a_song() { /* STOP, AUTOSTOP, "X +JUMP" each give end = Some(t) */ }
#[test]
fn a_song_ended_by_the_next_song_is_a_problem_and_keyless() {
    let m = build(&[cue("001 A {C 100}", 0.0), cue("002 B {D 100}", 50.0), cue("SONG END", 90.0)], &[]);
    assert_eq!(m.spans[0].key, None);
    assert_eq!(m.spans[0].end, None);
    assert_eq!(m.spans[1].key, Some(Key { root: 2, minor: false }));
    assert!(m.problems.iter().any(|p| p.kind == ProblemKind::NotEnded && p.song == Some(1)));
}
#[test]
fn free_text_after_the_key_is_refused() {
    let m = build(&[cue("001 T {A cappella}", 0.0), cue("SONG END", 9.0)], &[]);
    assert_eq!(m.spans[0].key, None);
    assert!(matches!(m.problems[0].kind, ProblemKind::BadKey(_)));
}
#[test]
fn a_song_without_a_description_has_no_key_and_no_problem() { /* "088 Dance", key None, problems empty */ }
#[test]
fn two_descriptions_make_the_song_keyless() { /* "001 T {C 100} {D}" -> TwoDescriptions, key None */ }
#[test]
fn a_duplicate_number_makes_both_keyless() { /* two "005 …" songs -> both key None, DuplicateNumber twice */ }
#[test]
fn two_locators_at_one_time_make_the_song_keyless() { /* "001 …" at 0, "SONG END" at 10, "002 …" at 10 -> SameTime, 001 keyless */ }
#[test]
fn other_entries_are_no_songs() { /* "START" at 0, "MODE 1" at 5, "001 T {E 90}" at 10, "SONG END" at 20 -> 1 span, number 1 */ }
#[test]
fn key_clips_change_the_key_from_their_start() {
    let m = build(&[cue("031 T {F 95 4/4}", 22766.0), cue("SONG END", 23501.0)],
                  &[clip("Verse", 22906.0), clip("Key {G}", 22897.0)]);
    assert_eq!(m.spans[0].changes, vec![KeyChange { time: 22897.0, key: Key { root: 7, minor: false } }]);
    assert_eq!(key_at(&m, 22896.9), Some((31, Key { root: 5, minor: false })));
    assert_eq!(key_at(&m, 22897.0), Some((31, Key { root: 7, minor: false })));
    assert_eq!(key_at(&m, 23501.0), None);
}
#[test]
fn a_key_clip_at_the_span_start_applies_from_it() { /* clip "Key {D}" at span.start -> key_at(start) == D */ }
#[test]
fn a_key_clip_at_a_song_end_belongs_to_no_span() { /* clip at 23501.0 -> ClipOutsideSong */ }
#[test]
fn a_clip_with_more_words_after_the_key_is_refused() { /* "Chorus {G loud}" -> BadClipKey, not applied */ }
#[test]
fn a_key_clip_in_a_keyless_song_stays_keyless() { /* song without description + "Key {G}" -> ClipInKeylessSong, key_at None */ }
#[test]
fn half_beat_changes_are_kept_exactly() { /* clips at 86963.5 and 86990.5 -> times exactly those */ }
#[test]
fn counts_sum_up_the_map() { /* 3 songs, 2 keyed, 1 change, 1 problem */ }
#[test]
fn classify_reads_each_kind() {
    assert_eq!(classify("> Chorus"), CueKind::Section);
    assert_eq!(classify("* TS"), CueKind::Ignored);
    assert_eq!(classify("() x"), CueKind::Ignored);
    assert_eq!(classify("SONGEND"), CueKind::SongEnd);
    assert_eq!(classify("AUTOSTOP"), CueKind::Stop);
    assert_eq!(classify(". 012 Late {G 80}"), CueKind::Song { number: 12, description: Some("G 80".into()) });
    assert_eq!(classify("START"), CueKind::Other);
    assert!(!needs_time(&CueKind::Section) && needs_time(&CueKind::Other));
}
```

  The bodies given as comments are written out the same way as the full ones, with the exact values in the comment.
- [ ] **Step 2: Run** `cargo fmt --all --check`. Tier 0: the tests run in CI.
- [ ] **Step 3: Implement** `classify`, `needs_time`, `clip_needs_time`, `build`, `key_at`, `counts`, `Problem::text` per the rules above. Iterators only, no index stepping.
- [ ] **Step 4: Push** with Tasks 1–2. Every `songmap::` test passes in CI. The mutation shards show 0 missed for the diff.
- [ ] **Step 5: Commit** `feat(proto): the song map from locators and Sections clips (#140)`

### Task 3: SimLive cue points and arrangement clips

**Files:**
- Modify: `sim/Live/__init__.py`, `sim/site_builder.py`, `sim/host.py`
- Create: `sim/tests/test_songmap.py`

**Interfaces:**
- Produces:
  - **`Live.CuePoint`** with `name` and `time` (both `_Prop`, observable) and `jump()`.
  - **`Live.Clip`** with `name` and `start_time` (observable), `end_time`, and `is_midi_clip` (True).
  - **`Song.cue_points`**, an observable list, kept sorted by `time` as Live keeps it.
  - **`Track.arrangement_clips`**, an observable list on a MIDI track.
  - **Fixture keys** (in `site_builder`):
    - the song's `"cue_points": [{"name": str, "time": float}]`;
    - a track's `"arrangement_clips": [{"name": str, "start_time": float, "end_time": float}]`.
  - **Host lines** (in `host.py` `control`):
    - `cue add <time> <name…>`, `cue rename <index> <name…>`, `cue time <index> <time>`, `cue remove <index>`;
    - `clip add <track index> <start> <end> <name…>`, `clip rename <track index> <clip index> <name…>`, `clip remove <track index> <clip index>`.
    - Each line fires the list's listener (add and remove) or the item's (rename and time).

- [ ] **Step 1: Write the failing tests** in `sim/tests/test_songmap.py`:
  - a fixture with 3 cue points and a `Sections` track with 2 clips builds them;
  - `cue_points` comes out sorted by time;
  - `cue add` fires the `cue_points` listener once and inserts in time order;
  - `cue rename` fires the cue's `name` listener and not the list's;
  - `clip add` fires `arrangement_clips`;
  - a bad host line (`cue rename 99 x`) answers an error and changes nothing.
- [ ] **Step 2: Run** `python3 -m unittest discover -s sim/tests -p 'test_songmap.py'`. Expected: FAIL (`AttributeError` on `cue_points`).
- [ ] **Step 3: Implement** the classes, the fixture keys and the host lines, following the existing `Device`/`Track` `_Prop` and `_listened` patterns.
- [ ] **Step 4: Run** `python3 -m unittest discover -s sim/tests` and `ruff check .`. Expected: all pass.
- [ ] **Step 5: Commit** `feat(sim): cue points and arrangement clips in SimLive (#140)`

### Task 4: The hub reads the map

**Files:**
- Create: `crates/fohmixer-hub/src/router/songmap.rs`, `crates/fohmixer-hub/src/router/songmap/tests.rs`
- Modify:
  - `crates/fohmixer-hub/src/router.rs`;
  - `crates/fohmixer-hub/src/routes.rs`;
  - `crates/fohmixer-hub/src/events.rs`;
  - `crates/fohmixer-proto/src/client.rs`;
  - `.claude/rules/hub-rust.md`.

**Interfaces:**
- Consumes: `fohmixer_proto::songmap::{Cue, ClipName, classify, needs_time, clip_needs_time, build, counts, SongMap, TIME_CHUNK}`; the subscription table and `LiveHandle::call`, as in `router/autotune.rs`.
- Produces:
  - `pub const SONGMAP_CLIENT: ClientId = ClientId::MAX - 4;`, added to the `Status` `own` lists.
  - `pub const SECTIONS: &str = "live_set tracks[name=Sections]";`
  - `pub struct SongMapIo { maps: BTreeMap<String, Read>, reading: BTreeSet<String>, dirty: BTreeSet<String>, reads: u64 }`, where `Read { map: SongMap, sections: SectionsState, at_ms: u64, read_no: u64 }`.
  - `pub enum SectionsState { None, One, FirstOf(usize) }` (serde lowercase / `first_of`).
  - **Pure helpers, each with a unit test:**
    - `time_commands(cues: &[(String, String)]) -> Vec<Vec<Value>>`: `(path, name)` pairs. It makes `get_prop time` for each `needs_time(classify(name))` cue, in chunks of `TIME_CHUNK`.
    - `clip_commands(clips: &[(String, String)]) -> Vec<Vec<Value>>`: `get_prop start_time` for each `clip_needs_time(name)`.
    - `sections_state(tracks_named_sections: usize) -> SectionsState`.
    - `next_read(reading: bool, dirty: bool) -> Option<…>`: whether a trigger starts a read now, marks dirty, or nothing.
    - `songmap_fields(instance, &Read, ms) -> Value`: the record.
  - `RouterMsg::SongMapRead { instance: String, read_no: u64, result: Result<(Vec<Cue>, Vec<ClipName>, SectionsState), String> }`.
  - **`GET /api/songmap?instance=<name>`** (token): `SongMapReport { instance, read_no, at_ms, sections, counts, spans, problems: [{kind, song, time, text}] }` as JSON. 404 for an unknown instance, 503 before a first read.
  - **`HubStatus.songmap: Vec<SongMapStatus { instance, songs, keyed, changes, problems, sections, at_ms }>`**, `#[serde(default)]`.
  - **The `songmap` record** `{instance, read_no, songs, keyed, changes, problems, sections, ms, error?}`, plus `problem_texts` (at most the first 20 texts). It is warn class when `problems > 0` or `error` is set (`events.rs` `is_warn`: add `"songmap"` with a predicate on those fields, not unconditionally).

**When it reads** (each a trigger through `next_read`):

- a connect;
- a `cue_points` value change (the list: an add, remove or move);
- a Sections `arrangement_clips` value change;
- the Sections name binding resolving or erroring;
- every transport change of that instance, so a stop or a start picks up renames made in Live (cue `name`/`time` and clip `name` are not followed one by one: 928 + 3508 listeners would be too many);
- the attach of a page, never.

A disconnect drops the instance's read in flight (its answer is void by `read_no`) and keeps the last map, marked `stale` in the report until the next read.

**How it reads** (one task per read, bounded, no loop a mutant can spin):

1. One `call`: `get_prop cue_points` on `live_set`, and `get_prop arrangement_clips` on `SECTIONS`.
   - An error on the second is `SectionsState::None`. To find `FirstOf(n)`, the same call carries `get_prop tracks` and counts the items named exactly `Sections`.
2. The `time_commands` chunks, one `call` each, sequential.
3. The `clip_commands` chunks, the same way.
4. `build` in the router on the answer. A failed call ends the read with `error` and keeps the old map.

- [ ] **Step 1: Write the failing router unit tests** in `router/songmap/tests.rs` (on `offline_router` / a connected test router as `router/autotune/tests.rs` does):
  - `a_connect_reads_the_map_once`: the `get_prop cue_points` command is sent; after the answers, `/api/status` `songmap` shows the counts.
  - `only_needed_cue_times_are_read`: 3 songs, 3 SONG ENDs, 4 sections and 5 ignored cues give exactly 6 `get_prop time` commands.
  - `times_are_read_in_chunks_of_200`: 450 needed cues give 3 calls (200, 200, 50).
  - `only_clips_with_a_brace_get_their_start_read`.
  - `a_trigger_during_a_read_reads_again_once_after_it`: two list changes during a read give exactly one more read after it.
  - `a_disconnect_voids_the_read_in_flight_and_keeps_the_last_map_stale`.
  - `a_set_without_sections_has_spans_and_no_changes`: the Sections binding errors, so `sections: none`, spans still built, no problem.
  - `the_report_names_a_second_sections_track`: `tracks` holds two items named `Sections`, so `sections: first_of(2)`.
  - `a_transport_change_reads_again`.
  - `the_record_is_warn_only_with_problems_or_an_error` (in `events.rs` tests).
  - `songmap_route_answers_404_for_an_unknown_instance_and_503_before_a_read` (in `routes.rs` tests).
- [ ] **Step 2: Run** `cargo fmt --all --check`. Push the red commit alone (`.claude/rules/ci.md`: prove CI-only reds) and read the failures.
- [ ] **Step 3: Implement** `router/songmap.rs` and the hooks in `router.rs`:
  - construct;
  - `LiveEvent::Connected` → `songmap_connected`;
  - `Disconnected` → `songmap_offline`;
  - `flush` hands `SONGMAP_CLIENT`'s deliveries to `songmap_delivery`, after AUTOTUNE's and before the transport;
  - `on_transport` → `songmap_transport`;
  - `RouterMsg::SongMapRead` → `songmap_read`;
  - `status`.

  Then the route and `HubStatus.songmap`.
- [ ] **Step 4: Push and read CI:** every new test green, the mutation shards 0 missed, the coverage floor held.
- [ ] **Step 5: Commit** `feat(hub): read every set's song map, serve it at /api/songmap (#140)`

### Task 5: End to end through SimLive, and the playbook

**Files:**
- Create: `crates/fohmixer-hub/tests/songmap.rs`, `#![cfg(unix)]`
- Modify: `.config/nextest.toml` (add `binary_id(fohmixer-hub::songmap)` to the `hub-hosts` filter), `.claude/rules/hub-rust.md`

**Interfaces:**
- Consumes: Task 3's fixture keys and host lines, Task 4's route and status, `support::Host::start_site`, `TestHub`.

- [ ] **Step 1: Write the host tests** (a site of their own, built in the test):
  - `the_hub_serves_the_sites_song_map`. The site has:
    - 2 songs with SONG END, invented names (`001 Test One {C 120 4/4}`, `002 Test Two {F#m 90}`);
    - a `* TS` and a `> Ending`;
    - a `Sections` MIDI track with `Verse` and `Key {G}` inside song 1.

    `/api/songmap?instance=band` shows 2 spans, song 1's change to G at its exact start, song 2 F# minor, 0 problems. `/api/status` `songmap` shows `songs: 2, keyed: 2, changes: 1`.
  - `a_locator_added_in_live_is_in_the_next_map`: `cue add 300 003 Test Three {D 100}` plus `cue add 400 SONG END` give 3 spans within 5 s.
  - `a_rename_is_read_at_the_next_transport_change`: `cue rename 0 001 Test One {Eb 120}` changes nothing until `start_playing`. After it, song 1 is Eb.
  - `a_big_set_reads_in_chunks`: 450 songs with SONG END. All 450 spans are served within 10 s, and the event log's `songmap` record has `songs: 450`.
  - `the_hub_never_writes_to_live_for_the_map`: the host's `listeners` and the event log show no `set_prop` from `SONGMAP_CLIENT`. Read the host's command log if it has one, else assert that the cue names and times are unchanged after the reads.
- [ ] **Step 2: Push and read CI.** The new binary runs in `hub-hosts`.
- [ ] **Step 3: Update `.claude/rules/hub-rust.md`** with a "song map (#140 PR 1)" bullet covering the triggers, the chunking, the `read_no` voiding, `SONGMAP_CLIENT`, the record's warn class, and the rename-at-transport rule.
- [ ] **Step 4: Commit** `test(hub): the song map end to end through SimLive (#140)`

### Task 6: PR, review, merge, deploy, the PC read

- [ ] **Step 1: Open the PR** `dev` → `master` after the first push, titled "Song map: the hub reads every set's locators and Sections key clips (#140)". The body states that it writes nothing to Live.
- [ ] **Step 2: Drive CI green** on both runs (push and PR): every job, the mutation shards with 0 missed, `mergeable_state: clean`.
- [ ] **Step 3: A fresh-context code review.** Fix its findings in one round.
- [ ] **Step 4: Merge** with `gh pr merge --merge`. Tag `v1.9.0`. Bump `dev` to 1.10.0 as the first commit after it.
- [ ] **Step 5: Deploy to the PC** by the `deploy-pc.md` flow. Not during an owner-declared event: ask for the event's time if it is not known that day.
- [ ] **Step 6: Post-deploy, read-only:**
  - `/api/songmap?instance=band` against verification 2's table: 165 keyed songs, 088 keyless, every span equal;
  - the problems list explained;
  - `/api/status` `songmap`.
  - Record the numbers on #140 (no names).
