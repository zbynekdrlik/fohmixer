//! The connect-time retries (#140; PR #145 review I1 and M10, the main
//! session's rulings): the song map's reads, AUTOTUNE's walks and the marker
//! keeper's reads are tried again after a failure while their Live set stays
//! connected, so a set still loading (the hub connects while Live loads it,
//! and its reads time out for a while) is read once Live answers. One bound
//! for all of them: AUTOTUNE's writes and REVERB's window wait for the
//! marker keeper's read, so its retries must last as long as the others'.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_connect_time_bound() {
        assert_eq!((RETRY_MS, CONNECT_RETRIES), (2000, 24));
    }
}
