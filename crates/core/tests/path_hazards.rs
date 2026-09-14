//! PATH HAZARDS: path_policy decides from a NAME ALONE, so this file argues with
//! the REAL Win32 filesystem about every name on that table: ask the policy, then
//! ACTUALLY try the write, then look at where the bytes landed. The three answers
//! that matter:
//!
//!   Allowed + cannot write       -> UX bug (a file the app promises, cannot make)
//!   Refused + would have worked  -> over-refusal (a real note it will not name)
//!   Allowed + wrote + ELSEWHERE  -> the dangerous class (dot, stream, drive-relative)
//!
//! Every attempted row runs in its own sub-dir of %TEMP%\notes-hazard-probe, and
//! the absolute shapes from the brief are rebuilt in the SAME shape under that
//! sub-dir: the PREFIX is what is tested, never the root of C:. Volumes, NT device
//! namespaces and every UNC spelling are judged by name only and deliberately
//! never touched -- PhysicalDrive0 is a disk, and an unreachable share is the
//! 2.68 s freeze this policy exists to prevent.
//!
//! SPELLING: a Windows path appears only inside a raw string (r"..."), never as a
//! hand-doubled escape, so no row can be quietly wrong because someone miscounted
//! separators.
//!
//! Clean-up deletes through the extended prefix (a trailing-dot name cannot be
//! removed by its own plain spelling); what survives is reported by
//! probe_scratch_is_cleanable instead of leaked quietly.
//!
//! Threading: every probe wipes and owns ITS OWN root, named after the test
//! (`scratch("<test>")`), and THAT naming - not any runner setting - is what makes the
//! default parallel run safe: a wipe can only ever delete the wiper's own evidence, so
//! two probes in flight cannot destroy each other's table. This sentence used to claim the
//! file is run "with --test-threads=1", which no file in this repo does: no harness setting
//! in Cargo.toml and no step in .github/workflows/ci.yml passes that flag, here or in CI, so
//! it described an enforcement that never existed - the same family of defect as a test that
//! reports a skip by failing. The naming is load-bearing, not incidental, and it is what a
//! future contributor must preserve: share one root between two probes and a mid-run wipe
//! deletes another test's evidence, which is a probe that destroys what it came to measure.
//!
//! PLATFORM: eleven of the fifteen tests below are `#[cfg(windows)]`, and the number is not
//! caution - it is the subject. They argue with Win32 NAME semantics (trailing-dot stripping,
//! the `\\?\` prefix, one-letter-with-a-colon streams, case-folded identity), and a kernel
//! without those rules cannot fail them, so a red there would be a fact about the runner.
//! Four stay ungated on purpose: the drive-relative pair and the scratch-location table assert
//! NAME-ONLY policy, which has no platform in it. Nothing here is `#[ignore]`d to make a
//! platform quiet; the ignored row is a child process driven by its own probe.
//!
//! Lints: notes-core denies unwrap_used/expect_used for ALL targets, so every
//! case here is ?-typed and asserts instead.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde as _;
use serde_json as _;
use tempfile as _;
use thiserror as _;
use toml as _;

use notes_core::{
    Detected, LineEnding, PathVerdict, RecentEntry, SessionError, StateDir, TextEncoding, clear,
    display_labels, ensure_scratch_dir, identity_key, is_notes_path, path_policy, push,
    save_document, scratch_note_path,
};

/// Reached only by the Windows-gated probe that asks whether core creates a missing state dir. On
/// Linux nothing calls it, and an unused import there is the very ubuntu red this fix exists to
/// remove - so a gate on a test and a gate on what only that test imports travel together.
#[cfg(windows)]
use notes_core::save_session_bytes;

/// Scratch root under %TEMP%.
#[cfg(windows)]
const ROOT_NAME: &str = "notes-hazard-probe";
/// The extended-length prefix: two separators, a question mark, one. A raw string
/// may end in a backslash, which is the only honest way to spell it here.
const EXT: &str = r"\\?\";
/// One path separator.
#[cfg(windows)]
const BS: &str = r"\";

fn utf8_det() -> Detected {
    Detected {
        encoding: TextEncoding::Utf8,
        line_ending: LineEnding::Lf,
        trailing_newline: false,
        bom_present: false,
    }
}

#[cfg(windows)]
fn ch(code: u32) -> char {
    char::from_u32(code).unwrap_or('?')
}

#[cfg(windows)]
fn rtl() -> char {
    ch(0x202E)
}

#[cfg(windows)]
fn zwsp() -> char {
    ch(0x200B)
}

fn verdict_str(v: PathVerdict) -> &'static str {
    match v {
        PathVerdict::Allowed => "Allowed",
        PathVerdict::UnboundedNetwork => "UnboundedNetwork",
        PathVerdict::ReservedDevice => "ReservedDevice",
        PathVerdict::StreamName => "StreamName",
        PathVerdict::DriveRelative => "DriveRelative",
        PathVerdict::StrippedName => "StrippedName",
    }
}

fn verdict_of(p: &Path) -> String {
    verdict_str(path_policy(p)).to_owned()
}

/// EXT + an absolute path: the only spelling under which Win32 stops mangling,
/// hence the only honest way to ask "does a file with THIS exact name exist?",
/// and the only way to delete one.
fn verb(p: &Path) -> PathBuf {
    let s = p.to_string_lossy().into_owned();
    if s.starts_with(EXT) {
        PathBuf::from(s)
    } else {
        PathBuf::from(format!("{EXT}{s}"))
    }
}

#[cfg(windows)]
fn exact_exists(p: &Path) -> bool {
    fs::symlink_metadata(verb(p)).is_ok()
}

#[cfg(windows)]
fn remove_hard(p: &Path) {
    let v = verb(p);
    if fs::remove_dir_all(&v).is_err() {
        let _ = fs::remove_file(&v);
    }
}

#[cfg(windows)]
fn scratch(test: &str) -> PathBuf {
    let mut root = std::env::temp_dir();
    root.push(format!("{ROOT_NAME}-{test}"));
    root
}

