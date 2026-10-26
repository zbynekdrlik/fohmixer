//! The song map (#140, spec §4 and Appendix A): a Live set's locators and
//! its `Sections` track's clips turned into songs, their spans, their keys
//! and key changes, with every problem as a value (never a panic, never a
//! guess: an affected song has no key).
//!
//! - [`classify`] reads one locator's name as AbleSet 2.6.7 reads it
//!   (trimmed; ignored, a section, an end, a song, or another entry).
//! - [`build`] makes the spans: a song runs from its locator to the next
//!   locator that is neither ignored nor a section, which must be `SONG
//!   END`, `STOP` or `AUTOSTOP`; its key is the first word of its only
//!   `{…}`, followed by the tempo or nothing. A key clip (a Sections clip
//!   whose `{…}` is one key token) changes the key from its start.
//! - [`key_at`] is the key in force at a song position.
//!
//! Pure, shared by the hub (which reads the set) and the tests.

use serde::{Deserialize, Serialize};

pub mod key;

pub use key::{Key, KeyError, parse_token};

/// The most `get_prop` commands one read call of the hub carries (a cue's
/// `time`, a clip's `start_time`), so no single call holds the script long.
pub const TIME_CHUNK: usize = 200;
/// The end locators' names, compared after the description, the flags,
/// the tags and the `[…]` words are taken out.
const SONG_ENDS: [&str; 2] = ["SONG END", "SONGEND"];
const STOPS: [&str; 2] = ["STOP", "AUTOSTOP"];
/// The flags that make any locator a stop.
const STOP_FLAGS: [&str; 2] = ["+STAY", "+JUMP"];
/// What may follow a key clip's root by mistake (spec Appendix A.3's
/// rejected suffixes and German spellings, #140 review I2 and round 2's
/// C2): a spelling with one of them after its root, alone or before `m`, is
/// a mistyped key.
const WRONG_SUFFIXES: [&str; 11] = [
    "min", "maj", "mol", "moll", "dur", "M", "♯", "♭", "is", "es", "s",
];

/// One locator: its name, and its time in beats when it was read (the
/// ignored ones and the sections have none).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cue {
    pub name: String,
    pub time: Option<f64>,
}

/// One clip of the `Sections` track: its name, and its start in beats when
/// it was read (only a clip whose name holds `{`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipName {
    pub name: String,
    pub start: Option<f64>,
}

/// What a locator is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CueKind {
    /// `*…`, `()…`, `[…]…`: not read at all.
    Ignored,
    /// `>…`: a section of the song before it.
    Section,
    /// `SONG END` / `SONGEND`.
    SongEnd,
    /// `STOP` / `AUTOSTOP`, or any locator with `+STAY` / `+JUMP`.
    Stop,
    /// `[.] NNN title [{description}]`.
    Song {
        number: u16,
        description: Option<String>,
    },
    /// Any other entry (START, MODE, …): it ends the entry before it.
    Other,
}

/// Which `Sections` track the hub read the key clips of (AbleSet finds the
/// track by its name and takes the first of several).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionsState {
    /// No track is named `Sections`: no key clips.
    None,
    /// One track is.
    One,
    /// Several are (their count): the first one's clips were read.
    FirstOf(usize),
    /// The first track named `Sections` has clips that cannot be read (a
    /// group track of that name, #140 review M9): no key clip applies, and
    /// the map says so in a problem.
    Unreadable,
}

/// A key clip applied: from its start, this key.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct KeyChange {
    pub time: f64,
    pub key: Key,
}

/// One song's span: from its locator to its end locator (none when the
/// entry did not end with `SONG END`, `STOP` or `AUTOSTOP`), its key (none:
/// no autotune in the whole span) and its key changes in time order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub number: u16,
    pub start: f64,
    pub end: Option<f64>,
    pub key: Option<Key>,
    pub changes: Vec<KeyChange>,
    /// Each change's key clip's name (trimmed), in the changes' order
    /// (#140 PR 3c: the panel names the coming one); not serialized.
    #[serde(skip)]
    pub clips: Vec<String>,
}

