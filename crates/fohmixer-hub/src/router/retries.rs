//! The connect-time retries (#140; PR #145 review I1 and M10, the main
//! session's rulings): the song map's reads, AUTOTUNE's walks and the marker
//! keeper's reads are tried again after a failure while their Live set stays
//! connected, so a set still loading (the hub connects while Live loads it,
//! and its reads time out for a while) is read once Live answers. One bound
//! for all of them: AUTOTUNE's writes and REVERB's window wait for the
//! marker keeper's read, so its retries must last as long as the others'.
//!
//! Only a transient failure is tried again ([`transient`], review M2): Live
//! did not answer in time, or the instance is offline. A definite refusal
//! (a path that does not resolve, no such property, a request the script
//! refused) is shown and waits for the next trigger. A retry that falls due
//! while Live is busy waits, uncounted, and asks Live nothing ([`due`],
//! review M1).

use super::Router;
use crate::live::client::LiveError;

/// The wait after a failed read before its retry, in ms.
pub(super) const RETRY_MS: u64 = 2000;

/// How many retries in a row a failed connect-time read gets before it
/// waits for the next trigger (#140, the PC on 24.10.2026: the hub connected
/// while the band Live was still loading its set, and every read for a
/// while timed out). Each try costs at most the 3 s answer timeout
/// (`live::client::REQUEST_TIMEOUT`) plus [`RETRY_MS`], so 24 retries cover
/// about two minutes: over twice the 54 s the PC's band Live took from its
/// start to a responding window.
pub(super) const CONNECT_RETRIES: u32 = 24;

/// Whether a read's failure is worth a retry (PR #145 review M2, the main
/// session's ruling): Live did not answer within the answer timeout, or the
/// instance is offline (`LiveError`'s own words, as every call's failure
/// reaches the router). Anything else is a definite refusal: the script's
/// words for a slot (a path that does not resolve, no such property, a
/// stale ref) or for the whole request.
pub(super) fn transient(error: &str) -> bool {
    error == LiveError::Timeout.to_string() || error == LiveError::Offline.to_string()
}

/// What a retry that falls due does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Due {
    /// It reads now (a counted retry).
    Read,
    /// It waits another [`RETRY_MS`], uncounted, and asks Live nothing.
    Wait,
}

/// What a due retry does (PR #145 review M1, the main session's ruling):
/// while its instance is busy (`live::busy_reason`: Live's main-thread tick
/// late or its heartbeats overdue, as while it loads a set) a read would
/// only wait in the script's queue behind the stall, time out, and still
/// cost the script its work once Live runs again; so the retry waits, not
/// counted, and reads once Live is ready.
pub(super) fn due(busy: bool) -> Due {
    if busy { Due::Wait } else { Due::Read }
}

impl Router {
    /// What `instance`'s due retry does now ([`due`] on its busy state, as
    /// the instance's heartbeats tell it).
    pub(super) fn retry_due(&self, instance: &str) -> Due {
        due(self.states.get(instance).is_some_and(|state| state.busy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_connect_time_bound() {
        assert_eq!((RETRY_MS, CONNECT_RETRIES), (2000, 24));
    }

    #[test]
    fn a_retry_due_while_live_is_busy_waits() {
        assert_eq!(due(true), Due::Wait);
        assert_eq!(due(false), Due::Read);
    }

    #[test]
    fn only_a_timeout_or_an_offline_live_is_transient() {
        assert!(transient("no result within 3 s"));
        assert!(transient("instance offline"));
        assert!(!transient("refused: unknown command"));
        assert!(!transient("live error: busy"));
        assert!(!transient("cue_points: no such property"));
        assert!(!transient(
            "live_set tracks 3 arrangement_clips: deleted: live_77"
        ));
        assert!(!transient("no result within 3 s, and more"));
        assert!(!transient(""));
    }
}