/// WHY THESE PROBES ARE GATED AND NOT IGNORED. This function used to begin
/// "if !cfg!(windows) { return Err(...) }", which looked like a skip and was not one: a
/// Result-returning #[test] that returns Err is a FAILED test, so on any non-Windows runner all
/// ten callers printed "this probe measures Win32 name behaviour" as a red against core, saying
/// nothing about core and everything about the machine. #[ignore] would have made the red go away
/// by deleting the evidence, which is worse. So the gate is a real cfg(windows) on each caller -
/// compiled only where its subject exists - and the lie here is gone. The portable rows in this
/// file (the drive-relative NAME policy, the scratch-location table, the roaming proof) stay
/// ungated, because they judge name-only code that has no platform in it.
#[cfg(windows)]
fn init_scratch(test: &str) -> Result<PathBuf, Box<dyn Error>> {
    let root = scratch(test);
    remove_hard(&root);
    fs::create_dir_all(&root)?;
    Ok(root)
}

#[cfg(windows)]
fn row_dir(root: &Path, i: usize) -> Result<PathBuf, Box<dyn Error>> {
    let dir = root.join(format!("t{i:02}"));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn io_line(e: &std::io::Error) -> String {
    format!("{:?}(os={:?})", e.kind(), e.raw_os_error())
}

#[cfg(windows)]
fn name_of(p: &Path) -> String {
    p.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// Make the invisible characters visible in the printed table.
#[cfg(windows)]
fn shown(name: &str) -> String {
    name.replace(&rtl().to_string(), "<U+202E>")
        .replace(&zwsp().to_string(), "<U+200B>")
        .replace(&ch(9).to_string(), "<TAB>")
}

#[derive(Debug)]
#[cfg(windows)]
struct Row {
    name: String,
    verdict: String,
    wrote: String,
    named: String,
    landed: Vec<String>,
    on_disk: Vec<String>,
    klass: String,
}

/// One row: what policy says, what the write does, what is really on disk.
#[cfg(windows)]
fn run_row(root: &Path, i: usize, display: &str, target: &Path) -> Result<Row, Box<dyn Error>> {
    let fallback = row_dir(root, i)?;
    let token = format!("HAZARD-{i:02}-PAYLOAD");
    let verdict = verdict_of(target);
    let wrote = match fs::write(target, token.as_bytes()) {
        Ok(()) => "Ok".to_owned(),
        Err(e) => io_line(&e),
    };
    // Scan where the name ACTUALLY points (an EXT/absolute row does not live in
    // this row's own dir).
    let dir = target
        .parent()
        .map_or_else(|| fallback.clone(), PathBuf::from);
    let mut landed = Vec::new();
    let mut on_disk = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for e in entries {
            let Ok(p) = e.map(|de| de.path()) else {
                continue;
            };
            let n = name_of(&p);
            let body = fs::read(verb(&p)).unwrap_or_default();
            on_disk.push(n.clone());
            if String::from_utf8_lossy(&body).contains(&token) {
                landed.push(n);
            }
        }
    }
    on_disk.sort();
    landed.sort();
    let named = name_of(target);
    let exists_exact = exact_exists(target);
    // "Did the name get its own file, and can the plain path open it again?" is
    // the only question that separates a correct refusal from an over-refusal:
    // Win32 returns Ok for a stripped, device or stream name too, and the bytes
    // land elsewhere. Open by the PLAIN path and compare sizes -- never read a
    // device (reading the console blocks), only ask its length.
    let plain_len = fs::File::open(target)
        .and_then(|f| f.metadata())
        .map(|m| m.len());
    let verb_len = fs::metadata(verb(target)).map(|m| m.len()).ok();
    let owns_it = landed.len() == 1
        && landed[0] == named
        && exists_exact
        && plain_len.as_ref().copied().ok() == verb_len;
    let klass = match (verdict.as_str(), wrote.as_str(), owns_it) {
        ("Allowed", "Ok", true) => "match".to_owned(),
        ("Allowed", "Ok", false) if landed.is_empty() && !exists_exact => {
            "SILENT LOSS: Allowed + Ok + NO file holds the bytes".to_owned()
        }
        ("Allowed", "Ok", false) if exists_exact => format!(
            "SILENT LOSS: Allowed + Ok, the plain path opens something else (plain len {plain_len:?} \
             vs verbatim {verb_len:?})"
        ),
        ("Allowed", "Ok", false) => {
            format!("DIVERGENCE: Allowed + Ok, bytes in {landed:?} not {named:?}")
        }
        ("Allowed", _, _) => "Allowed-but-cannot-write (UX)".to_owned(),
        (v, "Ok", true) => {
            // std auto-prefixes absolute paths, so a bare reserved name INSIDE a directory
            // really is made and re-opened through stdlib -- while `cmd /c echo X > dir\CON`
            // creates no file at all (measured separately). The refusal over-reaches only
            // against std; the note it prevents would be unopenable by every other tool.
            format!("half-real {v}: made and reopened through std, refused by cmd")
        }
        (v, "Ok", false) if !landed.is_empty() && landed[0] != named => format!(
            "correct refusal ({v}): the write landed in {landed:?}, not in the name it spells"
        ),
        (v, "Ok", false) if exists_exact => format!(
            "correct refusal ({v}): std made a literal file, but the plain path opens something \
             else (plain len {plain_len:?} vs verbatim {verb_len:?})"
        ),
        (v, "Ok", false) => {
            format!("correct refusal ({v}): the plain write landed in {landed:?}, not the name")
        }
        (v, _, _) => format!("correct refusal ({v}): Win32 refused the name too"),
    };
    Ok(Row {
        name: display.to_owned(),
        verdict,
        wrote,
        named,
        landed,
        on_disk,
        klass,
    })
}

#[cfg(windows)]
fn print_rows(title: &str, rows: &[Row]) {
    println!();
    println!("=== {title} ===");
    println!(
        "{:<34} {:<17} {:<30} CLASSIFICATION",
        "NAME", "POLICY", "REAL (fs::write)"
    );
    for r in rows {
        println!(
            "{:<34} {:<17} {:<30} {}",
            shown(&r.name),
            r.verdict,
            r.wrote,
            r.klass
        );
        if !r.on_disk.is_empty() && r.on_disk != r.landed {
            let names: Vec<String> = r.on_disk.iter().map(|n| shown(n)).collect();
            println!("{:<34} {:<17} dir holds: {names:?}", "", "");
        }
    }
}

/// THE TABLE: every name from the brief, judged by policy and then attempted for
/// real, with the directory itself as the witness of where the bytes landed.
#[test]
#[cfg(windows)]
fn policy_table_vs_real_win32() -> Result<(), Box<dyn Error>> {
    let root = init_scratch("policy_table_vs_real_win32")?;
    let mut rows: Vec<Row> = Vec::new();
    let mut i = 0usize;
    let rel = |name: &str, rows: &mut Vec<Row>, i: &mut usize| -> Result<(), Box<dyn Error>> {
        let dir = row_dir(&root, *i)?;
        rows.push(run_row(&root, *i, name, &dir.join(name))?);
        *i += 1;
        Ok(())
    };

    // Trailing dot / trailing space, plus the degenerate names. (The "." and
    // ".." rows print a dir listing of the row ABOVE them -- their parent() is
    // not the row dir. A probe artifact, not a product finding.)
    for n in [
        "notes.", "notes ", "a.notes.", "a.notes ", "...", "   ", ".", "..",
    ] {
        rel(n, &mut rows, &mut i)?;
    }
    // The classic DOS devices: bare, lower-case, with an extension, and the
    // two-digit forms the policy says were never reserved.
    for n in [
        "CON",
        "con",
        "CON.notes",
        "com1.txt",
        "COM0",
        "COM1",
        "LPT1",
        "LPT9",
        "LPT10",
        "NUL",
        "nul.txt",
        "aux.log",
        "PRN.notes",
        "AUX",
        "PRN",
    ] {
        rel(n, &mut rows, &mut i)?;
    }
    // Streams and colon shapes.
    for n in ["a:b.notes", "a.notes:x:y", "notes.notes::$DATA"] {
        rel(n, &mut rows, &mut i)?;
    }
    // A device name as a MIDDLE component: policy looks only at the last one.
    let dir = row_dir(&root, i)?;
    let mid_device = format!("CON{BS}x.notes");
    rows.push(run_row(
        &root,
        i,
        "CON as a PARENT dir",
        &dir.join(&mid_device),
    )?);
    i += 1;
    // Invisible characters inside a name.
    let (r, z) = (rtl(), zwsp());
    let invisibles = [
        format!("notes{r}.notes"),
        format!("txt.gnul{r}taolan.notes"),
        format!("notes{z}.notes"),
        format!("notes{z}2.notes"),
    ];
    for n in &invisibles {
        rel(n, &mut rows, &mut i)?;
    }
    // A 300-char name, plain and through the extended prefix.
    let long = format!("{}.notes", "l".repeat(295));
    let dir = row_dir(&root, i)?;
    rows.push(run_row(
        &root,
        i,
        "a 300-char name (plain)",
        &dir.join(&long),
    )?);
    i += 1;
    let dir = row_dir(&root, i)?;
    let via = PathBuf::from(format!("{EXT}{}{BS}{long}", dir.display()));
    rows.push(run_row(&root, i, "a 300-char name (via EXT)", &via)?);
    i += 1;

    // The absolute shapes from the brief, rebuilt under a real row dir so the
    // PREFIX is what is tested and never the root of C:.
    let d = row_dir(&root, i)?.to_string_lossy().into_owned();
    let shapes: Vec<(String, PathBuf)> = vec![
        (
            format!("{EXT}C:{BS}x.notes"),
            PathBuf::from(format!("{EXT}{d}{BS}x.notes")),
        ),
        (
            format!("{EXT}c:{BS}x.notes."),
            PathBuf::from(format!("{EXT}{d}{BS}x.notes.")),
        ),
        (
            format!("{EXT}C:{BS}CON"),
            PathBuf::from(format!("{EXT}{d}{BS}CON")),
        ),
        (
            format!("{EXT}C:{BS}nul.txt"),
            PathBuf::from(format!("{EXT}{d}{BS}nul.txt")),
        ),
        (
            format!("{EXT}C:{BS}sub{BS}..{BS}x.notes"),
            PathBuf::from(format!("{EXT}{d}{BS}sub{BS}..{BS}x.notes")),
        ),
    ];
    for (display, target) in &shapes {
        let idx = rows.len();
        rows.push(run_row(&root, idx, display, target)?);
    }
    print_rows(
        "POLICY TABLE vs REAL WIN32 (attempted rows, all under %TEMP%)",
        &rows,
    );

    // Name-only judgements: volumes, device namespaces, every network spelling.
    // NEVER touched -- one write to the wrong one destroys a partition, and an
    // unreachable share blocks for as long as the redirector pleases.
    println!();
    println!("=== NAME-ONLY JUDGEMENTS (never touched: a disk, or an UNC stall) ===");
    for n in [
        r"\\.\PhysicalDrive0",
        r"\\?\PhysicalDrive0",
        r"\\?\physicaldrive0",
        r"\\?\GLOBALROOT\Device\HarddiskVolume3\x.notes",
        r"\\?\C:\x.notes",
        r"\\?\c:\x.notes.",
        r"\??\C:\x.notes",
        r"\\?\UNC\host\share\x.notes",
        r"\\?\unc\host\share\x.notes",
        r"\\?\Unc\host\share\x.notes",
        r"\\?\\host\share\x.notes",
        r"\\host\share\x.notes",
        r"\\127.0.0.1\c$\x.notes",
        r"\\wsl$\Ubuntu\home\u\x.notes",
        r"\\WSL$\Ubuntu\notes.notes",
        r"\\\host\share\x.notes",
        r"//host/share/x.notes",
        r"C:",
        r"C:notes.notes",
        r"rel-hazard.notes",
    ] {
        println!(
            "{:<56} {:<17} (name only)",
            shown(n),
            verdict_of(Path::new(n))
        );
    }

    // The invariant that holds whatever else is found: an ALLOWED row's bytes may
    // only live in the file its own name spells. (For a REFUSED row the table
    // still performs the write -- that is how the refusal is shown to be earned.)
    let mut elsewhere = Vec::new();
    for (j, r) in rows.iter().enumerate() {
        if r.verdict != "Allowed" {
            continue;
        }
        for l in &r.landed {
            if *l != r.named {
                elsewhere.push(format!(
                    "row {j} ({}): bytes in {:?}",
                    shown(&r.name),
                    shown(l)
                ));
            }
        }
    }
    assert!(
        elsewhere.is_empty(),
        "Allowed rows wrote into another file: {elsewhere:?}"
    );
    Ok(())
}

/// A ONE-LETTER file name followed by a colon is not a drive: "a:secret.notes"
/// is an ALTERNATE DATA STREAM on the file "a". drive_separator() accepts any
/// ASCII letter before the colon, so the stream rule never fires for these.
#[test]
#[cfg(windows)]
fn one_letter_name_with_a_colon_is_a_stream_not_a_drive() -> Result<(), Box<dyn Error>> {
    let root = init_scratch("one_letter_name_with_a_colon_is_a_stream_not_a_drive")?;
    let dir = row_dir(&root, 95)?;
    let base = dir.join("a");
    fs::write(verb(&base), b"USER-DOCUMENT-A")?;
    // Built as a STRING, not with join(): join() treats "a:secret.notes" as a
    // drive-relative path and replaces the whole target, which would probe the
    // drive rules instead of the stream rules.
    let stream = PathBuf::from(format!("{EXT}{}{BS}a:secret.notes", dir.to_string_lossy()));
    let plain_stream = PathBuf::from(format!("{}{BS}a:secret.notes", dir.to_string_lossy()));
    let wrote_verb = match fs::write(&stream, b"HAZARD-STREAM-PAYLOAD") {
        Ok(()) => "Ok (EXT spelling)".to_owned(),
        Err(e) => io_line(&e),
    };
    let wrote_plain = match fs::write(&plain_stream, b"HAZARD-STREAM-PAYLOAD2") {
        Ok(()) => "Ok (plain spelling)".to_owned(),
        Err(e) => io_line(&e),
    };
    let listed: Vec<String> = fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .map(|de| name_of(&de.path()))
        .collect();
    let base_now = fs::read(verb(&base)).unwrap_or_default();
    let stream_now = fs::read(verb(&stream)).map(|b| String::from_utf8_lossy(&b).into_owned());
    println!();
    println!(
        "policy(EXT a:secret.notes) = {} | policy(plain) = {}",
        verdict_of(&stream),
        verdict_of(&plain_stream)
    );
    println!("  write EXT -> {wrote_verb} | write plain -> {wrote_plain}");
    println!(
        "  the dir lists {listed:?} | the host file still holds {:?}",
        String::from_utf8_lossy(&base_now)
    );
    println!(
        "  the stream itself holds {:?}",
        stream_now.unwrap_or_default()
    );
    println!(
        "  is_notes_path(a:secret.notes) = {}",
        is_notes_path(&stream)
    );
    // The BARE form is the hole: "a:b.notes" is not a stream of anything, it is
    // drive-relative on drive A:, and drive_separator() accepts any ASCII letter.
    let bare = PathBuf::from("a:secret.notes");
    let bare_wrote = match fs::write(&bare, b"HAZARD-BARE") {
        Ok(()) => "Ok (wrote somewhere on the per-drive CWD of A:)".to_owned(),
        Err(e) => io_line(&e),
    };
    println!(
        "policy(bare a:secret.notes) = {} | fs::write -> {bare_wrote}",
        verdict_of(&bare)
    );
    println!(
        "  the CWD this ran in: {}",
        std::env::current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_default()
    );
    assert!(
        !matches!(path_policy(&stream), PathVerdict::Allowed)
            && !matches!(path_policy(&bare), PathVerdict::Allowed),
        "BLOCKER: the EXT/plain stream verdicts are earned (they said {} / {}), but the BARE \
         one-letter form \"a:secret.notes\" is {} -- drive_separator() accepts any ASCII letter \
         before a colon, so a one-letter FILE name plus a colon is read as a drive and \
         resolves against that drive\u{2019}s current directory instead of the folder the user \
         named. The write answered {bare_wrote}.",
        verdict_of(&stream),
        verdict_of(&plain_stream),
        verdict_of(&bare),
    );
    Ok(())
}

/// THE ONE THAT MATTERS MOST: a drive-relative name. Win32 resolves "C:name"
/// against the PER-DRIVE current directory -- process state that changes over a
/// session -- so one NAME can mean two different FILES at two different times.
/// Each write runs in a child of this test binary with its own CWD, because
/// set_current_dir in-process would race the rest of the suite.
#[test]
fn drive_relative_name_lands_where_the_cwd_says() -> Result<(), Box<dyn Error>> {
    // A PRIVATE root: the child process writes from its own CWD outside the
    // scratch lock, so a shared root would race the sibling probes' wipes.
    let root = tempfile::tempdir()?.path().to_path_buf();
    let a = root.join("cwdA");
    let b = root.join("cwdB");
    fs::create_dir_all(&a)?;
    fs::create_dir_all(&b)?;
    let drive = root
        .to_string_lossy()
        .chars()
        .next()
        .filter(|c| c.is_ascii_alphabetic())
        .unwrap_or('C')
        .to_ascii_uppercase();
    let name = format!("{drive}:rel-hazard.notes");
    let target = PathBuf::from(&name);
    println!();
    println!(
        "drive-relative name {name:?} -> policy says {}",
        verdict_of(&target)
    );

    let exe = std::env::current_exe()?;
    let mut landed: Vec<String> = Vec::new();
    for (cwd, token) in [(&a, "REL-A"), (&b, "REL-B")] {
        let out = Command::new(&exe)
            .args([
                "--exact",
                "drive_relative_child",
                "--nocapture",
                "--ignored",
            ])
            .current_dir(cwd)
            .env("HAZARD_NAME", &name)
            .env("HAZARD_TOKEN", token)
            .output()?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        for line in text.lines().filter(|l| l.contains("policy=")) {
            println!("  child cwd={} : {}", cwd.display(), line.trim());
        }
        for d in [&a, &b] {
            let f = d.join("rel-hazard.notes");
            if let Ok(bytes) = fs::read(verb(&f)) {
                if String::from_utf8_lossy(&bytes).contains(token) {
                    landed.push(format!("{token} landed in {}", d.display()));
                }
            }
        }
    }
    let key_a = identity_key(&a.join("rel-hazard.notes"));
    let key_b = identity_key(&b.join("rel-hazard.notes"));
    let both = vec![
        RecentEntry {
            path: target.clone(),
            display: name.clone(),
            exists: true,
        },
        RecentEntry {
            path: target.clone(),
            display: name.clone(),
            exists: true,
        },
    ];
    println!("  {landed:?}");
    println!(
        "  the two real files get different identity keys? {}",
        key_a != key_b
    );
    println!(
        "  display_labels of two entries spelled {name:?}: {:?}",
        display_labels(&both)
    );
    println!("  is_notes_path({name:?}) = {}", is_notes_path(&target));
    assert!(
        !matches!(path_policy(&target), PathVerdict::Allowed),
        "BLOCKER: a DRIVE-RELATIVE name is Allowed by the name-only policy. The same \
         name written from two working directories produced {} separate files ({landed:?}); \
         identity_key gives those files two different keys and display_labels renders both as \
         one word -- the app holds one document title over two documents, the same class the \
         trailing-dot fix closed.",
        landed.len().max(1)
    );
    Ok(())
}

/// Child half of the drive-relative probe: writes the NAME exactly as the app
/// would (verbatim, relative) and reports what Win32 resolved it to.
#[test]
#[ignore = "driven by drive_relative_name_lands_where_the_cwd_says"]
fn drive_relative_child() -> Result<(), Box<dyn Error>> {
    let name = std::env::var("HAZARD_NAME").unwrap_or_default();
    let token = std::env::var("HAZARD_TOKEN").unwrap_or_default();
    if name.is_empty() {
        return Err("HAZARD_NAME is not set".into());
    }
    let target = PathBuf::from(&name);
    let wrote = match fs::write(&target, token.as_bytes()) {
        Ok(()) => "Ok".to_owned(),
        Err(e) => io_line(&e),
    };
    let saved = match save_document(&target, &token, utf8_det()) {
        Ok(o) => format!("Ok({})", o.path.display()),
        Err(e) => format!("Err({e})"),
    };
    let real = fs::canonicalize(&target)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|e| format!("canonicalize: {}", io_line(&e)));
    println!(
        "policy={} fs::write={wrote} save_document={saved} resolved={real}",
        verdict_of(&target),
    );
    Ok(())
}

