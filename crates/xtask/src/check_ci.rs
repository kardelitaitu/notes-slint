//! check-ci - the WORKFLOW must run the roster the local gate runs.
//!
//! check-arch proves the CODE obeys the layering rules; check-deps proves the
//! manifests obey the template. Neither says a word about ci.yml, which lists
//! its steps by hand while check.rs lists the same commands again in its own
//! table. The two agree today only because two workers were careful, and the
//! drift class is a step added to one list and not the other, or a command
//! edited in CI alone. Rules, one line per violation, rule id first, like every
//! other checker here:
//!
//! * [step-missing] - a check.rs row (gate OR advisory) whose command is not a
//!   run step in the gate job;
//! * [step-extra] - a run step in the gate job no check.rs row knows. The one
//!   exemption is the aggregate cargo run -p xtask check: that IS the roster,
//!   run as one command. Exact signature match only, so no stray step can hide
//!   behind it;
//! * [advisory-mismatch] - a step that is advisory on one side and a gate on the
//!   other (continue-on-error). Demoting a gate is a silent hole; promoting an
//!   advisory red-screens a run for an environmental reason;
//! * [no-pipefail] - a step that pipes without set -o pipefail. Actions runs
//!   shell: bash on Windows as bash -e WITHOUT pipefail, so a failing cargo
//!   behind | tee reports tee's success: a red step once looked green here. A
//!   rule now, not a comment;
//! * [no-shell-bash] - a step using bash-only syntax (case/esac, set -o, <())
//!   that does not declare shell: bash, or a continue-on-error step that
//!   declares NO shell at all. The default shell on windows-latest is pwsh,
//!   where case/esac is a syntax error, and continue-on-error makes that
//!   breakage permanent and invisible. An advisory step that DECLARES pwsh (the
//!   docs validator) is legitimate and stays silent: the hole being closed is an
//!   undeclared dialect, not pwsh itself.
//!
//! # What this parser deliberately does NOT understand
//!
//! Line-oriented, the way deps.rs reads manifests: no YAML crate, no new
//! dependency. It reads only the gate: job, only the step keys name, run, shell
//! and continue-on-error, and a run: | body as verbatim indented lines rather
//! than as a YAML block scalar. It does not resolve anchors, aliases, merge
//! keys, tags, folded scalars, with: maps, if: expressions or
//! strategy.matrix; it ignores uses: steps (no command to compare) and any run
//! line whose first token is not cargo or pwsh (echo, if, while, done, set and
//! exit are not gates). Anything it would have to guess at is REFUSED: a missing
//! gate job, an anonymous step, an anchor/alias/merge line, a matrix, or zero
//! parsed steps all print [cannot-parse] and exit 2 - never a green it did not
//! earn. An optional argument points it at another workflow file, which is how
//! the rule is proven able to fail (scratch copies under TEMP, never the repo).
//!
//! Exit codes: 0 no disagreement, 1 at least one violation, 2 the check itself
//! could not run, or refused to judge.

use std::fs;
use std::path::PathBuf;

const WORKFLOW_REL: &str = ".github/workflows/ci.yml";
const GATE_JOB: &str = "gate:";
const AGGREGATE: &str = "cargo run -p xtask check";

/// One row of the local gate, as far as CI is concerned.
#[derive(Debug, Clone)]
pub struct Row {
    pub name: &'static str,
    pub command: String,
    pub advisory: bool,
}

/// One step of the gate job.
#[derive(Debug, Clone)]
pub struct Step {
    pub name: String,
    /// Normalized signature of every command line the step carries.
    pub commands: Vec<String>,
    pub shell: Option<String>,
    pub continue_on_error: bool,
    pub line: usize,
    pub has_pipe: bool,
    pub has_bashism: bool,
    pub has_pipefail: bool,
}

impl Step {
    fn is_bash(&self) -> bool {
        self.shell.as_deref() == Some("bash")
    }
}

/// Normalize a command to the tokens that carry meaning, dropping the plumbing
/// CI wraps around the same command: --quiet, the bare -- separator, pwsh's
/// -NoProfile / -File / -NonInteractive, and cargo's --config plus its value.
/// Conservative on purpose: it never reorders and never truncates, so
/// --exclude notes-bridge-gpui and --exclude other-crate stay distinct.
pub fn signature(program: &str, args: &[&str]) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut skip = false;
    for arg in args {
        if skip {
            skip = false;
            continue;
        }
        match *arg {
            "--quiet" | "--" | "-NoProfile" | "-File" | "-NonInteractive" => {}
            "--config" => skip = true,
            other => out.push(other),
        }
    }
    let mut s = String::from(program);
    for arg in out {
        s.push(' ');
        s.push_str(arg);
    }
    s
}

