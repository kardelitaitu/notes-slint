//! xtask: repository automation, run through the "cargo xtask" alias.

mod arch;
mod fixtures;

use std::path::{Path, PathBuf};

/// Walk up from start to the nearest directory whose Cargo.toml declares a
/// [workspace] section. xtask lives inside the workspace, so its own exe path
/// is useless — the caller's CWD is the anchor. Shared by arch and fixtures.
pub(crate) fn find_workspace_root(start: &Path) -> Result<PathBuf, String> {
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

fn usage() {
    eprintln!("usage: cargo xtask check-arch");
    eprintln!(
        "       enforce the workspace layering rules via cargo metadata (exit 1 on a violation)"
    );
    eprintln!("usage: cargo xtask fixtures generate|verify");
    eprintln!(
        "       write / check the byte-exact round-trip fixtures (verify exits 1 on any difference)"
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check-arch") => std::process::exit(arch::run()),
        Some("fixtures") => match args.get(1).map(String::as_str) {
            Some("generate") => std::process::exit(fixtures::run_generate()),
            Some("verify") => std::process::exit(fixtures::run_verify()),
            _ => {
                eprintln!("xtask: fixtures needs 'generate' or 'verify'");
                usage();
                std::process::exit(2);
            }
        },
        Some(command) => {
            eprintln!("xtask: unknown command '{command}'");
            usage();
            std::process::exit(2);
        }
        None => {
            usage();
            std::process::exit(2);
        }
    }
}