/// Win32 strips a trailing dot from EVERY component, not just the last, and a
/// real dot-directory exists only through the extended prefix. If a plain write
/// to "sub.\x.notes" lands in "sub\x.notes", policy-Allowed named one file and
/// wrote another -- and save_document reports Ok for both.
#[test]
#[cfg(windows)]
fn stripped_middle_component_shadows_a_real_dot_directory() -> Result<(), Box<dyn Error>> {
    let root = init_scratch("stripped_middle_component_shadows_a_real_dot_directory")?;
    let dir = row_dir(&root, 92)?;
    let plain = dir.join("sub");
    let dotted = dir.join("sub.");
    fs::create_dir_all(&plain)?;
    fs::create_dir_all(verb(&dotted))?;
    fs::write(verb(&plain.join("x.notes")), b"SENDER-PLAIN")?;
    fs::write(verb(&dotted.join("x.notes")), b"SENDER-DOT")?;
    let named = dotted.join("x.notes");
    let verdict = verdict_of(&named);
    let wrote = match fs::write(&named, b"HAZARD-MIDDLE-PAYLOAD") {
        Ok(()) => "Ok".to_owned(),
        Err(e) => io_line(&e),
    };
    let saved = match save_document(&named, "MIDDLE-SAVE", utf8_det()) {
        Ok(o) => format!("Ok({})", o.path.display()),
        Err(e) => format!("Err({e})"),
    };
    let in_plain = fs::read(verb(&plain.join("x.notes"))).unwrap_or_default();
    let in_dot = fs::read(verb(&dotted.join("x.notes"))).unwrap_or_default();
    let same_identity = identity_key(&plain.join("x.notes")) == identity_key(&named);
    println!();
    println!("policy(sub.{BS}x.notes) = {verdict} | fs::write = {wrote} | save_document = {saved}");
    println!(
        "  sub{BS}x.notes   now holds {:?}",
        String::from_utf8_lossy(&in_plain)
    );
    println!(
        "  sub.{BS}x.notes  now holds {:?}",
        String::from_utf8_lossy(&in_dot)
    );
    println!("  the two spellings share one identity key? {same_identity}");
    // BLOCKER-2, fixed: the policy must NAME the redirect and the save
    // engine must REFUSE it. The raw fs::write above stays as the Win32
    // witness -- it still lands in sub\x.notes, which is exactly why the
    // app-level refusal is required (Win32 will always be this shape).
    assert_eq!(
        verdict, "StrippedName",
        "the middle-dot redirect must be named by the policy, got {verdict}"
    );
    assert!(
        saved.starts_with("Err("),
        "BLOCKER-2 regression: save_document answered {saved} for a middle-dot path -- \
         the app reported success for a file it did not write"
    );
    Ok(())
}

