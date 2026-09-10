//! check — the one command AGENTS.md says to run, with per-step verdicts.
//!
//! AGENTS.md "Before reporting work done" lists the commands; this runs them
//! IN ORDER, streams each child's output as it happens (stdio is inherited,
//! so the child's exit code stays authoritative — nothing is parsed), and
//! prints one verdict table at the end, because a person mid-edit wants the
//! whole picture, not fail-fast.
//!
//! Exit codes follow check-arch's convention: 0 everything green, 1 a
//! non-advisory step failed (or timed out), 2 a non-advisory step could not
//! be run at all. Advisory steps (the bridge check, the docs validator) are
//! REPORTED, never gate: the bridge is excluded from the workspace gate by
//! design (D1), and the docs validator is red on a clean checkout while the
//! docs split is uncommitted. Every child gets a wall-clock budget; a
//! timeout is its own outcome and fails the run if the step is a gate.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// How a step ended. Timeout and CouldNotRun are distinct outcomes so the
/// table can say WHAT happened, not just that something went wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail,
    Timeout,
    CouldNotRun,
    Skipped,
}

impl Outcome {
    fn label(self, advisory: bool) -> &'static str {
        match self {
            Outcome::Pass => "PASS",
            Outcome::Fail if advisory => "FAIL (advisory — does not gate)",
            Outcome::Fail => "FAIL",
            Outcome::Timeout if advisory => "TIMEOUT (advisory — does not gate)",
            Outcome::Timeout => "TIMEOUT",
            Outcome::CouldNotRun if advisory => "COULD NOT RUN (advisory — does not gate)",
            Outcome::CouldNotRun => "COULD NOT RUN",
            Outcome::Skipped => "SKIPPED (--quick)",
        }
    }

    fn gates(self) -> bool {
        matches!(
            self,
            Outcome::Fail | Outcome::Timeout | Outcome::CouldNotRun
        )
    }
}

/// One row of the verdict table. The aggregation is a pure function over
/// these, so the verdict math is testable without spawning cargo.
pub struct Verdict {
    pub name: &'static str,
    pub outcome: Outcome,
    pub advisory: bool,
}

/// The pure part: which exit code does this set of verdicts produce?
/// 2 beats 1 beats 0; advisory rows never contribute.
pub fn exit_code(verdicts: &[Verdict]) -> i32 {
    if verdicts
        .iter()
        .any(|v| !v.advisory && v.outcome == Outcome::CouldNotRun)
    {
        return 2;
    }
    if verdicts.iter().any(|v| !v.advisory && v.outcome.gates()) {
        return 1;
    }
    0
}

struct StepSpec {
    name: &'static str,
    display: &'static str,
    program: &'static str,
    args: &'static [&'static str],
    advisory: bool,
    /// --quick skips the slow workspace-wide steps and prints that it did.
    quick_skippable: bool,
    budget_secs: u64,
}

