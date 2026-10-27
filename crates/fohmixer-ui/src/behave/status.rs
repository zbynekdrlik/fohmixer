//! The strip's status light (spec F5, I5; #151, the owner's ruling of
//! 27.10: no blinking): red while the strip is not bound; while bound it is
//! not drawn at all. TouchOSC's yellow flash on every value is gone.

/// The light's `data-state`: `bound` (not drawn) or `unbound` (red).
pub fn status_state(bound: bool) -> &'static str {
    if bound { "bound" } else { "unbound" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bound_is_hidden_unbound_is_red() {
        assert_eq!(status_state(true), "bound");
        assert_eq!(status_state(false), "unbound");
    }
}