/// A trailing dot past \\?\ is a LITERAL character, so the file really is named
/// "x.notes." -- and no plain spelling of that name can reach it again.
#[test]
#[cfg(windows)]
fn extended_prefix_trailing_dot_is_a_file_no_plain_name_reaches() -> Result<(), Box<dyn Error>> {
    let root = init_scratch("extended_prefix_trailing_dot_is_a_file_no_plain_name_reaches")?;
    let dir = row_dir(&root, 93)?;
    let d = dir.to_string_lossy().into_owned();
    let verbatim_target = PathBuf::from(format!("{EXT}{d}{BS}x.notes."));
    let verdict = verdict_of(&verbatim_target);
    let saved = match save_document(&verbatim_target, "extended-dot", utf8_det()) {
        Ok(o) => format!("Ok({})", o.path.display()),
        Err(e) => format!("Err({e})"),
    };
    let exists = exact_exists(&verbatim_target);
    let plain_spelling = dir.join("x.notes.");
    let plain_open = fs::read(&plain_spelling)
        .map(|b| b.len())
        .map_err(|e| io_line(&e));
    println!();
    println!(
        "policy(EXT x.notes.) = {verdict} | save_document = {saved} | the literal name exists? {exists}"
    );
    println!(
        "policy(plain x.notes.) = {} | fs::read(plain spelling) = {plain_open:?}",
        verdict_of(&plain_spelling)
    );
    println!(
        "one identity key for both spellings? {}",
        identity_key(&verbatim_target) == identity_key(&plain_spelling)
    );
    println!(
        "is_notes_path(EXT) = {} | is_notes_path(plain) = {}",
        is_notes_path(&verbatim_target),
        is_notes_path(&plain_spelling)
    );
    assert!(
        !exists || plain_open.is_ok(),
        "MAJOR: the app saved a note whose displayed name is {:?}, and no plain Win32 spelling \
         of that name can open it again ({plain_open:?}) -- a note reachable only through the \
         extended prefix is invisible to the file dialog, to Explorer, and to every non-EXT path, \
         while identity_key says it IS the same note as the plain spelling that cannot be opened.",
        name_of(&plain_spelling)
    );
    Ok(())
}

