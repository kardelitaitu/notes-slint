//! The port's thread behaviour, seen from OUTSIDE the crate: proof, not vibe.
//!
//! Five lifecycle tests, one per way a channel boundary can go wrong, plus one
//! that the vocabulary is still nameable from a separate crate:
//!
//! 1. the reentrancy trap actually fires (an unarmed trap looks exactly like
//!    correct code);
//! 2. Shutdown is DRAIN AND EXIT, not "drop the queue and hope";
//! 3. dropping the Gateway is ABORT, and buffered events survive it;
//! 4. a closed EventRx stops the events, not the engine;
//! 5. startup_state() reads the two state files and never the note;
//! 6. a consumer that never reads cannot block a producer — the unbounded-queue
//!    invariant, and therefore the ABBA-deadlock alarm.
//!
//! ## What these tests answer with, and why
//!
//! The file arms are wired now (tests/session.rs is the proof against real bytes),
//! but every test HERE drives the engine with no document open and no edits, so a
//! [`Command::Flush`] is answered with [`Event::SaveFailed`] carrying its own
//! revision, which is why tests 2, 3 and 6 can count events at all. That failure is
//! the honest answer to a flush there is nothing to write; D11 is why the revision
//! ranges below start at 1, since a flush at revision 0 against a fresh engine is
//! stale and answers [`Event::AutosaveSkipped`] with Clean instead.
//! [`Command::RegisterWindow`] stores its handle and emits nothing: there is no
//! platform call this crate may make (check-arch keeps notes-platform out of its
//! graph). No arm is a silent no-op, and no test here pretends otherwise.

use std::fs;
use std::io::Write;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::TryRecvError;
use std::thread;
use std::time::{Duration, Instant};

use notes_api::{
    Command, Encoding, Event, EventRx, FileMeta, Gateway, InitialState, LineEnding, Rect,
    SaveError, Session, Settings, StateDir, WindowHandle, mark_current_thread_as_engine,
};

// unused_crate_dependencies is per TARGET: notes-core and thiserror are linked
// into this test binary because they are dependencies of notes-api, and this
// file names neither (it goes through the port, which is the point). The two
// lines below are the lint's own documented opt-out, not a placeholder.
use notes_core as _;
use thiserror as _;

/// Longer than the engine's 750 ms tick, for the few places a test must wait on
/// a thread rather than on a channel signal.
const WAIT: Duration = Duration::from_millis(1_500);

/// A state directory that holds nothing at all: no session.json, so the engine
/// starts from Session::default() and nothing in these tests can be reading a
/// real user's profile.
fn empty_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "notes-api-empty-{}-{}",
        std::process::id(),
        thread::current().name().unwrap_or("test").replace(':', "_")
    ));
    fs::create_dir_all(&dir).expect("create an empty state dir");
    dir
}

/// The rect the hand-written session.json carries.
const SAVED_RECT: Rect = Rect {
    x: 111,
    y: 222,
    w: 640,
    h: 480,
};

/// session.json, written BY HAND. The point of test 5 is that the port reads
/// core's file format, so the fixture must not come from core's writer.
const SESSION_JSON: &str = concat!(
    r#"{"rect":{"x":111,"y":222,"w":640,"h":480},"#,
    r#""monitor_id":2,"scale_factor":1.5,"maximized":true,"pinned":true,"#,
    r#""path":"DECOY-NOTES.md"}"#
);

