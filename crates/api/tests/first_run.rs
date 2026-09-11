//! THE FIRST-RUN STORY, END TO END, FROM OUTSIDE THE PORT (M2 exit).
//!
//! Every test here drives the real Gateway against a FRESH private temp
//! state dir - no session.json, no settings.toml, no window - and reads the
//! verdict from EVENTS and from the FILES, the way a first-run user experiences
//! the app. Nothing in here may touch a src file: a defect found by this file
//! is marked #[ignore] with the finding in the name, the way the core hazard
//! file turned five data-loss classes into a to-do list that cannot be lost.
//!
//! THE LOSS WINDOW, stated once and honestly: the engine writes text when the
//! Flush COMMAND is processed (sub-tick), so text the bridge has already
//! Flushed survives even a hard drop with no close(); text the bridge has NOT
//! yet Flushed is lost entirely, and the 750 ms tick bounds the SESSION write
//! (and a crashed engine's last session state), not text loss.

use std::path::Path;
use std::time::{Duration, Instant};

use notes_core::recent::RecentEntry;
use notes_core::session::read_session;
use notes_core::settings::{Settings, write_settings};

use notes_api::{Command, Event, Gateway, Settings as ApiSettings, StateDir, WindowHandle};

// thiserror is a dependency of notes-api, not of this target; naming it keeps
// the unused-crate-dependencies lint honest.
use notes_platform as _;
use thiserror as _;

/// The one setup step every scenario shares: the window exists before anything
/// is asked of it (whitepaper 5.5 step 3) - the state a bridge leaves the
/// engine in. A first-run user types INSIDE a window.
fn open_window(gateway: &Gateway) {
    let _ = gateway.send(Command::RegisterWindow {
        handle: WindowHandle(0x100),
    });
}

/// Long enough that a loaded machine cannot flake; short enough that a hung
/// engine fails the run instead of hanging it forever.
const ANSWER: Duration = Duration::from_secs(5);

/// The engine's one and only cadence (AUTOSAVE_IDLE): the tick that writes
/// session.json.
const TICK: Duration = Duration::from_millis(750);

fn scratch_of(dir: &Path) -> std::path::PathBuf {
    notes_core::paths::scratch_note_path(&StateDir(dir.to_path_buf()))
}

fn expect_event(rx: &std::sync::mpsc::Receiver<Event>, deadline: Instant) -> Event {
    rx.recv_timeout(ANSWER.max(deadline.saturating_duration_since(Instant::now())))
        .expect("the engine owed an event and never sent one")
}