/// A U+202E (RTL override) or U+200B (zero-width space) is a LEGAL NTFS name
/// character, so policy-Allowed is right and the file is real. The surface that
/// has to answer is the MENU: display_labels is core's own rule (8fc270d), and a
/// label that renders backwards, or two labels that render identically, is the
/// same "the name you see is not the file you get" class in user terms.
#[test]
#[cfg(windows)]
fn invisible_names_reach_the_menu_verbatim() -> Result<(), Box<dyn Error>> {
    let root = init_scratch("invisible_names_reach_the_menu_verbatim")?;
    let dir = row_dir(&root, 96)?;
    let r = rtl();
    let z = zwsp();
    // A name chosen so that, once rendered, it READS as the ordinary "notes.txt".
    let spoof = format!("tnuoc{r}txet.notes");
    let plain_twin = format!("notes{z}.notes");
    let other_twin = "notes.notes".to_owned();
    fs::write(dir.join(&spoof), b"A")?;
    fs::write(dir.join(&plain_twin), b"B")?;
    fs::write(dir.join(&other_twin), b"C")?;
    let entries: Vec<RecentEntry> = [spoof.as_str(), plain_twin.as_str(), other_twin.as_str()]
        .iter()
        .map(|n| RecentEntry {
            path: dir.join(n),
            display: (*n).to_owned(),
            exists: true,
        })
        .collect();
    let labels = display_labels(&entries);
    for (raw, label) in entries.iter().zip(&labels) {
        println!(
            "  raw {:?} -> label {:?}",
            shown(&name_of(&raw.path)),
            shown(label)
        );
    }
    let visible: Vec<String> = labels.iter().map(|l| shown(l)).collect();
    let mut stripped: Vec<String> = labels
        .iter()
        .map(|l| {
            l.chars()
                .filter(|c| *c != zwsp() && *c != rtl())
                .collect::<String>()
        })
        .collect();
    stripped.sort();
    let twins = stripped.windows(2).filter(|w| w[0] == w[1]).count();
    println!("  labels as printed: {visible:?}");
    println!("  pairs that render identically once the invisible char is dropped: {twins}");
    println!(
        "  the menu carries the bidi control unescaped? {}",
        labels.iter().any(|l| l.contains(&rtl().to_string()))
    );
    // MAJOR-6, ASSERTED (D45: a print is not a test): no raw invisible or
    // bidi control may reach a menu label, every control must be visibly
    // substituted, and two labels that used to render identically once the
    // invisible char was dropped must differ without any stripping.
    assert!(
        labels
            .iter()
            .all(|l| !l.contains(&rtl().to_string()) && !l.contains(&zwsp().to_string())),
        "a raw invisible or bidi control reached a menu label: {visible:?}"
    );
    assert!(
        labels.iter().any(|l| l.contains("[RTL]")) && labels.iter().any(|l| l.contains("[ZWSP]")),
        "the controls must be visibly substituted, got {visible:?}"
    );
    assert_eq!(
        twins, 0,
        "two labels still render identically once the control is dropped: {visible:?}"
    );
    Ok(())
}
/// CASE AND COLLISION: NTFS is case-insensitive and case-preserving, so a second
/// spelling of a name must not become a second document.
#[test]
#[cfg(windows)]
fn case_only_difference_is_one_file_and_one_identity() -> Result<(), Box<dyn Error>> {
    let root = init_scratch("case_only_difference_is_one_file_and_one_identity")?;
    let dir = row_dir(&root, 90)?;
    let lower = dir.join("notes.notes");
    let upper = dir.join("NOTES.NOTES");
    fs::write(verb(&lower), b"first")?;
    let second = fs::write(verb(&upper), b"second");
    let entries: Vec<String> = fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .map(|de| name_of(&de.path()))
        .collect();
    let list = push(
        push(Vec::new(), lower.clone(), "notes.notes"),
        upper.clone(),
        "NOTES.NOTES",
    );
    println!();
    println!(
        "second write {} | dir holds {entries:?} | identity equal = {} | recent entries = {} | labels {:?}",
        match &second {
            Ok(()) => "Ok".to_owned(),
            Err(e) => io_line(e),
        },
        identity_key(&lower) == identity_key(&upper),
        list.len(),
        display_labels(&list)
    );
    println!(
        "the one file on disk holds {:?}",
        String::from_utf8_lossy(&fs::read(verb(&lower)).unwrap_or_default())
    );
    println!(
        "the stored (case-preserved) spelling is now {:?}",
        list.first().map(|e| name_of(&e.path))
    );
    assert_eq!(
        entries.len(),
        1,
        "NTFS made two files for case-only spellings"
    );
    assert_eq!(
        identity_key(&lower),
        identity_key(&upper),
        "identity_key split one file"
    );
    assert_eq!(
        list.len(),
        1,
        "the recent list made two entries for one file"
    );
    Ok(())
}

