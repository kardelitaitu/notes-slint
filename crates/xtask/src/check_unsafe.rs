//! check-unsafe - the unsafe ledger as a RULE instead of a claim.
//!
//! AGENTS.md states an invariant: "unsafe only in platform, always with a
//! comment naming the API and the invariant". For fifteen waves the only thing
//! standing behind that sentence was the honesty of whoever typed the code, and
//! a count (14 blocks / 16 SAFETY comments) that turned out to be pinned by no
//! test at all. A number with no checker is a claim. This module makes it a
//! gate, and prints the counts on every run so the ledger is never again a
//! number somebody remembered.
//!
//! Rule ids:
//!
//! * [unsafe-outside-platform] - an unsafe code construct (block, call
//!   expression, or unsafe-qualified fn/impl/trait/extern) in any crate but
//!   notes-platform, tests and benches included. file:line.
//! * [unsafe-without-safety] - an unsafe block in platform with no comment
//!   containing the word SAFETY within [SAFETY_LOOKBACK] lines above it or two
//!   lines below (the honest shape here is a comment above a call whose
//!   arguments are built on the lines between).
//! * [local-allowance] - a per-item #[allow(unsafe_code)] /
//!   #[expect(unsafe_code)] / any unused_unsafe allowance. That is how a block
//!   escapes the ledger without being deleted. The ONE sanctioned shape is a
//!   module- or crate-root INNER #![allow(unsafe_code)] inside platform, and
//!   every one of those is printed rather than silently accepted.
//! * [allowance-outside-platform] - an inner allowance anywhere but platform.
//! * [ledger-imbalance] - more unsafe blocks than SAFETY comments. The
//!   direction is the point: a new block without a new comment is exactly the
//!   change this rule exists to catch, and two adjacent blocks can borrow one
//!   comment's lookback window, so the arithmetic catches what a per-site scan
//!   cannot.
//! * [lint-missing] / [lint-escape] - the workspace must set unsafe_code =
//!   "forbid" under [workspace.lints.rust], and notes-platform is the only
//!   manifest allowed to relax it, and never all the way to "allow".
//!
//! # What a text parse can NOT see (this is not an AST)
//!
//! The scan is line-wise, the way [crate::deps] reads manifests and
//! [crate::arch] reads metadata: no syn, no new dependency. Determinism bought,
//! completeness paid for, and the gaps are listed so nobody reads a green run as
//! a proof:
//!
//! * unsafe that exists only after macro expansion - the windows crate generating
//!   bindings, or any macro that emits unsafe - is invisible here. The workspace
//!   lint is the backstop for exactly that case, which is why [lint-missing] and
//!   [lint-escape] are part of the same rule set rather than a bonus.
//! * an "unsafe" inside a string or char literal is read as code. Comment and doc
//!   lines are skipped, so prose and docs do not count, but a literal containing
//!   the word could inflate the ledger: the error is in the strict direction.
//! * a block written as "unsafe" newline "{" is not matched as a block. rustfmt
//!   keeps them together and the fmt row gates first, so a shape that hides here
//!   does not survive the gate - alone, this checker would miss it.
//! * cfg-disabled code is counted anyway (the same conservatism [crate::arch]
//!   applies to cfg(target)): read the numbers as "at least this much unsafe is
//!   in the tree", never as "exactly this much compiles".
//! * an allowance reached through a derive, or written across several lines, is
//!   not matched.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The only crate allowed to contain unsafe, and to relax the workspace lint.
pub const PLATFORM_DIR: &str = "crates/platform";
pub const PLATFORM_CRATE: &str = "notes-platform";
pub const UNSAFE_OUTSIDE: &str = "unsafe-outside-platform";
pub const NO_SAFETY: &str = "unsafe-without-safety";
pub const LOCAL_ALLOWANCE: &str = "local-allowance";
pub const ALLOWANCE_OUTSIDE: &str = "allowance-outside-platform";
pub const LEDGER_IMBALANCE: &str = "ledger-imbalance";
pub const LINT_MISSING: &str = "lint-missing";
pub const LINT_ESCAPE: &str = "lint-escape";

/// How far above an unsafe block a SAFETY comment may sit.
pub const SAFETY_LOOKBACK: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule: &'static str,
    pub location: String,
    pub detail: String,
}

impl Finding {
    pub fn message(&self) -> String {
        format!(
            "UNSAFE VIOLATION: [{}] {} - {}",
            self.rule, self.location, self.detail
        )
    }
}

