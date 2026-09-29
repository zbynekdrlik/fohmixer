//! The SOLO ✕ pill (#21): shown while any solo of the page is on; a tap
//! turns off every solo that Live reports on (and only those).

/// The solos a tap turns off: the indices of those Live reports on.
pub fn soloed(states: &[Option<bool>]) -> Vec<usize> {
    states
        .iter()
        .enumerate()
        .filter_map(|(i, s)| (*s == Some(true)).then_some(i))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_solos_live_reports_on_are_cleared() {
        assert_eq!(
            soloed(&[Some(true), Some(false), None, Some(true)]),
            vec![0, 3]
        );
        assert_eq!(soloed(&[Some(false), None]), Vec::<usize>::new());
        assert_eq!(soloed(&[]), Vec::<usize>::new());
    }
}