/// A name the policy REFUSES still denotes an existing file ("notes." is stripped
/// to "notes"), yet identity_key is computed WITHOUT canonicalising a refused
/// name -- so one file on disk becomes two recent entries.
#[test]
#[cfg(windows)]
fn refused_spelling_of_an_existing_file_splits_the_recent_list() -> Result<(), Box<dyn Error>> {
    let root = init_scratch("refused_spelling_of_an_existing_file_splits_the_recent_list")?;
    let dir = row_dir(&root, 91)?;
    let real = dir.join("notes");
    fs::write(verb(&real), b"exists")?;
    let refused = dir.join("notes.");
    let same = identity_key(&real) == identity_key(&refused);
    let list = push(
        push(Vec::new(), real.clone(), "notes"),
        refused.clone(),
        "notes.",
    );
    println!();
    println!(
        "policy(notes.) = {} | one identity key for both spellings? {same}",
        verdict_of(&refused)
    );
    println!(
        "recent entries for the ONE file on disk: {} -> {:?}",
        list.len(),
        display_labels(&list)
    );
    assert!(
        same,
        "MAJOR: {:?} and {:?} are ONE file on disk (Win32 strips the dot) but identity_key \
         gives them two keys, so the recent list holds {} entries for one document -- and the \
         refused entry can never be opened, because the save path refuses the name it shows.",
        name_of(&real),
        name_of(&refused),
        list.len()
    );
    Ok(())
}

