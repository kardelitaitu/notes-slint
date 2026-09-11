//! CRASH SAFETY: the rename is the atomic commit point — the guarantee the
//! whole app rests on, held since the first review and now proved with a
//! REAL killed process, not a simulation.
//!
//! The child is THIS test binary re-invoked with --exact on a worker test
//! that reads its marching orders from the environment. No new binary, no
//! Cargo.toml change; the worker is a no-op in normal suite runs. The
//! parent waits for the child's READY marker on stdout, sleeps N
//! milliseconds into the write window, and kills (Child::kill =
//! TerminateProcess on Windows). For EVERY timing it asserts:
//!   * the target is byte-identical to the OLD bytes or the NEW bytes —
//!     never truncated, never zero-length, never a mix;
//!   * whatever litter the child left is our temp shape, is reclaimable by
//!     the sweep (backdated past the age gate), and is not the note;
//!   * the next successful save of the SAME path lands the new bytes.
//!
//! HONESTY: a sharing violation from a killed process's handles is reported
//! in the panic text, never retried into green.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use notes_core::{Detected, LineEnding, TextEncoding, save_document, save_session_bytes};

// Per-target unused-crate shims: the lint fires per test binary.
use serde as _;
use serde_json as _;
use thiserror as _;
use toml as _;

fn utf8_det() -> Detected {
    Detected {
        encoding: TextEncoding::Utf8,
        line_ending: LineEnding::Lf,
        trailing_newline: false,
        bom_present: false,
    }
}