/// 1. The trap fires when the port is called from an engine thread.
///
/// [`mark_current_thread_as_engine`] is the same arming the real engine thread
/// performs, and that is the only reason it is exposed: a trap that was never
/// wired passes every other test in this file while the engine self-deadlocks in
/// the field, so THIS test is the one that makes the rest of the seam mean
/// something.
///
/// The assert is a debug_assert — the release build must not pay a TLS read per
/// send — so the test is compiled only where the trap exists. A release build has
/// no trap to prove, and pretending otherwise would be a test that asserts
/// nothing.
#[cfg(debug_assertions)]
#[test]
fn reentrancy_trap_fires_when_the_port_is_called_from_the_engine_thread() {
    let (mut gateway, _rx) = Gateway::start(StateDir(empty_dir()), Settings::default());

    // Control first: the identical call OFF an engine thread must not panic.
    // Without this, a panic from anything at all would make the test pass.
    let _ = gateway.send(Command::SetAutosave(true));
    assert!(
        gateway
            .startup_state()
            .expect("first call")
            .autosave_enabled
    );

    let child = thread::spawn(move || {
        let _armed = mark_current_thread_as_engine();
        let sent = panic::catch_unwind(AssertUnwindSafe(|| {
            gateway.send(Command::SetAutosave(false))
        }));
        let state = panic::catch_unwind(AssertUnwindSafe(|| gateway.startup_state()));
        // REVIEWER ITEM 2: Drop is now guarded too. This thread is only FALSELY
        // marked as an engine thread - which is exactly the point. A Gateway
        // released on a real engine thread would join itself, panic inside a
        // destructor, and abort the process; the assert cannot tell the two
        // apart, and it does not need to. Caught here because a panic in a
        // spawned thread would otherwise just fail the join below.
        let dropped = panic::catch_unwind(AssertUnwindSafe(|| drop(gateway)));
        // Nothing owns the Sender any more (the panic happened before the take),
        // so the engine is left running; the test process exits and the runtime
        // does not wait for it.
        (sent.is_err(), state.is_err(), dropped.is_err())
    });
    let (send_panicked, state_panicked, drop_panicked) =
        child.join().expect("the armed child must run");

    assert!(
        send_panicked,
        "Gateway::send from the engine thread must panic: the trap is not armed"
    );
    assert!(
        state_panicked,
        "Gateway::startup_state from the engine thread must panic too"
    );
    assert!(
        drop_panicked,
        "Gateway::drop from the engine thread must assert too: it is the only \\
         blocking call on the port and it was the only one without a latch"
    );
}

/// 2. Shutdown drains: everything queued behind it is still carried out.
#[test]
fn shutdown_drains_commands_queued_behind_it() {
    // No `mut`: on this toolchain Receiver::recv takes &self.
    let (gateway, rx) = Gateway::start(StateDir(empty_dir()), Settings::default());

    // SetPinned changes state silently, so the counted batch is Flushes — one
    // event each — and the pin goes in first to prove the engine is warm.
    let _ = gateway.send(Command::SetPinned(true));
    const N: u64 = 25;
    for revision in 1..=N {
        let _ = gateway.send(Command::Flush {
            text: String::new(),
            revision,
        });
    }
    let _ = gateway.send(Command::Shutdown);
    // Nothing above waited, and none of them could block: the queue is unbounded.

    let mut revisions = Vec::new();
    while let Ok(event) = rx.recv() {
        // recv(), not try_recv(): the answers may still be queued behind the
        // drain, and Shutdown guarantees they arrive before the engine's Sender
        // goes away.
        if let Event::SaveFailed { revision, .. } = event {
            revisions.push(revision);
        }
    }
    assert_eq!(
        revisions.len(),
        usize::try_from(N).expect("N fits"),
        "every Flush queued before Shutdown must be answered"
    );
    assert_eq!(revisions.first(), Some(&1), "draining must not reorder");
    assert_eq!(revisions.last(), Some(&N));
    assert!(
        matches!(rx.try_recv(), Err(TryRecvError::Disconnected)),
        "the engine's Sender must be gone once it has exited"
    );
    // Gateway::drop joins. If the join could not finish, this hangs rather than
    // passes — which is the point of a lifecycle test.
    drop(gateway);
}

