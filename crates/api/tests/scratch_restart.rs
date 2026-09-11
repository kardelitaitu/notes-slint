//! THE UNTITLED NOTE'S IDENTITY ACROSS A RESTART (D69, the restart half).
//!
//! first_run.rs already proves the WRITE half: type, flush, quit, and the bytes
//! land in <StateDir>/notes/untitled.notes, and the session names that path so
//! the next launch can reopen it. What this file drives is the part a restart
//! actually exercises — relaunch OPENS the scratch, and a fixed path that is
//! opened has all the hazards a path can have:
//!
//! * a SECOND and THIRD launch must not duplicate the text into itself;
//! * the scratch is scratch: the user may DELETE it between runs, and the
//!   answer must be a fresh empty scratch, not an error banner;
//! * the scratch file is OURS, so it must not need the foreign-file arming
//!   rule to be worked around, and the check is asserted, not assumed;
//! * Save As rebinds the identity, and the abandoned scratch must not return as
//!   a ghost on the next fresh window.
//!
//! Everything goes through Gateway::send and the EventRx, plus the bytes on
//! disk: no engine internals, no window, no toolkit.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use notes_api::{Command, Event, Gateway, Settings as ApiSettings, StateDir, WindowHandle};
use notes_core::recent::identity_key;
use notes_core::session::read_session;

// notes-platform and thiserror are deps of notes-api but of no target in this
// file; naming them keeps the unused_crate_dependencies lint honest.
use notes_platform as _;
use thiserror as _;

/// Long enough that a real disk round trip cannot flake; short enough that a
/// hung engine fails the run instead of hanging it.
const ANSWER: Duration = Duration::from_secs(5);

/// The engine's cadence: the session write rides this tick.
const TICK: Duration = Duration::from_millis(750);

fn scratch_of(dir: &Path) -> PathBuf {
    notes_core::paths::scratch_note_path(&StateDir(dir.to_path_buf()))
}

/// The state a bridge leaves the engine in before anything is asked of it
/// (whitepaper 5.5 step 3).
fn open_window(gateway: &Gateway) {
    let _ = gateway.send(Command::RegisterWindow {
        handle: WindowHandle(0x100),
    });
}

fn headless(dir: &Path) -> (Gateway, Receiver<Event>) {
    Gateway::start_with_host(
        StateDir(dir.to_path_buf()),
        ApiSettings::default(),
        None,
        None,
    )
}

