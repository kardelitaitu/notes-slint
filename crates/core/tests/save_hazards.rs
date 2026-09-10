//! SAVE HAZARDS: black-box cases the atomic-save engine must not get wrong,
//! driven through the shipped path (save_document -> atomic_write).
//!
//! Two families:
//!
//! 1. The D12 sweep and link handling (the two proven data-loss blockers). A
//!    sweep that reclaims "any name starting with <target>.tmp-" deletes user
//!    files; a rename that runs over a symlink destroys the link and orphans
//!    the real file behind it while the app reports success.
//! 2. Windows path hostility: a name with a trailing dot, a trailing space, an
//!    embedded newline, a case-only difference, an over-long path, a leading
//!    UTF-8 BOM, a reserved device name. The only category worth a BLOCKER is
//!    a save that returns Ok while the bytes land in a file OTHER than the one
//!    named - probe() measures exactly that, per case.
//!
//! STATUS: every case below now runs. The sweep cases pass against the
//! exact-shape + .part + age-gate sweep; the link case passes against
//! refuse_reparse_point; the four trailing-name cases pass against the
//! Win32-stripped-name refusal in atomic_write (each was a proven blocker:
//! Ok reported, bytes landed in the stripped-name neighbour). Temp names in
//! these tests use the real engine shape "<file>.tmp-<pid>-<nanos>-<attempt>.part".
//!
//! Lints: notes-core denies unwrap_used/expect_used for ALL targets, so every
//! case here is ?-typed and asserts instead.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use serde as _;
use serde_json as _;
use thiserror as _;
use toml as _;

use notes_core::{Detected, LineEnding, TextEncoding, is_notes_path, save_document};

/// The bytes the app wants to write - distinct from every sentinel, so
/// "where did it land" is never ambiguous.
const APP_BYTES: &str = "APP-WROTE-THIS";
/// What a neighbour file holds before a save. If it changes, bytes landed
/// somewhere the caller did not name.
const SENTINEL: &[u8] = b"SENTINEL-USER-DATA";

/// What one hostile path actually did, as observed on disk.
#[derive(Debug)]
struct Probe {
    /// How save_document answered.
    outcome: String,
    /// Names in the scratch dir whose bytes are now APP_BYTES.
    landed: Vec<String>,
    /// Names that held a sentinel before and no longer do.
    clobbered: Vec<String>,
}

fn utf8_det() -> Detected {
    Detected {
        encoding: TextEncoding::Utf8,
        line_ending: LineEnding::Lf,
        trailing_newline: false,
        bom_present: false,
    }
}

