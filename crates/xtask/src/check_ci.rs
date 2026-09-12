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
//!   exemption is the aggregate cargo run --locked -p xtask check: that IS the roster,
//!   and it carries --locked because the step that runs the whole roster must resolve
//!   the SAME Cargo.lock that is in the commit, not one cargo quietly refreshed on the
//!   runner. The twelve rows this file compares against live in check.rs, so a
//!   --locked added HERE without the matching rows there reddens this judge on
//!   purpose - see the [step-missing] / [step-extra] pair.
//!   run as one command. Exact signature match only, so no stray step can hide
//!   behind it;
//! * [advisory-mismatch] - a step that is advisory on one side and a gate on the
//!   other (continue-on-error). Demoting a gate is a silent hole; promoting an
//!   advisory red-screens a run for an environmental reason;
//! * [no-pipefail] - a step that pipes without set -o pipefail. Actions runs
//!   shell: bash on Windows as bash -e WITHOUT pipefail, so a failing cargo
//!   behind | tee reports tee's success: a red step once looked green here. A
//!   rule now, not a comment;
//! * [arm-missing] / [arm-extra] - the exit-code contract smoke publishes (see
//!   smoke::CONTRACT) and the case arms of the smoke step must name the same set
//!   of codes. This exists because the roster models COMMANDS, and exit codes
//!   live on a different surface: 9b84bb7b added a documented return value with
//!   no arm and check-ci said 0 violations truthfully. [no-shell-bash] was
//!   written for the same class of silent mislabel and nobody thought to look
//!   here. LIMIT, and it is the limit of every checker in this file: a branch can
//!   be proven to EXIST and to RUN, never that its wording is HONEST. Whether
//!   "exit 5 means the app was never launched" is a fair sentence stays a human
//!   read of the arm body. Four times tonight a green meant less than it sounded.
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
const AGGREGATE: &str = "cargo run --locked -p xtask check";

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
    /// Every numeric case arm the step's body carries, in file order. Kept
    /// because the exit-code contract is a second surface the roster cannot
    /// describe: the roster models COMMANDS, and exit codes live on this one.
    pub arms: Vec<i32>,
    /// True when the body contains a 'case' statement at all, so an empty arms
    /// list can be told apart from an unparsed one.
    pub has_case: bool,
}
impl Step {
    /// The step that runs the GUI smoke, identified by its command rather than
    /// by its display name, because names are prose and get rewritten.
    pub fn is_smoke(&self) -> bool {
        self.commands
            .iter()
            .any(|c| c.split_whitespace().last() == Some("smoke"))
    }
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
            arms: Vec::new(),
            has_case: false,
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
                if let Some(code) = case_arm(t) {
                    step.arms.push(code);
                }
                if t.starts_with("case ") || t.starts_with("case\t") {
                    step.has_case = true;
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

/// A numeric bash case arm: a line whose first token is digits followed by ')'.
/// The catch-all '*' is DELIBERATELY not collected: an undocumented code being
/// absorbed by '*' is the drift this rule exists to catch, not a branch on it.
pub fn case_arm(trimmed: &str) -> Option<i32> {
    let head = trimmed.split_whitespace().next()?;
    let digits = head.strip_suffix(')')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// The verdict of the contract rule. A refusal is a VALUE here, not an early
/// return someone can forget, because "the reader could not see the arms" and
/// "the arms are all present" otherwise look identical from the outside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractOutcome {
    Judged(Vec<String>),
    Refused(String),
}

/// What this rule can and cannot prove. It proves a branch EXISTS and can RUN.
/// It cannot prove the branch's WORDING is honest: whether "exit 5 means the app
/// was never launched" or "exit 1 separates three unrelated readings" is a fair
/// sentence about the world is a human read of the arm body, and a green here
/// must not be over-trusted. Tonight a green meant less than it sounded four
/// separate times; this rule is where a fifth would be most comfortable hiding.
pub fn decide_contract(steps: &[Step], contract: &[crate::smoke::Contract]) -> ContractOutcome {
    let Some(step) = steps.iter().find(|s| s.is_smoke()) else {
        return ContractOutcome::Refused(
            "CI VIOLATION: [contract-unreadable] no gate step runs smoke, so its exit codes cannot be \
             checked against the published contract - this checker refuses to judge rather than report \
             a green it did not earn"
                .to_string(),
        );
    };
    if contract.is_empty() {
        return ContractOutcome::Refused(
            "CI VIOLATION: [contract-unreadable] smoke published an empty CONTRACT table, so there is \
             no side to compare against - refusing to judge".to_string(),
        );
    }
    if step.has_case && step.arms.is_empty() {
        return ContractOutcome::Refused(format!(
            "CI VIOLATION: [contract-unreadable] the smoke step (line {}) has a case statement with no \
             numeric arm this reader could parse, so a missing branch would look like a parse gap - \
             refusing to judge",
            step.line
        ));
    }
    if !step.has_case {
        return ContractOutcome::Refused(format!(
            "CI VIOLATION: [contract-unreadable] the smoke step '{}' (line {}) has no case statement at \
             all, so every smoke verdict falls through to a bare exit - refusing to judge",
            step.name, step.line
        ));
    }
    let mut out: Vec<String> = Vec::new();
    for c in contract {
        if !step.arms.contains(&c.0) {
            out.push(format!(
                "CI VIOLATION: [arm-missing] smoke publishes exit {} ({}) and the smoke step '{}' (line \
                 {}) has no case arm for it - add the arm, or retire the code from CONTRACT in \
                 smoke.rs; the two surfaces may not disagree (and note that a catch-all '*' is not \
                 counted as branching on it)",
                c.0, c.1, step.name, step.line
            ));
        }
    }
    for a in &step.arms {
        if !contract.iter().any(|c| c.0 == *a) {
            out.push(format!(
                "CI VIOLATION: [arm-extra] the smoke step '{}' (line {}) branches on exit {a}, which \
                 smoke's CONTRACT does not publish - either smoke can return it and the table is \
                 incomplete, or the arm is dead wording that should go",
                step.name, step.line
            ));
        }
    }
    ContractOutcome::Judged(out)
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

/// The bracketed rule ids in the run of `#` comment lines that starts at the
/// line containing `marker`. Found by its SENTENCE, never by a line number: a
/// rewrap moves lines and does not move words, and a pin that breaks because
/// somebody reflowed a comment is a pin that gets deleted instead of fixed.
///
/// This is the mechanism for the class of drift that is otherwise invisible: a
/// human-readable list of things a checker prints. The list is prose, check-ci
/// does not parse comments, and so a rule can be added to the checker and the
/// comment keeps claiming the old set forever - which is what happened to the
/// unsafe rule list at 4e when [raw-ffi-imbalance] landed.
pub fn bracketed_ids_in_prose(text: &str, marker: &str) -> Vec<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(at) = lines.iter().position(|l| l.contains(marker)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut i = at;
    let limit = (at + 16).min(lines.len());
    while i < limit {
        let t = lines[i].trim_start();
        if !t.starts_with('#') {
            break;
        }
        let body = t.trim_start_matches('#');
        let mut rest = body;
        while let Some(open) = rest.find('[') {
            let after = &rest[open + 1..];
            match after.find(']') {
                Some(close) => {
                    let id = &after[..close];
                    if !id.is_empty()
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b == b'-' || b == b'_')
                    {
                        out.push(id.to_string());
                    }
                    rest = &after[close + 1..];
                }
                None => {
                    rest = "";
                }
            }
        }
        i += 1;
    }
    out
}

/// Steps that actually carried a command - the number in the success line.
pub fn checked_count(steps: &[Step]) -> usize {
    steps.iter().filter(|s| !s.commands.is_empty()).count()
}

/// Rule id for a prose list of rule ids that has stopped matching the code.
pub const PROSE_RULES: &str = "prose-rule-list";

/// A human-readable list of rule ids in a comment is a CLAIM, and the claims this
/// file can check are only about commands - so the claim went unchecked, and
/// drifted, exactly once per rule added. This compares what the comment says
/// against what the checker can actually print, and it runs on every check-ci,
/// not only in a test: a unit test that nobody gates with is a second opinion
/// nobody asks for.
pub fn prose_list_findings(text: &str, marker: &str, want: &[&str]) -> Vec<String> {
    let found = bracketed_ids_in_prose(text, marker);
    if found.is_empty() {
        return vec![format!(
            "CI VIOLATION: [{PROSE_RULES}] no comment containing \"{marker}\" carries a bracketed rule \
             list - the prose was rewritten, deleted, or moved, and the pin cannot see what it was \
             replaced by. Put the marker sentence back or point the pin at the new one.",
        )];
    }
    let mut sorted = found.clone();
    sorted.sort();
    sorted.dedup();
    let mut expect: Vec<String> = want.iter().map(|s| s.to_string()).collect();
    expect.sort();
    expect.dedup();
    if sorted == expect {
        return Vec::new();
    }
    let missing: Vec<&str> = expect
        .iter()
        .filter(|e| !sorted.iter().any(|f| f == *e))
        .map(|s| s.as_str())
        .collect();
    let extra: Vec<&str> = sorted
        .iter()
        .filter(|f| !expect.iter().any(|e| e == *f))
        .map(|s| s.as_str())
        .collect();
    vec![format!(
        "CI VIOLATION: [{PROSE_RULES}] the list in the comment about \"{marker}\" says {} rule(s) and \
         the checker prints {}: missing {}, stale {}. A rule added to the code without the sentence \
         being updated is a claim about a tool that no longer describes it.",
        sorted.len(),
        expect.len(),
        if missing.is_empty() {
            "none".to_string()
        } else {
            missing.join(", ")
        },
        if extra.is_empty() {
            "none".to_string()
        } else {
            extra.join(", ")
        }
    )]
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
    let mut violations = decide(&rows, &parsed.steps);
    // The exit-code surface, in the same verdict rather than a second report
    // someone can skip reading.
    match decide_contract(&parsed.steps, crate::smoke::CONTRACT) {
        ContractOutcome::Refused(why) => {
            println!("{why}");
            for line in &violations {
                println!("{line}");
            }
            println!("check-ci: refused to judge {}", path.display());
            return 2;
        }
        ContractOutcome::Judged(found) => violations.extend(found),
    }
    // The prose surface, folded in the same way the exit-code surface is. A comment
    // that enumerates a checker's rule ids is a claim about that checker, and
    // decide() only ever compares COMMANDS, so the claim sat here unchecked and said
    // "six" on the day the code grew a seventh rule - check-ci reported 0 violations
    // truthfully and uselessly. Same class, third surface this judge owns.
    violations.extend(prose_list_findings(
        &text,
        "enforces the",
        crate::check_unsafe::RULE_IDS,
    ));
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

    /// The two bridge steps LEAD with `cargo --config <value>`, and signature()
    /// drops --config together with its value. So "put --locked immediately after
    /// the subcommand" is a different string depending on which token you call
    /// the subcommand, and check-ci compares strings. Pinned as behaviour rather
    /// than as a comment about ordering: a comment about a string comparison is
    /// the first thing to stop being true when someone rewraps a step.
    #[test]
    fn the_lock_sits_after_the_subcommand_because_the_config_token_is_dropped() {
        let ci_step =
            "cargo --config 'any=value' build --locked -p notes-bridge-gpui --bin notes-gpui";
        let row = "cargo build --locked -p notes-bridge-gpui --bin notes-gpui";
        assert_eq!(
            signature_of(ci_step),
            signature_of(row),
            "the bridge pair only matches with the flag AFTER the subcommand"
        );
        assert_ne!(
            signature_of("cargo --locked build -p notes-bridge-gpui --bin notes-gpui"),
            signature_of(row),
            "a --locked BEFORE the subcommand is a different signature: that is the trap"
        );
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
        // --locked is NOT plumbing: it is the difference between resolving against
        // the Cargo.lock in the commit and against one cargo refreshed on the
        // runner. signature() must keep it, which is also why adding the flag to a
        // step means adding it to the ROW, not to a comment.
        assert_eq!(
            signature_of("cargo run --locked -p xtask --quiet -- check"),
            "cargo run --locked -p xtask check"
        );
        assert_ne!(
            signature_of("cargo run -p xtask -- check"),
            signature_of("cargo run --locked -p xtask -- check"),
            "a normaliser that dropped --locked would make this judge blind to it"
        );
        // The aggregate is exempt by EXACT signature, not by prefix - and the
        // exact signature now carries the lock.
        let steps = parse_ok(&gate_text(
            "
      - name: drift
        shell: bash
        run: |
          set -o pipefail
          cargo run --locked -p xtask -- check 2>&1 | tee log
      ",
        ));
        assert_eq!(decide(&[], &steps), Vec::<String>::new());
        let unlocked = parse_ok(&gate_text(
            "
      - name: drift
        shell: bash
        run: |
          set -o pipefail
          cargo run -p xtask -- check 2>&1 | tee log
      ",
        ));
        assert!(
            decide(&[], &unlocked)
                .iter()
                .any(|v| v.contains("[step-extra]")),
            "an UNLOCKED aggregate step is drift now, not plumbing"
        );
        let sneaky = parse_ok(&gate_text(
            "
      - name: drift
        shell: bash
        run: |
          set -o pipefail
          cargo run --locked -p xtask -- check --extra-flag 2>&1 | tee log
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

    /// A step body that branches on exactly the given codes, so each direction of
    fn smoke_step(arms: &[i32]) -> String {
        let mut body = String::from(
            "      - name: smoke - smoke\n        shell: bash\n        continue-on-error: true\n        \
             run: |\n          set -o pipefail\n          cargo run -p xtask --quiet -- smoke 2>&1 | tee \
             log || status=$?\n          case \"$status\" in\n",
        );
        for a in arms {
            body.push_str(&format!("            {a}) echo code {a} ;;\n"));
        }
        body.push_str("            *) echo other ;;\n          esac\n          exit $status\n");
        body
    }
    fn full_arms() -> Vec<i32> {
        crate::smoke::contract_codes()
    }

    /// The whole table, branched on: clean. Without this the other two tests prove
    /// nothing, because a rule that always fires would also "catch" a good file.
    #[test]
    fn a_step_that_branches_on_every_contracted_code_is_clean() {
        let steps = parse_ok(&gate_text(&format!("{ARCH}{}", smoke_step(&full_arms()))));

        let out = decide_contract(&steps, crate::smoke::CONTRACT);
        assert_eq!(
            out,
            ContractOutcome::Judged(Vec::new()),
            "every code has an arm, so nothing may be claimed: {out:?}"
        );
    }

    /// Direction one: smoke returns a code the workflow never branches on. This
    /// is the shape 9b84bb7b left behind - a documented verdict with no arm, and
    /// a checker that said 0 violations truthfully because it modelled commands
    /// and not codes.
    #[test]
    fn a_code_with_no_case_arm_is_reported() {
        let mut arms = full_arms();
        arms.retain(|a| *a != 5);
        let steps = parse_ok(&gate_text(&format!("{ARCH}{}", smoke_step(&arms))));
        let out = decide_contract(&steps, crate::smoke::CONTRACT);
        let ContractOutcome::Judged(v) = out else {
            panic!("both sides were readable, so this is judged: {out:?}");
        };
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].contains("[arm-missing]"), "{v:?}");
        assert!(v[0].contains("exit 5"), "must name the code: {v:?}");
        assert!(
            v[0].contains("binary older than sources"),
            "and what it means: {v:?}"
        );
    }

    /// A prose list of rule ids in ci.yml is a claim with no instrument behind it,
    /// and it drifted the moment check_unsafe grew [raw-ffi-imbalance]: the comment
    /// said six, the code printed seven, check-ci said 0 violations because it only
    /// parses commands. This pins the sentence against the const the checker
    /// actually prints from, and pins the absence of a hand-typed count word - a
    /// checked list with a number in front of it just moves the lie one word left.
    #[test]
    fn the_unsafe_rule_list_in_ci_yml_is_the_real_one() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows/ci.yml");
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()));
        let found = bracketed_ids_in_prose(&text, "enforces the");
        let mut want: Vec<String> = crate::check_unsafe::RULE_IDS
            .iter()
            .map(|s| s.to_string())
            .collect();
        want.sort();
        let mut sorted = found.clone();
        sorted.sort();
        assert_eq!(
            sorted, want,
            "the ci.yml rule list and check_unsafe::RULE_IDS must be the same set; found {found:?}"
        );
        assert_eq!(
            found.len(),
            crate::check_unsafe::RULE_IDS.len(),
            "and listed once each, not repeated"
        );
        let line = text
            .lines()
            .find(|l| l.contains("enforces the"))
            .expect("the marker sentence must stay findable");
        for typed in [
            "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
        ] {
            assert!(
                !line.contains(&format!("{typed} rule")),
                "a hand-typed count next to a checked list is the drift that just happened: {line}"
            );
        }
    }