/// The first command of a line: everything up to the first pipe, redirection,
/// `||` or `;$. A step that chains `cargo ... | tee` still GATES on
/// `cargo ...`, which is the command the roster has to recognize.
fn first_segment(text: &str) -> &str {
    text.split("2>&1")
        .next()
        .unwrap_or(text)
        .split("||")
        .next()
        .unwrap_or(text)
        .split('|')
        .next()
        .unwrap_or(text)
        .split(';')
        .next()
        .unwrap_or(text)
        .trim()
}

/// Signature of a whole command line. The roster side and the YAML side both
/// come through here, so neither can be normalized more generously than the
/// other - that asymmetry is how a checker passes while lying.
pub fn signature_of(line: &str) -> String {
    let mut it = first_segment(line.trim()).split_whitespace();
    let Some(program) = it.next() else {
        return String::new();
    };
    signature(program, &it.collect::<Vec<_>>())
}

/// One line of a run body, if it is a command worth comparing.
fn command_line(line: &str) -> Option<String> {
    let text = line.trim();
    if text.is_empty() || text.starts_with('#') {
        return None;
    }
    let mut tokens = first_segment(text).split_whitespace();
    let program = tokens.next()?;
    if program != "cargo" && program != "pwsh" {
        return None;
    }
    Some(signature(program, &tokens.collect::<Vec<_>>()))
}

/// The parse result: the steps, or the reason nothing was judged.
#[derive(Debug, Default)]
pub struct Parsed {
    pub steps: Vec<Step>,
    /// Non-empty means the parser REFUSED. Judge nothing; exit 2.
    pub refusals: Vec<String>,
}

fn refused(out: &mut Parsed, line: usize, what: &str) {
    out.refusals.push(format!(
        "CI VIOLATION: [cannot-parse] line {line}: {what} - this reader does not understand it, \
         so it refuses to judge rather than report a green it did not earn"
    ));
}

/// Parse the gate job's steps out of workflow text.
pub fn parse(text: &str) -> Parsed {
    let mut out = Parsed::default();
    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|l| l.starts_with("  gate:")) else {
        out.refusals.push(format!(
            "CI VIOLATION: [cannot-parse] no two-space-indented '  {GATE_JOB}' job key: the gate \
             job cannot be located, so nothing is judged"
        ));
        return out;
    };
    // An anchored / tagged / inline-valued job key is structure this reader
    // would have to resolve to be sure - so it is refused, not skipped.
    let carried = lines[start].trim_start().trim_start_matches("gate:").trim();
    if !carried.is_empty() {
        refused(
            &mut out,
            start + 1,
            &format!(
                "the gate job key carries '{carried}': an anchor, tag or inline value this \
                      line-wise reader cannot resolve"
            ),
        );
        return out;
    }
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, l)| l.starts_with("  ") && !l.starts_with("    ") && l.trim_end().ends_with(':'))
        .map(|(i, _)| i)
        .unwrap_or(lines.len());
    for (i, line) in lines.iter().enumerate().take(end).skip(start) {
        let t = line.trim();
        if t.starts_with("<<:") || t.starts_with('&') || t.starts_with("* ") {
            refused(
                &mut out,
                i + 1,
                &format!("'{t}' is an anchor, alias or merge key"),
            );
            return out;
        }
        if t.starts_with("strategy:") || t.starts_with("matrix:") {
            refused(
                &mut out,
                i + 1,
                "'{t}': a matrix has no single roster to compare against",
            );
            return out;
        }
    }

    let mut i = start + 1;
    while i < end {
        let raw = lines[i];
        let Some(dash) = raw.find("- ") else {
            i += 1;
            continue;
        };
        if !raw[dash..].starts_with("- ") || !raw[..dash].chars().all(|c| c == ' ') {
            i += 1;
            continue;
        }
        let body = raw[dash + 2..].trim();
        let mut step = Step {
            name: String::new(),
            commands: Vec::new(),
            shell: None,
            continue_on_error: false,
            line: i + 1,
            has_pipe: false,
            has_bashism: false,
            has_pipefail: false,
        };
        if let Some(rest) = body.strip_prefix("name:") {
            step.name = rest.trim().to_string();
        } else if let Some(rest) = body.strip_prefix("uses:") {
            step.name = format!("<{}>", rest.trim());
        } else {
            refused(&mut out, i + 1, "a step with neither name: nor uses:");
            return out;
        }
        i += 1;
        let mut in_run = false;
        while i < end {
            let l = lines[i];
            let t = l.trim_start();
            let indent = l.len() - t.len();
            // A YAML/shell comment is documentation, never a pipe: the block
            // above the bridge step explains pipefail with a '| tee' in it, and
            // attributing that to the previous step invented a violation.
            let is_comment = t.starts_with('#');
            if in_run && (t.is_empty() || indent <= dash) {
                in_run = false;
            }
            if !in_run && t.starts_with("- ") && indent <= dash {
                break;
            }
            if in_run {
                if let Some(sig) = command_line(t) {
                    step.commands.push(sig);
                }
            } else if let Some(v) = t.strip_prefix("continue-on-error:") {
                step.continue_on_error = v.trim() == "true";
            } else if let Some(v) = t.strip_prefix("shell:") {
                step.shell = Some(v.trim().to_string());
            } else if let Some(v) = t.strip_prefix("run:") {
                let v = v.trim();
                if v.is_empty() || v.starts_with('|') || v.starts_with('>') {
                    in_run = true;
                } else if let Some(sig) = command_line(v) {
                    step.commands.push(sig);
                }
            }
            if !is_comment {
                if t.contains('|') {
                    step.has_pipe = true;
                }
                if t.contains("case ")
                    || t.contains("esac")
                    || t.contains("<(")
                    || t.contains("set -o ")
                {
                    step.has_bashism = true;
                }
                if t.starts_with("set -o pipefail") {
                    step.has_pipefail = true;
                }
            }
            i += 1;
        }
        out.steps.push(step);
    }
    if out.steps.is_empty() {
        out.refusals
            .push("CI VIOLATION: [cannot-parse] the gate job parsed to zero steps".to_string());
    }
    out
}