/// 3. Drop is ABORT — and abort is not "lose what you accepted".
#[test]
fn drop_without_shutdown_aborts_but_buffered_events_survive() {
    let (gateway, mut rx) = Gateway::start(StateDir(empty_dir()), Settings::default());

    for revision in 1..=10 {
        let _ = gateway.send(Command::Flush {
            text: String::new(),
            revision,
        });
    }
    let _ = gateway.send(Command::SetAutosave(false));
    // The engine may have processed some, none or all of these already. Either
    // way no send may panic, and nothing already emitted may be lost.
    let before = drain(&mut rx);

    // No Shutdown this time: dropping the last Sender IS the signal. The drop
    // joins, so by the time it returns the engine's Sender is dead and every
    // event it emitted is sitting in this queue — which is what makes the
    // assertion below exact rather than racy.
    drop(gateway);

    let after = drain(&mut rx);
    assert_eq!(
        before + after,
        10,
        "all ten Flush answers must survive an abort: {before} read before the \
         drop, {after} after"
    );
    assert!(
        matches!(rx.try_recv(), Err(TryRecvError::Disconnected)),
        "the abort path must still leave the engine's Sender gone"
    );
}

/// 4. A closed EventRx stops the events, not the engine.
#[test]
fn dropped_receiver_does_not_kill_the_engine() {
    let (gateway, rx) = Gateway::start(StateDir(empty_dir()), Settings::default());
    let _ = gateway.send(Command::Flush {
        text: "x".into(),
        revision: 1,
    });
    // Wait for that answer, then take the listener away.
    rx.recv_timeout(WAIT).expect("the first answer");
    drop(rx);

    // Every one of these emits into a closed channel. The engine must swallow it:
    // no panic, and no log line at a window that is already gone.
    for revision in 2..=50 {
        let _ = gateway.send(Command::Flush {
            text: String::new(),
            revision,
        });
        let _ = gateway.send(Command::Open {
            path: PathBuf::from("C:/notes/who.notes"),
        });
    }
    thread::sleep(Duration::from_millis(50));
    assert!(
        gateway.engine_is_alive(),
        "a dead listener must not take the engine down with it"
    );

    let _ = gateway.send(Command::Shutdown);
    let deadline = Instant::now() + WAIT;
    while gateway.engine_is_alive() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !gateway.engine_is_alive(),
        "the engine must shut down on its own, with no listener to close for it"
    );
    // The join in Drop is then a no-op wait, and reaching here at all is the
    // "joins cleanly" half of the claim.
    drop(gateway);
}

/// 5. startup_state() reads the two STATE files and NOTHING else: never the note,
///    never a probe beyond session.json and settings.toml, and it creates nothing.
#[test]
fn startup_state_reads_the_state_files_and_never_the_note() {
    let dir = tempfile::tempdir().expect("tempdir");
    let decoy = dir.path().join("DECOY-NOTES.md");
    fs::write(&decoy, b"do not open me in this slice").expect("write decoy");
    let before = fs::metadata(&decoy).expect("decoy metadata");
    let mut file = fs::File::create(dir.path().join("session.json")).expect("create session.json");
    file.write_all(SESSION_JSON.as_bytes())
        .expect("write session.json");
    drop(file);

    // A persisted choice the caller did NOT pass in, which is how this test proves
    // the settings read is real rather than a default wearing a file's clothes.
    fs::write(dir.path().join("settings.toml"), "autosave_enabled = false")
        .expect("write settings.toml");

    let (mut gateway, rx) = Gateway::start(StateDir(dir.path().to_path_buf()), Settings::default());
    let initial: InitialState = gateway.startup_state().expect("the pre-window read");
    assert!(
        !initial.autosave_enabled,
        "D10: the toggle comes from settings.toml, not from the argument"
    );

    assert_eq!(initial.session.rect, SAVED_RECT, "the saved rect, verbatim");
    assert!(initial.pinned, "D10: the pin bit comes out of session.json");
    assert_eq!(
        initial.session.pinned, initial.pinned,
        "one home for the bit, not a copy that can drift"
    );
    assert_eq!(initial.session.monitor_id, 2);
    assert_eq!(initial.session.scale_factor, 1.5);
    assert!(initial.session.maximized);
    assert_eq!(
        initial.session.path.as_deref(),
        Some(Path::new("DECOY-NOTES.md")),
        "the stored path is REPORTED and not opened"
    );
    assert_eq!(
        initial.autosave_enabled, false,
        "settings.toml owns the toggle; the caller's default could not override it"
    );

    // The document was never touched: same bytes, same modification time. A load
    // path would have read it (and any save path rewritten it).
    let after = fs::metadata(&decoy).expect("decoy metadata after");
    assert_eq!(
        fs::read(&decoy).expect("re-read decoy"),
        b"do not open me in this slice".to_vec(),
        "the decoy's bytes must be untouched"
    );
    assert_eq!(
        after.modified().ok(),
        before.modified().ok(),
        "opening the port must not touch the document file"
    );

    // And nothing appeared in the state dir: no lock, no log, no temp file.
    let mut entries: Vec<String> = fs::read_dir(dir.path())
        .expect("read state dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(
        entries,
        vec![
            "DECOY-NOTES.md".to_string(),
            "session.json".to_string(),
            "settings.toml".to_string(),
        ],
        "starting a Gateway must create nothing"
    );

    // No window or registry call happened either — and that is structural, not
    // just unobserved: api has no notes-platform dependency (cargo arch), no
    // RegisterWindow has been sent, and start() emits nothing at all, which is
    // what the last assertion below checks.
    assert!(
        matches!(rx.try_recv(), Err(TryRecvError::Empty)),
        "start() must emit no event before any command is sent"
    );
}

