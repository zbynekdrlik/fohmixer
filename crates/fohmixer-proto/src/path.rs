//! LOM paths, the grammar of the FohMixer script (S2 design note §3.1).
//!
//! A path is a root (`live_set` or `live_app`) followed by steps: `<attr>`,
//! `<attr> <index>` or `<attr>[name=<text>]`. The `[name=…]` text runs to the
//! matching `]`; its escapes are `\]` and `\\`. The script itself accepts
//! single spaces between tokens only; this parser also accepts runs of
//! whitespace (and trims the ends), and [`LomPath::text`] writes the
//! canonical form the script accepts. A layout binding's `path` is the same
//! grammar without the root ([`parse_steps`]).

use std::fmt;

/// The roots of a LOM path: the Song and the Application.
pub const ROOTS: [&str; 2] = ["live_set", "live_app"];

const NAME_OPEN: &str = "[name=";

/// One step of a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub attr: String,
    pub index: Option<u32>,
    pub name: Option<String>,
}

impl Step {
    /// Appends the step's canonical text to `out`.
    fn write(&self, out: &mut String) {
        out.push_str(&self.attr);
        if let Some(index) = self.index {
            out.push(' ');
            out.push_str(&index.to_string());
        } else if let Some(name) = &self.name {
            out.push_str(NAME_OPEN);
            out.push_str(&escape_name(name));
            out.push(']');
        }
    }
}

/// The text of `name` inside `[name=…]`: `\` and `]` escaped.
pub fn escape_name(name: &str) -> String {
    name.replace('\\', "\\\\").replace(']', "\\]")
}

/// A path that does not parse: `kind` is `syntax`, `bad root` or
/// `forbidden` (the script's words), `detail` says where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathError {
    pub kind: &'static str,
    pub detail: String,
}

impl PathError {
    fn new(kind: &'static str, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }

    fn syntax(detail: impl Into<String>) -> Self {
        Self::new("syntax", detail)
    }
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.detail)
    }
}

impl std::error::Error for PathError {}

/// A parsed absolute path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LomPath {
    pub root: String,
    pub steps: Vec<Step>,
}

impl LomPath {
    /// Parses an absolute path (root and steps).
    pub fn parse(text: &str) -> Result<Self, PathError> {
        let chars: Vec<char> = text.trim().chars().collect();
        let (root, pos) = ident(&chars, 0).ok_or_else(|| PathError::syntax(text))?;
        if !ROOTS.contains(&root.as_str()) {
            return Err(PathError::new("bad root", root));
        }
        let steps = steps_from(&chars, pos, true)?;
        Ok(Self { root, steps })
    }

    /// The canonical text: single spaces, names escaped.
    pub fn text(&self) -> String {
        let mut out = self.root.clone();
        for step in &self.steps {
            out.push(' ');
            step.write(&mut out);
        }
        out
    }

    /// The path of the first `n` steps (the object a longer path passes
    /// through).
    pub fn prefix(&self, n: usize) -> Self {
        Self {
            root: self.root.clone(),
            steps: self.steps.iter().take(n).cloned().collect(),
        }
    }
}

/// Parses a relative path: steps only, no root (a layout binding's `path`).
/// An empty text is no steps.
pub fn parse_steps(text: &str) -> Result<Vec<Step>, PathError> {
    let chars: Vec<char> = text.trim().chars().collect();
    steps_from(&chars, 0, false)
}

/// The canonical text of `steps` (no root).
pub fn steps_text(steps: &[Step]) -> String {
    let mut out = String::new();
    for (i, step) in steps.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        step.write(&mut out);
    }
    out
}

/// The canonical form of a target path, or (for text that does not parse)
/// the text with its whitespace runs collapsed.
pub fn canonical_target(text: &str) -> String {
    match LomPath::parse(text) {
        Ok(path) => path.text(),
        Err(_) => text.split_whitespace().collect::<Vec<_>>().join(" "),
    }
}

/// How many characters from `pos` on satisfy `pred`. (The scanner counts
/// runs with iterators, never with a hand-stepped index: no mutant of it can
/// loop or grow a result forever.)
fn run_of(chars: &[char], pos: usize, pred: impl Fn(&char) -> bool) -> usize {
    chars[pos..].iter().take_while(|&c| pred(c)).count()
}

/// An identifier at `pos`: `[A-Za-z_][A-Za-z0-9_]*`, and the position after it.
fn ident(chars: &[char], pos: usize) -> Option<(String, usize)> {
    let first = *chars.get(pos)?;
    let starts = first.is_ascii_alphabetic() || first == '_';
    if !starts {
        return None;
    }
    let end = pos + run_of(chars, pos, |c| c.is_ascii_alphanumeric() || *c == '_');
    Some((chars[pos..end].iter().collect(), end))
}

