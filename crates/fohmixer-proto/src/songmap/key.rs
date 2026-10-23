//! The song key token (#140, spec Appendix A.3 and A.4): the grammar of a
//! key as the set writes it (`F`, `F#m`, `Gb`, `F#/Gb`) and the values a
//! key is written as: Auto-Tune Artist's Key and Scale, Live's bar.
//!
//! A key is a root (C = 0 … B = 11) and major or minor (a trailing `m`:
//! the root plus Auto-Tune's natural "Minor", never the relative major).
//! Flat and double spellings name the key they spell (`Db` is C#, `F#/Gb`
//! is F#); anything else is refused with a [`KeyError`], never guessed.

use serde::{Deserialize, Serialize};

/// Auto-Tune's Key runs over 12 values, `p = k / KEY_STEPS`.
pub const KEY_STEPS: u8 = 11;
/// Auto-Tune's Scale runs over 29 values, `p = s / SCALE_STEPS`: Major is
/// 0, Minor 1 (Chromatic, its default, is 2).
pub const SCALE_STEPS: u8 = 28;

/// The key letters with their roots (A.3: upper case only).
const LETTERS: [(char, u8); 7] = [
    ('C', 0),
    ('D', 2),
    ('E', 4),
    ('F', 5),
    ('G', 7),
    ('A', 9),
    ('B', 11),
];
/// Every accepted root with an accidental, and its index (A.3).
const ACCIDENTALS: [(&str, u8); 10] = [
    ("C#", 1),
    ("Db", 1),
    ("D#", 3),
    ("Eb", 3),
    ("F#", 6),
    ("Gb", 6),
    ("G#", 8),
    ("Ab", 8),
    ("A#", 10),
    ("Bb", 10),
];
/// Each root's name in Auto-Tune's own spelling (A.4).
const NAMES: [&str; 12] = [
    "C", "C#", "D", "Eb", "E", "F", "F#", "G", "G#", "A", "Bb", "B",
];

/// A song's key: its root (0 = C … 11 = B) and whether it is minor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Key {
    pub root: u8,
    pub minor: bool,
}

/// Why a key token does not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyError {
    /// No token at all.
    Empty,
    /// No key letter first (lower case, `H`, anything else).
    Letter,
    /// `E#`, `B#`, `Cb` or `Fb`.
    Accidental,
    /// Anything after the root but nothing or `m` (`min`, `maj`, `M`, `♯`).
    Suffix,
    /// A double spelling of two different keys, or more than two spellings.
    TwoKeys,
}

impl KeyError {
    /// The problem in words (English, #141).
    pub fn text(&self) -> &'static str {
        match self {
            KeyError::Empty => "no key",
            KeyError::Letter => "a key starts with C, D, E, F, G, A or B",
            KeyError::Accidental => "E#, B#, Cb and Fb are not keys",
            KeyError::Suffix => "only m may follow the key",
            KeyError::TwoKeys => "the two spellings name different keys",
        }
    }
}

/// Reads a key token: `root [m]`, or a double spelling `root [m] / root
/// [m]` of one key.
pub fn parse_token(token: &str) -> Result<Key, KeyError> {
    let mut parts = token.split('/');
    let first = spelling(parts.next().unwrap_or(""))?;
    match (parts.next(), parts.next()) {
        (None, _) => Ok(first),
        (Some(second), None) if spelling(second)? == first => Ok(first),
        _ => Err(KeyError::TwoKeys),
    }
}

/// One spelling: a root, then nothing or `m`.
fn spelling(text: &str) -> Result<Key, KeyError> {
    let mut chars = text.chars();
    let letter = chars.next().ok_or(KeyError::Empty)?;
    let natural = LETTERS
        .iter()
        .find(|(l, _)| *l == letter)
        .map(|(_, root)| *root)
        .ok_or(KeyError::Letter)?;
    let rest = chars.as_str();
    let (root, suffix) = match rest.chars().next() {
        Some(accidental @ ('#' | 'b')) => {
            let spelled = format!("{letter}{accidental}");
            let root = ACCIDENTALS
                .iter()
                .find(|(name, _)| *name == spelled)
                .map(|(_, root)| *root)
                .ok_or(KeyError::Accidental)?;
            (root, &rest[1..])
        }
        _ => (natural, rest),
    };
    match suffix {
        "" => Ok(Key { root, minor: false }),
        "m" => Ok(Key { root, minor: true }),
        _ => Err(KeyError::Suffix),
    }
}

impl Key {
    /// The key in Auto-Tune's spelling, `m` for minor (`F#m`).
    pub fn name(&self) -> String {
        let root = NAMES.get(usize::from(self.root)).copied().unwrap_or("?");
        if self.minor {
            format!("{root}m")
        } else {
            root.to_string()
        }
    }

    /// Auto-Tune's Key value: `root / 11`.
    pub fn autotune_key(&self) -> f64 {
        f64::from(self.root) / f64::from(KEY_STEPS)
    }

    /// Auto-Tune's Scale value: Major 0, Minor `1 / 28`.
    pub fn autotune_scale(&self) -> f64 {
        if self.minor {
            1.0 / f64::from(SCALE_STEPS)
        } else {
            0.0
        }
    }

    /// Live's `scale_name` for the key.
    pub fn live_scale_name(&self) -> &'static str {
        if self.minor { "Minor" } else { "Major" }
    }
}

/// The Key index Auto-Tune shows for a value read back: `p × 11`, half up,
/// within 0..=11.
pub fn key_index_of(p: f64) -> u8 {
    index_of(p, KEY_STEPS)
}

/// The Scale index Auto-Tune shows for a value read back: `p × 28`, half
/// up, within 0..=28.
pub fn scale_index_of(p: f64) -> u8 {
    index_of(p, SCALE_STEPS)
}

/// `p × steps`, half up, within `0..=steps` (NaN reads 0).
fn index_of(p: f64, steps: u8) -> u8 {
    let max = f64::from(steps);
    (p * max + 0.5).floor().clamp(0.0, max) as u8
}

#[cfg(test)]
#[path = "key_tests.rs"]
mod tests;
