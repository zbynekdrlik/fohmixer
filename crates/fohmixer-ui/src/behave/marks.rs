//! The strip's marks line (#151, the owner's mockup v8): under the name
//! on the name button, one line of a problem's `!`, the linked mute's
//! chain, the fader link's mark and the `MUTED` word, left to right. What
//! gives way when the line fills is decided from what the strip carries,
//! never measured: a muted chain drops its count (it is red, and `MUTED`
//! says it); with three marks or more the fader link drops its "dB"; with
//! all four the gaps narrow; beside both `!` and a chain the fader link's
//! mark yields on a narrow strip only (the stylesheet: the mockup's case of
//! all four, and `!` with a mixed two-digit count such as `10/12`, which
//! leaves it no room at 64 px). `!`, the chain and `MUTED` never yield.

/// How the marks line draws its marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MarksLook {
    /// The mute chain without its count.
    pub chain_bare: bool,
    /// The fader link's mark without its "dB".
    pub flink_bare: bool,
    /// All four marks: the narrower gaps.
    pub crowd: bool,
    /// The fader link's mark yields (applied on a narrow strip only): beside
    /// both `!` and a chain.
    pub flink_yield: bool,
}

/// The look of a marks line holding a problem's `!` (`flag`), the mute
/// chain (`chain`), the fader link's mark (`flink`) and `MUTED` (`muted`).
pub fn marks_look(flag: bool, chain: bool, flink: bool, muted: bool) -> MarksLook {
    let n = [flag, chain, flink, muted]
        .into_iter()
        .filter(|b| *b)
        .count();
    MarksLook {
        chain_bare: chain && muted,
        flink_bare: flink && n >= 3,
        crowd: n >= 4,
        flink_yield: flink && flag && chain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_mark_crowds_alone() {
        assert_eq!(marks_look(true, false, false, false), MarksLook::default());
        assert_eq!(marks_look(false, true, false, false), MarksLook::default());
        assert_eq!(marks_look(false, false, true, false), MarksLook::default());
        assert_eq!(marks_look(false, false, false, true), MarksLook::default());
        assert_eq!(marks_look(false, false, false, false), MarksLook::default());
    }

    #[test]
    fn three_marks_bare_the_fader_link() {
        // Beside `!` and a chain it also yields on a narrow strip.
        assert_eq!(
            marks_look(true, true, true, false),
            MarksLook {
                flink_bare: true,
                flink_yield: true,
                ..MarksLook::default()
            }
        );
    }

    #[test]
    fn two_marks_keep_the_db() {
        assert_eq!(marks_look(true, false, true, false), MarksLook::default());
        assert_eq!(marks_look(false, true, true, false), MarksLook::default());
    }

    #[test]
    fn four_marks_crowd_and_the_fader_link_yields() {
        assert_eq!(
            marks_look(true, true, true, true),
            MarksLook {
                chain_bare: true,
                flink_bare: true,
                crowd: true,
                flink_yield: true,
            }
        );
    }

    #[test]
    fn a_muted_chain_drops_its_count() {
        assert_eq!(
            marks_look(false, true, false, true),
            MarksLook {
                chain_bare: true,
                ..MarksLook::default()
            }
        );
        // Three marks with the chain muted: the count goes, the fader keeps
        // nothing but its chain, nothing yields.
        assert_eq!(
            marks_look(false, true, true, true),
            MarksLook {
                chain_bare: true,
                flink_bare: true,
                ..MarksLook::default()
            }
        );
    }

    #[test]
    fn the_count_never_yields_without_a_chain() {
        assert_eq!(
            marks_look(true, false, true, true),
            MarksLook {
                flink_bare: true,
                ..MarksLook::default()
            }
        );
        // Three marks without a fader link: nothing bares or crowds.
        assert_eq!(
            marks_look(true, true, false, true),
            MarksLook {
                chain_bare: true,
                ..MarksLook::default()
            }
        );
    }
}