fn status_word(advisory: bool) -> &'static str {
    if advisory {
        "ADVISORY (continue-on-error)"
    } else {
        "a GATE"
    }
}

/// The comparison. Pure over the two lists, so every rule has a test.
pub fn decide(rows: &[Row], steps: &[Step]) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();

    for row in rows {
        let want = signature_of(&row.command);
        match steps.iter().find(|s| s.commands.contains(&want)) {
            None => v.push(format!(
                "CI VIOLATION: [step-missing] check.rs row '{}' is {} and runs '{want}', but the \
                 gate job has no such run step - add the step to ci.yml or delete the row; the two \
                 lists may not disagree",
                row.name,
                status_word(row.advisory)
            )),
            Some(step) => {
                if step.continue_on_error != row.advisory {
                    v.push(format!(
                        "CI VIOLATION: [advisory-mismatch] '{}' is {} in ci.yml (line {}) but {} in \
                         check.rs - a gate demoted on either side is a silent hole, and an \
                         advisory promoted to a gate red-screens runs for environmental reasons",
                        step.name,
                        status_word(step.continue_on_error),
                        step.line,
                        status_word(row.advisory)
                    ));
                }
            }
        }
    }

    for step in steps {
        if step.commands.is_empty() {
            continue;
        }
        let known = step.commands.iter().any(|c| {
            c.as_str() == AGGREGATE || rows.iter().any(|r| *c == signature_of(&r.command))
        });
        if !known {
            v.push(format!(
                "CI VIOLATION: [step-extra] the gate job runs '{}' in '{}' (line {}) and no check.rs \
                 row knows that command - add the row to check.rs or drop the step",
                step.commands.join("', '"),
                step.name,
                step.line
            ));
        }
        if step.has_pipe && !step.has_pipefail && step.shell.as_deref() != Some("pwsh") {
            v.push(format!(
                "CI VIOLATION: [no-pipefail] '{}' (line {}) pipes without 'set -o pipefail': Actions \
                 runs shell: bash on Windows as bash -e WITHOUT pipefail, so a failing command \
                 behind '| tee' reports tee's success and a red step prints green",
                step.name, step.line
            ));
        }
        if step.has_bashism && !step.is_bash() {
            v.push(format!(
                "CI VIOLATION: [no-shell-bash] '{}' (line {}) uses bash-only syntax but declares \
                 shell {:?}: on windows-latest the default is pwsh, where case/esac is a syntax \
                 error",
                step.name, step.line, step.shell
            ));
        }
        if step.continue_on_error && step.shell.is_none() {
            v.push(format!(
                "CI VIOLATION: [no-shell-bash] '{}' (line {}) is continue-on-error and declares no \
                 shell: the runner picks pwsh on windows-latest, where a bash script is a syntax \
                 error, and the advisory flag hides it permanently",
                step.name, step.line
            ));
        }
    }
    v
}