/// Polls until probe returns Some, bounded by ANSWER.
fn poll_until<T>(probe: impl Fn() -> Option<T>, what: &str) -> T {
    let deadline = Instant::now() + ANSWER;
    loop {
        if let Some(v) = probe() {
            return v;
        }
        assert!(Instant::now() < deadline, "never observed: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ---------------------------------------------------------------- case 1 --

/// M2 exit item 6, and the reason D69 exists: open the app, type, close,
/// reopen, and the text is there. Also pins the bind order (Saved strictly
/// before Rebound) on the way through.
#[test]
fn type_idle_quit_relaunch_and_the_text_comes_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let scratch = scratch_of(dir.path());

    let (first, rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        ApiSettings::default(),
        None,
        None,
    );
    open_window(&first);
    first
        .send(Command::Flush {
            text: "the morning thought\n".to_string(),
            revision: 1,
            epoch: 0,
        })
        .expect("queued");
    let mut saved_at = None;
    let mut rebound_at = None;
    let mut index = 0usize;
    let deadline = Instant::now() + ANSWER;
    while saved_at.is_none() || rebound_at.is_none() {
        assert!(
            Instant::now() < deadline,
            "Saved/Rebound never both arrived"
        );
        match expect_event(&rx, deadline) {
            Event::Saved { path, .. } if path == scratch => saved_at = Some(index),
            Event::Rebound { path, .. } if path == scratch => rebound_at = Some(index),
            _ => {}
        }
        index += 1;
    }
    assert!(
        saved_at.expect("saved") < rebound_at.expect("rebound"),
        "Saved announces the bytes BEFORE Rebound announces the new identity"
    );
    first.close().expect("a clean quit joins the engine");

    assert_eq!(
        std::fs::read(&scratch).expect("the scratch survives between runs"),
        b"the morning thought\n"
    );

    // Relaunch against the SAME state dir. Startup order 5.5: the engine
    // exposes InitialState (the session naming the scratch) and the BRIDGE
    // issues the Open - headless, this test plays the bridge's part.
    let (mut second, rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        ApiSettings::default(),
        None,
        None,
    );
    assert_eq!(
        second.startup_state().expect("startup state").session.path,
        Some(scratch.clone()),
        "the relaunch is pointed at the scratch note"
    );
    second
        .send(Command::Open {
            path: scratch.clone(),
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        match expect_event(&rx, deadline) {
            Event::Loaded { path, text, .. } if path == scratch => {
                assert_eq!(text, "the morning thought\n", "the text comes back");
                break;
            }
            _ => {}
        }
    }
    second.close().expect("second quit joins");
}

// ---------------------------------------------------------------- case 2 --

/// THE LOSS WINDOW, with no close() at all: a hard drop. Text already Flushed
/// is on disk before the drop (the Flush command writes when it is processed,
/// sub-tick), so it survives; text the bridge has not Flushed was never seen
/// by the engine and is lost ENTIRELY - the 750 ms tick bounds the session
/// write, not text loss. If this ever starts failing, the write moved off the
/// command path and product.md:50's promise needs re-measuring.
#[test]
fn dropping_without_close_flushed_text_survives_but_unflushed_text_since_the_last_750ms_tick_is_gone()
 {
    let dir = tempfile::tempdir().expect("tempdir");
    let scratch = scratch_of(dir.path());
    let (gateway, _rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        ApiSettings::default(),
        None,
        None,
    );
    open_window(&gateway);
    gateway
        .send(Command::Flush {
            text: "kept\n".to_string(),
            revision: 1,
            epoch: 0,
        })
        .expect("queued");
    poll_until(
        || std::fs::read(&scratch).ok().filter(|b| b == b"kept\n"),
        "the flushed write to land",
    );
    // And the session side catches up within one tick of processing: the
    // file names the scratch, so even a hard-kill recovery relaunch reopens
    // it. (This must be observed BEFORE the drop - after it, the engine is
    // gone and nothing further lands.)
    poll_until(
        || {
            read_session(dir.path())
                .ok()
                .filter(|s| s.path == Some(scratch.clone()))
        },
        "the session write (bounded by the tick) to land",
    );

    // THE HARD DROP: no close(), no drain - the process image just ends.
    drop(gateway);

    assert_eq!(
        std::fs::read(&scratch).expect("flushed text survives the drop"),
        b"kept\n",
    );
}

// ---------------------------------------------------------------- case 3 --

/// The user turned autosave OFF in settings.toml. The scratch path must NOT
/// override that choice: no file, and the typed reason (AutosaveDisabled).
/// If the untitled path ever writes here, D69 is wrong as decided, and this
/// test is the finding.
#[test]
fn autosave_disabled_means_no_scratch_file_ever() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_settings(
        &StateDir(dir.path().to_path_buf()),
        &Settings {
            autosave_enabled: false,
            ..Settings::default()
        },
    )
    .expect("seed settings.toml");
    let (gateway, rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        ApiSettings::default(),
        None,
        None,
    );
    open_window(&gateway);
    gateway
        .send(Command::Flush {
            text: "do not save me\n".to_string(),
            revision: 1,
            epoch: 0,
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        match expect_event(&rx, deadline) {
            Event::AutosaveSkipped { reason } => {
                assert_eq!(
                    format!("{reason:?}"),
                    "AutosaveDisabled",
                    "the honest reason, not a path complaint"
                );
                break;
            }
            Event::Saved { .. } | Event::Rebound { .. } => {
                panic!("D69 override: the scratch was written with autosave DISABLED");
            }
            _ => {}
        }
    }
    std::thread::sleep(TICK + Duration::from_millis(200));
    assert!(
        !scratch_of(dir.path()).exists(),
        "the scratch must not exist while autosave is disabled"
    );
    gateway.close().expect("shutdown joins");
}

// ---------------------------------------------------------------- case 4 --

/// Save As gives the note a name; the relaunch must open THE NAMED FILE, not
/// the scratch. If it opens the scratch, that is a bug in the D69 decision and
/// this test says so in the assertion.
#[test]
fn save_as_from_scratch_then_relaunch_opens_the_named_file_not_the_scratch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let named = dir.path().join("real.notes");
    let (gateway, rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        ApiSettings::default(),
        None,
        None,
    );
    open_window(&gateway);
    gateway
        .send(Command::SaveAs {
            path: named.clone(),
            text: "the named note\n".to_string(),
            revision: 1,
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        match expect_event(&rx, deadline) {
            Event::Saved { path, .. } if path == named => break,
            _ => {}
        }
    }
    gateway.close().expect("quit joins");

    // Relaunch: WHICH file opens? The session must name the NAMED file - the
    // scratch is abandoned the moment the note has a real name.
    let (mut second, rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        ApiSettings::default(),
        None,
        None,
    );
    assert_eq!(
        second.startup_state().expect("startup state").session.path,
        Some(named.clone()),
        "the relaunch opens the NAMED file; if this were the scratch, D69's\n         save-as-rebinds decision would be wrong"
    );
    second
        .send(Command::Open {
            path: named.clone(),
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        match expect_event(&rx, deadline) {
            Event::Loaded { path, .. } if path == named => break,
            _ => {}
        }
    }
    second.close().expect("second quit joins");
}

// ---------------------------------------------------------------- case 5 --

/// A DIRECTORY sits on session.json (this happened on a real machine). The
/// startup must say so (StateDirUnusable), the scratch note must still land
/// (the note is not the session), and the session write failure must be
/// reported ONCE, latched, not once per tick.
#[test]
fn a_directory_on_session_json_is_reported_and_the_scratch_still_lands() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join("session.json")).expect("plant the directory");
    let (mut gateway, rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        ApiSettings::default(),
        None,
        None,
    );
    // Startup DOES NOT report this one: a corrupt session.json defaults
    // silently (InitialState's contract - startup never fails on that file),
    // and the report arrives when the write cannot land.
    assert_eq!(
        gateway.startup_state().expect("startup state").session.path,
        None,
        "the corrupt session defaults to no path, startup does not fail"
    );
    open_window(&gateway);
    gateway
        .send(Command::Flush {
            text: "typed anyway\n".to_string(),
            revision: 1,
            epoch: 0,
        })
        .expect("queued");
    let scratch = scratch_of(dir.path());
    let deadline = Instant::now() + ANSWER;
    loop {
        match expect_event(&rx, deadline) {
            Event::Saved { path, .. } if path == scratch => break,
            _ => {}
        }
    }
    assert_eq!(
        std::fs::read(&scratch).expect("the note survives the broken session"),
        b"typed anyway\n"
    );
    let deadline = Instant::now() + ANSWER;
    loop {
        if let Event::StateWriteFailed { .. } = expect_event(&rx, deadline) {
            break;
        }
    }
    std::thread::sleep(TICK + Duration::from_millis(200));
    let mut second_report = false;
    let until = Instant::now() + TICK + Duration::from_millis(200);
    while Instant::now() < until {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Event::StateWriteFailed { .. }) => {
                second_report = true;
                break;
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }
    assert!(
        !second_report,
        "the session-write failure must be latched, not re-reported per tick"
    );
    gateway.close().ok();
}

// ---------------------------------------------------------------- case 6 --

/// A FILE sits where the notes/ directory belongs: the scratch place cannot
/// be made, so the fallback skip fires and NOTHING is written.
#[test]
fn a_file_where_the_notes_dir_belongs_still_skips_and_writes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("notes"), b"not a directory").expect("plant the file");
    let (gateway, rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        ApiSettings::default(),
        None,
        None,
    );
    open_window(&gateway);
    gateway
        .send(Command::Flush {
            text: "no place\n".to_string(),
            revision: 1,
            epoch: 0,
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        if let Event::AutosaveSkipped { reason } = expect_event(&rx, deadline) {
            assert_eq!(format!("{reason:?}"), "NeedsPath");
            break;
        }
    }
    assert!(!scratch_of(dir.path()).exists(), "no scratch was written");
    gateway.close().expect("shutdown joins");
}

// ---------------------------------------------------------------- case 7 --

/// The scratch place is a DANGLING junction (mklink /J to a nonexistent
/// target, no elevation needed): the engine must refuse fast, with the typed
/// fallback, and must not stall on the reparse point (the 2.68 s freeze is
/// what this repo refuses for).
#[test]
fn a_dangling_junction_on_the_notes_dir_is_refused_fast_with_needs_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let notes = dir.path().join("notes");
    let out = std::process::Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            &notes.to_string_lossy(),
            "C:\\nonexistent-target-for-n2-test",
        ])
        .output()
        .expect("run mklink");
    assert!(
        out.status.success(),
        "mklink /J failed: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let (gateway, rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        ApiSettings::default(),
        None,
        None,
    );
    let started = Instant::now();
    gateway
        .send(Command::Flush {
            text: "behind a broken link\n".to_string(),
            revision: 1,
            epoch: 0,
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        if let Event::AutosaveSkipped { reason } = expect_event(&rx, deadline) {
            assert_eq!(format!("{reason:?}"), "NeedsPath");
            break;
        }
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the refusal took {:?} - too close to a network stall",
        started.elapsed()
    );
    assert!(!scratch_of(dir.path()).exists());
    gateway.close().expect("shutdown joins");
}