/// The printed ledger. Every field is a measurement of the current tree.
#[derive(Debug, Default, Clone)]
pub struct Ledger {
    pub blocks: usize,
    pub safety_comments: usize,
    pub declarations: usize,
    pub allowances: Vec<String>,
    pub files: usize,
}
fn is_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//") && !t.starts_with("///") && !t.starts_with("//!")
}

fn is_doc(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("///") || t.starts_with("//!") || t.starts_with("#[doc")
}

/// Everything before a line comment: "unsafe" in trailing prose is not code.
fn code_part(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// True when the line holds the bare word "unsafe" as a token.
fn unsafe_token(code: &str) -> bool {
    let mut hit = false;
    for (idx, word) in code.split([' ', '\t', '{', '(', ';']).enumerate() {
        let _ = idx;
        if word == "unsafe" {
            hit = true;
        }
    }
    hit
}

/// An executed unsafe block or unsafe call expression: "unsafe {" / "unsafe (".
fn is_block(line: &str) -> bool {
    let code = code_part(line);
    let stripped = code.trim_start();
    unsafe_token(code)
        && (stripped.contains("unsafe {")
            || stripped.contains("unsafe{")
            || stripped.contains("unsafe ("))
}

/// unsafe fn / impl / trait / extern - a declaration, not an executed block.
fn is_declaration(line: &str) -> bool {
    let code = code_part(line);
    if !unsafe_token(code) {
        return false;
    }
    let mut seen = false;
    for word in code.split_whitespace() {
        if seen {
            let w = word.trim_start_matches('(').trim_start_matches('"');
            return w.starts_with("extern")
                || w.starts_with("fn")
                || w.starts_with("impl")
                || w.starts_with("trait");
        }
        if word == "unsafe" {
            seen = true;
        }
    }
    false
}

/// Which allowance this line is, and whether it is the inner (module-root) form.
fn is_allowance(line: &str) -> Option<(&'static str, bool)> {
    let t = line.trim_start();
    if !t.starts_with("#[") && !t.starts_with("#![") {
        return None;
    }
    let inner = t.starts_with("#![");
    let body = t.replace(' ', "");
    if body.contains("allow(unsafe_code)") || body.contains("expect(unsafe_code)") {
        return Some(("unsafe_code", inner));
    }
    if body.contains("unused_unsafe") {
        return Some(("unused_unsafe", inner));
    }
    None
}

/// Scan one platform file. Pure over text, so every failure mode is testable
/// without a workspace.
pub fn scan_platform(file: &str, text: &str) -> (Ledger, Vec<Finding>) {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut ledger = Ledger {
        files: 1,
        ..Default::default()
    };
    let mut findings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if is_doc(line) {
            continue;
        }
        if let Some((which, inner)) = is_allowance(line) {
            if inner && which == "unsafe_code" {
                ledger.allowances.push(format!("{file}:{}", idx + 1));
            } else {
                findings.push(Finding {
                    rule: LOCAL_ALLOWANCE,
                    location: format!("{file}:{}", idx + 1),
                    detail: format!(
                        "a per-item allowance ({which}) hides unsafe from the ledger; only a \
                         module-root #![allow(unsafe_code)] inside platform is sanctioned, and it \
                         is printed"
                    ),
                });
            }
            continue;
        }
        if is_comment(line) {
            if line.contains("SAFETY") {
                ledger.safety_comments += 1;
            }
            continue;
        }
        if is_block(line) {
            ledger.blocks += 1;
            let from = idx.saturating_sub(SAFETY_LOOKBACK);
            let above = lines[from..idx]
                .iter()
                .any(|l| is_comment(l) && l.contains("SAFETY"));
            let below = lines
                .get(idx + 1..idx + 3)
                .is_some_and(|w| w.iter().any(|l| is_comment(l) && l.contains("SAFETY")));
            if !above && !below {
                findings.push(Finding {
                    rule: NO_SAFETY,
                    location: format!("{file}:{}", idx + 1),
                    detail: format!(
                        "unsafe block with no comment naming SAFETY within {SAFETY_LOOKBACK} \
                         lines above or 2 below"
                    ),
                });
            }
        } else if is_declaration(line) {
            ledger.declarations += 1;
        }
    }
    (ledger, findings)
}

