//! Shared workspace/metadata access for the checkers, so no checker reads the
//! graph twice or invents its own root discovery.
//!
//! The rule both consumers live by: the graph comes from
//! `cargo metadata --format-version 1` (direct edges from
//! `packages[].dependencies`, never `cargo tree -i`, which is transitive and
//! lies in both directions), and the workspace root is resolved from the
//! CALLER's CWD - xtask lives inside the workspace, so its own exe path is
//! useless.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// Walk up from start to the nearest directory whose Cargo.toml declares a
/// [`workspace`] section.
pub fn find_workspace_root(start: &Path) -> Result<PathBuf, String> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        let manifest = d.join("Cargo.toml");
        if manifest.is_file() {
            let text = std::fs::read_to_string(&manifest).unwrap_or_default();
            if text.lines().any(|line| line.trim() == "[workspace]") {
                return Ok(d.to_path_buf());
            }
        }
        dir = d.parent();
    }
    Err(format!(
        "no workspace root (a Cargo.toml with a [workspace] section) found above {}",
        start.display()
    ))
}

/// Run cargo metadata for the workspace at root and parse it. `.output()`
/// drains both pipes to EOF concurrently, so the child cannot deadlock on a
/// full pipe.
///
/// `--all-features` is not decoration. An optional dependency behind a
/// non-default feature does not appear in `resolve.nodes[]` unless that
/// feature is on, so without the flag this reader is blind to exactly the
/// edges a layering rule has to see: `cargo add -p notes-core windows-sys
/// --optional --features x` would sail through today and break the rule the
/// moment anyone turns `x` on. The rules here are stated unconditionally, so
/// they are evaluated against the union of every feature combination - a
/// feature-gated edge is a real edge, because the graph carries it for as
/// long as the feature exists.
/// The exact cargo invocation. A constant so a test can hold it to the
/// contract: dropping --all-features is a silent weakening, not a refactor.
pub const METADATA_ARGS: &[&str] = &["metadata", "--format-version", "1", "--all-features"];

pub fn cargo_metadata(root: &Path) -> Result<Value, String> {
    let out = Command::new("cargo")
        .args(METADATA_ARGS)
        .current_dir(root)
        .output()
        .map_err(|e| format!("failed to spawn cargo metadata: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("cargo metadata failed: {}", stderr.trim()));
    }
    serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("cargo metadata printed invalid JSON: {e}"))
}

/// One call for the common case: resolve the root from the caller's CWD and
/// load its metadata.
pub fn load() -> Result<(PathBuf, Value), String> {
    let cwd =
        std::env::current_dir().map_err(|e| format!("cannot read the current directory: {e}"))?;
    let root = find_workspace_root(&cwd)?;
    let value = cargo_metadata(&root)?;
    Ok((root, value))
}