fn step_specs() -> Vec<StepSpec> {
    vec![
        StepSpec {
            name: "fmt",
            display: "cargo fmt --all -- --check",
            program: "cargo",
            args: &["fmt", "--all", "--", "--check"],
            advisory: false,
            quick_skippable: false,
            budget_secs: 120,
        },
        StepSpec {
            name: "clippy",
            display: "cargo clippy --workspace --exclude notes-bridge-gpui --all-targets -- -D warnings",
            program: "cargo",
            args: &[
                "clippy",
                "--workspace",
                "--exclude",
                "notes-bridge-gpui",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
            advisory: false,
            quick_skippable: true,
            budget_secs: 600,
        },
        StepSpec {
            name: "test",
            display: "cargo test --workspace --exclude notes-bridge-gpui",
            program: "cargo",
            args: &["test", "--workspace", "--exclude", "notes-bridge-gpui"],
            advisory: false,
            quick_skippable: true,
            budget_secs: 900,
        },
        StepSpec {
            name: "check-arch",
            display: "cargo run -p xtask -- check-arch",
            program: "cargo",
            args: &["run", "-p", "xtask", "--quiet", "--", "check-arch"],
            advisory: false,
            quick_skippable: false,
            budget_secs: 180,
        },
        StepSpec {
            name: "fixtures",
            display: "cargo run -p xtask -- fixtures verify",
            program: "cargo",
            args: &["run", "-p", "xtask", "--quiet", "--", "fixtures", "verify"],
            advisory: false,
            quick_skippable: false,
            budget_secs: 180,
        },
        StepSpec {
            name: "bridge",
            display: "cargo check -p notes-bridge-gpui (advisory: excluded from the gate by design, D1)",
            program: "cargo",
            args: &["check", "-p", "notes-bridge-gpui"],
            advisory: true,
            quick_skippable: false,
            budget_secs: 600,
        },
        StepSpec {
            name: "docs",
            display: "pwsh .agents/skills/doc-management/scripts/check-docs.ps1 (advisory: red while the docs split is uncommitted)",
            program: "pwsh",
            args: &[".agents/skills/doc-management/scripts/check-docs.ps1"],
            advisory: true,
            quick_skippable: false,
            budget_secs: 120,
        },
    ]
}

/// Spawn the child with inherited stdio (its output streams; its exit code is
/// the truth) and enforce a wall-clock budget. A deadlocked test suite must
/// come back as Timeout, not hang the gate forever.
fn run_child(spec: &StepSpec, root: &Path) -> Outcome {
    let child = Command::new(spec.program)
        .args(spec.args)
        .current_dir(root)
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(e) => {
            eprintln!("xtask check: could not spawn '{}': {e}", spec.display);
            return Outcome::CouldNotRun;
        }
    };
    let deadline = Instant::now() + Duration::from_secs(spec.budget_secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Outcome::Pass
                } else {
                    Outcome::Fail
                };
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    eprintln!(
                        "xtask check: '{}' exceeded its {}s budget and was killed",
                        spec.display, spec.budget_secs
                    );
                    return Outcome::Timeout;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                eprintln!("xtask check: waiting on '{}' failed: {e}", spec.display);
                return Outcome::CouldNotRun;
            }
        }
    }
}