/// What is wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKind {
    /// Two songs carry one number.
    DuplicateNumber,
    /// Two locators (or two key clips of a song) at one time.
    SameTime,
    /// A song locator holds more than one `{`.
    TwoDescriptions,
    /// A song's key does not read.
    BadKey(KeyError),
    /// A word after a song's key that is not its tempo (`{C 120bpm}`, `{A
    /// cappella}`, #140 review M5).
    BadTempo,
    /// A song locator's `{…}` is empty or never closed (`{}`, `{C 120`, #140
    /// review M6, round 2's M6b).
    BadBraces,
    /// A song's entry did not end with `SONG END`, `STOP` or `AUTOSTOP`.
    NotEnded,
    /// A key clip lies in no ended song.
    ClipOutsideSong,
    /// A Sections clip's `{…}` is no one key: a key with more words after
    /// it, a mistyped key (`{Amin}`, `{H}`), or any that does not read on a
    /// clip named `Key` (#140 review I2).
    BadClipKey,
    /// A key clip lies in a song without a key.
    ClipInKeylessSong,
    /// The Sections track's clips cannot be read (`SectionsState::
    /// Unreadable`, #140 review M9).
    SectionsUnreadable,
}

/// One problem: what, which song (if one), where (in beats), and the
/// Sections clip it is about (its trimmed name; none for a locator's).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Problem {
    pub kind: ProblemKind,
    pub song: Option<u16>,
    pub time: Option<f64>,
    #[serde(default)]
    pub clip: Option<String>,
}

impl Problem {
    /// The problem in words (English, #141), e.g. `song 031: two locators
    /// or key clips at one time (beat 22766)`; a clip's names the clip.
    pub fn text(&self) -> String {
        let what = match self.kind {
            ProblemKind::DuplicateNumber => "its number is on another song too".to_string(),
            ProblemKind::SameTime => "two locators or key clips at one time".to_string(),
            ProblemKind::TwoDescriptions => "its locator holds two {…}".to_string(),
            ProblemKind::BadKey(error) => format!("its key does not read: {}", error.text()),
            ProblemKind::BadTempo => "the word after the key must be the tempo".to_string(),
            ProblemKind::BadBraces => "its {…} is empty or never closed".to_string(),
            ProblemKind::NotEnded => "not ended by SONG END, STOP or AUTOSTOP".to_string(),
            ProblemKind::ClipOutsideSong => {
                format!(
                    "{} lies outside every ended song",
                    self.clip_words("key clip")
                )
            }
            ProblemKind::BadClipKey => {
                format!(
                    "{} does not read as one key",
                    self.clip_words("Sections clip")
                )
            }
            ProblemKind::ClipInKeylessSong => {
                format!(
                    "{} lies in a song without a key",
                    self.clip_words("key clip")
                )
            }
            ProblemKind::SectionsUnreadable => {
                "the clips of the track named Sections cannot be read: no key clip applies"
                    .to_string()
            }
        };
        let song = self
            .song
            .map(|n| format!("song {n:03}: "))
            .unwrap_or_default();
        let beat = self
            .time
            .map(|t| format!(" (beat {t})"))
            .unwrap_or_default();
        format!("{song}{what}{beat}")
    }

    /// The clip a problem is about, in words: `the key clip "Key {G}"`, or
    /// `a key clip` when its name is not known.
    fn clip_words(&self, what: &str) -> String {
        match &self.clip {
            Some(name) => format!("the {what} \"{name}\""),
            None => format!("a {what}"),
        }
    }
}

/// A set's song map: the songs' spans in time order, and every problem.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SongMap {
    pub spans: Vec<Span>,
    pub problems: Vec<Problem>,
}

/// The map summed up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub songs: usize,
    pub keyed: usize,
    pub changes: usize,
    pub problems: usize,
}