fn put(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, Box<dyn Error>> {
    let p = dir.join(name);
    fs::write(&p, bytes)?;
    Ok(p)
}

/// Pushes a file mtime into the past: the sweep age gate (M4) and the
/// "leftover from a crash weeks ago" scenarios need real mtimes, and sleeping
/// is not an option in a suite.
fn age_file(path: &Path, age: Duration) -> Result<(), Box<dyn Error>> {
    let past = SystemTime::now() - age;
    let f = fs::OpenOptions::new().write(true).open(path)?;
    f.set_times(fs::FileTimes::new().set_modified(past))?;
    Ok(())
}

fn name_of(dir: &Path, path: &Path) -> String {
    path.strip_prefix(dir)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn listing(dir: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let mut v = Vec::new();
    for e in fs::read_dir(dir)? {
        v.push(name_of(dir, &e?.path()));
    }
    v.sort();
    Ok(v)
}

/// Seeds a scratch dir with the neighbours (each holding a sentinel), saves
/// APP_BYTES to dir.join(hostile), and reports where the bytes went.
fn probe(hostile: &str, neighbours: &[&str]) -> Result<Probe, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    for n in neighbours {
        put(root, n, SENTINEL)?;
    }
    let target = root.join(hostile);
    let outcome = match save_document(&target, APP_BYTES, utf8_det()) {
        Ok(o) => format!("Ok(path={})", name_of(root, &o.path)),
        Err(e) => format!("Err({e})"),
    };
    let mut landed = Vec::new();
    let mut clobbered = Vec::new();
    for e in fs::read_dir(root)? {
        let p = e?.path();
        let name = name_of(root, &p);
        let Ok(bytes) = fs::read(&p) else { continue };
        if bytes == APP_BYTES.as_bytes() {
            landed.push(name.clone());
        }
        if neighbours.contains(&name.as_str()) && bytes != SENTINEL {
            clobbered.push(name);
        }
    }
    Ok(Probe {
        outcome,
        landed,
        clobbered,
    })
}

/// THE property, stated once: a save that reports success must put its bytes
/// in the file it was named for and change nothing else. A non-empty clobbered
/// list next to an Ok is the "silently writing somewhere else" category, i.e.
/// a BLOCKER.
fn assert_no_silent_elsewhere(p: &Probe, hostile: &str) {
    eprintln!(
        "  case {hostile:?}: outcome={} landed={:?} clobbered={:?}",
        p.outcome, p.landed, p.clobbered
    );
    if p.outcome.starts_with("Ok") {
        assert!(
            p.clobbered.is_empty(),
            "{hostile:?}: save returned {} but also rewrote {:?}",
            p.outcome,
            p.clobbered
        );
        assert!(
            p.landed.iter().any(|n| n == hostile),
            "{hostile:?}: save returned {} but its bytes are in {:?}",
            p.outcome,
            p.landed
        );
    } else {
        assert!(
            p.clobbered.is_empty(),
            "{hostile:?}: save failed ({}) yet a neighbour was rewritten: {:?}",
            p.outcome,
            p.clobbered
        );
    }
}

// ---------------------------------------------------------------------------
// 1. The sweep
// ---------------------------------------------------------------------------

/// A save leaves no temp litter, and never touches another target's temps.
/// Passes at HEAD; kept on as the regression net for the in-flight rewrite.
#[test]
fn save_leaves_only_the_target_and_spares_other_targets_temps() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let target = dir.path().join("x.notes");
    put(
        dir.path(),
        "y.notes.tmp-111-222-333",
        b"other target litter",
    )?;
    save_document(&target, "new", utf8_det())?;
    assert_eq!(
        listing(dir.path())?,
        vec!["x.notes".to_string(), "y.notes.tmp-111-222-333".to_string()]
    );
    Ok(())
}

/// B1, the user's diagnosis copy: "<target>.tmp-backup" is not ours to delete.
///
/// FAILS at HEAD: sweep_stale_temps deletes every name that
/// starts_with("<target>.tmp-"), so a save destroys the backup. The in-flight
/// exact-shape predicate (is_our_temp) covers this name.
#[test]
fn sweep_spares_a_named_backup_of_the_target() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let target = dir.path().join("x.notes");
    let backup = put(dir.path(), "x.notes.tmp-backup", b"MY ONLY COPY")?;
    age_file(&backup, Duration::from_secs(3600))?;
    save_document(&target, "new", utf8_det())?;
    assert_eq!(
        fs::read(&backup)?,
        b"MY ONLY COPY",
        "a file the user named after our temp pattern is not our litter"
    );
    Ok(())
}

/// B1, the ISO-dated copy: "<target>.tmp-2026-09-10" is a wholly natural name
/// for "the corrupt one I kept".
///
/// FAILS at HEAD, AND still fails against the in-flight is_our_temp: once
/// ".tmp-" is stripped, the remainder "2026-09-10" splits into exactly three
/// all-ASCII-digit fields, and a dated backup is by definition older than
/// SWEEP_MIN_AGE_SECS. THIS ONE IS A STILL-OPEN FINDING, not a fix waiting to
/// land.
#[test]
fn sweep_spares_a_dated_backup_of_the_target() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let target = dir.path().join("session.json");
    let backup = put(dir.path(), "session.json.tmp-2026-09-10", b"MY ONLY COPY")?;
    age_file(&backup, Duration::from_secs(3600 * 24 * 30))?;
    save_document(&target, "{}", utf8_det())?;
    assert!(
        backup.exists(),
        "the save deleted a 30-day-old dated backup of the target"
    );
    Ok(())
}

