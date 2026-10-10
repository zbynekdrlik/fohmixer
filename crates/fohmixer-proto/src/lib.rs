//! fohmixer protocol crate: the types shared by the hub and the UI.
//!
//! WASM-safe (no config, no I/O): the build and version info (copied from
//! iemmixer's `iem-core` @ 22372bc), the hub ⇄ client protocol
//! ([`client`], spec §2.4; the Pro-Q 4 screen's pieces, [`eq`], F28), the
//! layout document ([`layout`], spec §2.5), the Tuner markers that fill it
//! ([`markers`], D16) and the LOM path grammar both use ([`path`], S2 design
//! note §3.1).

use serde::{Deserialize, Serialize};

pub mod client;
pub mod eq;
pub mod layout;
pub mod markers;
pub mod path;

/// Application version: the workspace's `[workspace.package].version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Git commit hash at build time (7 characters), or "unknown".
pub fn git_hash() -> &'static str {
    option_env!("GIT_HASH").unwrap_or("unknown")
}

/// Git branch at build time (CI: `GITHUB_REF_NAME`), or "unknown".
pub fn git_branch() -> &'static str {
    option_env!("GIT_BRANCH").unwrap_or("unknown")
}

/// Build timestamp (unix seconds), or "0" when it was not set.
pub fn build_time() -> &'static str {
    option_env!("BUILD_TIME").unwrap_or("0")
}

/// Full version string for display (build time in UTC), e.g.
/// "0.1.0-dev.1 (24.09.2025 09:45)" for `BUILD_TIME` 1758707100.
pub fn full_version() -> String {
    full_version_at(build_time())
}

/// [`full_version`] for a `BUILD_TIME` value (unix seconds; "0" or
/// unparsable means a local build).
fn full_version_at(build_time: &str) -> String {
    let timestamp = build_time.parse::<i64>().unwrap_or(0);
    if timestamp == 0 {
        format!("{VERSION} (local)")
    } else {
        let datetime = chrono::DateTime::from_timestamp(timestamp, 0)
            .map(|dt| dt.format("%d.%m.%Y %H:%M").to_string())
            .unwrap_or_else(|| "unknown".to_string());
        format!("{VERSION} ({datetime})")
    }
}

/// Version label for display: `v` + the Cargo version (pre-releases already
/// carry `-dev.N`), e.g. "v0.1.0-dev.1".
pub fn version_label() -> String {
    format!("v{VERSION}")
}

/// What `GET /api/version` answers: the build the hub runs, for deploy
/// verification and the UI's version check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionInfo {
    pub version: String,
    pub git_hash: String,
    pub branch: String,
    pub build_time: String,
}

/// This build's [`VersionInfo`].
pub fn version_info() -> VersionInfo {
    VersionInfo {
        version: VERSION.to_string(),
        git_hash: git_hash().to_string(),
        branch: git_branch().to_string(),
        build_time: build_time().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_label_is_v_plus_the_cargo_version() {
        assert!(version_label().starts_with('v'));
        assert_eq!(version_label(), format!("v{}", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn version_info_carries_this_build() {
        let info = version_info();
        assert_eq!(info.version, VERSION);
        assert_eq!(info.git_hash, git_hash());
        assert_eq!(info.branch, git_branch());
        assert_eq!(info.build_time, build_time());
    }

    #[test]
    fn version_info_round_trips_through_json() {
        let info = version_info();
        let json = serde_json::to_string(&info).unwrap();
        let back: VersionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back, info);
        let fields: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(fields["version"], VERSION);
        assert_eq!(fields["git_hash"], git_hash());
        assert_eq!(fields["branch"], git_branch());
        assert_eq!(fields["build_time"], build_time());
    }

    #[test]
    fn git_hash_is_a_short_hex_hash_or_unknown() {
        let hash = git_hash();
        assert!(
            hash == "unknown" || (hash.len() >= 7 && hash.bytes().all(|b| b.is_ascii_hexdigit())),
            "{hash:?}"
        );
    }

    #[test]
    fn git_branch_is_named_and_is_ci_s_ref_in_ci() {
        let branch = git_branch();
        assert!(!branch.is_empty());
        // CI builds and tests in the same job: build.rs took this branch name.
        if let Ok(ci_ref) = std::env::var("GITHUB_REF_NAME") {
            assert_eq!(branch, ci_ref);
        }
    }

    #[test]
    fn build_time_is_the_unix_time_of_the_build() {
        let secs: u64 = build_time().parse().expect("BUILD_TIME is unix seconds");
        // After 2026-01-01: build.rs stamps the real clock, never "0".
        assert!(secs > 1_767_225_600, "{secs}");
    }

    #[test]
    fn full_version_starts_with_the_cargo_version() {
        assert!(full_version().starts_with(&format!("{VERSION} (")));
        assert_eq!(full_version(), full_version_at(build_time()));
    }

    #[test]
    fn full_version_names_a_local_build() {
        assert_eq!(full_version_at("0"), format!("{VERSION} (local)"));
        assert_eq!(
            full_version_at("not a number"),
            format!("{VERSION} (local)")
        );
    }

    #[test]
    fn full_version_shows_the_utc_build_time() {
        assert_eq!(
            full_version_at("1758707100"),
            format!("{VERSION} (24.09.2025 09:45)")
        );
    }

    #[test]
    fn full_version_of_an_out_of_range_time_is_unknown() {
        assert_eq!(
            full_version_at(&i64::MAX.to_string()),
            format!("{VERSION} (unknown)")
        );
    }
}