fn rest(chars: &[char], pos: usize) -> String {
    chars[pos..].iter().collect()
}

/// Steps from `pos` to the end. `need_sep`: a separator comes first (after
/// a root).
fn steps_from(chars: &[char], mut pos: usize, mut need_sep: bool) -> Result<Vec<Step>, PathError> {
    let mut steps: Vec<Step> = Vec::new();
    // Every round consumes at least one character, so the scan ends within
    // `len` rounds; the bound only stops a broken scanner from looping.
    for _ in 0..=chars.len() {
        if pos >= chars.len() {
            break;
        }
        if need_sep {
            if !chars[pos].is_whitespace() {
                return Err(PathError::syntax(rest(chars, pos)));
            }
            pos += run_of(chars, pos, |c| c.is_whitespace());
        }
        need_sep = true;
        if chars[pos].is_ascii_digit() {
            let end = pos + run_of(chars, pos, char::is_ascii_digit);
            let digits: String = chars[pos..end].iter().collect();
            let index: u32 = digits
                .parse()
                .map_err(|_| PathError::syntax(format!("index {digits} is out of range")))?;
            match steps.last_mut() {
                Some(last) if last.index.is_none() && last.name.is_none() => {
                    last.index = Some(index);
                }
                _ => {
                    return Err(PathError::syntax(format!(
                        "index {digits} has no list before it"
                    )));
                }
            }
            pos = end;
            continue;
        }
        let (attr, end) = ident(chars, pos).ok_or_else(|| PathError::syntax(rest(chars, pos)))?;
        if attr.starts_with('_') {
            return Err(PathError::new("forbidden", attr));
        }
        pos = end;
        let mut name = None;
        if chars.get(pos) == Some(&'[') {
            let open: Vec<char> = NAME_OPEN.chars().collect();
            if !chars[pos..].starts_with(&open) {
                return Err(PathError::syntax(rest(chars, pos)));
            }
            let (text, after) = read_name(chars, pos + open.len())?;
            name = Some(text);
            pos = after;
        }
        steps.push(Step {
            attr,
            index: None,
            name,
        });
    }
    Ok(steps)
}