// ---------------------------------------------------------------- case 8 --

/// The scratch FILE exists but is write-DENIED via icacls (no elevation
/// needed): the typed outcome is the fallback skip, and the stale bytes on
/// disk are untouched - a denied write must never truncate a file.
#[test]
fn a_write_denied_scratch_still_skips_and_leaves_the_stale_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let scratch = scratch_of(dir.path());
    std::fs::create_dir_all(scratch.parent().expect("notes dir")).expect("notes dir");
    std::fs::write(&scratch, b"the stale draft\n").expect("seed the scratch");
    // Deny FILE CREATION on the notes directory (SID form - locale-proof),
    // so the scratch place cannot accept a write. A deny on the FILE itself
    // proved ineffective on an elevated process: the sibling-temp-plus-rename
    // write landed anyway, which is worth knowing and is why the denial moved
    // to the directory.
    let notes_dir = scratch
        .parent()
        .expect("notes dir")
        .to_string_lossy()
        .into_owned();
    let deny = std::process::Command::new("icacls")
        .args([&notes_dir, "/deny", "*S-1-1-0:(WD,AD)"])
        .output()
        .expect("run icacls");
    assert!(deny.status.success(), "icacls deny failed");
    let (gateway, rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        ApiSettings::default(),
        None,
        None,
    );
    open_window(&gateway);
    gateway
        .send(Command::Flush {
            text: "will not land\n".to_string(),
            revision: 1,
            epoch: 0,
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        if let Event::AutosaveSkipped { reason } = expect_event(&rx, deadline) {
            assert_eq!(format!("{reason:?}"), "NeedsPath");
            break;
        }
    }
    assert_eq!(
        std::fs::read(&scratch).expect("read the denied file"),
        b"the stale draft\n",
        "a denied write must leave the old bytes whole"
    );
    gateway.close().expect("shutdown joins");
    std::process::Command::new("icacls")
        .args([&notes_dir, "/remove:d", "*S-1-1-0"])
        .output()
        .expect("run icacls remove");
}