/// The first `{` and the last `}` of `name`, with something between
/// (AbleSet's greedy `/\{(.+)\}/`).
fn braces(name: &str) -> Option<(usize, usize)> {
    let open = name.find('{')?;
    let close = name.rfind('}')?;
    (close > open + 1).then_some((open, close))
}

/// The description of a name: the text between its first `{` and last `}`.
pub fn description(name: &str) -> Option<&str> {
    braces(name).map(|(open, close)| &name[open + 1..close])
}

/// The words of a name outside its description.
fn outside(name: &str) -> Vec<&str> {
    match braces(name) {
        Some((open, close)) => name[..open]
            .split_whitespace()
            .chain(name[close + 1..].split_whitespace())
            .collect(),
        None => name.split_whitespace().collect(),
    }
}

/// Whether a (trimmed) name is ignored: it starts with `*` or `()`, or
/// with `[` and a `]` after at least one character.
fn ignored(name: &str) -> bool {
    let bracketed = name
        .strip_prefix('[')
        .is_some_and(|rest| rest.chars().skip(1).any(|c| c == ']'));
    name.starts_with('*') || name.starts_with("()") || bracketed
}

/// A song's number: after an optional `.` and spaces, three digits and a
/// space.
fn song_number(name: &str) -> Option<u16> {
    let rest = name.strip_prefix('.').unwrap_or(name).trim_start();
    let head = rest.as_bytes().get(..4)?;
    let digits = head.iter().take(3).all(u8::is_ascii_digit);
    if digits && head.get(3) == Some(&b' ') {
        rest[..3].parse().ok()
    } else {
        None
    }
}

/// What a locator's name is (it is trimmed first).
pub fn classify(name: &str) -> CueKind {
    let name = name.trim();
    if ignored(name) {
        return CueKind::Ignored;
    }
    if name.starts_with('>') {
        return CueKind::Section;
    }
    let words = outside(name);
    if words.iter().any(|w| STOP_FLAGS.contains(w)) {
        return CueKind::Stop;
    }
    let plain = words
        .iter()
        .copied()
        .filter(|w| !w.starts_with(['+', '#', '[']))
        .collect::<Vec<&str>>()
        .join(" ");
    if SONG_ENDS.contains(&plain.as_str()) {
        return CueKind::SongEnd;
    }
    if STOPS.contains(&plain.as_str()) {
        return CueKind::Stop;
    }
    match song_number(name) {
        Some(number) => CueKind::Song {
            number,
            description: description(name).map(str::to_string),
        },
        None => CueKind::Other,
    }
}

/// Whether a locator of this kind gets its time read (every kind but the
/// ignored ones and the sections).
pub fn needs_time(kind: &CueKind) -> bool {
    !matches!(kind, CueKind::Ignored | CueKind::Section)
}

/// Whether a Sections clip gets its start read: its name holds `{`.
pub fn clip_needs_time(name: &str) -> bool {
    name.contains('{')
}

/// Whether a word is a tempo: digits, optionally `.` and digits.
fn is_tempo(word: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match word.split_once('.') {
        Some((whole, fraction)) => digits(whole) && digits(fraction),
        None => digits(word),
    }
}

/// A song's key from its description: the first word, then the tempo or
/// nothing (a key that does not read is `BadKey`, a word after it that is
/// no tempo `BadTempo`). No description: no key, and a problem only when
/// the name still holds a `{` (`{}`, a `{` never closed: a tag that does not
/// read, spec §4, #140 review M6), else none; that and an empty `{ }` are
/// `BadBraces` (round 2's M6b: the key may be there, the braces are wrong).
fn song_key(name: &str, description: Option<&str>) -> Result<Option<Key>, ProblemKind> {
    let Some(description) = description else {
        return if name.contains('{') {
            Err(ProblemKind::BadBraces)
        } else {
            Ok(None)
        };
    };
    let mut words = description.split_whitespace();
    let first = words.next().ok_or(ProblemKind::BadBraces)?;
    let key = parse_token(first).map_err(ProblemKind::BadKey)?;
    match words.next() {
        Some(word) if !is_tempo(word) => Err(ProblemKind::BadTempo),
        _ => Ok(Some(key)),
    }
}