/// Steps that actually carried a command - the number in the success line.
pub fn checked_count(steps: &[Step]) -> usize {
    steps.iter().filter(|s| !s.commands.is_empty()).count()
}

/// Entry point. The optional argument is a workflow path, so the rules can be
/// shown to fail against scratch copies without touching the repo's ci.yml.
pub fn run(args: &[String]) -> i32 {
    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("check-ci: cannot read the current directory: {e}");
            return 2;
        }
    };
    let root = match crate::metadata::find_workspace_root(&cwd) {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("check-ci: {e}");
            return 2;
        }
    };
    // An argument that starts with '-' is a FLAG, not a workflow path. Reading
    // one as a path produces "cannot read --offline", which looks like a broken
    // checkout rather than what it is: an unsupported option. Say what the one
    // positional argument actually is.
    let mut path_arg: Option<&String> = None;
    for arg in args {
        if arg.starts_with('-') {
            eprintln!("check-ci: unsupported flag '{arg}' - this subcommand takes no flags");
            eprintln!("check-ci: usage: cargo xtask check-ci [PATH-TO-WORKFLOW]");
            eprintln!("check-ci:   the one optional argument is a path to a workflow file,");
            eprintln!("check-ci:   default {WORKFLOW_REL}; a flag is never a path");
            return 2;
        }
        if path_arg.is_some() {
            eprintln!("check-ci: more than one workflow path given ('{arg}')");
            eprintln!("check-ci: usage: cargo xtask check-ci [PATH-TO-WORKFLOW]");
            return 2;
        }
        path_arg = Some(arg);
    }
    let path: PathBuf = match path_arg {
        Some(p) => PathBuf::from(p),
        None => root.join(WORKFLOW_REL),
    };
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("check-ci: cannot read {}: {e}", path.display());
            return 2;
        }
    };
    let parsed = parse(&text);
    if !parsed.refusals.is_empty() {
        for r in &parsed.refusals {
            println!("{r}");
        }
        println!("check-ci: refused to judge {}", path.display());
        return 2;
    }
    let rows = crate::check::roster();
    let violations = decide(&rows, &parsed.steps);
    for line in &violations {
        println!("{line}");
    }
    println!(
        "ci: {} steps checked, {} violations",
        checked_count(&parsed.steps),
        violations.len()
    );
    if violations.is_empty() { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &'static str, command: &str, advisory: bool) -> Row {
        Row {
            name,
            command: command.to_string(),
            advisory,
        }
    }

    /// The two rows every test assumes: one gate, one advisory.
    fn roster() -> Vec<Row> {
        vec![
            row(
                "check-arch",
                "cargo run -p xtask --quiet -- check-arch",
                false,
            ),
            row("smoke", "cargo run -p xtask --quiet -- smoke", true),
        ]
    }

    const ARCH: &str = "
      - name: arch - check-arch
        shell: bash
        run: |
          set -o pipefail
          status=0
          cargo run -p xtask -- check-arch 2>&1 | tee log || status=$?
          exit $status
";

    const SMOKE: &str = "
      - name: smoke - smoke
        continue-on-error: true
        shell: bash
        timeout-minutes: 10
        run: |
          set -o pipefail
          cargo run -p xtask --quiet -- smoke 2>&1 | tee log || status=$?
          case \"$status\" in
            0) echo ok ;;
          esac
          exit $status
";

    const SMOKE_NO_SHELL: &str = "
      - name: smoke - smoke
        continue-on-error: true
        timeout-minutes: 10
        run: |
          set -o pipefail
          cargo run -p xtask --quiet -- smoke 2>&1 | tee log || status=$?
          case \"$status\" in
            0) echo ok ;;
          esac
          exit $status
