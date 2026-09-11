//! READ-SIDE / WRITE-SIDE POLICY PARITY - a DIAGNOSTIC file, not a green suite.
//!
//! The invariant under test is one sentence: a path whose NAME notes_core's
//! path_policy refuses must be refused on the READ path too, before the
//! filesystem is touched. On the write path that is a call
//! (save::atomic_write -> path_policy(target) -> SaveError::InvalidPath). On the
//! read path (Engine::open) there is no such call: the sequence is
//! fs::metadata -> is_oversize(len) -> fs::read, and the predicate is reached
//! only later, via is_notes_path -> file_kind, to decide ARMING.
//!
//! HONESTY RULE: every test here asserts the behaviour we believe is CORRECT (a
//! name-level refusal), never today's hole. The two tests marked RED BY DESIGN
//! fail on the current tree and print what the engine actually did; the two
//! marked CONTROL pass today and show which verdicts are already covered by
//! incidental OS behaviour, so the reds are not confused with "nothing works".

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use notes_api::{Command, Event, Gateway, Settings as ApiSettings, StateDir};
use notes_core::{PathVerdict, path_policy};

// notes-platform and thiserror are dependencies of notes-api, not of this target;
// naming them keeps the unused-crate-dependencies lint honest (tests/first_run.rs
// does the same).
use notes_platform as _;
use thiserror as _;

/// Long enough for a slow disk, short enough that a stalled engine fails the run.
const ANSWER: Duration = Duration::from_secs(5);

fn verdict_of(p: &Path) -> &'static str {
    match path_policy(p) {
        PathVerdict::Allowed => "Allowed",
        PathVerdict::UnboundedNetwork => "UnboundedNetwork",
        PathVerdict::ReservedDevice => "ReservedDevice",
        PathVerdict::StreamName => "StreamName",
        PathVerdict::DriveRelative => "DriveRelative",
        PathVerdict::StrippedName => "StrippedName",
    }
}

/// Sends Open and returns the engine's verdict event, ignoring startup chatter.
fn open_and_watch(rx: &Receiver<Event>, path: &Path) -> Event {
    let deadline = Instant::now() + ANSWER;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            remaining.as_millis() > 0,
            "no Loaded/LoadFailed verdict within 5s for {}",
            path.display()
        );
        match rx.recv_timeout(remaining) {
            Ok(ev @ (Event::Loaded { .. } | Event::LoadFailed { .. })) => return ev,
            Ok(_other) => {}
            Err(_) => panic!("timeout awaiting the verdict on {}", path.display()),
        }
    }
}

/// Waits for the Saved/SaveFailed verdict on `path`, ignoring startup chatter.
fn save_and_watch(rx: &Receiver<Event>, path: &Path) -> Event {
    let deadline = Instant::now() + ANSWER;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            remaining.as_millis() > 0,
            "no Saved/SaveFailed verdict within 5s for {}",
            path.display()
        );
        match rx.recv_timeout(remaining) {
            Ok(ev @ (Event::Saved { .. } | Event::SaveFailed { .. })) => return ev,
            Ok(_other) => {}
            Err(_) => panic!("timeout awaiting the save verdict on {}", path.display()),
        }
    }
}

fn describe(ev: &Event) -> String {
    match ev {
        Event::Loaded { text, .. } => format!("Loaded ({} text bytes)", text.len()),
        Event::LoadFailed { reason, .. } => format!("LoadFailed({:?})", reason),
        other => format!("{:?}", other),
    }
}

fn name(ev: &Event) -> &'static str {
    match ev {
        Event::Loaded { .. } => "Loaded",
        Event::LoadFailed { .. } => "LoadFailed",
        _ => "other",
    }
}

/// The shared premise, asserted so no test below can rot silently: the WRITE side
/// refuses this name, and the READ side is being asked about the same spelling.
fn assert_write_side_refuses(path: &Path, expected: PathVerdict) {
    assert_eq!(
        path_policy(path),
        expected,
        "the premise of this file is that the name is refused by policy; \
         path_policy({}) said otherwise",
        path.display()
    );
}

fn host(dir: &Path) -> (Gateway, Receiver<Event>) {
    Gateway::start_with_host(
        StateDir(dir.to_path_buf()),
        ApiSettings::default(),
        None,
        None,
    )
}

// ------------------------------------------------------------------ test 1 --