/// Whether a clip follows the owner's naming of a key clip, `Key {X}`
/// (spec §2.1): its words outside the braces are exactly `Key`; with a `{`
/// that opens no `{…}` (`Key {}`, `Key {G`), the words before that `{`.
fn named_key(name: &str) -> bool {
    let words: Vec<&str> = match (braces(name), name.find('{')) {
        (Some(_), _) => outside(name),
        (None, Some(open)) => name[..open].split_whitespace().collect(),
        (None, None) => return false,
    };
    words == ["Key"]
}

/// Whether a description's first word, which does not read (`error`), is
/// a mistyped key rather than a word (#140 review I2 b, round 2's C1, C2,
/// C2b): a double spelling of two keys, or one of its `/` spellings spelled
/// wrong (`spelled_wrong`).
fn mistyped(word: &str, error: KeyError) -> bool {
    error == KeyError::TwoKeys || word.split('/').any(spelled_wrong)
}

/// Whether one spelling (no `/`) is a key spelled wrong: with `H` or a
/// lower-case `a`–`h` first, when it reads as a key, or is spelled wrong,
/// with that letter upper-cased (`h`/`H` as `B`; round 2's C1: a plain word
/// such as `bridge` is no attempt at a key); else when its root is wrong
/// (`wrong_root`).
fn spelled_wrong(spelling: &str) -> bool {
    match letter_fixed(spelling) {
        Some(fixed) => parse_token(&fixed).is_ok() || wrong_root(&fixed),
        None => wrong_root(spelling),
    }
}

/// A spelling with its first letter read as a key letter (`H`/`h` as `B`, a
/// lower-case `a`–`g` upper-cased), none when it starts otherwise.
fn letter_fixed(spelling: &str) -> Option<String> {
    let mut chars = spelling.chars();
    let letter = match chars.next()? {
        'H' | 'h' => 'B',
        lower @ 'a'..='g' => lower.to_ascii_uppercase(),
        _ => return None,
    };
    Some(format!("{letter}{}", chars.as_str()))
}

/// Whether a spelling's root is an accidental that is no key (`E#`, `Cb`),
/// or is followed by a wrong suffix, alone or before `m` (`Amin`, `F♯m`,
/// `Fis`, `Gism`).
fn wrong_root(spelling: &str) -> bool {
    let wrong = |rest: &str| {
        WRONG_SUFFIXES.contains(&rest)
            || rest
                .strip_suffix('m')
                .is_some_and(|r| WRONG_SUFFIXES.contains(&r))
    };
    parse_token(spelling) == Err(KeyError::Accidental)
        || key::after_root(spelling).is_some_and(wrong)
}

/// A clip's key: none when it is no key clip (no description, or words
/// such as `{soft}`, `{Chorus 2}`); the key, or `Err` when its description
/// is no one key (#140 review I2, round 2's C2b): more words after a key, a
/// first word that is a mistyped key (`{Amin}`, `{h}`, `{Fis}`, `{Amin
/// loud}`, `{Amin/C}`), or anything that does not read on a clip named
/// `Key`.
fn clip_key(name: &str) -> Option<Result<Key, ()>> {
    let name = name.trim();
    let named = named_key(name);
    let Some(description) = description(name) else {
        return named.then_some(Err(()));
    };
    let mut words = description.split_whitespace();
    let word = words.next().unwrap_or("");
    let more = words.next().is_some();
    match (parse_token(word), more) {
        (Ok(_), true) => Some(Err(())),
        (Ok(key), false) => Some(Ok(key)),
        (Err(error), _) if named || mistyped(word, error) => Some(Err(())),
        (Err(_), _) => None,
    }
}

/// A locator placed in time.
struct Placed<'a> {
    time: f64,
    kind: CueKind,
    name: &'a str,
}

