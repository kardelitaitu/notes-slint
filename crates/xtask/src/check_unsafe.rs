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
//! * [raw-ffi-imbalance] - THE TRIPWIRE, and the reason this rule counts extern
//!   blocks at all. A crate holding MORE extern blocks than unsafe-qualified
//!   declarations has by construction at least one BARE `extern "..." { }`, which
//!   is the shape every other rule here is blind to. It is arithmetic rather than
//!   a pattern list on purpose: it fires on a spelling this file has never seen,
//!   and its message names the subtraction, mirroring [crate::arch]'s
//!   lineage-unrecognised rule, which says out loud that it keys on names it may
//!   not know.
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
//! * a BARE extern "system" { fn ... } declaration is still INVISIBLE to every PATTERN
//!   rule here: the rules key on the unsafe token, and the 2024 edition does not
//!   require unsafe to DECLARE a foreign function. Measured and pinned by
//!   [the_scanner_sees_the_2024_form_and_is_blind_to_the_bare_one], which this slice
//!   deliberately leaves true - [scan_foreign] does not match a bare block, and if a
//!   future edit makes it match, that test is the thing that has to be rewritten on
//!   purpose rather than a number quietly re-aimed. What closed is the OTHER half: the
//!   ledger now counts extern BLOCKS per crate ([count_ffi]) and subtracts the
//!   unsafe-qualified declarations from them ([ffi_findings]), so a crate that carries a
//!   bare one is caught by arithmetic without anyone having to think of the spelling.
//!   Funded shape, now built: per-crate count, allowed home printed, tripwire firing on
//!   the count rather than on the pattern.
//! * what the tripwire still cannot see, stated so a green row is not read as a proof:
//!   an extern block whose `{` sits on the next line (rustfmt keeps it on the header
//!   line and the fmt row gates first, but alone this would miss it, exactly as the
//!   "unsafe" newline "{" limit above); an extern block emitted by a macro, which the
//!   text reader never sees at all; and the MASKING case - one bare block plus one
//!   unrelated `unsafe fn` in the same crate is 1 block against 1 declaration, so the
//!   subtraction says nothing. The mask only exists where unsafe declarations are
//!   already legal: outside notes-platform an unsafe declaration is itself a
//!   [unsafe-outside-platform] finding, so in every crate that is not the sanctioned
//!   home the denominator is zero by construction and a single bare block fires. That is
//!   why the rule is trusted where it matters and soft where it is allowed to be.
//! * a build script adding `-lkernel32` with no call anywhere is invisible to every
//!   instrument in this repo, not just to this one: check-arch reads metadata edges and
//!   a link flag is not an edge, and no self-check can notice an ABSENCE of a call. That
//!   hole is documented on purpose and stays open.

use std::collections::{BTreeMap, BTreeSet};
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
/// The tripwire: a crate holding MORE extern blocks than unsafe-qualified
/// declarations is holding a bare one, counted rather than pattern-matched.
pub const RAW_FFI_IMBALANCE: &str = "raw-ffi-imbalance";
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

/// One crate's share of the raw-FFI row of the ledger, counted over EVERY file the
/// package owns - sources, tests, benches, examples and build.rs. `extern_blocks`
/// counts block HEADERS whether or not they carry the unsafe keyword; that is the
/// half this file could not previously see. `unsafe_declarations` is the same
/// measurement as [Ledger::declarations], kept per crate because the tripwire
/// compares the two INSIDE a crate: across the workspace the numbers are not
/// comparable at all.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FfiCensus {
    pub extern_blocks: usize,
    pub unsafe_declarations: usize,
}

impl FfiCensus {
    /// Fold one file into its crate row. Sum, never max: two files holding one
    /// bare block each must not read as one.
    pub fn add(&mut self, other: &FfiCensus) {
        self.extern_blocks += other.extern_blocks;
        self.unsafe_declarations += other.unsafe_declarations;
    }