/// Scan a file in a crate that may not contain unsafe at all. A SAFETY comment
/// buys nothing here: the location is the violation.
pub fn scan_foreign(file: &str, text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for (idx, line) in text.split('\n').enumerate() {
        if is_doc(line) || is_comment(line) {
            continue;
        }
        if let Some((which, inner)) = is_allowance(line) {
            out.push(Finding {
                rule: ALLOWANCE_OUTSIDE,
                location: format!("{file}:{}", idx + 1),
                detail: format!(
                    "a {which} allowance{} outside platform - unsafe is only allowed in {}",
                    if inner { " (inner)" } else { "" },
                    PLATFORM_CRATE
                ),
            });
            continue;
        }
        if is_block(line) || is_declaration(line) {
            out.push(Finding {
                rule: UNSAFE_OUTSIDE,
                location: format!("{file}:{}", idx + 1),
                detail: format!(
                    "unsafe code outside platform: {}",
                    code_part(line).trim().chars().take(60).collect::<String>()
                ),
            });
        }
    }
    out
}
/// The lint configuration, read line-wise out of the manifests the way
/// [crate::deps] reads them: the workspace forbids, platform may relax, nobody
/// else may touch it. "crates" is (package name, manifest text).
pub fn check_lints(workspace_manifest: &str, crates: &[(String, String)]) -> Vec<Finding> {
    fn unsafe_code_level(text: &str) -> Option<String> {
        let mut in_lints = false;
        for line in text.split('\n') {
            let t = line.trim();
            if t.starts_with('[') {
                in_lints = t == "[lints.rust]"
                    || t == "[workspace.lints.rust]"
                    || t == "[lints]"
                    || t == "[workspace.lints]";
                continue;
            }
            if in_lints && t.starts_with("unsafe_code") {
                return t
                    .split_once('=')
                    .map(|(_, v)| v.trim().trim_matches('"').to_string());
            }
        }
        None
    }
    let mut findings = Vec::new();
    match unsafe_code_level(workspace_manifest).as_deref() {
        None => findings.push(Finding {
            rule: LINT_MISSING,
            location: "Cargo.toml [workspace.lints.rust]".to_string(),
            detail: "no unsafe_code setting - unsafe is not forbidden by default, so every crate \
                     is one missing attribute away from allowing it"
                .to_string(),
        }),
        Some(level) if level != "forbid" && level != "deny" => findings.push(Finding {
            rule: LINT_MISSING,
            location: "Cargo.toml [workspace.lints.rust]".to_string(),
            detail: format!("workspace unsafe_code = \"{level}\"; the invariant is forbid"),
        }),
        Some(_) => {}
    }
    for (name, manifest) in crates {
        let Some(level) = unsafe_code_level(manifest) else {
            continue;
        };
        if name == PLATFORM_CRATE {
            if level == "allow" {
                findings.push(Finding {
                    rule: LINT_ESCAPE,
                    location: format!("{PLATFORM_DIR}/Cargo.toml"),
                    detail: "platform sets unsafe_code = \"allow\", which deletes the invariant: \
                             unsafe would need no comment at all"
                        .to_string(),
                });
            }
            continue;
        }
        // "forbid" again (core does) and "deny" are both stricter-than-nothing
        // and fine; anything looser outside platform is the escape.
        if level != "forbid" && level != "deny" {
            findings.push(Finding {
                rule: LINT_ESCAPE,
                location: name.clone(),
                detail: format!(
                    "unsafe_code = \"{level}\" outside platform - only {PLATFORM_CRATE} may relax \
                     the workspace setting"
                ),
            });
        }
    }
    findings
}

/// Fold the ledger arithmetic in. Two adjacent blocks can both sit inside one
/// comment's lookback window and pass the per-site rule; the count is what says
/// something is under-explained. One direction only - more comments than blocks
/// is a well-documented crate, not a violation.
pub fn ledger_findings(ledger: &Ledger) -> Option<Finding> {
    if ledger.blocks <= ledger.safety_comments {
        return None;
    }
    Some(Finding {
        rule: LEDGER_IMBALANCE,
        location: PLATFORM_CRATE.to_string(),
        detail: format!(
            "{} unsafe blocks against {} SAFETY comments: something was added without naming what \
             it makes safe",
            ledger.blocks, ledger.safety_comments
        ),
    })
}

fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            walk_rs(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn member_packages(meta: &Value) -> Result<Vec<(String, PathBuf)>, String> {
    let members: BTreeSet<String> = meta
        .get("workspace_members")
        .and_then(Value::as_array)
        .ok_or_else(|| "metadata: missing workspace_members".to_string())?
        .iter()
        .filter_map(|m| m.as_str().map(str::to_string))
        .collect();
    let mut out = Vec::new();
    for pkg in meta
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "metadata: missing packages".to_string())?
    {
        let Some(id) = pkg.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !members.contains(id) {
            continue;
        }
        let name = pkg
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();
        let Some(manifest) = pkg.get("manifest_path").and_then(Value::as_str) else {
            continue;
        };
        out.push((name, PathBuf::from(manifest)));
    }
    Ok(out)
}

/// Entry point for "cargo xtask check-unsafe". Takes no arguments at all.
pub fn run(args: &[String]) -> i32 {
    for arg in args {
        if arg.starts_with('-') {
            eprintln!(
                "check-unsafe: unsupported flag '{arg}' - this subcommand takes no arguments"
            );
            eprintln!("check-unsafe: usage: cargo xtask check-unsafe");
            return 2;
        }
    }
    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("check-unsafe: {e}");
            return 2;
        }
    };
    let root = match crate::metadata::find_workspace_root(&cwd) {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("check-unsafe: {e}");
            return 2;
        }
    };
    let meta = match crate::metadata::cargo_metadata(&root) {
        Ok(value) => value,
        Err(e) => {
            eprintln!("check-unsafe: {e}");
            return 2;
        }
    };
    let packages = match member_packages(&meta) {
        Ok(p) if p.is_empty() => {
            eprintln!("check-unsafe: metadata named no workspace members; refusing to judge");
            return 2;
        }
        Ok(p) => p,
        Err(e) => {
            eprintln!("check-unsafe: {e}");
            return 2;
        }
    };
    let workspace_manifest = match fs::read_to_string(root.join("Cargo.toml")) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("check-unsafe: cannot read the workspace manifest: {e}");
            return 2;
        }
    };
    let mut manifests = Vec::new();
    let mut findings = Vec::new();
    let mut ledger = Ledger::default();
    let mut saw_platform = false;
    for (name, manifest_path) in &packages {
        let dir = manifest_path.parent().unwrap_or(Path::new("."));
        manifests.push((
            name.clone(),
            fs::read_to_string(manifest_path).unwrap_or_default(),
        ));
        let is_platform = dir
            .file_name()
            .map(|d| d.to_string_lossy().to_lowercase())
            .is_some_and(|d| d == "platform");
        saw_platform |= is_platform;
        let mut files = Vec::new();
        for sub in ["src", "tests", "benches", "examples"] {
            walk_rs(&dir.join(sub), &mut files);
        }
        if dir.join("build.rs").is_file() {
            files.push(dir.join("build.rs"));
        }
        if is_platform && files.is_empty() {
            eprintln!("check-unsafe: {PLATFORM_CRATE} has no readable .rs files; refusing");
            return 2;
        }
        for path in files {
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(path.as_path())
                .to_string_lossy()
                .replace('\\', "/");
            let Ok(source) = fs::read_to_string(&path) else {
                findings.push(Finding {
                    rule: LINT_MISSING,
                    location: rel,
                    detail: "a member source file could not be read, so its unsafe count is \
                             unknown"
                        .to_string(),
                });
                continue;
            };
            if is_platform {
                let (one, mut f) = scan_platform(&rel, &source);
                ledger.blocks += one.blocks;
                ledger.safety_comments += one.safety_comments;
                ledger.declarations += one.declarations;
                ledger.files += one.files;
                ledger.allowances.extend(one.allowances);
                findings.append(&mut f);
            } else {
                findings.extend(scan_foreign(&rel, &source));
                ledger.files += 1;
            }
        }
    }
    if !saw_platform {
        eprintln!("check-unsafe: no {PLATFORM_DIR} member in metadata; refusing to judge");
        return 2;
    }
    findings.extend(check_lints(&workspace_manifest, &manifests));
    findings.extend(ledger_findings(&ledger));

    for line in &findings {
        println!("{}", line.message());
    }
    println!(
        "unsafe: {} blocks, {} SAFETY comments in {PLATFORM_CRATE} ({} source files scanned, {} \
         unsafe declarations)",
        ledger.blocks, ledger.safety_comments, ledger.files, ledger.declarations
    );
    if ledger.allowances.is_empty() {
        println!("unsafe: no module-root #![allow(unsafe_code)] anywhere");
    } else {
        println!(
            "unsafe: module-root allowances printed, not assumed: {}",
            ledger.allowances.join("; ")
        );
    }
    println!("unsafe: {} violations", findings.len());
    if findings.is_empty() { 0 } else { 1 }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn plat(text: &str) -> (Ledger, Vec<Finding>) {
        scan_platform("crates/platform/src/windows/x.rs", &fix(text))
    }

    /// The fixtures spell the keyword ~U~ on purpose: this module's own scanner
    /// reads a string literal containing the real thing as code (its documented
    /// text-parse limit, and the strict direction), so a test written normally
    /// would flag the test file as containing unsafe. The normaliser only
    /// substitutes the keyword back; it cannot hide a violation.
    fn fix(text: &str) -> String {
        text.replace("~U~", &["un", "safe"].concat())
    }

    /// The shape the crate already uses: a SAFETY line, some argument building,
    /// then the block. This must be green, or the rule is decoration.
    #[test]
    fn a_block_with_a_safety_comment_is_clean_and_counted() {
        let src = "fn f(h: HWND) -> bool {\n    // SAFETY: the handle came from IsWindow.\n    let flags = 0;\n    if ~U~ { IsWindow(Some(h)) }.as_bool() {\n        let _ = flags;\n        true\n    } else {\n        false\n    }\n}\n";
        let (ledger, findings) = plat(src);
        assert!(findings.is_empty(), "{findings:?}");
        assert_eq!((ledger.blocks, ledger.safety_comments), (1, 1));
    }

    #[test]
    fn an_uncommented_unsafe_block_is_red_with_a_file_and_line() {
        let src =
            "fn sneaky(h: HWND) {\n    let x = 1;\n    ~U~ { DoTheThing(h) };\n    let _ = x;\n}\n";
        let (ledger, findings) = plat(src);
        assert_eq!(ledger.blocks, 1);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].rule, NO_SAFETY);
        assert_eq!(findings[0].location, "crates/platform/src/windows/x.rs:3");
        assert!(findings[0].message().contains("[unsafe-without-safety]"));
    }

    #[test]
    fn the_lookback_is_finite_so_a_far_comment_does_not_excuse_a_block() {
        let mut src = String::from("    // SAFETY: about a different call entirely\n");
        for i in 0..30 {
            src.push_str(&format!("    let pad = {i};\n"));
        }
        src.push_str("    ~U~ { Another(h) };\n");
        let (_, findings) = plat(&src);
        assert_eq!(findings.len(), 1, "the excuse must be nearby: {findings:?}");
        assert_eq!(findings[0].rule, NO_SAFETY);
    }

    #[test]
    fn a_per_item_allowance_is_red_and_a_module_root_one_is_printed() {
        let outer = "#[allow(unsafe_code)]\nfn hidden() {\n    ~U~ { X() }\n}\n";
        let (_, findings) = plat(outer);
        assert!(
            findings.iter().any(|f| f.rule == LOCAL_ALLOWANCE),
            "an item allowance must not hide: {findings:?}"
        );
        for line in ["#[expect(unsafe_code)]\n", "#![allow(unused_unsafe)]\n"] {
            let f = plat(line).1;
            assert_eq!(f.len(), 1, "must be red: {f:?}");
            assert_eq!(f[0].rule, LOCAL_ALLOWANCE);
        }
        let (ledger, findings) = scan_platform(
            "crates/platform/src/windows/mod.rs",
            "#![allow(unsafe_code)]\n",
        );
        assert!(findings.is_empty(), "the sanctioned shape: {findings:?}");
        assert_eq!(ledger.allowances, ["crates/platform/src/windows/mod.rs:1"]);
    }

    /// The arithmetic catches what a per-site window cannot: two blocks, one
    /// comment, both inside the lookback.
    #[test]
    fn more_blocks_than_comments_fails_even_when_each_block_is_covered() {
        let src = "// SAFETY: this explains the first call.\n~U~ { A() };\n~U~ { B() };\n";
        let (ledger, findings) = plat(src);
        assert_eq!((ledger.blocks, ledger.safety_comments), (2, 1));
        assert!(findings.iter().all(|f| f.rule != NO_SAFETY), "{findings:?}");
        let all = ledger_findings(&ledger).expect("2 blocks / 1 comment must be an imbalance");
        assert_eq!(all.rule, LEDGER_IMBALANCE);
        assert_eq!(ledger.declarations, 0);
        // The other direction is a documented crate, not a violation.
        let over = Ledger {
            blocks: 1,
            safety_comments: 25,
            ..Default::default()
        };
        assert!(ledger_findings(&over).is_none());
    }

    #[test]
    fn unsafe_in_a_core_path_file_is_red_even_with_a_safety_comment() {
        let cases = [
            ("crates/core/src/save.rs", "    ~U~ { libc::free(p) };\n"),
            (
                "crates/api/tests/geometry.rs",
                "    // SAFETY: does not matter outside platform\n    ~U~ { Call() };\n",
            ),
            ("crates/bridge-gpui/src/main.rs", "~U~ fn raw() {}\n"),
            (
                "crates/xtask/src/smoke.rs",
                "    let x = ~U~ { std::mem::zeroed() };\n",
            ),
        ];
        for (file, src) in cases {
            let findings = scan_foreign(file, &fix(src));
            assert_eq!(findings.len(), 1, "{file} must be red: {findings:?}");
            assert_eq!(findings[0].rule, UNSAFE_OUTSIDE);
            assert!(
                findings[0].location.contains(file),
                "must name the file: {}",
                findings[0].location
            );
            assert!(
                findings[0].location.contains(':'),
                "must name the line: {}",
                findings[0].location
            );
        }
        let f = scan_foreign("crates/core/src/lib.rs", "#![allow(unsafe_code)]\n");
        assert_eq!(f[0].rule, ALLOWANCE_OUTSIDE);
    }

    #[test]
    fn the_lint_config_is_part_of_the_same_rule() {
        let good_ws =
            "[workspace]\nmembers = []\n\n[workspace.lints.rust]\nunsafe_code = \"forbid\"\n";
        let platform = (
            PLATFORM_CRATE.to_string(),
            "[lints.rust]\nunsafe_code = \"deny\"\n".to_string(),
        );
        assert!(
            check_lints(good_ws, std::slice::from_ref(&platform)).is_empty(),
            "the shape the real tree has must be clean"
        );
        let core_forbids = (
            "notes-core".to_string(),
            "[lints.rust]\nunsafe_code = \"forbid\"\n".to_string(),
        );
        assert!(
            check_lints(good_ws, &[platform, core_forbids]).is_empty(),
            "restating forbid is not an escape"
        );
        let none = check_lints("[workspace]\nmembers = []\n", &[]);
        assert_eq!(none[0].rule, LINT_MISSING);
        let loose = check_lints("[workspace.lints.rust]\nunsafe_code = \"warn\"\n", &[]);
        assert_eq!(loose[0].rule, LINT_MISSING);
        let escape = check_lints(
            good_ws,
            &[(
                "notes-core".to_string(),
                "[lints.rust]\nunsafe_code = \"allow\"\n".to_string(),
            )],
        );
        assert_eq!(escape[0].rule, LINT_ESCAPE);
        let too_far = check_lints(
            good_ws,
            &[(
                PLATFORM_CRATE.to_string(),
                "[lints.rust]\nunsafe_code = \"allow\"\n".to_string(),
            )],
        );
        assert_eq!(
            too_far[0].rule, LINT_ESCAPE,
            "platform may not delete the rule either"
        );
    }

    #[test]
    fn docs_and_prose_are_not_unsafe_code() {
        let src = "/// Calls unsafe SetWindowPos when it must.\n//! unsafe lives here, always with a comment.\nfn f() {}\n";
        let (ledger, findings) = scan_platform("crates/platform/src/lib.rs", &fix(src));
        assert_eq!(
            (ledger.blocks, ledger.safety_comments, ledger.declarations),
            (0, 0, 0)
        );
        assert!(findings.is_empty(), "{findings:?}");
        assert!(scan_foreign("crates/core/src/lib.rs", src).is_empty());
        let one = plat("    ~U~ { A() }; // one block, not two\n").0;
        assert_eq!(one.blocks, 1);
    }

    #[test]
    fn a_declaration_is_counted_separately_from_a_block() {
        let src = "pub ~U~ extern \"system\" {\n    fn SetWindowPos() -> i32;\n}\n";
        let (ledger, findings) = plat(src);
        assert_eq!(ledger.declarations, 1);
        assert_eq!(
            ledger.blocks, 0,
            "an extern block declares, it does not execute"
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn it_takes_no_flags_and_says_so() {
        // The check-ci trap, pre-empted: a flag is never read as a path.
        assert_eq!(run(&["--offline".to_string()]), 2);
    }
}