/// LONG PATH: which owner breaks a path over 260 chars -- the manifest, the API
/// used, or the policy. This test binary carries NO app.manifest, so anything
/// that works here works without the manifest, and a failure here cannot be
/// blamed on the manifest.
#[test]
#[cfg(windows)]
fn long_path_round_trip_plain_vs_extended() -> Result<(), Box<dyn Error>> {
    let root = init_scratch("long_path_round_trip_plain_vs_extended")?;
    let dir = row_dir(&root, 94)?;
    let mut deep = dir.clone();
    for _ in 0..6 {
        deep.push("deep-folder-name-that-is-long-on-purpose-0123456789");
    }
    let made = match fs::create_dir_all(&deep) {
        Ok(()) => "Ok".to_owned(),
        Err(e) => io_line(&e),
    };
    let plain = deep.join("note.notes");
    let via = PathBuf::from(format!("{EXT}{}", plain.to_string_lossy()));
    let save_plain = match save_document(&plain, "plain-deep", utf8_det()) {
        Ok(_) => "Ok".to_owned(),
        Err(e) => format!("Err({e})"),
    };
    let read_plain = match fs::read(&plain) {
        Ok(b) => format!("Ok({} bytes)", b.len()),
        Err(e) => io_line(&e),
    };
    let save_via = match save_document(&via, "prefix-deep", utf8_det()) {
        Ok(_) => "Ok".to_owned(),
        Err(e) => format!("Err({e})"),
    };
    // MAJOR-4 regression guard: the reversal must not eat the LEGITIMATE long
    // path. A >260-char \\?\ path with no stripped component stays Allowed
    // and savable; a >260-char path with a stripped COMPONENT is refused —
    // the rule keys on the component, never on the length.
    assert!(
        plain.to_string_lossy().chars().count() > 260,
        "the probe must actually exceed MAX_PATH: {}",
        plain.to_string_lossy()
    );
    assert_eq!(
        verdict_of(&via),
        "Allowed",
        "a legitimate long path past the prefix must stay Allowed"
    );
    assert_eq!(
        verdict_of(&deep.join("dot.")),
        "StrippedName",
        "a stripped component is refused at any length"
    );
    let read_via = match fs::read(&via) {
        Ok(b) => format!("Ok({} bytes)", b.len()),
        Err(e) => io_line(&e),
    };
    println!();
    println!(
        "deep path is {} chars | create_dir_all -> {made}",
        plain.to_string_lossy().chars().count()
    );
    println!(
        "policy(plain) = {} | save = {save_plain} | read = {read_plain}",
        verdict_of(&plain)
    );
    println!(
        "policy(EXT)   = {} | save = {save_via} | read = {read_via}",
        verdict_of(&via)
    );
    assert!(
        save_plain == "Ok" && read_plain == "Ok(10 bytes)",
        "the plain deep path did not round-trip: save={save_plain} read={read_plain}"
    );
    assert!(
        save_via == "Ok" && read_via == "Ok(11 bytes)",
        "the EXT deep path did not round-trip: save={save_via} read={read_via}"
    );
    Ok(())
}

/// STATE DIR (D54): core does NOT create the directory it saves into -- that is
/// api's create_dir_all. Measured so the manager knows which side the fix is on.
#[test]
#[cfg(windows)]
fn core_does_not_create_the_state_dir() -> Result<(), Box<dyn Error>> {
    let base = init_scratch("core_does_not_create_the_state_dir")?.join("state-missing");
    remove_hard(&base);
    let missing = base.join("notes-gpui");
    let session = match save_session_bytes(&missing, b"{}") {
        Ok(()) => "Ok".to_owned(),
        Err(e) => format!("Err({e})"),
    };
    let direct = match fs::write(missing.join("session.json"), b"{}") {
        Ok(()) => "Ok".to_owned(),
        Err(e) => io_line(&e),
    };
    println!();
    println!("save_session_bytes into a missing dir -> {session}");
    println!("plain fs::write into a missing dir    -> {direct}");
    println!("did either create the dir? {}", missing.exists());
    remove_hard(&base);
    Ok(())
}

/// Honest clean-up: a hostile NAME can survive a cleanup that throws on it.
#[test]
#[cfg(windows)]
fn probe_scratch_is_cleanable() -> Result<(), Box<dyn Error>> {
    // Its OWN root, wiped fresh, then deliberately polluted with a hostile
    // name this test creates itself: the proof is that removal still wins.
    let root = init_scratch("cleanup_proof")?;
    fs::write(verb(&root.join("hostile.notes.")), b"x")?;
    fs::write(root.join("plain.txt"), b"y")?;
    remove_hard(&root);
    let left = fs::symlink_metadata(&root).is_ok();
    println!();
    println!("scratch {} survived clean-up? {left}", root.display());
    if left {
        let mut names = Vec::new();
        if let Ok(entries) = fs::read_dir(verb(&root)) {
            for e in entries.filter_map(|e| e.ok()) {
                names.push(name_of(&e.path()));
            }
        }
        println!("  leftovers: {names:?}");
        println!(
            "  remove by hand: cmd /c rmdir /s /q \"{EXT}{root}\"",
            root = root.display()
        );
    }
    assert!(
        !left,
        "the scratch root survived every deletion route -- see the printed path"
    );
    Ok(())
}