/// RED BY DESIGN. "dir\target.notes:hidden" is StreamName on the write side. If
/// the read path loads it, the bytes the user sees are not the file they named,
/// the name is one is_notes_path refuses to call a .notes document (so the buffer
/// is UNARMED and looks like a foreign file), and remember() then writes that
/// spelling into session.json - so the next launch re-opens it with no user asking.
#[test]
fn a_stream_name_is_refused_on_read_by_name_not_by_luck() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("target.notes"),
        b"the note the user wrote\n",
    )
    .expect("host file");
    let stream = dir.path().join("target.notes:hidden");
    std::fs::write(&stream, b"bytes from a stream\n").expect("ADS write");
    assert_write_side_refuses(&stream, PathVerdict::StreamName);

    let (gateway, rx) = host(dir.path());
    gateway
        .send(Command::Open {
            path: stream.clone(),
        })
        .expect("queued");
    let ev = open_and_watch(&rx, &stream);
    println!(
        "OPEN {} -> {} | policy={}",
        stream.display(),
        describe(&ev),
        verdict_of(&stream)
    );
    assert_eq!(
        name(&ev),
        "LoadFailed",
        "READ-SIDE HOLE: {:?} is {} on the write side and was loaded as a document \
         on the read side.",
        stream,
        verdict_of(&stream)
    );
    drop(gateway);
}

// ------------------------------------------------------------------ test 2 --

/// RED BY DESIGN. "x.notes." (trailing dot) is StrippedName on the write side.
/// Win32 strips the dot on the read too, so the REAL file's bytes come back under
/// a name that does not exist on disk: titled as the ghost, identified as the
/// ghost by identity_key, and stored in session.json as the ghost.
#[test]
fn a_stripped_name_is_refused_on_read_by_name_not_by_luck() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("x.notes"), b"the real file\n").expect("write real");
    let ghost = dir.path().join("x.notes.");
    assert_write_side_refuses(&ghost, PathVerdict::StrippedName);

    let (gateway, rx) = host(dir.path());
    gateway
        .send(Command::Open {
            path: ghost.clone(),
        })
        .expect("queued");
    let ev = open_and_watch(&rx, &ghost);
    println!(
        "OPEN {} -> {} | policy={}",
        ghost.display(),
        describe(&ev),
        verdict_of(&ghost)
    );
    assert_eq!(
        name(&ev),
        "LoadFailed",
        "READ-SIDE HOLE: {:?} is {} on the write side and was loaded as a document \
         on the read side.",
        ghost,
        verdict_of(&ghost)
    );
    drop(gateway);
}

// ------------------------------------------------------------------ test 3 --

/// RED BY DESIGN (shape, not outcome). "\\.\NUL" is ReservedDevice and the read
/// path DOES refuse it - but with the OS's errno, which is only possible because a
/// filesystem call ran first. So the
/// safety is the OS's, not ours, and it is not owed: the same prefix reaches
/// "\\.\pipe\<name>" and "\\.\CON", where the OS answers by blocking or streaming,
/// on the one thread whose stall is a frozen window.
#[test]
fn a_reserved_device_is_refused_by_the_os_and_says_so() {
    let dir = tempfile::tempdir().expect("tempdir");
    let device = PathBuf::from(r"\\.\NUL");
    assert_write_side_refuses(&device, PathVerdict::ReservedDevice);

    let (gateway, rx) = host(dir.path());
    gateway
        .send(Command::Open {
            path: device.clone(),
        })
        .expect("queued");
    let ev = open_and_watch(&rx, &device);
    println!(
        "OPEN {} -> {} | policy={}",
        device.display(),
        describe(&ev),
        verdict_of(&device)
    );
    let Event::LoadFailed { reason, .. } = &ev else {
        panic!("NUL must never load; observed {}", describe(&ev))
    };
    println!("  refused by: {reason}");
    assert!(
        !describe(&ev).contains("os error"),
        "READ-SIDE HOLE (the shape of it): a device is refused with the OS's own \
         errno ({}), which proves a filesystem call already ran. A policy refusal \
         would name the RULE, the way SaveError::InvalidPath does on the write side.",
        reason
    );
    drop(gateway);
}

// ------------------------------------------------------------------ test 4 --

/// CONTROL, green today: a directory is refused - incidentally, by the OS, and
/// only because fs::read cannot read one. Nothing on the path consulted the name.
#[test]
fn a_directory_is_refused_and_names_its_mechanism() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (gateway, rx) = host(dir.path());
    let target = dir.path().to_path_buf();
    gateway
        .send(Command::Open {
            path: target.clone(),
        })
        .expect("queued");
    let ev = open_and_watch(&rx, &target);
    println!(
        "OPEN {} -> {} | policy={}",
        target.display(),
        describe(&ev),
        verdict_of(&target)
    );
    assert!(
        matches!(ev, Event::LoadFailed { .. }),
        "a directory must not load; observed {}",
        describe(&ev)
    );
    drop(gateway);
}