// ---------------------------------------------------------------- case 9 --

/// Nine recents plus the scratch (ten - the cap), then the scratch file is DELETED from
/// underneath and the app relaunches: features.md:89 says grey out, do not
/// silently delete - the entry must survive as stale and the app must not
/// crash or lose the other nine.
///
/// THE RECENTS SPLIT, and why this file still plants a scratch entry: the engine
/// stopped adding the scratch to the MRU at all (identity for restore, absence
/// from the list), so a scratch row can only be here because a pre-reversal
/// launch put it there. That is exactly the state this case is about: a stale
/// row must grey out, never be silently deleted, and never cost the other nine.
/// The engine writes nothing to the list here - the point is what a RELAUNCH does
/// with bytes it did not choose to write.
#[test]
fn ten_entries_plus_a_deleted_scratch_survive_the_relaunch_as_stale() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut recents: Vec<RecentEntry> = (0..9)
        .map(|i| RecentEntry {
            path: std::path::PathBuf::from(format!("C:/notes/n{i}.notes")),
            display: format!("n{i}.notes"),
            exists: false,
        })
        .collect();
    let scratch = scratch_of(dir.path());
    recents.insert(
        0,
        RecentEntry {
            path: scratch.clone(),
            display: "untitled.notes".to_string(),
            exists: true,
        },
    );
    write_settings(
        &StateDir(dir.path().to_path_buf()),
        &Settings {
            recents,
            ..Settings::default()
        },
    )
    .expect("seed ten-plus-one recents");
    // The recents entry claims the scratch exists: make that true, so the
    // deletion below deletes a REAL file.
    std::fs::create_dir_all(scratch.parent().expect("notes dir")).expect("notes dir");
    std::fs::write(&scratch, b"the draft to delete\n").expect("seed the scratch");

    let (gateway, rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        ApiSettings::default(),
        None,
        None,
    );
    let deadline = Instant::now() + ANSWER;
    loop {
        if let Event::RecentsUpdated(entries) = expect_event(&rx, deadline) {
            assert_eq!(entries.len(), 10, "the seeded list, at the ten cap");
            break;
        }
    }
    gateway.close().expect("quit joins");

    std::fs::remove_file(&scratch).expect("delete the scratch");
    let (second, rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        ApiSettings::default(),
        None,
        None,
    );
    let deadline = Instant::now() + ANSWER;
    loop {
        if let Event::RecentsUpdated(entries) = expect_event(&rx, deadline) {
            let stale = entries.iter().find(|e| e.path == scratch);
            assert!(
                stale.is_some(),
                "features.md:89: the deleted scratch must survive as stale, not vanish"
            );
            assert!(
                !stale.expect("the stale entry").exists,
                "stale means exists == false"
            );
            assert_eq!(entries.len(), 10, "and the other nine are still there");
            break;
        }
    }
    second.close().expect("second quit joins");
}