/// A song's span while it is built: whether something made it keyless.
struct Building {
    span: Span,
    keyless: bool,
}

/// The song map of a set's locators and Sections clips.
pub fn build(cues: &[Cue], clips: &[ClipName]) -> SongMap {
    let mut placed: Vec<Placed> = cues
        .iter()
        .filter_map(|cue| {
            let kind = classify(&cue.name);
            Some(Placed {
                time: cue.time.filter(|_| needs_time(&kind))?,
                kind,
                name: cue.name.trim(),
            })
        })
        .collect();
    placed.sort_by(|a, b| a.time.total_cmp(&b.time));
    let mut problems = Vec::new();
    let mut songs = spans(&placed, &mut problems);
    duplicates(&mut songs, &mut problems);
    same_times(&placed, &mut songs, &mut problems);
    let mut spans: Vec<Span> = songs
        .into_iter()
        .map(|mut b| {
            if b.keyless {
                b.span.key = None;
            }
            b.span
        })
        .collect();
    key_clips(clips, &mut spans, &mut problems);
    SongMap { spans, problems }
}

/// Each song's span from the placed locators, with its own problems.
fn spans(placed: &[Placed], problems: &mut Vec<Problem>) -> Vec<Building> {
    let nexts = placed.iter().skip(1).map(Some).chain(std::iter::once(None));
    let mut songs = Vec::new();
    for (cue, next) in placed.iter().zip(nexts) {
        let CueKind::Song {
            number,
            description,
        } = &cue.kind
        else {
            continue;
        };
        let problem = |kind| Problem {
            kind,
            song: Some(*number),
            time: Some(cue.time),
            clip: None,
        };
        let end = next
            .filter(|n| matches!(n.kind, CueKind::SongEnd | CueKind::Stop))
            .map(|n| n.time);
        let mut keyless = end.is_none();
        if keyless {
            problems.push(problem(ProblemKind::NotEnded));
        }
        let key = if cue.name.matches('{').count() > 1 {
            problems.push(problem(ProblemKind::TwoDescriptions));
            keyless = true;
            None
        } else {
            match song_key(cue.name, description.as_deref()) {
                Ok(key) => key,
                Err(kind) => {
                    problems.push(problem(kind));
                    keyless = true;
                    None
                }
            }
        };
        songs.push(Building {
            span: Span {
                number: *number,
                start: cue.time,
                end,
                key,
                changes: Vec::new(),
                clips: Vec::new(),
            },
            keyless,
        });
    }
    songs
}

/// Every song whose number another song carries too is keyless.
fn duplicates(songs: &mut [Building], problems: &mut Vec<Problem>) {
    let numbers: Vec<u16> = songs.iter().map(|b| b.span.number).collect();
    for song in songs.iter_mut() {
        let number = song.span.number;
        if numbers.iter().filter(|n| **n == number).count() > 1 {
            song.keyless = true;
            problems.push(Problem {
                kind: ProblemKind::DuplicateNumber,
                song: Some(number),
                time: Some(song.span.start),
                clip: None,
            });
        }
    }
}

/// Whether a span starts at `time` or reaches it (its end included).
fn touches(span: &Span, time: f64) -> bool {
    time >= span.start && (time == span.start || span.end.is_some_and(|end| time <= end))
}

/// Two placed locators at one time: every song that starts at it or
/// reaches it is keyless (its entry's bounds are not certain); one problem
/// with no song when none does.
fn same_times(placed: &[Placed], songs: &mut [Building], problems: &mut Vec<Problem>) {
    let mut times: Vec<f64> = placed
        .windows(2)
        .filter(|pair| pair[0].time == pair[1].time)
        .map(|pair| pair[0].time)
        .collect();
    times.dedup();
    for time in times {
        let problem = |song| Problem {
            kind: ProblemKind::SameTime,
            song,
            time: Some(time),
            clip: None,
        };
        let mut found = false;
        for song in songs.iter_mut().filter(|b| touches(&b.span, time)) {
            song.keyless = true;
            found = true;
            problems.push(problem(Some(song.span.number)));
        }
        if !found {
            problems.push(problem(None));
        }
    }
}