// ------------------------------------------------------------------ test 5 --

/// Diagnostic, asserts nothing about the product: WHICH of the two calls in
/// Engine::open answers for each hostile name, because "metadata said so" and
/// "read said so" are different exposures (the second one opened the object).
#[test]
fn diagnostic_which_call_answers_for_each_refused_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("y.notes"), b"body\n").expect("write");
    let mut rows = Vec::new();
    for (label, raw) in [
        ("ReservedDevice NUL", r"\\.\NUL".to_string()),
        (
            "ReservedDevice device-root",
            r"\\.\PIPE\notes-nope".to_string(),
        ),
        ("Allowed dir", dir.path().display().to_string()),
        (
            "StrippedName",
            dir.path().join("y.notes.").display().to_string(),
        ),
        (
            "Allowed then read",
            dir.path().join("y.notes").display().to_string(),
        ),
    ] {
        let path = PathBuf::from(&raw);
        let stat = match std::fs::metadata(&path) {
            Ok(m) => format!("Ok(len={} is_file={})", m.len(), m.is_file()),
            Err(e) => format!("Err({})", e),
        };
        let read = match std::fs::read(&path) {
            Ok(b) => format!("Ok({} bytes)", b.len()),
            Err(e) => format!("Err({})", e),
        };
        rows.push(format!(
            "  {label:28} policy={:16} metadata={:60} read={}",
            verdict_of(&path),
            stat,
            read
        ));
    }
    println!("{}", rows.join("\n"));
}

// ------------------------------------------------------------------ test 6 --

/// The second read-side funnel, and the one that runs BEFORE A WINDOW EXISTS:
/// the recents probe stats up to ten stored names in Engine::with_host. A name
/// the read gate refuses must come back greyed WITHOUT the stat - a stream
/// answers is_file() == true, so without the gate a launch lights up a recent
/// the app will refuse to open, and stats the very name the gate exists for.
#[test]
fn a_refused_recent_name_is_never_statted_by_the_probe() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("target.notes"),
        b"the note the user wrote\n",
    )
    .expect("host file");
    let stream = dir.path().join("target.notes:hidden");
    std::fs::write(&stream, b"bytes from a stream\n").expect("ADS write");
    assert_write_side_refuses(&stream, PathVerdict::StreamName);
    // THE PREMISE, measured in this process so the test cannot rot silently:
    // the probe's stat WOULD answer true, so exists == false below is the
    // gate's doing, not the disk's.
    assert!(
        stream.is_file(),
        "premise broken: the probe's stat would not have lit this entry up, \
         so the assertion below would prove nothing"
    );

    let settings = ApiSettings {
        recents: vec![notes_core::RecentEntry {
            path: stream.clone(),
            display: "target.notes:hidden".to_string(),
            exists: false,
        }],
        ..ApiSettings::default()
    };
    let (gateway, rx) =
        Gateway::start_with_host(StateDir(dir.path().to_path_buf()), settings, None, None);
    // THE STARTUP ANNOUNCE is the probe's only report: nothing sets exists
    // before it (mark_missing only CLEARS the flag), so exists == false in
    // the announce is exactly "the probe never statted this name".
    let deadline = Instant::now() + ANSWER;
    let entries = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(remaining.as_millis() > 0, "no RecentsUpdated within 5s");
        match rx.recv_timeout(remaining) {
            Ok(Event::RecentsUpdated(entries)) => break entries,
            Ok(_other) => {}
            Err(_) => panic!("timeout awaiting the startup announce"),
        }
    };
    let entry = entries.iter().find(|e| e.path == stream).expect(
        "the refused entry must stay in the list, greyed - a menu that \
                 silently eats entries is the worse bug",
    );
    assert!(
        !entry.exists,
        "READ-SIDE HOLE (the second funnel): the recents probe statted {} - a \
         name the read gate refuses - and lit it up as existing",
        stream.display()
    );
    drop(gateway);
}

// ------------------------------------------------------------------ test 7 --

