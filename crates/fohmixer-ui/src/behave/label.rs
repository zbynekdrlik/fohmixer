//! The strip label (spec F4): the first word of the track name, after a
//! leading return prefix (`X-`: a letter and a hyphen) is dropped.

/// The label a strip shows for track `name`.
pub fn strip_label(name: &str) -> String {
    let mut chars = name.chars();
    let prefixed = matches!(
        (chars.next(), chars.next()),
        (Some(letter), Some('-')) if letter.is_alphabetic()
    );
    let rest = if prefixed { chars.as_str() } else { name };
    rest.split_whitespace().next().unwrap_or("").to_string()
}

/// The length the stylesheet sizes a label's font by (`--n`, #21): its
/// characters, not its bytes, and at least 1 (an empty label divides by
/// nothing).
pub fn label_chars(label: &str) -> usize {
    label.chars().count().max(1)
}

/// The length a rail button's font is sized by (`--n`, #21): the characters
/// of its longest word (a word is never broken), at least 1.
pub fn longest_word_chars(text: &str) -> usize {
    text.split_whitespace()
        .map(|word| word.chars().count())
        .max()
        .unwrap_or(0)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_is_the_first_word_without_the_return_prefix() {
        assert_eq!(strip_label("B-Main repro #"), "Main");
        assert_eq!(strip_label("Vocal 1 repro#"), "Vocal");
        assert_eq!(strip_label("A-Reverb #"), "Reverb");
        assert_eq!(strip_label("Hand1 #"), "Hand1");
        assert_eq!(strip_label("TechAlert #"), "TechAlert");
        assert_eq!(strip_label("  Keys 1"), "Keys");
    }

    #[test]
    fn a_label_counts_its_characters_and_at_least_one() {
        assert_eq!(label_chars("TechAlert"), 9);
        assert_eq!(label_chars("Klavír"), 6);
        assert_eq!(label_chars("B"), 1);
        assert_eq!(label_chars(""), 1);
    }

    #[test]
    fn a_rail_text_counts_its_longest_word() {
        assert_eq!(longest_word_chars("TechAlert"), 9);
        assert_eq!(longest_word_chars("REFRESH ALL"), 7);
        assert_eq!(longest_word_chars("SOLO Podklady"), 8);
        assert_eq!(longest_word_chars("Klavír x"), 6);
        assert_eq!(longest_word_chars("  "), 1);
        assert_eq!(longest_word_chars(""), 1);
    }

    #[test]
    fn only_a_letter_and_a_hyphen_is_a_prefix() {
        assert_eq!(strip_label("1-Mic"), "1-Mic");
        assert_eq!(strip_label("AB-Mic x"), "AB-Mic");
        assert_eq!(strip_label("-Mic"), "-Mic");
        assert_eq!(strip_label("Č-Mic"), "Mic");
        assert_eq!(strip_label("B-"), "");
        assert_eq!(strip_label("B"), "B");
        assert_eq!(strip_label(""), "");
    }
}