/// The span a time lies in: `start <= time < end` (an ended span only).
fn span_at(spans: &[Span], time: f64) -> Option<usize> {
    spans
        .iter()
        .position(|s| s.start <= time && s.end.is_some_and(|end| time < end))
}

/// The key clips applied to their spans, in time order; two at one time in
/// a span: the later in clip order wins.
fn key_clips(clips: &[ClipName], spans: &mut [Span], problems: &mut Vec<Problem>) {
    for clip in clips {
        let (Some(start), Some(read)) = (clip.start, clip_key(&clip.name)) else {
            continue;
        };
        let at = span_at(spans, start);
        let problem = |kind, song| Problem {
            kind,
            song,
            time: Some(start),
            clip: Some(clip.name.trim().to_string()),
        };
        let song = at.map(|i| spans[i].number);
        match (read, at) {
            (Err(()), _) => problems.push(problem(ProblemKind::BadClipKey, song)),
            (Ok(_), None) => problems.push(problem(ProblemKind::ClipOutsideSong, None)),
            (Ok(_), Some(i)) if spans[i].key.is_none() => {
                problems.push(problem(ProblemKind::ClipInKeylessSong, song));
            }
            (Ok(key), Some(i)) => {
                spans[i].changes.push(KeyChange { time: start, key });
                spans[i].clips.push(clip.name.trim().to_string());
            }
        }
    }
    for span in spans.iter_mut() {
        let mut pairs: Vec<(KeyChange, String)> = std::mem::take(&mut span.changes)
            .into_iter()
            .zip(std::mem::take(&mut span.clips))
            .collect();
        pairs.sort_by(|a, b| a.0.time.total_cmp(&b.0.time));
        let mut kept: Vec<(KeyChange, String)> = Vec::new();
        for pair in pairs {
            if kept.last().is_some_and(|last| last.0.time == pair.0.time) {
                kept.pop();
                problems.push(Problem {
                    kind: ProblemKind::SameTime,
                    song: Some(span.number),
                    time: Some(pair.0.time),
                    clip: None,
                });
            }
            kept.push(pair);
        }
        (span.changes, span.clips) = kept.into_iter().unzip();
    }
}

/// The song whose span holds `time`, keyed or not (#140 PR 2a review M2: a
/// keyless song is still a song); none outside every span.
pub fn song_at(map: &SongMap, time: f64) -> Option<u16> {
    span_at(&map.spans, time).map(|i| map.spans[i].number)
}

/// The key in force at `time`: the song whose span holds it, if it has a
/// key, and its last change at or before `time`, else the song's key.
pub fn key_at(map: &SongMap, time: f64) -> Option<(u16, Key)> {
    let span = &map.spans[span_at(&map.spans, time)?];
    let key = span.key?;
    let now = span
        .changes
        .iter()
        .rev()
        .find(|c| c.time <= time)
        .map_or(key, |c| c.key);
    Some((span.number, now))
}

/// The coming key change of the song whose span holds `time`: its first
/// change after `time` (later than it), its key and its clip's name (#140
/// PR 3c: the panel's head); none outside every song or after its last
/// change (a keyless song has none: its clips are problems).
pub fn next_change(map: &SongMap, time: f64) -> Option<(Key, &str)> {
    let span = &map.spans[span_at(&map.spans, time)?];
    let at = span.changes.iter().position(|c| c.time > time)?;
    Some((span.changes[at].key, span.clips.get(at)?.as_str()))
}

/// The map summed up: songs, those with a key, the key changes, the
/// problems.
pub fn counts(map: &SongMap) -> Counts {
    Counts {
        songs: map.spans.len(),
        keyed: map.spans.iter().filter(|s| s.key.is_some()).count(),
        changes: map.spans.iter().map(|s| s.changes.len()).sum(),
        problems: map.problems.len(),
    }
}

#[cfg(test)]
mod tests;
