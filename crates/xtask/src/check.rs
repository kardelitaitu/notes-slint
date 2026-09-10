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
    /// The step ran and DECLINED: exit 3 means "this environment cannot host
    /// this check", which is neither a pass nor a failure. Only smoke uses it -
    /// a session with no interactive desktop must not be held against anyone.
    Declined,
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
            Outcome::Declined => "DECLINED (no desktop / foreign state on the session path)",
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
            name: "check-deps",
            display: "cargo run -p xtask -- check-deps",
            program: "cargo",
            args: &["run", "-p", "xtask", "--quiet", "--", "check-deps"],
            advisory: false,
            quick_skippable: false,
            budget_secs: 180,
        },
        StepSpec {
            name: "check-ci",
            display: "cargo run -p xtask -- check-ci",
            program: "cargo",
            args: &["run", "-p", "xtask", "--quiet", "--", "check-ci"],
            // Blocking, and never --quick-skipped: it is cheap, it reads two
            // text files, and it is the row that protects every other row from
            // drifting away from ci.yml.
            advisory: false,
            quick_skippable: false,
            budget_secs: 60,
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
            name: "smoke",
            display: "cargo run -p xtask -- smoke (advisory: opens a real window on this desktop)",
            program: "cargo",
            args: &["run", "-p", "xtask", "--quiet", "--", "smoke"],
            // Advisory, never a gate: a runner with no desktop is not a broken
            // repo, and a GUI row that goes red for an environmental reason
            // trains people to ignore the row. --quick skips it too.
            advisory: true,
            quick_skippable: true,
            budget_secs: 120,
        },
        // The bridge is GATED, and by more than a type-check: CI builds the
        // exe (the link has to work) and lints every target. Raising the local
        // roster to what CI already runs is the resolution check-ci pointed at
        // - weakening CI was not on the table, and cargo check was weaker than
        // both of these.
        StepSpec {
            name: "bridge-build",
            display: "cargo build -p notes-bridge-gpui --bin notes-gpui",
            program: "cargo",
            args: &["build", "-p", "notes-bridge-gpui", "--bin", "notes-gpui"],
            advisory: false,
            quick_skippable: true,
            budget_secs: 900,
        },
        StepSpec {
            name: "bridge-clippy",
            display: "cargo clippy -p notes-bridge-gpui --all-targets -- -D warnings",
            program: "cargo",
            args: &[
                "clippy",
                "-p",
                "notes-bridge-gpui",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
            advisory: false,
            quick_skippable: true,
            budget_secs: 900,
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

/// A step exits 3 to say it DECLINED (see Outcome::Declined). It can never
/// gate: an environment without a desktop is not a broken repo.
const DECLINED_EXIT: i32 = 3;

/// The gate's own roster, in the shape check-ci compares against ci.yml. One
/// source of truth on this side: it is derived from step_specs(), so the list
/// CI is checked against cannot drift from the list check() actually runs.
pub fn roster() -> Vec<crate::check_ci::Row> {
    step_specs()
        .into_iter()
        .map(|spec| crate::check_ci::Row {
            name: spec.name,
            command: format!("{} {}", spec.program, spec.args.join(" ")),
            advisory: spec.advisory,
        })
        .collect()
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
                } else if status.code() == Some(DECLINED_EXIT) {
                    Outcome::Declined
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
    let root = match crate::metadata::find_workspace_root(&cwd) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("check: {e}");
            return 2;
        }
    };
    println!("check: workspace root {}", root.display());
    if quick {
        println!(
            "check: --quick — the workspace-wide clippy and test steps and the GUI smoke row are              SKIPPED; this is not a full verdict"
        );
    }
    let specs = step_specs();
    let total = specs.len();
    let mut verdicts: Vec<Verdict> = Vec::new();
    for (i, spec) in specs.into_iter().enumerate() {
        let outcome = if quick && spec.quick_skippable {
            Outcome::Skipped
        } else {
            println!("==> [{}/{}] {}", i + 1, total, spec.display);
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
        let names: Vec<&str> = verdicts
            .iter()
            .filter(|v| v.outcome == Outcome::Skipped)
            .map(|v| v.name)
            .collect();
        println!(
            "check: --quick pass is NOT a full pass — steps {} were skipped",
            names.join(", ")
        );
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

    /// A checker that is not in the roster cannot fail anyone, which is worse
    /// than no checker: assert the gate actually runs the in-repo checkers.
    #[test]
    fn every_in_repo_checker_is_wired_into_the_gate() {
        let names: Vec<&'static str> = step_specs().iter().map(|s| s.name).collect();
        for expected in ["check-arch", "check-deps", "check-ci", "fixtures"] {
            assert!(
                names.contains(&expected),
                "{expected} is not wired: {names:?}"
            );
        }
        let deps = step_specs()
            .into_iter()
            .find(|s| s.name == "check-deps")
            .expect("check-deps row");
        assert!(!deps.advisory, "a drift checker cannot be advisory");
        assert!(
            !deps.quick_skippable,
            "check-deps reads manifests only; --quick must not skip it"
        );
        let ci = step_specs()
            .into_iter()
            .find(|s| s.name == "check-ci")
            .expect("check-ci row");
        assert!(
            !ci.advisory,
            "the drift lock cannot be advisory - that is the hole it closes"
        );
        assert!(
            !ci.quick_skippable,
            "check-ci protects the other rows, so --quick must not skip it"
        );
        // The roster check-ci compares against must be this same list.
        let roster = roster();
        assert_eq!(
            roster.len(),
            step_specs().len(),
            "roster() must not lag step_specs()"
        );
        assert!(
            roster
                .iter()
                .any(|r| r.name == "check-ci" && r.command.contains("check-ci")),
            "{roster:?}"
        );
    }

    /// xtask is inside the workspace gate, so a step that opens a real window
    /// must not be able to run by default: it has to be advisory AND skipped by
    /// --quick, or a headless runner goes red for an environmental reason.
    #[test]
    fn the_gui_step_can_never_gate_and_never_runs_by_default() {
        let smoke = step_specs()
            .into_iter()
            .find(|s| s.name == "smoke")
            .expect("smoke row");
        assert!(smoke.advisory, "smoke opens a window; it must never gate");
        assert!(smoke.quick_skippable, "--quick must not open a window");
        assert!(
            smoke.budget_secs > 60,
            "the row budget must exceed the harness's own deadlines"
        );
        let declined_gates = Outcome::Declined.gates();
        assert!(!declined_gates, "a declined row cannot gate anything");
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
    fn a_declined_step_never_gates() {
        // exit 3 from smoke: reported, never fatal, and never a PASS either.
        let verdicts = vec![
            v("fmt", Outcome::Pass, false),
            v("smoke", Outcome::Declined, true),
        ];
        assert_eq!(exit_code(&verdicts), 0);
        assert!(!Outcome::Declined.gates(), "a decline is not a failure");
        assert!(
            !Outcome::Declined.label(true).contains("PASS"),
            "a decline must not read as a pass"
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