/// M4: a temp of OUR shape belonging to a LIVE second instance (milliseconds
/// old) must not be reclaimed.
///
/// FAILS at HEAD: no age gate exists there at all.
#[test]
fn sweep_spares_a_live_temp_of_another_instance() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let target = dir.path().join("x.notes");
    let live = dir.path().join("x.notes.tmp-999999-123456-0.part");
    fs::write(&live, b"another instance is writing this")?;
    save_document(&target, "new", utf8_det())?;
    assert!(
        live.exists(),
        "the sweep reclaimed a temp created milliseconds ago"
    );
    Ok(())
}

/// The reclaim we DO want: genuine crash litter of our own shape, old enough
/// to be dead.
#[test]
fn sweep_reclaims_old_crash_litter() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let target = dir.path().join("x.notes");
    let litter = put(
        dir.path(),
        "x.notes.tmp-999999-123456-0.part",
        b"crashed mid-write",
    )?;
    age_file(&litter, Duration::from_secs(3600))?;
    save_document(&target, "new", utf8_det())?;
    assert!(
        !litter.exists(),
        "old crash litter of our own shape should be reclaimed"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// 2. Links
// ---------------------------------------------------------------------------

/// B2: a file symlink must be refused, never renamed over. Needs Developer
/// Mode / SeCreateSymbolicLinkPrivilege; skips (passing) when the OS refuses
/// to make the link, so it stays green on a locked-down box.
///
/// FAILS at HEAD: no reparse check exists, so the rename replaces the link
/// with a plain file, the real file behind it keeps the OLD bytes, and
/// save_document returns Ok.
#[test]
fn save_refuses_a_symlink_target() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let real = dir.path().join("real.notes");
    fs::write(&real, SENTINEL)?;
    let link = dir.path().join("link.notes");
    #[cfg(windows)]
    {
        use std::os::windows::fs::symlink_file;
        if let Err(e) = symlink_file(&real, &link) {
            eprintln!("skip: cannot create a file symlink here ({e})");
            return Ok(());
        }
    }
    let result = save_document(&link, APP_BYTES, utf8_det());
    // Judged by EFFECT, not by a variant name, so the same test compiles
    // against the pre-fix and the post-fix SaveError enum.
    let still_a_link = link.is_symlink();
    let real_bytes = fs::read(&real)?;
    if still_a_link && real_bytes.as_slice() == SENTINEL {
        eprintln!("  link intact, save answered {result:?}");
        if result.is_ok() {
            return Err("the link survived but the save reported Ok".into());
        }
        return Ok(());
    }
    panic!(
        "the symlink was destroyed: save answered {result:?}; is_symlink={still_a_link}; real file now holds {:?}",
        String::from_utf8_lossy(&real_bytes)
    );
}

