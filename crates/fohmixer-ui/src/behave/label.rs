//! The strip label (spec F4): the first word of the track name, after a
//! leading return prefix (`X-`: a letter and a hyphen) is dropped. One
//! function with the hub (#68: the migration labels a marker with what the
//! strip shows), so it lives in the protocol crate.

pub use fohmixer_proto::layout::strip_label;

/// The name a strip shows (#68): a marker strip's own label as written,
/// else [`strip_label`] of its track's name.
pub fn shown_label(label: Option<&str>, name: &str) -> String {
    label.map_or_else(|| strip_label(name), str::to_string)
}

/// How a marker strip's problem shows (#68): its `data-mark` and the word
/// on its readout line; none without a problem.
pub fn mark_look(
    mark: Option<fohmixer_proto::layout::StripMark>,
) -> Option<(&'static str, &'static str)> {
    use fohmixer_proto::layout::StripMark;
    match mark? {
        StripMark::Conflict => Some(("conflict", "CONFLICT")),
        StripMark::Problem => Some(("problem", "TAG?")),
    }
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

/// The Live set a strip says it belongs to (#211, the owner's approved
/// `docs/mockups/strip-instance-v1.html`): its instance's name in capitals
/// (`BAND`, `MASTER`, `MBC`; the layout gives an instance no other display
/// name), dim in its info column. None when the instance is empty, or when
/// the strip's side text (`group-side`: its group's name, or an untitled
/// shared group's or a mixed sector layout's set, which the stylesheet
/// shows in capitals) already says that word: every strip says its set
/// once in its info column. (A mixed rows-page group's strip keeps its
/// `strip-tag` pill in the head too, #63.)
pub fn set_text(instance: &str, side: Option<&str>) -> Option<String> {
    let text = instance.trim().to_uppercase();
    let said = side.is_some_and(|s| s.trim().to_uppercase() == text);
    (!text.is_empty() && !said).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_problem_shows_its_mark_and_word() {
        use fohmixer_proto::layout::StripMark;
        assert_eq!(
            mark_look(Some(StripMark::Conflict)),
            Some(("conflict", "CONFLICT"))
        );
        assert_eq!(
            mark_look(Some(StripMark::Problem)),
            Some(("problem", "TAG?"))
        );
        assert_eq!(mark_look(None), None);
    }

    #[test]
    fn a_marker_strip_shows_its_own_label_whole() {
        assert_eq!(
            shown_label(Some("Vox 1 mastered"), "Vox 1 #"),
            "Vox 1 mastered"
        );
        assert_eq!(shown_label(None, "Vox 1 #"), "Vox");
        assert_eq!(shown_label(None, "A-Reverb #"), "Reverb");
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
    fn a_strip_says_its_live_set_once() {
        assert_eq!(set_text("band", None).as_deref(), Some("BAND"));
        assert_eq!(set_text("master", None).as_deref(), Some("MASTER"));
        assert_eq!(set_text("mbc", None).as_deref(), Some("MBC"));
        // A group's title leaves the set to the mark, another set's word too.
        assert_eq!(set_text("band", Some("Stage")).as_deref(), Some("BAND"));
        assert_eq!(set_text("mbc", Some("MASTER")).as_deref(), Some("MBC"));
        // The side already says it (shown in capitals): no second word.
        assert_eq!(set_text("band", Some("BAND")), None);
        assert_eq!(set_text("master", Some("Master")), None);
        assert_eq!(set_text("master", Some(" master ")), None);
        assert_eq!(set_text(" mbc ", Some("MBC")), None);
        // No instance, no mark.
        assert_eq!(set_text("", None), None);
        assert_eq!(set_text("  ", Some("")), None);
    }
}