/// A refused Open does not REBIND the buffer: the bridge keeps showing the
/// document it had, per the LoadFailed contract. So a refusal of a hostile
/// name must not make THAT document unsavable - the user was mid-sentence in
/// it. The c92494f3 shape, asked from the new gate's direction: does the
/// refusal latch a session-wide flag that Save As then obeys?
#[test]
fn a_refused_open_does_not_hold_the_loaded_document_hostage() {
    let dir = tempfile::tempdir().expect("tempdir");
    let a = dir.path().join("a.notes");
    std::fs::write(&a, b"the note the user wrote\n").expect("note A");
    let (gateway, rx) = host(dir.path());
    gateway
        .send(Command::Open { path: a.clone() })
        .expect("queued");
    let loaded = open_and_watch(&rx, &a);
    assert_eq!(
        name(&loaded),
        "Loaded",
        "premise: note A is on screen and typed into"
    );

    let stream = dir.path().join("a.notes:hidden");
    std::fs::write(&stream, b"bytes from a stream\n").expect("ADS write");
    assert_write_side_refuses(&stream, PathVerdict::StreamName);
    gateway
        .send(Command::Open {
            path: stream.clone(),
        })
        .expect("queued");
    let refused = open_and_watch(&rx, &stream);
    assert_eq!(
        name(&refused),
        "LoadFailed",
        "the gate refused the stream; observed {}",
        describe(&refused)
    );

    // Save As carries the text the BRIDGE holds - A's typed buffer. The only
    // question is whether the engine lets it land.
    let saved = dir.path().join("saved.notes");
    gateway
        .send(Command::SaveAs {
            path: saved.clone(),
            text: "typed\n".to_string(),
            revision: 0,
        })
        .expect("queued");
    let ev = save_and_watch(&rx, &saved);
    println!("SAVE AS {} -> {}", saved.display(), describe(&ev));
    assert!(
        matches!(ev, Event::Saved { .. }),
        "SAVABILITY HOLE: a refused Open of {} made the loaded document \
         unsavable; Save As answered {}",
        stream.display(),
        describe(&ev)
    );
    assert_eq!(
        std::fs::read(&saved).expect("saved bytes"),
        b"typed\n",
        "the typed bytes must land whole"
    );
    drop(gateway);
}

// ------------------------------------------------------------------ test 8 --

/// The OTHER direction: the refusal arrives with NOTHING loaded behind it -
/// the untitled startup buffer is the only thing the user has (and session.json
/// naming a hostile path is exactly how STEP 5 produces this on every launch).
/// The typed bytes in that buffer are real work; Save As must take them.
#[test]
fn a_refused_first_open_leaves_the_untitled_buffer_a_way_out() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (gateway, rx) = host(dir.path());
    let device = PathBuf::from(r"\\.\NUL");
    assert_write_side_refuses(&device, PathVerdict::ReservedDevice);
    gateway
        .send(Command::Open {
            path: device.clone(),
        })
        .expect("queued");
    let refused = open_and_watch(&rx, &device);
    assert_eq!(
        name(&refused),
        "LoadFailed",
        "the gate refused the device; observed {}",
        describe(&refused)
    );

    let out = dir.path().join("out.notes");
    gateway
        .send(Command::SaveAs {
            path: out.clone(),
            text: "typed before anything loaded\n".to_string(),
            revision: 0,
        })
        .expect("queued");
    let ev = save_and_watch(&rx, &out);
    println!("SAVE AS {} -> {}", out.display(), describe(&ev));
    assert!(
        matches!(ev, Event::Saved { .. }),
        "SAVABILITY HOLE (first open): the refused Open left the untitled \
         buffer unsavable; Save As answered {}",
        describe(&ev)
    );
    assert_eq!(
        std::fs::read(&out).expect("saved bytes"),
        b"typed before anything loaded\n",
        "the typed bytes must land whole"
    );
    drop(gateway);
}

// ------------------------------------------------------------------ test 9 --

/// The guard is NOT a nuisance: Save As onto the refused name itself stays
/// refused, because that is the one target whose bytes the app never read -
/// the measured 0-byte overwrite this guard exists for. Whatever scoping the
/// two tests above force, this must survive.
#[test]
fn save_as_onto_the_refused_path_itself_stays_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.notes"), b"host\n").expect("host file");
    let stream = dir.path().join("a.notes:hidden");
    std::fs::write(&stream, b"bytes from a stream\n").expect("ADS write");
    assert_write_side_refuses(&stream, PathVerdict::StreamName);
    let (gateway, rx) = host(dir.path());
    gateway
        .send(Command::Open {
            path: stream.clone(),
        })
        .expect("queued");
    let refused = open_and_watch(&rx, &stream);
    assert_eq!(name(&refused), "LoadFailed", "premise: the gate refused it");

    gateway
        .send(Command::SaveAs {
            path: stream.clone(),
            text: "over the never-read bytes\n".to_string(),
            revision: 0,
        })
        .expect("queued");
    let ev = save_and_watch(&rx, &stream);
    println!("SAVE AS {} -> {}", stream.display(), describe(&ev));
    assert!(
        matches!(ev, Event::SaveFailed { .. }),
        "Save As onto the refused name must not land; answered {}",
        describe(&ev)
    );
    drop(gateway);
}
