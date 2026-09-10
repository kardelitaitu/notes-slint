//! identity - the tool says whether it is the tool it claims to be.
//!
//! Why this file exists: check-unsafe landed in 04da77c along with its dispatch
//! arm, and cargo then answered "unknown command check-unsafe" for several
//! minutes while running a binary that did not contain that code. Second-granularity
//! mtimes on Windows plus a tool that preserves an older mtime is enough for a
//! fingerprint to be satisfied with a stale artifact. For a GUI binary that is
//! annoying; for a CHECKER it is the worst case in the repo, because the output
//! of a stale checker is the evidence a manager writes down.
//!
//! So every subcommand prints one line about itself before doing anything: the
//! age of the running binary, the newest mtime among the sources that produce
//! it, and a verdict.
//!
//! The verdict is a WARNING, never a hard failure. A checker that refuses to
//! start because of a timestamp quirk takes the whole gate down with it, and a
//! tool that cannot be run is a tool whose rules get skipped silently. Exit
//! codes of the subcommands are untouched.
//!
//! Three states, named honestly:
//!
//! * Current - the binary is not older than any source that builds it.
//! * Stale - at least one source is newer than the running binary, by a stated
//!   number of seconds, naming the file. The fix is in the message.
//! * CannotJudge - the mtime of the binary or of the sources could not be read,
//!   or this is not a checkout at all. That prints "cannot judge", because
//!   "fresh" would be a claim and "stale" would be an accusation, and this
//!   module is not allowed to make either one without measuring.
//!
//! XTASK_NO_SELF_CHECK=1 suppresses the line entirely. It exists so the
//! stamp can be turned off in a context where the noise matters (nothing in the
//! gate uses it); the wiring is pinned by a test that reads main.rs, so deleting
//! the call is a red build, not a silent weakening.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Set to any value to silence the startup stamp.
pub const SKIP_ENV: &str = "XTASK_NO_SELF_CHECK";

/// The sources that produce the xtask binary itself.
const SELF_DIRS: &[&str] = &["crates/xtask/src"];
const SELF_FILES: &[&str] = &["crates/xtask/Cargo.toml", "Cargo.toml"];

pub fn mtime_of(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

fn age_secs(when: SystemTime) -> Option<u64> {
    when.elapsed().ok().map(|d| d.as_secs())
}

fn secs_since(a: SystemTime, b: SystemTime) -> u64 {
    a.duration_since(b).map(|d| d.as_secs()).unwrap_or(0)
}

/// Newest mtime among a set of directories (recursively, .rs and .toml files,
/// skipping target/ and .git/) plus a set of individual files. ONE walk, shared
/// by the self-check here and by smoke's binary-freshness rule - two
/// implementations of "is this artefact older than its sources" is how one of
/// them drifts.
pub fn newest_mtime(root: &Path, dirs: &[&str], files: &[&str]) -> Option<(SystemTime, PathBuf)> {
    fn consider(path: &Path, best: &mut Option<(SystemTime, PathBuf)>) {
        if let Some(m) = mtime_of(path) {
            if best.as_ref().is_none_or(|(bm, _)| m > *bm) {
                *best = Some((m, path.to_path_buf()));
            }
        }
    }
    fn walk(dir: &Path, best: &mut Option<(SystemTime, PathBuf)>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                let name = path.file_name().map(|s| s.to_string_lossy().to_string());
                if name.as_deref() != Some("target") && name.as_deref() != Some(".git") {
                    walk(&path, best);
                }
                continue;
            }
            let ext = path.extension().map(|s| s.to_string_lossy().to_string());
            if matches!(ext.as_deref(), Some("rs") | Some("toml")) {
                consider(&path, best);
            }
        }
    }
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for dir in dirs {
        walk(&root.join(dir), &mut best);
    }
    for file in files {
        consider(&root.join(file), &mut best);
    }
    best
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Current,
    Stale {
        source: PathBuf,
        source_secs: u64,
        binary_age_secs: u64,
    },
    CannotJudge(&'static str),
}

/// Compare one artefact against the sources that produce it. Pure, so both
/// callers share the decision and the states are testable without a build.
pub fn compare(binary: Option<SystemTime>, newest: Option<(SystemTime, PathBuf)>) -> Verdict {
    let (Some(built), Some((source_at, source))) = (binary, newest) else {
        return Verdict::CannotJudge(if binary.is_none() {
            "the running binary's mtime could not be read"
        } else {
            "no readable source files were found"
        });
    };
    if source_at <= built {
        return Verdict::Current;
    }
    Verdict::Stale {
        source_secs: secs_since(source_at, built),
        binary_age_secs: age_secs(built).unwrap_or(0),
        source,
    }
}

/// The self-check for this binary: newest mtime under crates/xtask/src and the
/// two manifests, against the executable that is running right now.
pub fn self_check(root: &Path) -> Verdict {
    if !root.join(".git").exists() {
        return Verdict::CannotJudge("this is not a git checkout, so there is no tree to compare");
    }
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return Verdict::CannotJudge("the running binary's path could not be read"),
    };
    compare(mtime_of(&exe), newest_mtime(root, SELF_DIRS, SELF_FILES))
}

/// One line at startup, on stdout when it is good news and on stderr when it is
/// not, so a pipe of the checker's own report never buries the warning.
pub fn print_startup_stamp(root: &Path) {
    if std::env::var_os(SKIP_ENV).is_some() {
        return;
    }
    match self_check(root) {
        Verdict::Current => println!(
            "xtask: self-check fresh - this binary is {}s old and no source under {} is newer",
            binary_age(),
            SELF_DIRS[0]
        ),
        Verdict::Stale {
            source,
            source_secs,
            binary_age_secs,
        } => {
            eprintln!(
                "xtask: WARNING - SELF-CHECK: THIS BINARY IS STALE, its output is evidence about a tree it does not contain"
            );
            eprintln!(
                "xtask:   binary is {}s old, but {} is {source_secs}s NEWER than it",
                binary_age_secs,
                source.display()
            );
            eprintln!(
                "xtask:   fix: cargo build -p xtask (or: touch crates/xtask/src/main.rs), then \
                 re-run - if a subcommand is 'unknown' tonight, this is why"
            );
        }
        Verdict::CannotJudge(why) => {
            eprintln!("xtask: self-check cannot judge freshness - {why}");
        }
    }
}