/// The name text from `pos` to its closing `]`, and the position after it.
/// (An iterator, not index arithmetic: every step consumes a character, so
/// no mutant of it can loop while the name grows.)
fn read_name(chars: &[char], pos: usize) -> Result<(String, usize), PathError> {
    let mut out = String::new();
    let mut rest = chars.iter().enumerate().skip(pos);
    while let Some((i, &c)) = rest.next() {
        match c {
            '\\' => match rest.next() {
                Some((_, &escaped)) if escaped == ']' || escaped == '\\' => out.push(escaped),
                _ => return Err(PathError::syntax("bad escape in [name=")),
            },
            ']' => return Ok((out, i + 1)),
            c => out.push(c),
        }
    }
    Err(PathError::syntax("unterminated [name="))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(attr: &str, index: Option<u32>, name: Option<&str>) -> Step {
        Step {
            attr: attr.to_string(),
            index,
            name: name.map(str::to_string),
        }
    }

    #[test]
    fn parses_indexes_names_and_plain_steps() {
        let path = LomPath::parse("live_set tracks[name=Hand1 #] mixer_device volume").unwrap();
        assert_eq!(path.root, "live_set");
        assert_eq!(
            path.steps,
            vec![
                step("tracks", None, Some("Hand1 #")),
                step("mixer_device", None, None),
                step("volume", None, None),
            ]
        );
        let path = LomPath::parse("live_set tracks 3 devices 0").unwrap();
        assert_eq!(
            path.steps,
            vec![
                step("tracks", Some(3), None),
                step("devices", Some(0), None)
            ]
        );
        assert_eq!(LomPath::parse("live_app").unwrap().steps, vec![]);
    }

    #[test]
    fn the_canonical_text_round_trips() {
        for text in [
            "live_set",
            "live_set tracks 3 mixer_device volume",
            "live_set tracks 12 devices 10 parameters 107",
            "live_set tracks[name=Vocal 1 repro#] devices[name=EQ Eight] parameters 1",
            r"live_set tracks[name=a\]b\\c]",
        ] {
            assert_eq!(LomPath::parse(text).unwrap().text(), text, "{text}");
        }
    }

    #[test]
    fn whitespace_runs_between_steps_are_collapsed_names_are_kept() {
        let path = LomPath::parse("  live_set   tracks[name=A  B]\t mute ").unwrap();
        assert_eq!(path.text(), "live_set tracks[name=A  B] mute");
        assert_eq!(
            canonical_target("live_set  tracks 0   mute"),
            "live_set tracks 0 mute"
        );
    }

    #[test]
    fn escapes_are_decoded_and_bad_ones_refused() {
        let path = LomPath::parse(r"live_set tracks[name=x\]y\\z]").unwrap();
        assert_eq!(path.steps[0].name.as_deref(), Some(r"x]y\z"));
        let err = LomPath::parse(r"live_set tracks[name=x\y]").unwrap_err();
        assert_eq!(err.kind, "syntax");
        assert!(err.detail.contains("bad escape"), "{err}");
    }

    #[test]
    fn an_unbalanced_name_is_a_syntax_error() {
        let err = LomPath::parse("live_set tracks[name=Hand1 #").unwrap_err();
        assert_eq!(err.to_string(), "syntax: unterminated [name=");
        let err = LomPath::parse(r"live_set tracks[name=Hand1 #\]").unwrap_err();
        assert_eq!(err.to_string(), "syntax: unterminated [name=");
    }

    #[test]
    fn bad_roots_steps_and_indexes_are_refused() {
        assert_eq!(LomPath::parse("song tracks").unwrap_err().kind, "bad root");
        assert_eq!(LomPath::parse("").unwrap_err().kind, "syntax");
        assert_eq!(LomPath::parse("3 tracks").unwrap_err().kind, "syntax");
        assert_eq!(
            LomPath::parse("live_set _private").unwrap_err(),
            PathError::new("forbidden", "_private")
        );
        assert_eq!(
            LomPath::parse("live_set tracks[id=3]").unwrap_err(),
            PathError::syntax("[id=3]")
        );
        assert_eq!(
            LomPath::parse("live_settracks").unwrap_err().kind,
            "bad root",
            "a root is a whole identifier"
        );
        assert_eq!(
            LomPath::parse("live_set tracks-1").unwrap_err(),
            PathError::syntax("-1")
        );
        assert_eq!(
            LomPath::parse("live_set 3").unwrap_err().to_string(),
            "syntax: index 3 has no list before it"
        );
        assert_eq!(
            LomPath::parse("live_set tracks 1 2")
                .unwrap_err()
                .to_string(),
            "syntax: index 2 has no list before it"
        );
        assert_eq!(
            LomPath::parse("live_set tracks[name=x] 2")
                .unwrap_err()
                .to_string(),
            "syntax: index 2 has no list before it"
        );
        assert_eq!(
            LomPath::parse("live_set tracks 99999999999")
                .unwrap_err()
                .to_string(),
            "syntax: index 99999999999 is out of range"
        );
        assert_eq!(
            LomPath::parse("live_set tracks[name=x]mute").unwrap_err(),
            PathError::syntax("mute")
        );
    }

    #[test]
    fn a_relative_path_has_no_root() {
        let steps = parse_steps("devices[name=EQ Eight] parameters 1").unwrap();
        assert_eq!(
            steps,
            vec![
                step("devices", None, Some("EQ Eight")),
                step("parameters", Some(1), None)
            ]
        );
        assert_eq!(steps_text(&steps), "devices[name=EQ Eight] parameters 1");
        assert_eq!(parse_steps("  ").unwrap(), vec![]);
        assert_eq!(steps_text(&[]), "");
        assert_eq!(parse_steps("1 parameters").unwrap_err().kind, "syntax");
        assert_eq!(
            parse_steps("devices[name=x").unwrap_err().to_string(),
            "syntax: unterminated [name="
        );
    }

    #[test]
    fn a_prefix_keeps_the_first_steps() {
        let path = LomPath::parse("live_set tracks[name=A] devices[name=B] parameters 1").unwrap();
        assert_eq!(path.prefix(0).text(), "live_set");
        assert_eq!(path.prefix(1).text(), "live_set tracks[name=A]");
        assert_eq!(
            path.prefix(2).text(),
            "live_set tracks[name=A] devices[name=B]"
        );
        assert_eq!(path.prefix(9), path);
    }

    #[test]
    fn a_target_that_does_not_parse_is_only_whitespace_collapsed() {
        assert_eq!(canonical_target(" bad   target "), "bad target");
    }

    #[test]
    fn escape_name_escapes_backslash_then_bracket() {
        assert_eq!(escape_name(r"a]b\c"), r"a\]b\\c");
        assert_eq!(escape_name("plain"), "plain");
    }
}