/// Entry point for "cargo xtask check [--quick]". Runs every step (never
/// fail-fast), streams output, prints the table, returns the exit code.
pub fn run(quick: bool) -> i32 {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("check: cannot read the current directory: {e}");
            return 2;
        }
    };
    let root = match crate::find_workspace_root(&cwd) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("check: {e}");
            return 2;
        }
    };
    println!("check: workspace root {}", root.display());
    if quick {
        println!(
            "check: --quick — the workspace-wide clippy and test steps are SKIPPED; this is not a full verdict"
        );
    }
    let mut verdicts: Vec<Verdict> = Vec::new();
    for (i, spec) in step_specs().into_iter().enumerate() {
        let outcome = if quick && spec.quick_skippable {
            Outcome::Skipped
        } else {
            println!("==> [{}/{}] {}", i + 1, 7, spec.display);
            run_child(&spec, &root)
        };
        println!(
            "<== [{}] {}
",
            spec.name,
            outcome.label(spec.advisory)
        );
        verdicts.push(Verdict {
            name: spec.name,
            outcome,
            advisory: spec.advisory,
        });
    }

    println!(
        "check: verdict table{}",
        if quick { " (--quick)" } else { "" }
    );
    for (i, v) in verdicts.iter().enumerate() {
        println!(
            "  {}. {:<12} {}",
            i + 1,
            v.name,
            v.outcome.label(v.advisory)
        );
    }
    let gate_failures = verdicts
        .iter()
        .filter(|v| !v.advisory && v.outcome.gates())
        .count();
    let advisory_failures = verdicts
        .iter()
        .filter(|v| v.advisory && v.outcome.gates())
        .count();
    let skipped = verdicts
        .iter()
        .filter(|v| v.outcome == Outcome::Skipped)
        .count();
    let code = exit_code(&verdicts);
    println!(
        "check: {gate_failures} gate failures, {advisory_failures} advisory failures, {skipped} skipped -> exit {code}"
    );
    if quick && code == 0 {
        println!("check: --quick pass is NOT a full pass — steps clippy and test were skipped");
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(name: &'static str, outcome: Outcome, advisory: bool) -> Verdict {
        Verdict {
            name,
            outcome,
            advisory,
        }
    }

    #[test]
    fn all_green_exits_zero() {
        let verdicts = vec![
            v("fmt", Outcome::Pass, false),
            v("clippy", Outcome::Pass, false),
            v("test", Outcome::Pass, false),
            v("check-arch", Outcome::Pass, false),
            v("fixtures", Outcome::Pass, false),
            v("bridge", Outcome::Pass, true),
            v("docs", Outcome::Pass, true),
        ];
        assert_eq!(exit_code(&verdicts), 0);
    }

    #[test]
    fn one_gate_failure_exits_one() {
        let verdicts = vec![
            v("fmt", Outcome::Pass, false),
            v("clippy", Outcome::Fail, false),
            v("test", Outcome::Pass, false),
        ];
        assert_eq!(exit_code(&verdicts), 1);
    }

    #[test]
    fn a_timeout_fails_a_gate_step() {
        let verdicts = vec![
            v("test", Outcome::Timeout, false),
            v("docs", Outcome::Pass, true),
        ];
        assert_eq!(exit_code(&verdicts), 1);
    }

    #[test]
    fn advisory_failures_never_gate() {
        let verdicts = vec![
            v("fmt", Outcome::Pass, false),
            v("bridge", Outcome::Fail, true),
            v("docs", Outcome::Fail, true),
        ];
        assert_eq!(
            exit_code(&verdicts),
            0,
            "a red docs validator on a clean checkout must not fail the gate"
        );
    }

    #[test]
    fn gate_failure_beats_nothing_but_could_not_run_beats_failure() {
        let failed = vec![v("test", Outcome::Fail, false)];
        assert_eq!(exit_code(&failed), 1);
        let both = vec![
            v("test", Outcome::Fail, false),
            v("fmt", Outcome::CouldNotRun, false),
        ];
        assert_eq!(
            exit_code(&both),
            2,
            "could-not-run is the more severe condition"
        );
    }

    #[test]
    fn an_advisory_step_that_cannot_run_does_not_change_the_exit_code() {
        let verdicts = vec![
            v("fmt", Outcome::Pass, false),
            v("docs", Outcome::CouldNotRun, true),
        ];
        assert_eq!(exit_code(&verdicts), 0);
        let verdicts = vec![
            v("fmt", Outcome::Fail, false),
            v("docs", Outcome::CouldNotRun, true),
        ];
        assert_eq!(
            exit_code(&verdicts),
            1,
            "the gate failure still decides: 1, not 2"
        );
    }

    #[test]
    fn skipped_steps_are_neither_pass_nor_failure() {
        let verdicts = vec![
            v("fmt", Outcome::Pass, false),
            v("clippy", Outcome::Skipped, false),
            v("test", Outcome::Skipped, false),
            v("check-arch", Outcome::Pass, false),
            v("fixtures", Outcome::Pass, false),
        ];
        assert_eq!(exit_code(&verdicts), 0);
        let verdicts = vec![
            v("fmt", Outcome::Pass, false),
            v("clippy", Outcome::Skipped, false),
            v("test", Outcome::Skipped, false),
            v("check-arch", Outcome::Fail, false),
        ];
        assert_eq!(
            exit_code(&verdicts),
            1,
            "--quick still reports real failures"
        );
    }

    #[test]
    fn labels_distinguish_advisory_from_gate() {
        assert_eq!(Outcome::Fail.label(false), "FAIL");
        assert_eq!(Outcome::Fail.label(true), "FAIL (advisory — does not gate)");
        assert_eq!(Outcome::Skipped.label(false), "SKIPPED (--quick)");
        assert_eq!(Outcome::Timeout.label(false), "TIMEOUT");
    }
}