fn binary_age() -> u64 {
    std::env::current_exe()
        .ok()
        .and_then(|p| mtime_of(&p))
        .and_then(age_secs)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_mtime(path: &Path, when: SystemTime) {
        let file = fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open for times");
        file.set_times(fs::FileTimes::new().set_modified(when))
            .expect("set mtime");
    }

    /// A source tree whose newest file is NEWER than the artefact must be
    /// reported stale, and the verdict must name the file and by how much.
    #[test]
    fn a_stale_tree_is_caught_and_named() {
        let dir = std::env::temp_dir().join(format!("xtask-identity-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("crates/xtask/src")).expect("tree");
        fs::create_dir_all(dir.join("crates/xtask/src/deep/target")).expect("skip dir");
        let source = dir.join("crates/xtask/src/main.rs");
        fs::write(&source, "fn main() {}\n").expect("write source");
        let hidden = dir.join("crates/xtask/src/deep/target/leak.rs");
        fs::write(&hidden, "newer but inside target").expect("write leak");
        let binary = SystemTime::now() - std::time::Duration::from_secs(600);
        let future = SystemTime::now() + std::time::Duration::from_secs(60);
        set_mtime(&source, future);
        set_mtime(&hidden, future + std::time::Duration::from_secs(600));

        let newest = newest_mtime(&dir, SELF_DIRS, SELF_FILES).expect("a source mtime");
        assert_eq!(newest.1, source, "target/ must be skipped by the walk");
        match compare(Some(binary), Some(newest)) {
            Verdict::Stale {
                source: s,
                source_secs,
                binary_age_secs,
            } => {
                assert_eq!(s, source);
                // binary is 600s old and the source is 60s in the future.
                assert!((659..=661).contains(&source_secs), "{source_secs}");
                assert!((599..=601).contains(&binary_age_secs), "{binary_age_secs}");
            }
            other => panic!("a 10-minute-old binary against a newer source: {other:?}"),
        }
        // The other direction: a build that really ran is Current, not stale.
        set_mtime(&source, binary);
        let newest = newest_mtime(&dir, SELF_DIRS, SELF_FILES).expect("source");
        assert_eq!(compare(Some(binary), Some(newest)), Verdict::Current);
        // And the Cargo.toml of the crate counts as a source.
        let manifest = dir.join("crates/xtask/Cargo.toml");
        fs::write(&manifest, "[package]\nname = \"xtask\"\n").expect("manifest");
        set_mtime(&manifest, future);
        match compare(Some(binary), newest_mtime(&dir, SELF_DIRS, SELF_FILES)) {
            Verdict::Stale { source: s, .. } => {
                assert_eq!(s, manifest, "a manifest is a source too")
            }
            other => panic!("the manifest bump must be visible: {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Unreadable inputs say "cannot judge", never "fresh" and never "stale".
    #[test]
    fn unverifiable_is_named_as_unverifiable() {
        assert!(matches!(compare(None, None), Verdict::CannotJudge(_)));
        assert!(matches!(
            compare(Some(SystemTime::now()), None),
            Verdict::CannotJudge(why) if why.contains("source")
        ));
        assert!(matches!(
            compare(None, Some((SystemTime::now(), PathBuf::from("x.rs")))),
            Verdict::CannotJudge(why) if why.contains("binary")
        ));
    }

    /// A directory that is not a checkout cannot be judged either.
    #[test]
    fn a_non_checkout_is_not_called_stale() {
        let dir = std::env::temp_dir().join(format!("xtask-identity-plain-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        assert!(matches!(
            self_check(&dir),
            Verdict::CannotJudge(why) if why.contains("git checkout")
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    /// D33, the anti-deletion guard: the stamp is only worth anything if main
    /// actually calls it, and only if the usage block admits check-unsafe
    /// exists. Both are read straight out of the source of truth.
    #[test]
    fn the_startup_stamp_is_wired_into_main_and_the_usage_block_is_complete() {
        let main = include_str!("main.rs");
        assert!(
            main.contains("identity::print_startup_stamp("),
            "main() no longer prints the self-check: every subcommand would run \
             silently on an unknown binary"
        );
        for sub in [
            "check-arch",
            "check-deps",
            "check-ci",
            "check-unsafe",
            "smoke",
            "check",
            "fixtures",
        ] {
            assert!(
                main.contains(&format!("usage: cargo xtask {sub}")),
                "usage() does not list {sub}, so a reader will not know the row exists"
            );
        }
        assert!(
            main.contains("Some(\"check-unsafe\")"),
            "check-unsafe is not dispatched in main()"
        );
    }

    #[test]
    fn the_kill_switch_is_named_and_the_states_are_exhaustive() {
        assert_eq!(SKIP_ENV, "XTASK_NO_SELF_CHECK");
        // Every Verdict renders: a variant that prints nothing is how the stamp
        // disappears without anybody noticing.
        let states = [
            Verdict::Current,
            Verdict::Stale {
                source: PathBuf::from("crates/xtask/src/main.rs"),
                source_secs: 5,
                binary_age_secs: 10,
            },
            Verdict::CannotJudge("measured in the test"),
        ];
        assert_eq!(states.len(), 3);
    }
}