// --------------------------------------------------------------- case 10 --

/// The deterministic name: two launches, two untitled buffers, ONE scratch
/// file. No untitled-2.notes, no new name scheme.
#[test]
fn two_launches_reuse_one_scratch_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let scratch = scratch_of(dir.path());
    let (first, _rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        ApiSettings::default(),
        None,
        None,
    );
    open_window(&first);
    first
        .send(Command::Flush {
            text: "launch one\n".to_string(),
            revision: 1,
            epoch: 0,
        })
        .expect("queued");
    poll_until(
        || {
            std::fs::read(&scratch)
                .ok()
                .filter(|b| b == b"launch one\n")
        },
        "the first launch to write",
    );
    first.close().expect("quit joins");

    let (second, _rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        ApiSettings::default(),
        None,
        None,
    );
    second
        .send(Command::Flush {
            text: "launch two\n".to_string(),
            revision: 1,
            epoch: 0,
        })
        .expect("queued");
    poll_until(
        || {
            std::fs::read(&scratch)
                .ok()
                .filter(|b| b == b"launch two\n")
        },
        "the second launch to write",
    );
    second.close().expect("second quit joins");

    let names: Vec<String> = std::fs::read_dir(scratch.parent().expect("notes dir"))
        .expect("list notes")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        vec!["untitled.notes".to_string()],
        "exactly one deterministic scratch"
    );
}