";

    const SMOKE_NO_COE: &str = "
      - name: smoke - smoke
        shell: bash
        run: |
          set -o pipefail
          cargo run -p xtask --quiet -- smoke 2>&1 | tee log || status=$?
          case \"$status\" in
            0) echo ok ;;
          esac
          exit $status
";

    const ARCH_NO_PF: &str = "
      - name: arch - check-arch
        shell: bash
        run: |
          status=0
          cargo run -p xtask -- check-arch 2>&1 | tee log || status=$?
          exit $status
";

    const EXTRA: &str = "
      - name: something nobody registered
        shell: bash
        run: |
          set -o pipefail
          cargo clippy --workspace --all-targets -- -D warnings
";

    /// A workflow with a gate job AND a second job, so the scoping is tested.
    fn gate_text(steps: &str) -> String {
        format!(
            "name: CI\njobs:\n  gate:\n    runs-on: windows-latest\n    steps:\n{steps}\n  \
             bridge-gpui-advisory:\n    steps:\n      - name: other job\n        run: cargo test \
             -p notes-bridge-gpui\n"
        )
    }

    fn parse_ok(text: &str) -> Vec<Step> {
        let parsed = parse(text);
        assert!(parsed.refusals.is_empty(), "{:?}", parsed.refusals);
        parsed.steps
    }

    #[test]
    fn a_matching_workflow_is_clean() {
        let steps = parse_ok(&gate_text(&format!("{ARCH}{SMOKE}")));
        assert_eq!(steps.len(), 2, "only the gate job's steps: {steps:?}");
        let v = decide(&roster(), &steps);
        assert!(v.is_empty(), "{v:?}");
        assert_eq!(checked_count(&steps), 2);
    }

    #[test]
    fn only_the_gate_job_is_compared() {
        // The second job runs cargo test on the bridge, which is NOT a gate
        // row. Reading past the job boundary would either invent a phantom
        // [step-extra] or, worse, let a gate row be satisfied by a step that
        // gates nothing at all.
        let steps = parse_ok(&gate_text(&format!("{ARCH}{SMOKE}")));
        assert!(
            steps.iter().all(|s| s.name != "other job"),
            "the advisory job leaked into the gate: {steps:?}"
        );
        assert!(decide(&roster(), &steps).is_empty());
    }

    #[test]
    fn ci_plumbing_around_the_same_command_is_not_drift() {
        assert_eq!(
            signature_of("cargo run -p xtask --quiet -- check-arch"),
            "cargo run -p xtask check-arch"
        );
        assert_eq!(
            signature_of("cargo --config 'x' clippy -p y --all-targets -- -D warnings"),
            "cargo clippy -p y --all-targets -D warnings"
        );
        assert_eq!(
            signature_of("pwsh -NoProfile -File .agents/skills/check-docs.ps1"),
            "pwsh .agents/skills/check-docs.ps1"
        );
        assert_eq!(
            signature_of("cargo run -p xtask -- check-arch 2>&1 | tee log || status=$?"),
            "cargo run -p xtask check-arch"
        );
        // The aggregate is exempt by EXACT signature, not by prefix.
        let steps = parse_ok(&gate_text(
            "
      - name: drift
        shell: bash
        run: |
          set -o pipefail
          cargo run -p xtask -- check 2>&1 | tee log
",
        ));
        assert_eq!(decide(&[], &steps), Vec::<String>::new());
        let sneaky = parse_ok(&gate_text(
            "
      - name: drift
        shell: bash
        run: |
          set -o pipefail
          cargo run -p xtask -- check --extra-flag 2>&1 | tee log
",
        ));
        assert!(
            decide(&[], &sneaky)
                .iter()
                .any(|v| v.contains("[step-extra]")),
            "an aggregate-like step must not smuggle extra work past the lock"
        );
    }

    #[test]
    fn a_step_ci_dropped_is_step_missing() {
        let steps = parse_ok(&gate_text(SMOKE));
        let v = decide(&roster(), &steps);
        assert!(
            v.iter()
                .any(|x| x.contains("[step-missing]") && x.contains("check-arch")),
            "{v:?}"
        );
    }

    #[test]
    fn a_step_ci_added_alone_is_step_extra() {
        let steps = parse_ok(&gate_text(&format!("{ARCH}{SMOKE}{EXTRA}")));
        let v = decide(&roster(), &steps);
        assert!(
            v.iter()
                .any(|x| x.contains("[step-extra]") && x.contains("cargo clippy --workspace")),
            "{v:?}"
        );
    }

    #[test]
    fn advisory_on_one_side_and_a_gate_on_the_other_is_mismatch() {
        // smoke loses continue-on-error in CI: an environmental red now blocks.
        let steps = parse_ok(&gate_text(&format!("{ARCH}{SMOKE_NO_COE}")));
        let v = decide(&roster(), &steps);
        assert!(
            v.iter().any(|x| x.contains("[advisory-mismatch]")),
            "expected a mismatch, got {v:?}"
        );
    }

    #[test]
    fn a_piped_step_without_pipefail_is_no_pipefail() {
        let steps = parse_ok(&gate_text(&format!("{ARCH_NO_PF}{SMOKE}")));
        let v = decide(&roster(), &steps);
        assert!(
            v.iter().any(|x| x.contains("[no-pipefail]")),
            "a red behind | tee prints green: {v:?}"
        );
        // and the clean fixture must NOT carry that violation
        let clean = decide(&roster(), &parse_ok(&gate_text(&format!("{ARCH}{SMOKE}"))));
        assert!(
            !clean.iter().any(|x| x.contains("[no-pipefail]")),
            "{clean:?}"
        );
    }

    #[test]
    fn bash_syntax_without_shell_bash_is_no_shell_bash() {
        // Losing 'shell: bash' on the smoke step: it uses case/esac AND is
        // advisory, so both halves of the rule fire - the syntax error and the
        // hidden exit code are two separate holes.
        let steps = parse_ok(&gate_text(&format!("{ARCH}{SMOKE_NO_SHELL}")));
        let v = decide(&roster(), &steps);
        assert_eq!(
            v.iter().filter(|x| x.contains("[no-shell-bash]")).count(),
            2,
            "{v:?}"
        );
    }

    #[test]
    fn a_declared_pwsh_advisory_step_is_not_a_violation() {
        // The docs validator is pwsh on purpose. Flagging it would buy an
        // allowlist comment and lose the rule.
        let docs = || row("docs", "pwsh -NoProfile -File scripts/check-docs.ps1", true);
        let steps = parse_ok(&gate_text(
            "
      - name: docs
        continue-on-error: true
        shell: pwsh
        run: |
          pwsh -NoProfile -File scripts/check-docs.ps1
          exit $LASTEXITCODE
",
        ));
        let v = decide(&[docs()], &steps);
        assert!(v.is_empty(), "a declared pwsh advisory step is fine: {v:?}");
        // Same step with the shell key removed is not.
        let steps = parse_ok(&gate_text(
            "
      - name: docs
        continue-on-error: true
        run: |
          pwsh -NoProfile -File scripts/check-docs.ps1
",
        ));
        assert!(
            decide(&[docs()], &steps)
                .iter()
                .any(|x| x.contains("[no-shell-bash]"))
        );
    }

    #[test]
    fn an_unreadable_structure_refuses_to_judge() {
        let anchored = gate_text(&format!("{ARCH}{SMOKE}")).replace("  gate:", "  gate: &g");
        let parsed = parse(&anchored);
        assert!(
            parsed.refusals.iter().any(|r| r.contains("[cannot-parse]")),
            "an anchor must refuse, not guess: {:?}",
            parsed.refusals
        );

        let matrixed = gate_text(&format!("{ARCH}{SMOKE}")).replace(
            "    runs-on: windows-latest",
            "    strategy:\n      matrix:\n        os: [windows, ubuntu]",
        );
        assert!(
            !parse(&matrixed).refusals.is_empty(),
            "a matrix has no one roster"
        );

        let merged = gate_text(&format!("{ARCH}{SMOKE}"))
            .replace("    runs-on: windows-latest", "    <<: *shared");
        assert!(
            !parse(&merged).refusals.is_empty(),
            "a merge key must not be ignored"
        );

        let anonymous =
            gate_text(&format!("{ARCH}{SMOKE}")).replace("      - name: arch", "      - id: arch");
        assert!(
            !parse(&anonymous).refusals.is_empty(),
            "an anonymous step cannot be matched"
        );

        let gone = "name: CI\njobs:\n  other:\n    steps: []\n";
        assert!(!parse(gone).refusals.is_empty(), "no gate job must refuse");
        // A refusal is never a violation list: decide() is not reached.
        assert!(
            decide(&roster(), &[])
                .iter()
                .all(|v| v.contains("[step-missing]"))
        );
    }
}