    /// The other direction, on a synthetic string rather than on the repo file: the
    /// helper has to be able to see a rule added to the checker and not to the
    /// comment, and a rule in the comment that the checker does not print.
    #[test]
    fn the_prose_list_helper_sees_both_directions_of_the_drift() {
        let with_extra = "# enforces the rules\n# [alpha], [beta], [gamma-not-a-rule]\n# plain\n";
        assert_eq!(
            bracketed_ids_in_prose(with_extra, "enforces the"),
            ["alpha", "beta", "gamma-not-a-rule"]
                .map(String::from)
                .to_vec(),
            "an id in the comment that no checker prints must be visible to the pin"
        );
        let stopped = "# [before]\n# enforces the rules\n# [alpha]\nnot a comment\n# [zulu]\n";
        assert_eq!(
            bracketed_ids_in_prose(stopped, "enforces the"),
            [String::from("alpha")].to_vec(),
            "the run ends at the first non-comment line, so it cannot swallow the file"
        );
        assert!(
            bracketed_ids_in_prose("# nothing here\n", "enforces the").is_empty(),
            "no marker is not a pass: the pin asserts the set, and an empty set is a failure"
        );
    }

    /// Direction two: an arm for a code smoke does not publish. Dead wording, or
    /// an undocumented return value - either way the two surfaces disagree.
    ///
    /// The probe number used to be a hardcoded 9, which was a second bug in
    /// disguise: 9 is a published verdict as of the live-UI trace claim
    /// (smoke::TRACE_FAILED_EXIT), so the "unpublished" arm silently became a
    /// legitimate one and this test would have gone GREEN BY MATCHING NOTHING.
    /// Derived from the top of the table instead - one past the highest contracted
    /// code - so it cannot collide with a verdict again without somebody adding
    /// eleven codes at once.
    #[test]
    fn an_arm_for_an_unpublished_code_is_reported() {
        let mut arms = full_arms();
        let unpublished = crate::smoke::contract_codes()
            .iter()
            .max()
            .copied()
            .unwrap_or(0)
            + 1;
        assert!(
            !arms.contains(&unpublished),
            "the probe number {unpublished} is already a published verdict"
        );
        arms.push(unpublished);
        let steps = parse_ok(&gate_text(&format!("{ARCH}{}", smoke_step(&arms))));
        let out = decide_contract(&steps, crate::smoke::CONTRACT);
        let ContractOutcome::Judged(v) = out else {
            panic!("judged: {out:?}");
        };
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].contains("[arm-extra]"), "{v:?}");
        assert!(
            v[0].contains(&format!("exit {unpublished}")),
            "the violation must name the code it is about: {:?}",
            v[0]
        );
    }

    /// The refusal posture, both ways it can arise. A parse gap must never be
    /// reported as compliance.
    #[test]
    fn an_unreadable_side_refuses_to_judge() {
        let steps = parse_ok(&gate_text(ARCH));
        let out = decide_contract(&steps, crate::smoke::CONTRACT);
        assert!(matches!(out, ContractOutcome::Refused(_)), "{out:?}");
        assert!(format!("{out:?}").contains("[contract-unreadable]"));
        let empty: Vec<crate::smoke::Contract> = Vec::new();
        let steps = parse_ok(&gate_text(&format!("{ARCH}{}", smoke_step(&full_arms()))));
        assert!(matches!(
            decide_contract(&steps, &empty),
            ContractOutcome::Refused(_)
        ));
    }

    /// The catch-all is not an arm on a code: absorbing 8 into '*' is how an
    /// undocumented verdict becomes invisible, so a '*' does not close a hole.
    #[test]
    fn a_catch_all_does_not_count_as_branching() {
        let steps = parse_ok(&gate_text(&format!("{ARCH}{}", smoke_step(&[0]))));
        let out = decide_contract(&steps, crate::smoke::CONTRACT);
        let ContractOutcome::Judged(v) = out else {
            panic!("arms were parsed, so this is judged: {out:?}");
        };
        assert_eq!(v.len(), crate::smoke::CONTRACT.len() - 1, "{v:?}");
    }
}