    /// What the subtraction proves about this crate. Positive means a bare block
    /// exists; zero is NOT a proof that none does - one unrelated unsafe
    /// declaration in the same crate balances it there.
    pub fn bare_by_count(&self) -> usize {
        self.extern_blocks.saturating_sub(self.unsafe_declarations)
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
    /// Raw FFI by crate. A BTreeMap so the printout is in name order and a diff
    /// of two runs is a diff of the tree, not of a hash map's iteration.
    pub ffi: BTreeMap<String, FfiCensus>,
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

/// True when the line OPENS an extern block: `extern` (optionally preceded by
/// `unsafe`, optionally followed by an ABI string) with the `{` on the same line.
/// Deliberately does NOT require the unsafe keyword - that is the whole point, the
/// 2024 edition makes the keyword optional at declaration time. Requires an ABI
/// string or a brace so it does not read `extern crate`, nor a one-line
/// `extern "system" fn f() {}`, as a block.
fn is_extern_block(line: &str) -> bool {
    let words: Vec<&str> = code_part(line).split_whitespace().collect();
    let Some(at) = words.iter().position(|w| *w == "extern") else {
        return false;
    };
    // The header is `extern`, then at most one ABI string, then `{`. An ABI is
    // matched by SHAPE, not from a list of spellings - a new calling convention
    // is still an extern block, and a name list here would be the thing
    // [crate::arch]'s lineage rule refuses to be.
    let rest = match words.get(at + 1) {
        Some(w) => *w,
        None => return false,
    };
    // `extern {` - no ABI, C by default, and it opens a block right here.
    if rest.starts_with('{') {
        return true;
    }
    // An ABI literal must OPEN with a real quote in the source text. A backslash
    // does not: `extern \"system\"` is a string in somebody's fixture, not a block.
    if !rest.starts_with('"') {
        return false; // `extern crate`, `extern "C" fn ...`: not this line's block
    }
    let after_quote = &rest[1..];
    match after_quote.find('"') {
        // `"system"{` or `"system"` - the brace is either glued on or the next word.
        Some(i) => {
            let tail = &after_quote[i + 1..];
            tail.contains('{') || words.get(at + 2).is_some_and(|w| w.starts_with('{'))
        }
        None => false,
    }
}

/// The raw-FFI census for one file, over EVERY line the ledger ignores nothing:
/// doc and comment prose is skipped like everywhere else here, so a paragraph
/// about extern blocks does not arm the tripwire.
pub fn count_ffi(text: &str) -> FfiCensus {
    let mut census = FfiCensus::default();
    for line in text.split('\n') {
        if is_doc(line) || is_comment(line) {
            continue;
        }
        if is_extern_block(line) {
            census.extern_blocks += 1;
        }
        if is_declaration(line) {
            census.unsafe_declarations += 1;
        }
    }
    census
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

/// THE TRIPWIRE, and the reason the extern count is in the ledger at all. Inside
/// one crate, `extern_blocks > unsafe_declarations` can only mean the crate holds
/// at least `blocks - declarations` BARE extern blocks - the spelling every other
/// rule here keys on `unsafe` for, and therefore cannot see. No pattern list is
/// involved, so an ABI or a formatting trick nobody thought of still fires it.
///
/// It also says where its own eyes are wrong, the way [crate::arch]'s
/// lineage-unrecognised rule does: an unrelated `unsafe fn` in the same crate
/// balances the subtraction and a bare block then reads green, and a block emitted
/// by a macro is counted on neither side. A clean row here is arithmetic, not a
/// proof - which is exactly what the message of a red row is worth.
pub fn ffi_findings(ffi: &BTreeMap<String, FfiCensus>) -> Vec<Finding> {
    let mut out = Vec::new();
    for (name, census) in ffi {
        if census.extern_blocks <= census.unsafe_declarations {
            continue;
        }
        let bare = census.extern_blocks - census.unsafe_declarations;
        out.push(Finding {
            rule: RAW_FFI_IMBALANCE,
            location: name.clone(),
            detail: format!(
                "{} extern blocks against {} unsafe-qualified declarations: {} - {} = {} BARE \
                 extern block(s). A bare block carries no unsafe token, so every rule above is blind to it by \
                 construction; this finding is the SUBTRACTION, so it never needed to recognise the spelling that \
                 hid it. It is also admitting where its own eyes are: a block a macro emits is counted on neither \
                 side, one more unsafe declaration in this crate would have balanced the subtraction to green, and \
                 a build script passing -lkernel32 leaves no count for anything to compare. {} is the only crate \
                 allowed to declare foreign functions{}",
                census.extern_blocks,
                census.unsafe_declarations,
                census.extern_blocks,
                census.unsafe_declarations,
                bare,
                PLATFORM_CRATE,
                if name == PLATFORM_CRATE {
                    "- so here it is legal Rust the SAFETY ledger cannot see at all: write the API \
                     and the invariant into a comment above the block, or qualify the block as unsafe \
                     extern so the ledger counts it"
                        .to_string()
                } else {
                    format!(" - and this is not it; raw FFI belongs in {PLATFORM_DIR}")
                }
            ),
        });
    }
    out
}

/// The tripwire's own arithmetic, spelled out for the green case too: the largest
/// bare-by-count total the ledger can prove. Zero means no crate has more blocks
/// than declarations, which is a claim about subtraction, not about the scanner
/// having seen every shape.
pub fn bare_by_count(ffi: &BTreeMap<String, FfiCensus>) -> usize {
    ffi.values().map(FfiCensus::bare_by_count).sum()
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
            // The raw-FFI census is taken over EVERY crate, platform included: the
            // tripwire is per-crate arithmetic, so the sanctioned home needs its own
            // row as much as the crate that might be smuggling one does.
            let census = count_ffi(&source);
            ledger.ffi.entry(name.clone()).or_default().add(&census);
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
    findings.extend(ffi_findings(&ledger.ffi));

    for line in &findings {
        println!("{}", line.message());
    }
    println!(
        "unsafe: {} blocks, {} SAFETY comments in {PLATFORM_CRATE} ({} source files scanned, {} \
         unsafe declarations)",
        ledger.blocks, ledger.safety_comments, ledger.files, ledger.declarations
    );
    let extern_total: usize = ledger.ffi.values().map(|c| c.extern_blocks).sum();
    let declared_total: usize = ledger.ffi.values().map(|c| c.unsafe_declarations).sum();
    let bare_total = bare_by_count(&ledger.ffi);
    let (worst_crate, worst) = ledger
        .ffi
        .iter()
        .map(|(n, c)| (n.clone(), c.bare_by_count()))
        .max_by_key(|(_, b)| *b)
        .unwrap_or_else(|| ("no crate".to_string(), 0));
    let rows: Vec<String> = ledger
        .ffi
        .iter()
        .map(|(n, c)| {
            format!(
                "{n}: {} - {} = {}",
                c.extern_blocks,
                c.unsafe_declarations,
                c.bare_by_count()
            )
        })
        .collect();
    println!(
        "unsafe: raw FFI per crate, extern blocks - unsafe-qualified declarations = bare by \
         count. The only home allowed to declare foreign functions is {PLATFORM_CRATE} at \
         {PLATFORM_DIR}, so every row but that one must read 0 - 0 = 0: {}",
        rows.join(", ")
    );
    println!(
        "unsafe: tripwire {extern_total} extern blocks - {declared_total} unsafe-qualified \
         declarations = {bare_total} bare across the tree; the worst single crate is \
         {worst_crate} at {worst}. A positive number is a [raw-ffi-imbalance] finding above. \
         Zero is arithmetic, not a survey: a bare block still matches no pattern here, a block \
         a macro emits is counted on neither side, and a build script passing -lkernel32 leaves \
         no count anywhere to subtract."
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
    /// would flag the test file as containing unsafe. ~E~ is the same trap for the
    /// raw-FFI census, which keys on the word `extern` and not on `unsafe`: a
    /// fixture that opened an extern block in ordinary text would arm the tripwire
    /// against crates/xtask itself. The normaliser only substitutes the keywords
    /// back; it cannot hide a violation.
    fn fix(text: &str) -> String {
        text.replace("~U~", &["un", "safe"].concat())
            .replace("~E~", &["ex", "tern"].concat())
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
        let src = "pub ~U~ ~E~ \"system\" {\n    fn SetWindowPos() -> i32;\n}\n";
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

    /// WHERE THE TEXT READER ENDS. Measured, not assumed: the real declaration in
    /// crates/platform/src/windows/paths.rs is the 2024 form 'unsafe extern
    /// "system"', which this scanner DOES see, so a hand-spelled kernel binding in
    /// core, api or xtask is a finding today. What the scanner cannot see is a
    /// BARE 'extern "system"' block, which the 2024 edition allows for declaring.
    /// That residual gap is stated rather than papered over, and it is narrow: the
    /// edition still requires an unsafe block to CALL such a function, so a bare
    /// declaration that is actually used is caught at the call site, and one that
    /// is not used reaches nothing at all.
    #[test]
    fn the_scanner_sees_the_2024_form_and_is_blind_to_the_bare_one() {
        // Spelled with the ~U~ sentinel and normalised by fix(), per this module's
        // own convention: the scanner reads a literal containing the keyword as
        // code, so a test written normally would flag its own file.
        let ffi_2024 =
            "~U~ ~E~ \"system\" {\n    fn GetFinalPathNameByHandleW(a: u32) -> i32;\n}\n";
        let found: Vec<String> = scan_foreign("crates/core/src/x.rs", &fix(ffi_2024))
            .iter()
            .map(|f| f.rule.to_string())
            .collect();
        assert_eq!(found, vec![UNSAFE_OUTSIDE.to_string()], "{found:?}");
        let call = "    let code = ~U~ { GetFinalPathNameByHandleW(h) };\n";
        assert!(
            !scan_foreign("crates/core/src/x.rs", &fix(call)).is_empty(),
            "the USE of any FFI needs an unsafe block in every edition, and that is caught"
        );
        let bare = "~E~ \"system\" {\n    fn GetFinalPathNameByHandleW(a: u32) -> i32;\n}\n";
        assert!(
            scan_foreign("crates/core/src/x.rs", &fix(bare)).is_empty(),
            "documented blind spot: a bare extern DECLARATION is invisible to a reader that
             keys on the unsafe keyword - and on its own it is also dead code"
        );
    }

    /// THE OTHER HALF of the gap pinned above, and the funded reason for this
    /// slice: the scanner still cannot match the bare block - the census counts it,
    /// and the subtraction is what fires.
    #[test]
    fn the_census_counts_the_bare_block_the_scanner_cannot_see() {
        let bare = "~E~ \"system\" {\n    fn GetFinalPathNameByHandleW(a: u32) -> i32;\n}\n";
        let c = count_ffi(&fix(bare));
        assert_eq!(
            (c.extern_blocks, c.unsafe_declarations),
            (1, 0),
            "a bare block is counted as a block and as nothing else",
        );
        let qualified = "~U~ ~E~ \"system\" {\n    fn SetWindowPos() -> i32;\n}\n";
        let q = count_ffi(&fix(qualified));
        assert_eq!(
            (q.extern_blocks, q.unsafe_declarations),
            (1, 1),
            "the 2024 form is one block AND one unsafe declaration",
        );
        assert_eq!(q.bare_by_count(), 0, "1 - 1 proves nothing was hidden");
        assert_eq!(c.bare_by_count(), 1, "1 - 0 is the finding");
    }

    #[test]
    fn the_tripwire_message_names_the_arithmetic_and_the_allowed_home() {
        let mut ffi = BTreeMap::new();
        ffi.insert(
            "notes-core".to_string(),
            FfiCensus {
                extern_blocks: 3,
                unsafe_declarations: 1,
            },
        );
        let f = ffi_findings(&ffi);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule, RAW_FFI_IMBALANCE);
        assert_eq!(f[0].location, "notes-core");
        let m = f[0].message();
        for needed in [
            "3 extern blocks against 1 unsafe-qualified declarations",
            "3 - 1 = 2 BARE",
            PLATFORM_CRATE,
            PLATFORM_DIR,
            "SUBTRACTION",
            "macro",
            "-lkernel32",
        ] {
            assert!(m.contains(needed), "the message must say {needed}: {m}");
        }
        assert!(
            m.contains("and this is not it"),
            "a crate that is not the home is told so: {m}"
        );
        let in_home = {
            let mut one = BTreeMap::new();
            one.insert(
                PLATFORM_CRATE.to_string(),
                FfiCensus {
                    extern_blocks: 2,
                    unsafe_declarations: 0,
                },
            );
            ffi_findings(&one)
        };
        assert_eq!(
            in_home.len(),
            1,
            "the sanctioned home is not exempt from arithmetic"
        );
        assert!(
            in_home[0]
                .message()
                .contains("legal Rust the SAFETY ledger cannot see"),
            "but it is told a different thing: {}",
            in_home[0].message()
        );
    }

    /// The two mechanisms must be shown COMPLEMENTARY: the qualified form is
    /// balanced arithmetic (silent tripwire) and is carried by the keyword rule,
    /// while the bare form is the reverse.
    #[test]
    fn the_qualified_form_is_the_other_rules_job_and_the_bare_form_is_this_one() {
        let qualified = "~U~ ~E~ \"C\" {\n    fn W() -> i32;\n}\n";
        let keyword = scan_foreign("crates/core/src/ffi.rs", &fix(qualified));
        assert_eq!(
            keyword[0].rule, UNSAFE_OUTSIDE,
            "caught by the pattern rule"
        );
        let mut ffi = BTreeMap::new();
        ffi.insert("notes-core".to_string(), count_ffi(&fix(qualified)));
        assert!(
            ffi_findings(&ffi).is_empty(),
            "1 - 1 is no evidence of a bare block, so the tripwire stays quiet"
        );
        let bare = "~E~ \"C\" {\n    fn W() -> i32;\n}\n";
        assert!(
            scan_foreign("crates/core/src/ffi.rs", &fix(bare)).is_empty(),
            "the pattern rule is still blind - the gap test above must stay true"
        );
        let mut ffi2 = BTreeMap::new();
        ffi2.insert("notes-core".to_string(), count_ffi(&fix(bare)));
        assert_eq!(
            ffi_findings(&ffi2).len(),
            1,
            "the arithmetic is what catches it"
        );
    }

    /// The limit stated in the header, pinned so it cannot be forgotten: an
    /// unrelated unsafe declaration in the same crate balances the subtraction.
    #[test]
    fn an_unrelated_unsafe_declaration_masks_a_bare_block_and_says_so() {
        let masked = "~U~ fn helper() {}\n~E~ \"system\" {\n    fn W();\n}\n";
        let c = count_ffi(&fix(masked));
        assert_eq!((c.extern_blocks, c.unsafe_declarations), (1, 1));
        assert_eq!(
            c.bare_by_count(),
            0,
            "documented blind spot of the arithmetic"
        );
    }

    #[test]
    fn a_pointer_a_crate_and_prose_are_not_extern_blocks() {
        for src in [
            "type WndProc = ~U~ ~E~ \"system\" fn(HWND, u32) -> LRESULT;\n",
            "~E~ crate nothing;\n",
            "~U~ ~E~ \"system\" fn wndproc(h: HWND) -> i32 { 0 }\n",
            "// ~E~ \"system\" {\n",
            "/// opens an ~E~ \"C\" { block in prose\n",
            "let s = \"~E~ \\\"system\\\" { is only text\";\n",
        ] {
            assert_eq!(count_ffi(&fix(src)).extern_blocks, 0, "not a block: {src}");
        }
        assert_eq!(
            count_ffi(&fix("~E~ {\n    fn f();\n}\n")).extern_blocks,
            1,
            "extern with no ABI is still a block"
        );
    }

    /// THE FIXTURE HAZARD, pinned: this module is a workspace member, so its own
    /// source is scanned by the rule it tests. A fixture that read as an extern
    /// block would arm the tripwire against crates/xtask and ship the repo red.
    #[test]
    fn the_census_counts_nothing_in_its_own_source() {
        let own = count_ffi(include_str!("check_unsafe.rs"));
        assert_eq!(
            own.extern_blocks, 0,
            "a fixture that reads as an extern block arms the tripwire against this very crate: spell the word ~E~ and let fix() put it back"
        );
    }

    #[test]
    fn the_census_and_the_declaration_count_agree_on_platform_shapes() {
        let src = "~U~ ~E~ \"system\" {\n    fn W() -> i32;\n}\n~U~ { C() };\n";
        let (ledger, _) = plat(src);
        let census = count_ffi(&fix(src));
        assert_eq!(
            ledger.declarations, census.unsafe_declarations,
            "one predicate, two folds: they must not drift"
        );
        assert_eq!(census.extern_blocks, 1);
        assert_eq!(
            ledger.blocks, 1,
            "the block is still counted as a block elsewhere"
        );
    }
}
