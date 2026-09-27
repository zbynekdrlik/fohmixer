//! Build info for `fohmixer_proto::{git_hash, git_branch, build_time}`
//! (copied from iemmixer's `iem-core/build.rs` @ 22372bc).
//!
//! One change: git is asked only when it answers. Without a `.git`
//! (cargo-mutants' scratch copy, a source archive) `git rev-parse` fails with
//! an empty stdout; iemmixer then stamps an empty hash, here the hash comes
//! from CI's `GITHUB_SHA` (the same commit) or reads "unknown".

use std::process::Command;

/// `git <args>`'s trimmed stdout, if git ran, succeeded and printed something.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn main() {
    // Git commit hash (7 characters).
    let hash = git(&["rev-parse", "--short", "HEAD"])
        .or_else(|| {
            std::env::var("GITHUB_SHA")
                .ok()
                .map(|sha| sha.chars().take(7).collect())
        })
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_HASH={hash}");

    // Git branch. CI sets GITHUB_REF_NAME (the checkout is a detached HEAD,
    // so git itself would answer "HEAD").
    let branch = std::env::var("GITHUB_REF_NAME").unwrap_or_else(|_| {
        match git(&["rev-parse", "--abbrev-ref", "HEAD"]) {
            Some(name) if name != "HEAD" => name,
            _ => "unknown".to_string(),
        }
    });
    println!("cargo:rustc-env=GIT_BRANCH={branch}");

    // Build time as a unix timestamp.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_secs();
    println!("cargo:rustc-env=BUILD_TIME={now}");

    // Rebuild when git HEAD moves. Without a `.git` (cargo-mutants' scratch
    // copy, a source archive) key the rerun on the CI commit instead, so
    // BUILD_TIME does not change on every build and force a full rebuild.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let git_head = std::path::Path::new(&manifest_dir).join("../../.git/HEAD");
    if git_head.exists() {
        println!("cargo:rerun-if-changed={}", git_head.display());
    } else {
        println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    }
}