/// SCRATCH LOCATION (D69): the untitled note owns a fixed home at
/// <StateDir>/notes/untitled.notes. This table walks the StateDir shapes
/// core can produce and asserts, per row:
/// * the scratch name is a REAL .notes name per is_notes_path — so the
///   ADR-0001 arming rule applies to it like to any other note;
/// * the verdict is never a stripped or reparse spelling, for any shape;
/// * the parent-directory rule COMPOSES: a stripped component in the state
///   dir itself is refused through the notes path (the BLOCKER-2 rule);
/// * the UNC exe directory REFUSES as an observable outcome —
///   ensure_scratch_dir consults the name-only policy BEFORE any filesystem
///   call, so an unreachable host can never turn "save the untitled note"
///   into a network stall.
///
/// ACCEPTED FAILURE MODES, stated as decisions (see the companion test):
/// a roaming or OneDrive-redirected APPDATA moves the note body along with
/// the state — the same exposure portable mode already accepted.
#[test]
fn scratch_location_over_state_dir_shapes() -> Result<(), Box<dyn Error>> {
    let installed = StateDir(PathBuf::from(r"C:\Users\u\AppData\Roaming\notes-gpui"));
    let portable = StateDir(PathBuf::from(r"C:\Apps\Notes Portable\Data"));
    let unc_exe = StateDir(PathBuf::from(r"\\no-such-host\share\App\Data"));
    let dotted_state = StateDir(PathBuf::from(r"C:\u\sub."));
    for (dir, expected) in [
        (&installed, "Allowed"),
        (&portable, "Allowed"),
        (&unc_exe, "UnboundedNetwork"),
    ] {
        let scratch = scratch_note_path(dir);
        assert_eq!(verdict_of(&scratch), *expected, "{scratch:?}");
        assert_ne!(
            verdict_of(&scratch),
            "StrippedName",
            "never a stripped spelling: {scratch:?}"
        );
        assert_ne!(
            verdict_of(&scratch),
            "ReservedDevice",
            "never a device spelling: {scratch:?}"
        );
    }
    // A real .notes name on the ALLOWED shapes: the ADR-0001 arming rule
    // applies to the scratch note like to any other note. On the UNC shape
    // the AGREEMENT rule (is_notes_path) answers false too — a path core
    // refuses is not a .notes document, so the scratch name can never be
    // armed for a network location.
    assert!(is_notes_path(&scratch_note_path(&installed)));
    assert!(is_notes_path(&scratch_note_path(&portable)));
    assert!(!is_notes_path(&scratch_note_path(&unc_exe)));
    // The parent rule composes: the state dir's own stripped component is
    // refused through the notes path (per-component judging, BLOCKER-2).
    assert_eq!(
        verdict_of(&scratch_note_path(&dotted_state)),
        "StrippedName",
        "a stripped state-dir component composes into the refusal"
    );
    // THE UNC REFUSAL, observable: the error is the policy gate (typed as
    // PermissionDenied), raised BEFORE any filesystem call — the test would
    // hang for seconds here if the gate ever moved behind the I/O.
    let Err(e) = ensure_scratch_dir(&unc_exe) else {
        panic!("a UNC state dir must refuse the scratch dir, not write across the network");
    };
    assert!(
        matches!(
            &e,
            SessionError::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied
        ),
        "the refusal is the policy gate, not a network error: {e:?}"
    );
    Ok(())
}

/// ACCEPTED FAILURE MODES, named so they are decisions and not accidents:
/// * a roaming or OneDrive-redirected APPDATA moves the note body along
///   with the state — asserted here by the scratch note living INSIDE the
///   state dir; portable mode accepted the same exposure first;
/// * ClearRecents removes the menu pointer but NOT session.path and NOT the
///   scratch file — a known gap, out of scope (api owns session.path; core
///   owns the file and does not delete user text).
#[test]
fn scratch_note_roams_with_the_state_and_clear_recents_spares_it() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let state = StateDir(dir.path().to_path_buf());
    ensure_scratch_dir(&state)?;
    let scratch = scratch_note_path(&state);
    fs::write(&scratch, b"typed text")?;
    // The exposure, made explicit: the note body lives INSIDE the state dir,
    // so a redirected APPDATA relocates it together with the session.
    assert!(
        scratch.starts_with(&state.0),
        "the accepted roaming exposure: the scratch note is inside the state dir"
    );
    let listed = push(Vec::new(), scratch.clone(), "untitled.notes");
    assert_eq!(listed.len(), 1, "the scratch note was on the menu");
    let list = clear();
    assert!(list.is_empty(), "ClearRecents empties the menu pointer");
    assert!(
        scratch.exists(),
        "KNOWN GAP: ClearRecents does not remove the scratch file (or session.path) -- accepted, out of scope"
    );
    Ok(())
}

/// TWO DISTINCT REAL FILES, ONE MRU IDENTITY KEY (the auditor's U+0130
/// finding): NTFS does not fold U+0130 (I-with-dot) with i+U+0307 the way
/// full-Unicode to_lowercase does, so both names EXIST on disk as separate
/// files while the old key rule called them one — a silent shadow that made
/// the MRU drop a slot and reopen the stored path, not the opened file.
/// The canonicalise-SUCCESS arm now keys with the OS's own casing verbatim.
#[test]
#[cfg(windows)]
fn dotted_capital_i_and_i_combining_dot_are_two_files_two_keys() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let dotted_capital = dir.path().join("\u{0130}.notes");
    let i_combining = dir.path().join("i\u{0307}.notes");
    fs::write(verb(&dotted_capital), b"A")?;
    fs::write(verb(&i_combining), b"B")?;
    assert!(
        fs::symlink_metadata(verb(&dotted_capital)).is_ok()
            && fs::symlink_metadata(verb(&i_combining)).is_ok(),
        "the host must really hold two distinct files for this proof"
    );
    let key_a = identity_key(&dotted_capital);
    let key_b = identity_key(&i_combining);
    assert_ne!(
        key_a, key_b,
        "two distinct on-disk files must have two identity keys"
    );
    // And both reopen: the MRU would not hold a row pointing at a shadow.
    assert_eq!(fs::read(verb(&dotted_capital))?, b"A");
    assert_eq!(fs::read(verb(&i_combining))?, b"B");
    Ok(())
}