/// The worker: a no-op until a parent orders a write. The parent spawns
/// THIS binary with --exact on this test; the order is
/// "note|<target>|<bytes>" or "session|<state_dir>|<bytes>".
#[test]
fn crash_child_worker_executes_the_ordered_write() {
    let Ok(order) = std::env::var("CRASH_SAFETY_ORDER") else {
        return; // normal suite run: nothing ordered, nothing to do
    };
    let mut parts = order.split('|');
    let kind = parts.next().unwrap_or("");
    let path = PathBuf::from(parts.next().unwrap_or(""));
    let len: usize = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let body = "x".repeat(len);
    println!("CRASH-SAFETY-CHILD-READY");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    match kind {
        "note" => {
            let _ = save_document(&path, &body, utf8_det());
        }
        "session" => {
            let _ = save_session_bytes(&path, body.as_bytes());
        }
        _ => {}
    }
    println!("CRASH-SAFETY-CHILD-DONE");
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

/// Spawn the child, wait for READY, sleep `delay` into the write, kill.
fn spawn_and_kill_at(order: &str, delay: Duration) -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let mut child: Child = Command::new(exe)
        .args([
            "crash_child_worker_executes_the_ordered_write",
            "--exact",
            "--nocapture",
        ])
        .env("CRASH_SAFETY_ORDER", order)
        .stdout(Stdio::piped())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or("the child's stdout was not captured")?;
    let mut lines = BufReader::new(stdout).lines();
    let mut ready = false;
    for line in lines.by_ref() {
        if line?.contains("CRASH-SAFETY-CHILD-READY") {
            ready = true;
            break;
        }
    }
    if !ready {
        let _ = child.kill();
        let _ = child.wait();
        return Err("the child exited before signalling READY".into());
    }
    std::thread::sleep(delay);
    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

/// Backdate a file's mtime so the sweep's age gate releases it.
fn age_file(path: &Path, seconds_ago: u64) -> Result<(), std::io::Error> {
    let f = std::fs::File::options().append(true).open(path)?;
    let past = std::time::SystemTime::now() - Duration::from_secs(seconds_ago);
    f.set_times(std::fs::FileTimes::new().set_modified(past))?;
    Ok(())
}

/// The temp litter of one target: our shape, in the target's parent.
fn temp_litter(parent: &Path, target: &Path) -> Vec<PathBuf> {
    let prefix = format!(
        "{}.tmp-",
        target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    );
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(parent) {
        for e in entries.flatten() {
            let p = e.path();
            if let Some(n) = p.file_name().map(|n| n.to_string_lossy().into_owned()) {
                if n.starts_with(&prefix) && n.ends_with(".part") {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// THE NOTE SWEEP: kill at eight points across the write window. The note
/// is sized just under the 8 MiB guard so the write takes real time; the
/// parent prints which timings landed mid-window (original intact AND
/// litter present) so the sweep cannot quietly miss the window.
#[test]
fn a_crash_at_any_point_of_the_write_never_damages_the_note()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let target = dir.path().join("note.notes");
    let old = "the original note, byte for byte";
    save_document(&target, old, utf8_det())?;
    let new_len = 7 * 1024 * 1024 + 512 * 1024; // under the 8 MiB guard
    let new_body: Vec<u8> = vec![b'x'; new_len];
    let mut mid_window = 0usize;
    for delay_ms in [0u64, 2, 5, 10, 20, 40, 80, 160] {
        let order = format!("note|{}|{new_len}", target.display());
        spawn_and_kill_at(&order, Duration::from_millis(delay_ms))?;
        // (a) byte-intact: old or new, never anything else.
        let now = std::fs::read(&target)?;
        assert!(
            now == old.as_bytes() || now == new_body,
            "timing {delay_ms}ms: the note is DAMAGED — {} bytes, expected {} (old) or {} (new)",
            now.len(),
            old.len(),
            new_len
        );
        let litter = temp_litter(dir.path(), &target);
        if now == old.as_bytes() && !litter.is_empty() {
            mid_window += 1;
        }
        println!(
            "timing {delay_ms}ms: {} , litter: {}",
            if now == old.as_bytes() {
                "original intact"
            } else {
                "rename had landed"
            },
            litter.len()
        );
        // (b) the litter is reclaimable and is not the note.
        for l in &litter {
            age_file(l, 120)?;
        }
        save_document(&target, "next save", utf8_det())?;
        for l in &litter {
            assert!(
                !l.exists(),
                "timing {delay_ms}ms: the sweep did not reclaim {}",
                l.display()
            );
        }
        assert_eq!(
            std::fs::read(&target)?,
            b"next save",
            "timing {delay_ms}ms: the next save did not land"
        );
        // restore the pre-crash state for the next timing
        save_document(&target, old, utf8_det())?;
    }
    assert!(
        mid_window > 0,
        "no timing landed mid-window (original intact AND temp present) — the sweep measured nothing"
    );
    println!("mid-window timings: {mid_window}");
    Ok(())
}

/// THE SAME SHAPE FOR OUR OWN STATE (item 3): session.json is written on
/// every exit; a crash mid-write must leave the PREVIOUS session intact —
/// identical bytes to a well-formed document the engine itself wrote is the
/// parseability proof (the crate's own reader parses those bytes; serde_json
/// is not a dev-dependency, so parseability rides on byte identity).
#[test]
fn a_crash_during_a_session_write_leaves_the_previous_session_parseable()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let old: &[u8] = br#"{"rect":{"x":11,"y":22,"w":33,"h":44}}"#;
    save_session_bytes(dir.path(), old)?;
    let new_len = 32 * 1024 * 1024; // no size guard on sessions: a long window
    let new_body: Vec<u8> = vec![b'x'; new_len];
    let mut mid_window = 0usize;
    for delay_ms in [0u64, 5, 15, 40, 100, 200, 400] {
        let order = format!("session|{}|{new_len}", dir.path().display());
        spawn_and_kill_at(&order, Duration::from_millis(delay_ms))?;
        let now = std::fs::read(dir.path().join("session.json"))?;
        assert!(
            now == old || now == new_body,
            "timing {delay_ms}ms: session.json is DAMAGED — {} bytes, expected {} (old) or {} (new)",
            now.len(),
            old.len(),
            new_len
        );
        // A kill AFTER the rename finds the new bytes: that is a COMPLETED
        // save, not damage. Mid-window is the interesting case: the previous
        // session still in place while our temp sits beside it.
        if now == old && !temp_litter(dir.path(), &dir.path().join("session.json")).is_empty() {
            mid_window += 1;
        }
        println!(
            "timing {delay_ms}ms: {}",
            if now == old {
                "previous session intact"
            } else {
                "rename had landed"
            }
        );
    }
    assert!(
        mid_window > 0,
        "no session timing landed mid-window — the sweep measured nothing"
    );
    Ok(())
}
