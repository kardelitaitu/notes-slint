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
//! be run at all. The advisory steps (smoke, docs) are REPORTED, never gate: a
//! runner with no desktop is not a broken repo, and the docs validator is red on
//! a clean checkout while the docs split is uncommitted.
//!
//! Do not read the workspace rows as the bridge gate: `--exclude notes-bridge-gpui`
//! removes ONE member from those two invocations, and it was never a statement
//! about either bridge being ungated. Both bridges are GATED, each by a named row -
//! gpui by bridge-build/bridge-clippy (its own steps, because the exclusion takes it
//! out of the workspace pair) and slint by slint-build/slint-clippy for the LINK and
//! the LINT, with its tests riding the workspace `clippy`/`test` rows it is not
//! excluded from. Every child gets a wall-clock budget; a
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
            display: "cargo clippy --locked --workspace --exclude notes-bridge-gpui --all-targets -- -D warnings",
            program: "cargo",
            args: &[
                "clippy",
                "--locked",
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
            display: "cargo test --locked --workspace --exclude notes-bridge-gpui",
            program: "cargo",
            args: &[
                "test",
                "--locked",
                "--workspace",
                "--exclude",
                "notes-bridge-gpui",
            ],
            advisory: false,
            quick_skippable: true,
            budget_secs: 900,
        },
        StepSpec {
            name: "check-arch",
            display: "cargo run --locked -p xtask -- check-arch",
            program: "cargo",
            args: &[
                "run",
                "--locked",
                "-p",
                "xtask",
                "--quiet",
                "--",
                "check-arch",
            ],
            advisory: false,
            quick_skippable: false,
            budget_secs: 180,
        },
        StepSpec {
            name: "check-deps",
            display: "cargo run --locked -p xtask -- check-deps",
            program: "cargo",
            args: &[
                "run",
                "--locked",
                "-p",
                "xtask",
                "--quiet",
                "--",
                "check-deps",
            ],
            advisory: false,
            quick_skippable: false,
            budget_secs: 180,
        },
        StepSpec {
            name: "check-ci",
            display: "cargo run --locked -p xtask -- check-ci",
            program: "cargo",
            args: &[
                "run", "--locked", "-p", "xtask", "--quiet", "--", "check-ci",
            ],
            // Blocking, and never --quick-skipped: it is cheap, it reads two
            // text files, and it is the row that protects every other row from
            // drifting away from ci.yml.
            advisory: false,
            quick_skippable: false,
            budget_secs: 60,
        },
        StepSpec {
            name: "manifest",
            display: "cargo run --locked -p xtask -- manifest",
            program: "cargo",
            args: &[
                "run", "--locked", "-p", "xtask", "--quiet", "--", "manifest",
            ],
            // Blocking, not advisory, and this is the one row whose reason is NOT
            // "cheap so why skip": a wrong DPI declaration is not a desktop
            // dependency, it is a shipped binary that disagrees with the
            // assumptions under D40/D42. It mutates the exe it verifies, so it is
            // deliberately NOT quick-skippable - skipping it is the exact state
            // the step exists to detect.
            advisory: false,
            quick_skippable: false,
            budget_secs: 120,
        },
        StepSpec {
            name: "check-unsafe",
            display: "cargo run --locked -p xtask -- check-unsafe",
            program: "cargo",
            args: &[
                "run",
                "--locked",
                "-p",
                "xtask",
                "--quiet",
                "--",
                "check-unsafe",
            ],
            // Blocking and never --quick-skipped: AGENTS.md calls unsafe a hard
            // invariant, and until this row existed the only thing enforcing it
            // was the honesty of whoever wrote the block. Cheap to run, so there
            // is no reason to skip it on any lane.
            advisory: false,
            quick_skippable: false,
            budget_secs: 60,
        },
        StepSpec {
            name: "fixtures",
            display: "cargo run --locked -p xtask -- fixtures verify",
            program: "cargo",
            args: &[
                "run", "--locked", "-p", "xtask", "--quiet", "--", "fixtures", "verify",
            ],
            advisory: false,
            quick_skippable: false,
            budget_secs: 180,
        },
        StepSpec {
            name: "smoke",
            display: "cargo run --locked -p xtask -- smoke (advisory: the PRODUCT by default - DEFAULT_BINARY=slint - opens a real window on this desktop)",
            program: "cargo",
            args: &["run", "--locked", "-p", "xtask", "--quiet", "--", "smoke"],
            // Advisory, never a gate: a runner with no desktop is not a broken
            // repo, and a GUI row that goes red for an environmental reason
            // trains people to ignore the row. --quick skips it too.
            //
            // BARE, and the bareness is load-bearing. smoke with no --binary judges
            // DEFAULT_BINARY = "slint" (smoke.rs:291-305), so this row IS the product
            // leg now; and it is also the row ci.yml 3e's exit-code CONTRACT arms are
            // judged against, because check_ci::Step::is_smoke() recognises the smoke
            // step by the LAST TOKEN of its command being "smoke". Put a flag on this
            // command and no step is recognised, decide_contract returns
            // [contract-unreadable], and check-ci exits 2 - it refuses rather than
            // reports green. So the product travels by default, never by a flag here.
            // The arms stay 0,1,2,3,4,5,6,7,9 as published by smoke::CONTRACT: 6/7/9
            // are the gpui needle schedule's verdicts and simply cannot fire on the
            // product leg until the re-earnings (S8/S9) re-earn them - which is not a
            // licence to delete the arms, since the schedule stays frozen, not gone.
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
            display: "cargo build --locked -p notes-bridge-gpui --bin notes-gpui",
            program: "cargo",
            args: &[
                "build",
                "--locked",
                "-p",
                "notes-bridge-gpui",
                "--bin",
                "notes-gpui",
            ],
            advisory: false,
            quick_skippable: true,
            budget_secs: 900,
        },
        StepSpec {
            name: "bridge-clippy",
            display: "cargo clippy --locked -p notes-bridge-gpui --all-targets -- -D warnings",
            program: "cargo",
            args: &[
                "clippy",
                "--locked",
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
        // M5-S0: THE OTHER BRIDGE GETS THE SAME TWO ROWS, because until this commit
        // the SHIPPING product had never been linked by any gate in this file. The
        // asymmetry was invisible by construction: notes-bridge-slint is NOT the
        // excluded member, so it appeared in `clippy` and `test` above and looked
        // fully covered - but `cargo test` links a test harness, not a bin, and
        // `cargo clippy` never links at all. So a bridge that does not compile into
        // notes-slint.exe was green here, and is green in CI, every day. Same shape,
        // same budget, same quick-skippability as bridge-build/bridge-clippy, and the
        // same reason: BUILD owns the link verdict, CLIPPY owns every target.
        //
        // No --config future-incompat-report here, unlike the gpui step: that flag
        // exists for ONE measured footer in gpui's graph (proc-macro-error2, ci.yml
        // step 3b's comment). Slint's graph was not measured for it, and a
        // suppression copied without a measurement is how a warning silently stops
        // being visible. Add it with a number, not by analogy.
        StepSpec {
            name: "slint-build",
            display: "cargo build --locked -p notes-bridge-slint --bin notes-slint",
            program: "cargo",
            args: &[
                "build",
                "--locked",
                "-p",
                "notes-bridge-slint",
                "--bin",
                "notes-slint",
            ],
            advisory: false,
            quick_skippable: true,
            budget_secs: 900,
        },
        StepSpec {
            name: "slint-clippy",
            display: "cargo clippy --locked -p notes-bridge-slint --all-targets -- -D warnings",
            program: "cargo",
            args: &[
                "clippy",
                "--locked",
                "-p",
                "notes-bridge-slint",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
            advisory: false,
            quick_skippable: true,
            budget_secs: 900,
        },
        // M5-S2: THIS ROW IS THE FROZEN SCHEDULE, NOW EXPLICITLY PINNED. It used to run
        // `smoke --binary=slint`, and that stopped being a distinct thing the moment
        // DEFAULT_BINARY landed in smoke.rs: the bare "smoke" row above judges the very
        // same product leg, so this row had become a second cell asking for the same
        // verdict. Deleting it is what FREEZE LAW (ADR-0006) forbids - rows may not
        // LOOSEN, and the needle schedule's automated coverage must not silently die -
        // so the row is REPURPOSED, not removed: it now names the thing the default no
        // longer covers, gpui's needle schedule, pinned by an explicit `--binary=gpui`
        // instead of by the accident of a default. Advisory exactly as before; the
        // schedule still runs, still on a real window, still nowhere near a gate.
        //
        // NOT the exit-code CONTRACT row - that is the bare "smoke" above, the only
        // command whose last token is "smoke" (check_ci::Step::is_smoke). So ci.yml's
        // arms here are human wording, and they are wording about the schedule this
        // step really does run: 6/7/9 can fire HERE even though they cannot fire on the
        // product leg. Budget and quick-skippability are the "smoke" row's, unchanged.
        StepSpec {
            name: "smoke-gpui",
            display: "cargo run --locked -p xtask -- smoke --binary=gpui (advisory: the frozen gpui needle schedule - ADR-0006 - opens a real window on this desktop)",
            program: "cargo",
            args: &[
                "run",
                "--locked",
                "-p",
                "xtask",
                "--quiet",
                "--",
                "smoke",
                "--binary=gpui",
            ],
            advisory: true,
            quick_skippable: true,
            budget_secs: 120,
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

fn joined(names: &[&str]) -> String {
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
}

/// The gate's own roster, in the shape check-ci compares against ci.yml. One
/// source of truth on this side: it is derived from step_specs(), so the list
/// CI is checked against cannot drift from the list check() actually runs.
///
/// THIS IS ALSO THE INNER COMMAND LINE. The aggregate step in ci.yml runs
/// `cargo run --locked -p xtask -- check`, and that outer --locked governs only
/// the resolve that BUILDS xtask; the fourteen cargo rows this roster spawns each
/// resolve AGAIN, in `run_child` below, from `spec.args`. So the authority the
/// gate is supposed to have over Cargo.lock passes through here or not at all,
/// and `every_row_that_resolves_a_graph_carries_the_lock` is what keeps that
/// sentence true instead of aspirational. A row without the flag is not a slower
/// step, it is a step that will quietly refresh a lock nobody committed - measured,
/// not asserted: in a copy of this commit with one unplanted dependency added to
/// `crates/core`, the verbatim step `cargo run --locked -p xtask -- check-deps`
/// exits 101 with "cannot update the lock file ... because --locked was passed" and
/// Cargo.lock's md5 is IDENTICAL before and after (18ac197e), while the same step
/// with the flag removed exits 0, prints "5 member crates, 0 violations", and
/// rewrites Cargo.lock to a new md5 (6382fdcc). Green, and the drift is gone: not
/// fixed, just un-recorded. That is the failure this flag exists to refuse.
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
    let specs = step_specs();
    let total = specs.len();
    if quick {
        // A tool has to say what it did NOT check. --quick is not "the same
        // gate, only faster": the bridge rows are deliberately out of it, so a
        // local --quick green says nothing at all about the bridge compiling,
        // and only a full cargo xtask check runs them. Name every skipped row,
        // and separate the gating ones, because those are the difference between
        // a fast lane and a verdict.
        let skipped: Vec<&StepSpec> = specs.iter().filter(|s| s.quick_skippable).collect();
        let gates: Vec<&str> = skipped
            .iter()
            .filter(|s| !s.advisory)
            .map(|s| s.name)
            .collect();
        let advisories: Vec<&str> = skipped
            .iter()
            .filter(|s| s.advisory)
            .map(|s| s.name)
            .collect();
        println!(
            "check: --quick skipped {} of {} rows; this is NOT a full verdict",
            skipped.len(),
            total
        );
        println!(
            "check:   GATING rows not run (only a full 'cargo xtask check' proves these): {}",
            joined(&gates)
        );
        println!("check:   advisory rows not run: {}", joined(&advisories));
    }
    let mut verdicts: Vec<Verdict> = Vec::new();
    for (i, spec) in specs.into_iter().enumerate() {
        let outcome = if quick && spec.quick_skippable {
            Outcome::Skipped
        } else {
            println!("==> [{}/{}] {}", i + 1, total, spec.display);
            run_child(&spec, &root)
        };
        println!("<== [{}] {}", spec.name, outcome.label(spec.advisory));
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
        for expected in [
            "check-arch",
            "check-deps",
            "check-ci",
            "check-unsafe",
            "manifest",
            "fixtures",
        ] {
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

    /// The roster IS the inner command line: run_child spawns spec.args, and
    /// check-ci compares the same list against ci.yml. So a row that does not
    /// carry --locked is an unlockable resolve no matter what the workflow says,
    /// and this is the half of the coupling the workflow judge cannot see.
    /// fmt never resolves a graph and docs is pwsh: they are the two exemptions,
    /// and the count is asserted so a row added tomorrow without the flag trips
    /// BOTH the per-row message and the roster size.
    #[test]
    fn every_row_that_resolves_a_graph_carries_the_lock() {
        let exempt = ["fmt", "docs"];
        let mut locked = 0;
        for spec in step_specs() {
            if exempt.contains(&spec.name) {
                continue;
            }
            assert!(
                spec.args.contains(&"--locked"),
                "{} resolves the dependency graph and carries no --locked: {}. Lock the row or exempt it by name with a reason.",
                spec.name,
                spec.args.join(" "),
            );
            locked += 1;
        }
        // 14 and 16, and they move together on purpose: this is the assertion the
        // M5-S0 slint pair was expected to trip, and tripping it is the POINT - a
        // roster that grows without this count being re-read is a roster nobody is
        // looking at. fmt (no graph) and docs (pwsh) are still the only two exemptions,
        // so 16 - 2 = 14, and the row the M5-S1 slint smoke added carries --locked like
        // every other cargo row above it (the per-row assert is what proves that half).
        assert_eq!(
            locked, 14,
            "fourteen rows resolve a graph here; another number means the roster changed shape and this statement is now about a different list",
        );
        assert_eq!(
            step_specs().len(),
            16,
            "fmt and docs are the two exempt rows"
        );
    }
    /// The demotion of a gating row to advisory is exactly the silent weakening
    /// this checker exists to catch. An indirect guard - the roster length
    /// assertion, or check-ci going red - would not name WHICH decision broke,
    /// and a guard that cannot name its own violation is not a guard. So this
    /// one says it out loud.
    #[test]
    fn the_bridge_is_gated_by_a_build_and_a_lint_never_an_advisory_check() {
        let specs = step_specs();
        let names: Vec<&'static str> = specs.iter().map(|s| s.name).collect();
        let build = specs
            .iter()
            .find(|s| s.name == "bridge-build")
            .unwrap_or_else(|| panic!("bridge-build row is gone: {names:?}"));
        let lint = specs
            .iter()
            .find(|s| s.name == "bridge-clippy")
            .unwrap_or_else(|| panic!("bridge-clippy row is gone: {names:?}"));

        assert!(
            !build.advisory,
            "bridge-build was demoted to advisory: CI BUILDS the bridge and gates it, so the \
             local gate must too"
        );
        assert!(
            !lint.advisory,
            "bridge-clippy was demoted to advisory: CI lints every bridge target and gates it"
        );
        // The exact commands, because "cargo check" passing looks identical in a
        // name-only assertion and is strictly weaker than both.
        assert_eq!(
            build.args.join(" "),
            "build --locked -p notes-bridge-gpui --bin notes-gpui"
        );
        assert_eq!(
            lint.args.join(" "),
            "clippy --locked -p notes-bridge-gpui --all-targets -- -D warnings"
        );
        // Still quick-skippable: a desktop-less runner must be able to run
        // check --quick. Skipping is not the same as not gating.
        assert!(
            build.quick_skippable && lint.quick_skippable,
            "the bridge rows must stay out of --quick"
        );

        // And the row that used to be here - an advisory cargo check - stays gone.
        assert!(
            !names.contains(&"bridge"),
            "the advisory cargo-check bridge row is back: {names:?}"
        );
        // And the same two rows for the SECOND bridge, because "the bridge" stopped
        // naming one crate the moment slint shipped. Named out loud rather than by a
        // loop over prefixes: a slint row that quietly becomes advisory or disappears
        // has to be reported by name, which is the whole reason this test exists.
        let slint_build = specs
            .iter()
            .find(|s| s.name == "slint-build")
            .unwrap_or_else(|| panic!("slint-build row is gone: {names:?}"));
        let slint_lint = specs
            .iter()
            .find(|s| s.name == "slint-clippy")
            .unwrap_or_else(|| panic!("slint-clippy row is gone: {names:?}"));
        assert!(
            !slint_build.advisory && !slint_lint.advisory,
            "a slint row was demoted to advisory: notes-slint.exe is the shipping artifact of              the bridge this project chose, so its link and its lint gate"
        );
        assert_eq!(
            slint_build.args.join(" "),
            "build --locked -p notes-bridge-slint --bin notes-slint"
        );
        assert_eq!(
            slint_lint.args.join(" "),
            "clippy --locked -p notes-bridge-slint --all-targets -- -D warnings"
        );
        assert!(
            slint_build.quick_skippable && slint_lint.quick_skippable,
            "the slint rows must stay out of --quick, exactly like the gpui pair"
        );

        for spec in &specs {
            for pkg in ["notes-bridge-gpui", "notes-bridge-slint"] {
                if spec.args.contains(&pkg) && spec.args.contains(&"check") {
                    panic!(
                        "{} re-introduces an advisory-shaped cargo check of {pkg}",
                        spec.name
                    );
                }
            }
        }
    }
}
