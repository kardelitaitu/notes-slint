//! PURITY PINS for the rules the dependency graph cannot see. check-arch
//! reads cargo metadata; a hand-declared extern block creates no dependency
//! edge, an isize holding an OS handle is just a number to it, and
//! forbid(unsafe_code) does not fire on an extern block at all. These pins
//! are the grep-shaped backstop.

use std::path::Path;

// Per-target unused-crate shims: this file greps sources as text, so it
// imports nothing anyone would guess it does not need.
use notes_core as _;
use serde as _;
use serde_json as _;
use tempfile as _;
use thiserror as _;
use toml as _;

fn source(name: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name))
        .unwrap_or_else(|e| panic!("core source {name} must be readable: {e}"))
}

/// The token gate over the four files that hold state and identity. Reason
/// this is a grep and not a type check: check-arch reads cargo metadata and
/// CANNOT SEE a type or a hand-declared extern block, so an isize holding an
/// OS handle in core would be invisible to the only judge policing that
/// crate. The one tolerated spelling is ERROR_HANDLE_DISK_FULL in save.rs —
/// a Windows error-CODE constant whose NAME contains HANDLE, not a handle.
#[test]
fn core_never_mentions_an_os_handle() {
    for name in ["recent.rs", "document.rs", "save.rs", "settings.rs"] {
        let src = source(name);
        for line in src
            .lines()
            .filter(|l| !l.contains("ERROR_HANDLE_DISK_FULL"))
        {
            for token in ["isize", "HANDLE", "as_raw_handle", "os::windows"] {
                assert!(
                    !line.contains(token),
                    "{name} mentions an OS handle ({token:?}): {line} — core handles bytes and paths, never handles"
                );
            }
        }
    }
}

/// FORBID(UNSAFE_CODE) IS CARRIED: core's Cargo.toml sets unsafe_code =
/// "forbid", and this pin documents the reason the lint alone is not the
/// whole story — an extern block needs no unsafe keyword. If this test ever
/// fails, someone introduced the exact hole the lint cannot catch.
#[test]
fn core_carries_forbid_unsafe_code() {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .unwrap_or_else(|e| panic!("core Cargo.toml must be readable: {e}"));
    assert!(
        manifest.contains("unsafe_code = \"forbid\""),
        "core must keep forbid(unsafe_code) — it is half of the two-lock defence with the token pin above"
    );
}