/// The accepted trade-off, pinned so nobody "fixes" it by accident: a HARD
/// LINK is not a reparse point, so the save replaces the named entry and the
/// sibling link keeps the old content.
#[cfg(windows)]
#[test]
fn hard_link_target_is_replaced_and_the_sibling_keeps_the_old_bytes() -> Result<(), Box<dyn Error>>
{
    let dir = tempfile::tempdir()?;
    let target = dir.path().join("n.notes");
    fs::write(&target, SENTINEL)?;
    let sibling = dir.path().join("sibling.notes");
    let out = Command::new("cmd")
        .args(["/C", "mklink", "/H"])
        .arg(&sibling)
        .arg(&target)
        .output()?;
    if !out.status.success() {
        eprintln!(
            "skip: mklink /H failed: {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
        return Ok(());
    }
    save_document(&target, APP_BYTES, utf8_det())?;
    assert_eq!(fs::read(&target)?, APP_BYTES.as_bytes());
    assert_eq!(
        fs::read(&sibling)?,
        SENTINEL,
        "documented behaviour: the sibling hard link keeps the old content"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// 3. Windows path hostility (item 6)
// ---------------------------------------------------------------------------

/// A line feed as a char, so no source escape is needed inside a file name.
fn nl() -> char {
    10u8 as char
}

/// Trailing DOT: Win32 strips trailing dots from the final component, so
/// "notes." resolves to "notes". With a real "notes" file in the folder the
/// save lands THERE and reports Ok at a path that does not exist.
#[test]
fn trailing_dot_path() -> Result<(), Box<dyn Error>> {
    let p = probe("notes.", &["notes"])?;
    assert_no_silent_elsewhere(&p, "notes.");
    Ok(())
}

/// Trailing SPACE: same strip, same hazard.
#[test]
fn trailing_space_path() -> Result<(), Box<dyn Error>> {
    let p = probe("notes ", &["notes"])?;
    assert_no_silent_elsewhere(&p, "notes ");
    Ok(())
}

/// Both at once - the shape a shell-completed or OCRd path can carry.
#[test]
fn trailing_dot_and_space_path() -> Result<(), Box<dyn Error>> {
    let p = probe("notes. ", &["notes"])?;
    assert_no_silent_elsewhere(&p, "notes. ");
    Ok(())
}

/// The same strip applied to a .notes name, where the neighbour is the app's
/// own autosave target. Also records what is_notes_path makes of the dotted
/// name, because that answer decides whether autosave is armed for it.
#[test]
fn trailing_dot_on_a_notes_name() -> Result<(), Box<dyn Error>> {
    eprintln!(
        "  is_notes_path(a.notes.) = {} / is_notes_path(a.notes) = {}",
        is_notes_path(Path::new("a.notes.")),
        is_notes_path(Path::new("a.notes"))
    );
    let p = probe("a.notes.", &["a.notes"])?;
    assert_no_silent_elsewhere(&p, "a.notes.");
    Ok(())
}

/// A FILE NAME containing a newline (legal on NTFS, one drag away).
#[test]
fn embedded_newline_in_the_name() -> Result<(), Box<dyn Error>> {
    let name = format!("a{}b.notes", nl());
    let p = probe(&name, &["a", "b.notes"])?;
    assert_no_silent_elsewhere(&p, &name);
    Ok(())
}

/// Case-only difference: NTFS is case-insensitive, so "NOTES.TXT" and
/// "notes.txt" are ONE file. The save must therefore produce exactly one file,
/// and this is the fact the recent-files rule (canonicalise for identity, keep
/// the original for display) has to be built on.
#[test]
fn case_only_difference_is_the_same_file() -> Result<(), Box<dyn Error>> {
    let p = probe("NOTES.TXT", &["notes.txt"])?;
    eprintln!("  case-only: outcome={} landed={:?}", p.outcome, p.landed);
    if p.outcome.starts_with("Ok") {
        assert_eq!(
            p.landed.len(),
            1,
            "a case-only name must not create a second file: {:?}",
            p.landed
        );
    }
    Ok(())
}

/// A very long name (300 chars) under a tempdir root: past the 260 MAX_PATH
/// line, and core adds no extended-length prefix of its own. Must fail loudly
/// or land in the file it named - never anywhere else.
#[test]
fn very_long_name() -> Result<(), Box<dyn Error>> {
    let name = format!("{}.notes", "x".repeat(300));
    let p = probe(&name, &[])?;
    eprintln!(
        "  long name: {} chars, outcome={}, dir now holds {:?}",
        name.chars().count(),
        p.outcome,
        p.landed
    );
    assert_no_silent_elsewhere(&p, "a 300-char name");
    Ok(())
}

/// A leading UTF-8 BOM inside the FILE NAME (not the content): legal bytes on
/// NTFS, and must not become the neighbour's file.
#[test]
fn bom_in_the_name() -> Result<(), Box<dyn Error>> {
    let bom = char::from_u32(0xFEFF).unwrap_or(' ');
    let name = format!("{bom}x.notes");
    let p = probe(&name, &["x.notes"])?;
    assert_no_silent_elsewhere(&p, &name);
    Ok(())
}

/// Reserved device names: to part of the Win32 surface "CON" with an extension
/// is still CON. A save that reports Ok with no file on disk means the bytes
/// went to the console device, which this probe catches because landed is
/// empty.
#[test]
fn reserved_device_name() -> Result<(), Box<dyn Error>> {
    let p = probe("CON.notes", &[])?;
    assert_no_silent_elsewhere(&p, "CON.notes");
    Ok(())
}
