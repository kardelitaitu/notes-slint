//! RECENTS IDENTITY: one file, one entry. AGENTS.md's rule is canonicalise
//! for identity, keep the original for display; identity_of is the single
//! choke point (recent.rs) and strip_verbatim removes the \\?\ and
//! \\?\UNC\ prefixes consistently, so the prefix-vs-plain split the review
//! hypothesised cannot happen in our code. What CAN differ is the VOLUME a
//! path resolves to. This probe pins, per filesystem arrangement reachable
//! from a private TEMP root, whether two spellings of one file share one
//! identity key — and documents the boundary where they do not.

use std::path::Path;
use std::process::Command;

use notes_core::{RecentEntry, identity_key, push};

// Per-target unused-crate shims: the lint fires per test binary.
use serde as _;
use serde_json as _;
use thiserror as _;
use toml as _;

fn mklink_junction(link: &Path, target: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let out = Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            &link.to_string_lossy(),
            &target.to_string_lossy(),
        ])
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("mklink /J failed: {}", String::from_utf8_lossy(&out.stderr)).into())
    }
}

fn hardlink(link: &Path, target: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let out = Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/H",
            &link.to_string_lossy(),
            &target.to_string_lossy(),
        ])
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("mklink /H failed: {}", String::from_utf8_lossy(&out.stderr)).into())
    }
}

#[test]
#[cfg(windows)]
fn recents_identity_per_filesystem_arrangement() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let real_dir = dir.path().join("real");
    std::fs::create_dir_all(&real_dir)?;
    let plain = real_dir.join("note.notes");
    std::fs::write(&plain, b"the bytes")?;

    // Control: the same file, two CASINGS — must already be one identity.
    let upper = real_dir.join("NOTE.NOTES");
    assert_eq!(
        identity_key(&plain),
        identity_key(&upper),
        "case variants of one file must share one identity key"
    );

    // Arrangement 1: a JUNCTION to the directory — the same file reached
    // through a different path prefix. Canonicalise resolves through the
    // junction, so this must already unify; if it does not, that is the bug.
    let junction_dir = dir.path().join("linked");
    match mklink_junction(&junction_dir, &real_dir) {
        Ok(()) => {
            let via_junction = junction_dir.join("note.notes");
            assert!(
                via_junction.exists(),
                "the junction must actually reach the file"
            );
            assert_eq!(
                identity_key(&plain),
                identity_key(&via_junction),
                "a junction to the same directory must share one identity key: plain={:?} junction={:?}",
                identity_key(&plain),
                identity_key(&via_junction)
            );
        }
        Err(e) => println!("junction unavailable on this box, skipped: {e}"),
    }

    // Arrangement 2: a HARDLINK — the same CONTENT under a second name. Two
    // distinct directory entries are two legitimate names for one file: the
    // path-based identity keeps them separate BY DESIGN (each name opens
    // "its own" file from the user's point of view; deleting one must not
    // grey out the other). Documented boundary, not a defect.
    let linked = real_dir.join("link.notes");
    match hardlink(&linked, &plain) {
        Ok(()) => {
            let same_key = identity_key(&plain) == identity_key(&linked);
            println!(
                "hardlink shares identity key: {same_key} (documented boundary: path identity, not file-id identity)"
            );
        }
        Err(e) => println!("hardlink unavailable on this box, skipped: {e}"),
    }

    // Arrangement 3 (the review's volume case: substituted drive vs real
    // path, mapped network drive vs UNC) cannot be arranged inside a private
    // TEMP root — subst and net use are machine-global. If canonicalise ever
    // returns different volumes for the same file, the fix needs the
    // platform fact; see the note on identity_of in recent.rs for the
    // signature core would consume.
    Ok(())
}

/// THE DISPLAY RULE, pinned: the same file opened by two spellings holds ONE
/// entry, and the display becomes the spelling the OS itself answered with
/// (the canonical one) — the first spelling only until a canonical one is
/// known. Justification: the canonical spelling survives the junction or the
/// alternate prefix being deleted, so it is the name that keeps working; a
/// non-canonical spelling never overwrites an existing entry.
#[test]
#[cfg(windows)]
fn one_entry_for_one_file_and_the_display_becomes_the_canonical_spelling()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let real_dir = dir.path().join("real");
    std::fs::create_dir_all(&real_dir)?;
    let plain = real_dir.join("note.notes");
    std::fs::write(&plain, b"the bytes")?;
    let junction_dir = dir.path().join("linked");
    if mklink_junction(&junction_dir, &real_dir).is_err() {
        return Ok(()); // junction unavailable on this box: the pin above still holds
    }
    let via_junction = junction_dir.join("note.notes");
    let list = push(Vec::new(), via_junction, "linked\\note.notes");
    let list = push(list, plain.clone(), "real\\note.notes");
    assert_eq!(list.len(), 1, "one file, one entry: {list:?}");
    let RecentEntry {
        display,
        path,
        exists,
    } = &list[0];
    assert_eq!(
        display, "real\\note.notes",
        "the canonical spelling wins the display"
    );
    assert_eq!(path, &plain, "the stored path is the canonical spelling");
    assert!(*exists);
    Ok(())
}