/// Waits for the session.json write to catch up with the events: the identity
/// is only durable once the FILE says so, and the write is behind a tick.
fn wait_for_session_path(dir: &Path, want: &Path) {
    let want_key = identity_key(want);
    let deadline = Instant::now() + ANSWER + TICK;
    loop {
        if let Ok(session) = read_session(dir) {
            if session
                .path
                .as_deref()
                .is_some_and(|p| identity_key(p) == want_key)
            {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "session.json never named {want:?} (a restart has nothing to restore from)"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Drives the startup restore the way the bridge does it: read the session, ask
/// the port for the path it names, and hand back the answer event.
fn restore(rx: &Receiver<Event>, gateway: &Gateway, path: &Path) -> Event {
    open_window(gateway);
    gateway
        .send(Command::Open {
            path: path.to_path_buf(),
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        let event = rx
            .recv_timeout(ANSWER)
            .expect("the restore owed an event and never sent one");
        // RecentsUpdated is the startup announce; it is not the answer.
        if matches!(event, Event::RecentsUpdated(_)) {
            continue;
        }
        assert!(
            Instant::now() < deadline,
            "no answer to the restore Open of {path:?}"
        );
        return event;
    }
}

// ------------------------------------------------------------------ test 1 --

/// The rebind case, three launches deep: the scratch is ONE fixed path, so a
/// relaunch that opens it and writes again must REPLACE the draft, never append
/// to it, and must never invent a sibling. The text a launch reads is the text
/// the launch before it wrote, and the text on disk is exactly what was last
/// typed.
#[test]
fn three_launches_of_one_untitled_note_never_duplicate_its_text() {
    let dir = tempfile::tempdir().expect("tempdir");
    let scratch = scratch_of(dir.path());

    // Launch 1: type, flush, quit.
    let (first, rx) = headless(dir.path());
    open_window(&first);
    first
        .send(Command::Flush {
            text: "launch one\n".to_string(),
            revision: 1,
            epoch: 0,
        })
        .expect("queued");
    let mut saved = None;
    let deadline = Instant::now() + ANSWER;
    while saved.is_none() {
        assert!(Instant::now() < deadline, "the scratch write never landed");
        if let Event::Saved { path, .. } = rx.recv_timeout(ANSWER).expect("an event") {
            assert_eq!(path, scratch);
            saved = Some(());
        }
    }
    first.close().expect("quit joins");
    wait_for_session_path(dir.path(), &scratch);

    // Launch 2: the restore OPENS the scratch, the user types, the flush writes.
    let (second, rx) = headless(dir.path());
    let event = restore(&rx, &second, &scratch);
    match &event {
        Event::Loaded { path, text, .. } => {
            assert_eq!(path, &scratch, "the restored document IS the scratch");
            assert_eq!(
                text, "launch one\n",
                "the restore hands back the draft, not a copy of it"
            );
        }
        other => panic!("the restore must answer Loaded, got {other:?}"),
    }
    // The engine bumped its open generation for that Open, so a bridge that is
    // faithful about the epoch sends 1 here (the bridge lane is stamping this).
    second
        .send(Command::Flush {
            text: "launch two\n".to_string(),
            revision: 2,
            epoch: 1,
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        assert!(Instant::now() < deadline, "launch two never saved");
        match rx.recv_timeout(ANSWER).expect("an event") {
            Event::Saved { path, .. } if path == scratch => break,
            Event::AutosaveSkipped { reason } => {
                panic!("launch two's flush was skipped: {reason:?}")
            }
            _ => {}
        }
    }
    second.close().expect("quit joins");

    // Launch 3: the text is what launch two typed — once.
    let (third, rx) = headless(dir.path());
    match restore(&rx, &third, &scratch) {
        Event::Loaded { text, .. } => assert_eq!(
            text, "launch two\n",
            "THREE launches of one fixed scratch must not accumulate"
        ),
        other => panic!("the third restore must answer Loaded, got {other:?}"),
    }
    assert_eq!(
        fs::read(&scratch).expect("read the scratch"),
        b"launch two\n"
    );
    let names: Vec<String> = fs::read_dir(scratch.parent().expect("notes dir"))
        .expect("list notes")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["untitled.notes".to_string()], "one scratch");
    third.close().expect("third quit joins");
}

// ------------------------------------------------------------------ test 2 --

/// The scratch is SCRATCH: a user may delete the file (or the whole notes dir)
/// between runs. The relaunch names it, so the relaunch opens it — and the
/// answer must be a FRESH EMPTY SCRATCH, not an error banner, and not a poisoned
/// Save As. A missing scratch is not a missing document: there is nothing to
/// lose, and the app owns the file.
#[test]
fn a_scratch_deleted_between_launches_comes_back_as_a_fresh_empty_scratch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let scratch = scratch_of(dir.path());

    let (first, rx) = headless(dir.path());
    open_window(&first);
    first
        .send(Command::Flush {
            text: "a draft that gets deleted\n".to_string(),
            revision: 1,
            epoch: 0,
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        assert!(Instant::now() < deadline, "the scratch write never landed");
        if let Event::Saved { path, .. } = rx.recv_timeout(ANSWER).expect("an event") {
            assert_eq!(path, scratch);
            break;
        }
    }
    first.close().expect("quit joins");
    wait_for_session_path(dir.path(), &scratch);

    // The user cleans house: the scratch file AND its directory go.
    fs::remove_dir_all(scratch.parent().expect("notes dir")).expect("delete notes/");
    assert!(!scratch.exists());

    let (second, rx) = headless(dir.path());
    assert_eq!(
        identity_key(
            read_session(dir.path())
                .expect("session readable")
                .path
                .as_deref()
                .expect("the session names the scratch")
        ),
        identity_key(&scratch),
        "the session still names the deleted scratch"
    );
    let event = restore(&rx, &second, &scratch);
    match &event {
        Event::Loaded { path, text, meta } => {
            assert_eq!(path, &scratch, "still the scratch: same identity");
            assert_eq!(text, "", "a deleted scratch is a FRESH empty scratch");
            assert!(
                meta.armed,
                "the scratch is OURS (.notes), so autosave must be armed without 
                 any arming workaround: {:?}",
                meta
            );
            assert!(!meta.read_only, "we just found our own file missing");
            assert!(!meta.oversize);
        }
        other => panic!(
            "a missing scratch must answer Loaded (fresh + empty), got {other:?} — 
             LoadFailed is an error banner for a document nobody can have lost"
        ),
    }

    // And the note still WORKS from there: typing autosaves into a scratch that
    // is recreated, and Save As is not refused by a load that never "failed".
    second
        .send(Command::Flush {
            text: "typed after the deletion\n".to_string(),
            revision: 1,
            epoch: 1,
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        assert!(
            Instant::now() < deadline,
            "typing into the fresh scratch never saved"
        );
        match rx.recv_timeout(ANSWER).expect("an event") {
            Event::Saved { path, .. } if path == scratch => break,
            Event::AutosaveSkipped { reason } => {
                panic!("the fresh scratch refused to autosave: {reason:?}")
            }
            _ => {}
        }
    }
    assert_eq!(
        fs::read(&scratch).expect("the scratch was recreated"),
        b"typed after the deletion\n",
        "the whole file is the new text"
    );

    let named = dir.path().join("rescued.notes");
    second
        .send(Command::SaveAs {
            path: named.clone(),
            text: "typed after the deletion\n".to_string(),
            revision: 2,
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        assert!(
            Instant::now() < deadline,
            "Save As after a deleted scratch never answered"
        );
        match rx.recv_timeout(ANSWER).expect("an event") {
            Event::Saved { path, .. } if path == named => break,
            Event::SaveFailed { reason, .. } => panic!(
                "Save As refused after a missing scratch: {reason:?} — the port 
                 mistook a scratch that never existed for a refused load",
            ),
            _ => {}
        }
    }
    second.close().expect("quit joins");
}

// ------------------------------------------------------------------ test 3 --

/// The guard must not swallow every missing file: a note the USER named that
/// went away is still a load failure, still says why, and still refuses the
/// blind overwrite. Only the scratch is exempt.
#[test]
fn a_deleted_note_that_is_not_the_scratch_still_reports_load_failed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let gone = dir.path().join("was-here.notes");
    let (gateway, rx) = headless(dir.path());
    open_window(&gateway);
    gateway
        .send(Command::Open { path: gone.clone() })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        assert!(Instant::now() < deadline, "a missing file answered nothing");
        match rx.recv_timeout(ANSWER).expect("an event") {
            Event::LoadFailed { path, reason } => {
                assert_eq!(path, gone);
                assert_eq!(
                    format!("{reason:?}"),
                    "NotFound",
                    "a vanished document is still a vanished document"
                );
                break;
            }
            Event::Loaded { path, .. } => {
                panic!("a deleted non-scratch note answered Loaded for {path:?}")
            }
            _ => {}
        }
    }
    gateway.close().ok();
}

// ------------------------------------------------------------------ test 4 --

/// THE SPLIT, proven the way it matters: type into an untitled note, quit,
/// relaunch, and the text is back WHILE the recents list stays empty. Identity
/// for restore, absence from the list - session.path names the scratch (that is
/// what makes the restart work) and settings.toml holds no row for it (that is
/// what keeps a real note from being pushed out of a ten-slot menu).
///
/// This replaces the assertion this file used to carry - "the scratch joins the
/// recents once" - which pinned D69 as first decided. The reversal is named in
/// Engine::remember, in tests/session.rs and in first_run.rs case 9, so a future
/// reader sees a decision that moved rather than a test that was annoying.
#[test]
fn the_scratch_is_a_restore_target_and_never_a_recent_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let scratch = scratch_of(dir.path());
    let (first, rx) = headless(dir.path());
    open_window(&first);
    first
        .send(Command::Flush {
            text: "a draft, not a choice\n".to_string(),
            revision: 1,
            epoch: 0,
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        assert!(Instant::now() < deadline, "the scratch write never landed");
        match rx.recv_timeout(ANSWER).expect("an event") {
            Event::Saved { path, .. } if path == scratch => break,
            Event::RecentsUpdated(list) => panic!("the menu blinked for the scratch: {list:?}"),
            _ => {}
        }
    }
    first.close().expect("quit joins");
    // The session DID bind the scratch: that half of the split must survive.
    wait_for_session_path(dir.path(), &scratch);

    let (second, rx) = headless(dir.path());
    match restore(&rx, &second, &scratch) {
        Event::Loaded { path, text, .. } => {
            assert_eq!(path, scratch, "restored INTO the scratch");
            assert_eq!(text, "a draft, not a choice\n", "and the text is back");
        }
        other => panic!("the restore must answer Loaded, got {other:?}"),
    }
    // Two ticks of the engine, and the list stays as empty as the user left it.
    let until = Instant::now() + TICK * 2 + Duration::from_millis(200);
    while Instant::now() < until {
        if let Ok(Event::RecentsUpdated(list)) = rx.recv_timeout(Duration::from_millis(50)) {
            assert!(
                list.is_empty(),
                "a relaunch that opened the scratch wrote {list:?} into the menu"
            );
        } else {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let stored = notes_core::settings::read_settings(&StateDir(dir.path().to_path_buf()))
        .expect("settings readable")
        .unwrap_or_default();
    assert!(
        stored.recents.iter().all(|e| e.path != scratch),
        "settings.toml must hold no scratch row: {:?}",
        stored.recents
    );
    second.close().expect("second quit joins");
}

/// The mixed case, which is where the old behaviour cost something: a scratch
/// draft plus TWO files the user actually opened. The list is exactly the two,
/// in most-recent-first order, and the scratch is nowhere in it - so the draft
/// cannot take the slot a real note earned.
#[test]
fn a_scratch_draft_plus_two_opened_files_leaves_exactly_two_recents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let scratch = scratch_of(dir.path());
    let one = dir.path().join("first-real.notes");
    let two = dir.path().join("second-real.notes");
    fs::write(&one, b"the first real note\n").expect("seed one");
    fs::write(&two, b"the second real note\n").expect("seed two");

    let (gateway, rx) = headless(dir.path());
    open_window(&gateway);
    gateway
        .send(Command::Flush {
            text: "typed with no title\n".to_string(),
            revision: 1,
            epoch: 0,
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        assert!(Instant::now() < deadline, "the scratch write never landed");
        if let Event::Saved { path, .. } = rx.recv_timeout(ANSWER).expect("an event") {
            assert_eq!(path, scratch);
            break;
        }
    }

    // Each Open bumps the engine's generation, and the flush that follows carries
    // the stamp the bridge would be echoing at that moment.
    let mut list: Vec<notes_api::RecentEntry> = Vec::new();
    for (path, revision, epoch) in [(&one, 2u64, 1u64), (&two, 3u64, 2u64)] {
        gateway
            .send(Command::Open { path: path.clone() })
            .expect("queued");
        let deadline = Instant::now() + ANSWER;
        loop {
            assert!(Instant::now() < deadline, "open of {path:?} never answered");
            if let Event::Loaded { path: loaded, .. } = rx.recv_timeout(ANSWER).expect("an event") {
                assert_eq!(&loaded, path);
                break;
            }
        }
        // A real file, opened by the user, then edited: it earns the top slot.
        gateway
            .send(Command::Flush {
                text: "edited in place\n".to_string(),
                revision,
                epoch,
            })
            .expect("queued");
        let deadline = Instant::now() + ANSWER;
        loop {
            assert!(Instant::now() < deadline, "the edit never saved");
            match rx.recv_timeout(ANSWER).expect("an event") {
                Event::Saved { path: saved, .. } if &saved == path => break,
                // The list the Open of this file pushed, still in the channel:
                // caught here rather than waited for after the fact.
                Event::RecentsUpdated(entries) => list = entries,
                _ => {}
            }
        }
    }

    let paths: Vec<PathBuf> = list.iter().map(|e| e.path.clone()).collect();
    assert_eq!(
        paths,
        vec![two.clone(), one.clone()],
        "exactly the two files the user chose, most recent first"
    );
    assert!(
        list.iter()
            .all(|e| identity_key(&e.path) != identity_key(&scratch)),
        "the scratch took no slot: {list:?}"
    );
    gateway.close().expect("quit joins");

    // And the restart promise is untouched by the exclusion: the session named
    // the LAST file the user opened, not the scratch.
    wait_for_session_path(dir.path(), &two);
    let (relaunch, rx) = headless(dir.path());
    match restore(&rx, &relaunch, &two) {
        Event::Loaded { path, text, .. } => {
            assert_eq!(path, two);
            assert_eq!(text, "edited in place\n");
        }
        other => panic!("the relaunch must load the real note, got {other:?}"),
    }
    relaunch.close().expect("second quit joins");
}

// ------------------------------------------------------------------ test 6 --

/// THE EPOCH LOCKSTEP, seen from the api side, and the reason the untitled
/// case is the dangerous one. The bridge mirrors the engine open generation at
/// the SEND of an Open or a Save As and bumps NOTHING on a pathless startup, so
/// a fresh scratch sits at generation 0 on both sides. The flush guard compares
/// those counters for EQUALITY, which means the scratch binding inside Flush
/// must not bump the engine either: no rebind command was sent to get there, so
/// a bump would put the engine one generation ahead of the edits already in the
/// debounce and silently discard the autosave of the one document this product
/// always has. Two flushes, both at generation 0: the first proves the scratch
/// is reachable at all, the second proves the BIND left the counter alone.
#[test]
fn the_scratch_bind_does_not_bump_the_epoch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let scratch = scratch_of(dir.path());
    let (gateway, rx) = headless(dir.path());
    open_window(&gateway);

    for (revision, text) in [(1u64, "first draft\n"), (2u64, "second draft\n")] {
        gateway
            .send(Command::Flush {
                text: text.to_string(),
                revision,
                // Generation 0: the pathless startup, mirrored by a bridge
                // counter that never moved either.
                epoch: 0,
            })
            .expect("queued");
        let deadline = Instant::now() + ANSWER;
        loop {
            assert!(Instant::now() < deadline, "flush {revision} never answered");
            match rx.recv_timeout(ANSWER).expect("an event") {
                Event::Saved { path, .. } if path == scratch => break,
                Event::AutosaveSkipped { reason } => panic!(
                    "flush {revision} at generation 0 was skipped as {reason:?}: the scratch bind 
                     moved the engine generation without the bridge"
                ),
                _ => {}
            }
        }
    }
    assert_eq!(
        fs::read(&scratch).expect("read the scratch"),
        b"second draft\n",
        "both flushes landed in the same scratch, in order"
    );
    gateway.close().expect("quit joins");
}

/// The other side of the same rule: a restore that OPENS the scratch moves the
/// engine counter exactly once, at the top of open, and a flush still stamped
/// with the pre-Open generation is discarded and says so. Test 1 flushes at
/// epoch 1 after a restore because that is what a mirrored bridge sends; this
/// pins the guard itself, so an equality test that quietly stops comparing
/// anything cannot come back as a green suite.
#[test]
fn a_flush_stamped_before_the_restore_open_is_discarded_not_written() {
    let dir = tempfile::tempdir().expect("tempdir");
    let scratch = scratch_of(dir.path());
    let (first, rx) = headless(dir.path());
    open_window(&first);
    first
        .send(Command::Flush {
            text: "the restored draft\n".to_string(),
            revision: 1,
            epoch: 0,
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        assert!(Instant::now() < deadline, "the scratch write never landed");
        if let Event::Saved { .. } = rx.recv_timeout(ANSWER).expect("an event") {
            break;
        }
    }
    first.close().expect("quit joins");
    wait_for_session_path(dir.path(), &scratch);

    let (second, rx) = headless(dir.path());
    assert!(matches!(
        restore(&rx, &second, &scratch),
        Event::Loaded { .. }
    ));
    second
        .send(Command::Flush {
            text: "an edit from the previous generation\n".to_string(),
            revision: 2,
            epoch: 0,
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        assert!(
            Instant::now() < deadline,
            "the stale flush answered nothing"
        );
        match rx.recv_timeout(ANSWER).expect("an event") {
            Event::AutosaveSkipped { reason } => {
                assert_eq!(
                    format!("{reason:?}"),
                    "Superseded",
                    "a flush from a replaced generation is discarded, honestly named"
                );
                break;
            }
            Event::Saved { .. } => panic!(
                "a stale flush WROTE the scratch: the equality guard stopped comparing the two 
                 generations"
            ),
            _ => {}
        }
    }
    assert_eq!(
        fs::read(&scratch).expect("read the scratch"),
        b"the restored draft\n",
        "and the file is untouched by the discarded edit"
    );
    second.close().expect("second quit joins");
}

// ------------------------------------------------------------------ test 5 --

/// Save As is the rebind: the session must name the NEW path, the new file must
/// hold the text, and the scratch must not come back as a ghost. The abandoned
/// draft stays on disk (accepted by a_note_saved_away_stops_using_the_scratch_
/// and_the_draft_is_kept) — what this pins is that it can never RETURN: the
/// session, and therefore every later fresh window, points at the named file.
#[test]
fn save_as_rebinds_the_identity_and_the_scratch_ghost_never_returns() {
    let dir = tempfile::tempdir().expect("tempdir");
    let scratch = scratch_of(dir.path());
    let named = dir.path().join("a-real-name.notes");

    let (first, rx) = headless(dir.path());
    open_window(&first);
    first
        .send(Command::Flush {
            text: "draft in the scratch\n".to_string(),
            revision: 1,
            epoch: 0,
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        assert!(Instant::now() < deadline, "the scratch write never landed");
        if let Event::Rebound { path, .. } = rx.recv_timeout(ANSWER).expect("an event") {
            assert_eq!(path, scratch);
            break;
        }
    }
    first
        .send(Command::SaveAs {
            path: named.clone(),
            text: "draft in the scratch\n".to_string(),
            revision: 2,
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        assert!(Instant::now() < deadline, "Save As never answered");
        if let Event::Saved { path, .. } = rx.recv_timeout(ANSWER).expect("an event") {
            assert_eq!(path, named, "Save As wrote the named file");
            break;
        }
    }
    first.close().expect("quit joins");
    wait_for_session_path(dir.path(), &named);
    let ghost = fs::read(&scratch).expect("the abandoned draft");

    // Relaunch: the session names the NAMED file, and a new launch writes there.
    let (second, rx) = headless(dir.path());
    let session = read_session(dir.path()).expect("session readable");
    assert_eq!(
        identity_key(session.path.as_deref().expect("a named path")),
        identity_key(&named),
        "after Save As the session must name the NEW path, not the scratch"
    );
    match restore(&rx, &second, &named) {
        Event::Loaded { path, text, meta } => {
            assert_eq!(path, named);
            assert_eq!(text, "draft in the scratch\n");
            assert!(meta.armed, "a .notes the app saved is armed");
        }
        other => panic!("the relaunch must load the named file, got {other:?}"),
    }
    second
        .send(Command::Flush {
            text: "edited after the rename\n".to_string(),
            revision: 2,
            epoch: 1,
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        assert!(Instant::now() < deadline, "the named file never autosaved");
        match rx.recv_timeout(ANSWER).expect("an event") {
            Event::Saved { path, .. } if path == named => break,
            Event::Saved { path, .. } => panic!(
                "the edit landed in {path:?} — after a Save As the scratch must 
                 never be written again"
            ),
            Event::AutosaveSkipped { reason } => {
                panic!("the renamed note refused to autosave: {reason:?}")
            }
            _ => {}
        }
    }
    second.close().expect("second quit joins");

    assert_eq!(
        fs::read(&named).expect("read the named file"),
        b"edited after the rename\n"
    );
    assert_eq!(
        fs::read(&scratch).expect("the scratch"),
        ghost,
        "the ghost stays frozen: it is a draft, not a second copy, and the 
         session no longer points at it"
    );
    let session = read_session(dir.path()).expect("session readable");
    assert_eq!(
        identity_key(session.path.as_deref().expect("a named path")),
        identity_key(&named),
        "the ghost must not creep back into the session"
    );
}