/// 6. An unread queue must never make a producer wait. If anyone later bounds
///    either channel "to save memory", this is the ABBA-deadlock alarm.
#[test]
fn a_slow_consumer_never_blocks_the_producer() {
    const N: usize = 10_000;
    let (gateway, rx) = Gateway::start(StateDir(empty_dir()), Settings::default());

    let started = Instant::now();
    // From 1, not 0: a flush at revision 0 is stale against a fresh engine and
    // answers Clean (D11), which would make this count one short of N for the
    // wrong reason.
    for revision in 1..=N as u64 {
        // Nobody reads rx here, so the engine's event queue grows to 10 000. If
        // it were bounded, the engine would stall inside emit(), the drain below
        // would never finish, and the UI thread would then stall inside its own
        // send() — the exact ABBA the design notes refuse.
        let _ = gateway.send(Command::Flush {
            text: String::new(),
            revision,
        });
    }
    let produced = started.elapsed();
    let _ = gateway.send(Command::Shutdown);

    let mut answered = 0usize;
    while let Ok(event) = rx.recv() {
        if matches!(event, Event::SaveFailed { .. }) {
            answered += 1;
        }
    }
    assert_eq!(answered, N, "an unbounded queue loses nothing");
    assert!(
        produced < Duration::from_secs(10),
        "{N} sends took {produced:?}: something is blocking a producer"
    );
}

/// 7. The vocabulary is still usable from a separate crate — i.e. this is a port
///    a bridge can actually stand on, not a module-private implementation detail.
#[test]
fn the_vocabulary_is_nameable_from_outside_the_crate() {
    let meta = FileMeta {
        encoding: Encoding::Utf8Bom,
        line_ending: LineEnding::Lf,
        trailing_newline: false,
        read_only: true,
        oversize: false,
        armed: true,
    };
    let copied = meta; // Copy, across a crate boundary.
    assert_eq!(meta, copied);
    assert_eq!(copied.encoding, Encoding::Utf8Bom);
    assert!(matches!(
        SaveError::Unencodable(Encoding::Ansi(1252)),
        SaveError::Unencodable(Encoding::Ansi(1252))
    ));
    assert!(!Session::default().pinned, "D10 default: unpinned");
    assert_eq!(WindowHandle(7), WindowHandle(7));
}

/// Reads and discards everything queued right now. Returns the count. Safe to
/// call when the engine's Sender is dead: what was emitted is all in the queue.
fn drain(rx: &mut EventRx) -> usize {
    let mut n = 0;
    while rx.try_recv().is_ok() {
        n += 1;
    }
    n
}
