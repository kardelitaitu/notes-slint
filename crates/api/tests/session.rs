//! The headless "play a session" proof M1 exists for (architecture.md 5.4): a
//! whole user session with NO window, NO toolkit and NO mock engine - real files
//! on disk, real Events out of the port.
//!
//! Everything goes through [`Gateway::send`] and the
//! [`EventRx`](notes_api::EventRx) the caller was handed, and nothing else: no
//! engine internals, and no core call standing in for the app. That is the point.
//! A behaviour that is only visible from inside the crate is not a behaviour a
//! bridge can rely on. Assertions are Events plus bytes on disk, and every byte
//! comparison compares BYTE SLICES: a String comparison would pass on a file whose
//! line endings had been rewritten, and preserving them is the whole rule (§4.5).
//!
//! The scenarios are the ones ADR-0001 was written about - a foreign file opens,
//! reports its format honestly, refuses to autosave, says why, and is armed by one
//! explicit save - plus D11's revision rule, D12's session restored across a
//! restart, and requirement 2: arming never leaks from one document to the next.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use notes_api::{
    Command, Encoding, Event, FileMeta, Gateway, LineEnding, Rect, SaveError, Session, Settings,
    SkipReason, StateDir, StateFile, WindowHandle,
};
// The session file is read back with core's own parser: what is being asserted is
// that the engine wrote a file CORE can read, not one whose bytes we guessed.
use notes_core::session::{FILE_NAME, read_session, write_session};

// thiserror is a dependency of notes-api, not of this test target; naming it
// keeps the unused-crate-dependencies lint honest about that. Same for
// notes-platform: the port owns its host now (D46), this test never calls it.
use notes_platform as _;
use thiserror as _;

/// Long enough that a real disk round trip cannot time out on a loaded machine,
/// short enough that a hung engine fails the run instead of hanging it forever.
const ANSWER: Duration = Duration::from_secs(5);

/// The engine's idle cadence, mirrored here because AUTOSAVE_IDLE is the
/// engine's private constant. The negative assertions below wait out two of
/// these: a failure must be reported ONCE, not once per tick (M5).
const TICK: Duration = Duration::from_millis(750);

/// Two tick periods plus change - the silence window of a negative assertion.
fn ticks(n: u64) -> Duration {
    Duration::from_millis(n * TICK.as_millis() as u64 + 100)
}

/// M5: a failing session write is reported EXACTLY ONCE, then latched until a
/// write succeeds; the next distinct failure is one event again. Without the
/// latch this goes red immediately: the failed write leaves the pending bit
/// set, every tick retries it, and the old code emitted one SaveFailed per
/// tick forever, claiming revision 0 - a number a session file does not have.
///
/// The block is a DIRECTORY sitting where session.json belongs: core's atomic
/// write lands a temp beside it and renames onto it, and the OS refuses a
/// rename onto a directory - a failure with no cooperation from core, no
/// platform calls, and no clock assumption beyond the engine's own tick.
#[test]
fn a_failing_session_write_is_reported_once_until_it_succeeds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let session_path = root.join(FILE_NAME);
    write_session(&root, &Session::default()).expect("seed a writable session");
    let mut app = Harness::at(root, Settings::default());

    let assert_one_failure = |app: &mut Harness, want: &str| {
        match app.until(want, |ev| {
            matches!(
                ev,
                Event::StateWriteFailed {
                    file: StateFile::Session,
                    ..
                }
            )
        }) {
            Event::StateWriteFailed { file, reason } => {
                assert_eq!(file, StateFile::Session, "the SUBJECT must be nameable");
                assert!(!reason.trim().is_empty(), "renderable: core's own sentence");
            }
            other => panic!("expected StateWriteFailed ({want}), got {other:?}"),
        }
        // EXACTLY ONE: wait out two tick periods. Any second event here is
        // the flood this test exists to fail on.
        let quiet = Instant::now() + ticks(2);
        while let Ok(event) = app
            .rx
            .recv_timeout(quiet.saturating_duration_since(Instant::now()))
        {
            assert!(
                !matches!(event, Event::StateWriteFailed { .. }),
                "the failure was reported more than once: {event:?}"
            );
        }
    };

    fs::remove_file(&session_path).expect("remove the seed");
    fs::create_dir(&session_path).expect("block the path with a directory");
    app.send(Command::SetPinned(true));
    assert_one_failure(&mut app, "the first session failure");

    // Success clears the latch - SILENTLY: the port has no session-saved
    // event, so this half waits out two tick periods for the retry to land
    // before re-arming the failure. No signal exists to wait on; the clock
    // here is a bound on silence, not a substitute for one.
    fs::remove_dir(&session_path).expect("unblock the path");
    app.send(Command::SetPinned(false));
    std::thread::sleep(ticks(2));

    // The retry LANDED: the file exists now, which is what cleared the latch -
    // the port emits no session-saved event, so this is the only observable
    // proof of the success half.
    assert!(
        session_path.is_file(),
        "the unblocked write must have produced session.json"
    );
    // Take the succeeded file away before re-blocking: the block is a
    // DIRECTORY sitting where session.json belongs, and it cannot coexist
    // with the write that just succeeded.
    fs::remove_file(&session_path).expect("remove the succeeded file");
    fs::create_dir(&session_path).expect("block the path again");
    app.send(Command::SetPinned(true));
    assert_one_failure(&mut app, "the second session failure");
}

/// THE MISSING SHAPE the first smoke run proved (d8fc569): every headless test
/// in this file hands the Gateway a TempDir that ALREADY EXISTS, so not one of
/// the 55 green tests could see that nothing in the process creates the state
/// directory. A fresh install persisted NOTHING - the write returned NotFound,
/// the failure was latched into one event nobody rendered, and the app looked
/// clean. This is the one shape that catches it: a state directory whose
/// PARENT exists and which does not. Startup must succeed, the session write
/// must land, and the file must be readable afterwards.
#[test]
fn startup_creates_a_state_directory_that_does_not_exist_yet() {
    let parent = tempfile::tempdir().expect("tempdir");
    let state = parent.path().join("notes-gpui");
    assert!(
        !state.exists(),
        "the fixture is a directory that does NOT exist yet"
    );

    let (mut gateway, rx) =
        Gateway::start_with_host(StateDir(state.clone()), Settings::default(), None, None);
    // The pre-window read is the proof startup itself survived the directory:
    // a start that died here would have no snapshot to hand over.
    assert!(
        gateway.startup_state().is_some(),
        "startup must succeed on a machine that has never run the app"
    );
    assert!(
        state.is_dir(),
        "the port creates the directory it writes into"
    );

    gateway.send(Command::SetPinned(true)).expect("alive");
    gateway.close().expect("shutdown joins the engine");

    // Drain what the run emitted: a fresh install must report NO failure of
    // any kind on the way to the first session write.
    let mut strays = Vec::new();
    while let Ok(event) = rx.recv() {
        strays.push(event);
    }
    assert!(
        !strays.iter().any(|event| {
            matches!(
                event,
                Event::StateWriteFailed { .. } | Event::StateDirUnusable { .. }
            )
        }),
        "a fresh install must not report a failure: {strays:?}"
    );

    let restored = read_session(&state).expect("session.json must land in the created directory");
    assert!(restored.pinned, "and it is the session the engine held");
}

/// A running engine, its events, and the directory it writes into.
///
/// The TempDir is a field rather than a local so the files outlive every
/// assertion, and each test gets its own so no run can see another session.json.
struct Harness {
    gateway: Gateway,
    rx: Receiver<Event>,
    root: PathBuf,
    /// Held so the directory outlives every assertion; None when the harness is
    /// resuming a directory that some other owner already keeps alive.
    epoch: u64,
    _dir: Option<tempfile::TempDir>,
}

impl Harness {
    fn new() -> Self {
        Harness::with_settings(Settings::default())
    }

    /// The same engine with a caller-supplied [`Settings`] - which since this
    /// slice is core's type, means the ANSI code page is a test input too.
    fn with_settings(settings: Settings) -> Self {
        let dir = tempfile::tempdir().expect("a temp state dir");
        let root = dir.path().to_path_buf();
        let mut app = Harness::at(root, settings);
        app._dir = Some(dir);
        app
    }

    /// An engine on a directory that already exists - which is how a restart is
    /// played: same [`StateDir`], fresh [`Gateway`], nothing in memory.
    fn at(root: PathBuf, settings: Settings) -> Self {
        let dir: Option<tempfile::TempDir> = None;
        // No host on this harness: none of these scenarios exercise window
        // geometry, and the port now OWNS the platform object (D46), so a real
        // Win32 backend here would answer a made-up handle with refusals.
        // facts = None is also exactly the no-codepage state the D27 test below
        // relies on: with no host there is no gap-filler, so an ANSI file is
        // refused rather than guessed.
        let (gateway, rx) = Gateway::start_with_host(StateDir(root.clone()), settings, None, None);
        // 5.5 step 3, once per scenario: the window exists before anything is
        // asked of it, so the engine is in the state a bridge would leave it in.
        let _ = gateway.send(Command::RegisterWindow {
            handle: WindowHandle(0x100),
        });
        Harness {
            gateway,
            rx,
            root,
            _dir: dir,
            epoch: 0,
        }
    }

    /// Writes a fixture file into the state directory and returns its path.
    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join(name);
        fs::write(&path, bytes).expect("write the fixture file");
        path
    }

    /// Every command in these scenarios must be ACCEPTED. A send that returns Err
    /// means the engine exited mid-session, which would otherwise show up as a
    /// confusing timeout on an event that can never arrive.
    fn send(&self, command: Command) {
        assert!(
            self.gateway.send(command.clone()).is_ok(),
            "the engine must still be accepting commands: {command:?}"
        );
    }

    fn bytes(&self, path: &Path) -> Vec<u8> {
        fs::read(path).unwrap_or_default()
    }

    /// Reads events until one satisfies [`want`] and returns it.
    ///
    /// Skipped events are collected and printed on failure: an unexpected
    /// AutosaveSkipped IS a result, and swallowing one quietly would hide exactly
    /// the ADR-0001 behaviour under test.
    fn until<F>(&mut self, want: &str, f: F) -> Event
    where
        F: Fn(&Event) -> bool,
    {
        let mut seen = Vec::new();
        let deadline = Instant::now() + ANSWER;
        loop {
            let timeout = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(timeout) {
                Ok(event) => {
                    if f(&event) {
                        return event;
                    }
                    seen.push(event);
                }
                Err(err) => panic!("never saw {want}; saw {seen:?} first ({err:?})"),
            }
        }
    }

    /// Waits until the engine stops answering - the join, observed from outside.
    fn drain_events(&mut self) {
        while self.rx.recv_timeout(ANSWER).is_ok() {}
    }

    /// An Open that does not need the Loaded payload still moves the OPEN
    /// GENERATION - the harness mirrors the engine, or its buffered Flushes
    /// would be discarded as stale.
    fn open_generation(&mut self, path: &Path) {
        self.gateway
            .send(Command::Open {
                path: path.to_path_buf(),
            })
            .expect("queued");
        self.epoch += 1;
    }

    fn open(&mut self, path: &Path) -> (String, FileMeta) {
        self.send(Command::Open {
            path: path.to_path_buf(),
        });
        self.epoch += 1;
        match self.until(
            "Loaded",
            |ev| matches!(ev, Event::Loaded { path: p, .. } if p == path),
        ) {
            Event::Loaded { text, meta, .. } => (text, meta),
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    /// A flush that is expected to SAVE, returning the saved revision.
    fn flush(&mut self, path: &Path, text: &str, revision: u64) -> u64 {
        self.send(Command::Flush {
            text: text.to_string(),
            revision,
            epoch: self.epoch,
        });
        match self.until(
            "Saved",
            |ev| matches!(ev, Event::Saved { path: p, .. } if p == path),
        ) {
            Event::Saved { revision, .. } => revision,
            other => panic!("expected Saved, got {other:?}"),
        }
    }

    /// A flush that must NOT save: the reason it skipped. Asserting there was no
    /// Saved for it is the caller's job, via the file bytes.
    fn flush_skipped(&mut self, text: &str, revision: u64) -> SkipReason {
        self.send(Command::Flush {
            text: text.to_string(),
            revision,
            epoch: self.epoch,
        });
        match self.until("a skip", |ev| {
            matches!(ev, Event::AutosaveSkipped { .. } | Event::Saved { .. })
        }) {
            Event::AutosaveSkipped { reason } => reason,
            other => panic!("expected AutosaveSkipped, got {other:?}"),
        }
    }

    /// Save As carries the text (D30): the engine holds no buffer of its own, so
    /// this is the moment the command's own text field earns its keep. Also waits
    /// for [`Event::Rebound`], which is the point of the variant.
    fn save_as(&mut self, path: &Path, text: &str, revision: u64) -> u64 {
        self.send(Command::SaveAs {
            path: path.to_path_buf(),
            text: text.to_string(),
            revision,
        });
        // A rebind is a new OPEN GENERATION; the harness mirrors the engine.
        self.epoch += 1;
        match self.until("Saved", |ev| match ev {
            Event::Saved { path: p, .. } => p == path,
            Event::SaveFailed { path: p, .. } => p == path,
            _ => false,
        }) {
            Event::Saved { revision, .. } => {
                // The rebind must follow the save: the UI's path, arming and
                // encoding all change at this moment and nothing else says so.
                match self.until(
                    "Rebound",
                    |ev| matches!(ev, Event::Rebound { path: p, .. } if p == path),
                ) {
                    Event::Rebound {
                        meta, revision: r, ..
                    } => {
                        assert_eq!(r, revision, "Saved and Rebound agree on the revision");
                        assert!(meta.armed, "Save As arms the document (ADR-0001 req 4)");
                        assert!(!meta.oversize);
                        revision
                    }
                    other => unreachable!("filtered to Rebound, got {other:?}"),
                }
            }
            Event::SaveFailed { reason, .. } => {
                panic!("SaveAs to {} failed: {reason}", path.display())
            }
            other => unreachable!("filtered to Saved/SaveFailed, got {other:?}"),
        }
    }
}

/// A foreign .md with CRLF and NO trailing newline - the shape that catches an app
/// that "tidies" files. ADR-0001 says it opens disarmed.
#[test]
fn a_foreign_file_reports_its_format_refuses_to_autosave_and_arms_on_one_save() {
    let mut app = Harness::new();
    let md = app.file("notes.md", b"line one\r\nline two");
    let original = app.bytes(&md);
    assert_eq!(
        original, b"line one\r\nline two",
        "fixture is CRLF, unterminated"
    );

    let (text, meta) = app.open(&md);
    // No frontmatter in a .md, so the .notes seam must be a no-op. Asserted, not
    // assumed: this is the byte-identical round trip §4.5 and R10 are about.
    assert_eq!(text.as_bytes(), b"line one\r\nline two");
    assert_eq!(meta.encoding, Encoding::Utf8);
    assert_eq!(
        meta.line_ending,
        LineEnding::CrLf,
        "detected, never normalised"
    );
    assert!(!meta.trailing_newline, "a missing final newline is a FACT");
    assert!(!meta.read_only);
    assert!(!meta.oversize);
    assert!(!meta.armed, "ADR-0001: a foreign file is NOT armed on open");

    // Silence is forbidden (requirement 1): the flush is answered with the reason
    // nothing happened - and nothing happened.
    assert_eq!(
        app.flush_skipped("line one\r\nline two edited", 1),
        SkipReason::ForeignFileNotArmed,
        "the skip must name the reason the user can act on"
    );
    assert_eq!(
        app.bytes(&md),
        original,
        "a disarmed foreign file must not be written by an autosave"
    );

    // The one explicit act arms it (requirement 4), and Save As is the ONLY write.
    let target = app.root.join("kept.notes");
    assert_eq!(
        app.save_as(&target, "line one\r\nline two edited", 1),
        1,
        "Save As writes the snapshot it was given"
    );
    assert_eq!(
        app.bytes(&target),
        b"line one\r\nline two edited",
        "the bytes are the buffer's, in the buffer's own line endings"
    );
    assert_eq!(
        app.bytes(&md),
        original,
        "the ORIGINAL foreign file is untouched - kept.notes is the only write"
    );

    // Armed now, so the very next flush writes without being asked twice.
    assert_eq!(app.flush(&target, "edit again", 2), 2);
    assert_eq!(
        app.bytes(&target),
        b"edit again",
        "autosave wrote it, in the same detected format"
    );
    assert_eq!(
        app.bytes(&md),
        original,
        "and the foreign file never got touched"
    );
}

/// D11: a monotonic u64, and a Flush at or below the last saved revision means
/// Clean - no write, no error, and no Saved pretending work happened.
#[test]
fn increasing_revisions_save_and_a_stale_flush_is_clean() {
    let mut app = Harness::new();
    let notes = app.file("idea.notes", b"first\r\n");
    let (_, meta) = app.open(&notes);
    assert!(meta.armed, "a .notes file arms on open (ADR-0001)");
    assert_eq!(meta.line_ending, LineEnding::CrLf);
    assert!(meta.trailing_newline);

    let mut seen = Vec::new();
    for revision in 1..=3u64 {
        let body = format!("edit {revision}\r\n");
        let saved = app.flush(&notes, &body, revision);
        assert!(
            seen.last().is_none_or(|last: &u64| saved > *last),
            "Saved revisions must strictly increase: {seen:?} then {saved}"
        );
        seen.push(saved);
    }
    assert_eq!(seen, vec![1, 2, 3]);
    assert_eq!(app.bytes(&notes), b"edit 3\r\n");

    // The stale case: a debounced autosave observed revision 2 after revision 3
    // had already reached disk.
    let on_disk = app.bytes(&notes);
    let stamp = fs::metadata(&notes).expect("metadata").modified().ok();
    assert_eq!(app.flush_skipped("edit 2\r\n", 2), SkipReason::Clean);
    assert_eq!(app.bytes(&notes), on_disk, "a stale flush writes nothing");
    assert_eq!(
        fs::metadata(&notes).expect("metadata").modified().ok(),
        stamp,
        "a Clean skip does not even move the mtime"
    );
}

/// A failed save is data, not a crash: the reason must be renderable, and the
/// engine must go on serving.
#[test]
fn a_failed_save_arrives_as_an_event_and_the_engine_keeps_working() {
    let mut app = Harness::new();
    let notes = app.file("keep.notes", b"content\n");
    app.open(&notes);

    // The canonical "where did my file go": a directory that does not exist.
    let nowhere = app.root.join("no-such-folder").join("deep.notes");
    app.send(Command::SaveAs {
        path: nowhere.clone(),
        text: "content\n".to_string(),
        revision: 0,
    });
    // A rebind is a new generation (the engine bumped; the harness mirrors).
    app.epoch += 1;
    let event = app.until(
        "SaveFailed",
        |ev| matches!(ev, Event::SaveFailed { path, .. } if path == &nowhere),
    );
    let Event::SaveFailed {
        reason, revision, ..
    } = event
    else {
        unreachable!()
    };
    let copy = reason.to_string();
    assert!(
        !copy.trim().is_empty(),
        "a failure with no text cannot be rendered"
    );
    assert_eq!(revision, 0, "the revision it was working at is echoed back");
    assert!(
        matches!(
            reason,
            SaveError::NotFound | SaveError::PermissionDenied | SaveError::Other(_)
        ),
        "unexpected failure shape: {reason:?} ({copy})"
    );
    assert_eq!(
        app.bytes(&notes),
        b"content\n",
        "a refused Save As must not damage the file already open"
    );

    // Seconds later the same engine still does its job: the failure was an event,
    // not a teardown.
    assert_eq!(app.flush(&notes, "still working\n", 7), 7);
    assert_eq!(app.bytes(&notes), b"still working\n");
}

/// The session file, restored: geometry and the pin bit that were changed and then
/// shut down come back through a NEW engine on the same StateDir, with no temp
/// litter and no second copy of the pin to disagree with the first (D10, D12).
#[test]
fn geometry_and_the_pin_bit_survive_a_restart_and_leave_no_temp_litter() {
    let dir = tempfile::tempdir().expect("tempdir");
    let rect = Rect::new(40, 24, 1024, 700);
    // GeometryChanged is PAYLOAD-FREE now: a rect reaches the session through
    // a measured frame read (needs a host) or through the persisted file being
    // restored. Headless, the honest fixture is a session seeded BEFORE the
    // engine reads it - which is exactly what a restart is.
    write_session(
        dir.path(),
        &Session {
            rect,
            ..Session::default()
        },
    )
    .expect("seed the session");
    let mut app = Harness::at(dir.path().to_path_buf(), Settings::default());
    app._dir = Some(dir);
    app.send(Command::SetPinned(true));
    app.send(Command::GeometryChanged);
    // Shutdown drains, and the final session write happens INSIDE the drain -
    // which is what makes the assertions below possible at all.
    app.send(Command::Shutdown);
    app.drain_events();
    drop(app.gateway);

    let restored = read_session(&app.root).expect("session.json must be core-readable");
    assert_eq!(restored.rect, rect, "the NEW rect, not the monitor default");
    assert!(
        restored.pinned,
        "D10: the pin bit has one home and it is here"
    );
    assert!(app.root.join(FILE_NAME).is_file());

    // Atomic: core sweeps stale temps, so no session.json.tmp-* survives.
    let names: Vec<String> = fs::read_dir(&app.root)
        .expect("read the state dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        !names.iter().any(|name| name.contains(".tmp-")),
        "an atomic write must not leave temp files behind: {names:?}"
    );

    // A second engine on the same directory restores it BEFORE any window exists:
    // 5.5 step 1, the one synchronous call in the app. The TempDir is still held by
    // the harness, so this is the same directory and not a copy of it.
    let (mut again, _rx) = Gateway::start(StateDir(app.root.clone()), Settings::default());
    let initial = again
        .startup_state()
        .expect("the pre-window snapshot is handed over once");
    assert_eq!(initial.session.rect, rect);
    assert!(
        initial.pinned,
        "the bridge applies topmost from this read, not from guesswork"
    );
    drop(again);
}

/// D30, the data-loss class the snapshot used to carry: a Flush is debounced, so
/// between typing and choosing Save As the engine has seen NOTHING of the newest
/// text. With Save As carrying its own text, the file written at the new path is
/// what the editor is showing - and there is no longer an engine-side copy that
/// could be older than the window.
#[test]
fn save_as_writes_the_text_it_was_given_not_a_last_flush_snapshot() {
    let mut app = Harness::new();
    let src = app.file(
        "draft.notes",
        b"typed first
",
    );
    app.open(&src);
    // One flush lands, then the user keeps typing and never triggers another.
    app.flush(
        &src,
        "typed first
",
        1,
    );
    let target = app.root.join("renamed.notes");
    assert_eq!(
        app.save_as(&target, "typed first\nand then a lot more", 2),
        2,
        "the Save As revision is the one reported"
    );
    assert_eq!(
        app.bytes(&target),
        b"typed first\nand then a lot more",
        "the NEW text is on disk, not the snapshot the last flush left behind"
    );
}

/// ADR-0001 requirement 2, the case only a second document can expose.
#[test]
fn arming_never_leaks_from_one_document_to_the_next() {
    let mut app = Harness::new();
    let a = app.file("a.notes", b"notes file\n");
    let b = app.file("b.md", b"foreign file\r\n");

    let (_, meta_a) = app.open(&a);
    assert!(meta_a.armed, "A is native, so it is armed");
    assert_eq!(app.flush(&a, "changed A\n", 1), 1);

    let (_, meta_b) = app.open(&b);
    assert!(
        !meta_b.armed,
        "B is foreign and opens DISARMED even though A was armed and saved"
    );
    let before = app.bytes(&b);
    assert_eq!(
        app.flush_skipped("changed B\r\n", 2),
        SkipReason::ForeignFileNotArmed
    );
    assert_eq!(app.bytes(&b), before, "and B really was not written");
    assert_eq!(before, b"foreign file\r\n", "byte-identical, as promised");
}

/// D13 from outside the crate: a newest-first list that ClearRecents empties, and a
/// vanished file reported as a load failure whose copy the UI can show.
#[test]
fn the_recent_list_is_reported_cleared_and_rebuilt() {
    let mut app = Harness::new();
    let one = app.file("one.notes", b"1\n");
    let two = app.file("two.notes", b"2\n");
    let gone = app.file("three.md", b"3\n");

    app.open(&one);
    app.open(&two);
    fs::remove_file(&gone).expect("delete the third file");

    app.send(Command::ClearRecents);
    app.until(
        "an empty RecentsUpdated",
        |ev| matches!(ev, Event::RecentsUpdated(entries) if entries.is_empty()),
    );

    // Opening a file that vanished fails with renderable copy: the variant this
    // slice added, because the frozen vocabulary had nowhere honest for it.
    app.open_generation(&gone.clone());
    match app.until(
        "LoadFailed",
        |ev| matches!(ev, Event::LoadFailed { path, .. } if path == &gone),
    ) {
        Event::LoadFailed { reason, .. } => {
            assert_eq!(reason.to_string(), "file no longer exists", "pinned copy");
        }
        other => panic!("expected LoadFailed, got {other:?}"),
    }

    let (text, meta) = app.open(&two);
    assert_eq!(text.as_bytes(), b"2\n");
    assert!(meta.armed, ".notes is native, so it is armed on open");
    let list = match app.until(
        "a non-empty RecentsUpdated",
        |ev| matches!(ev, Event::RecentsUpdated(entries) if !entries.is_empty()),
    ) {
        Event::RecentsUpdated(entries) => entries,
        other => panic!("expected RecentsUpdated, got {other:?}"),
    };
    assert_eq!(list[0].path, two, "most recent first");
    assert!(list[0].exists, "a file just opened exists");
    assert!(
        list.len() <= 10,
        "D13's cap holds from outside the crate: {}",
        list.len()
    );
}

/// MINOR 4 / D27, and the reason [`detect`] is now handed a code page at all:
/// an ANSI file must be read at the page the CALLER supplied. With 1252 it decodes
/// to the right characters; with a page core cannot write back it must be refused
/// rather than guessed - guessing is how a French note turns into mojibake on a
/// Japanese machine and then gets SAVED that way.
#[test]
fn an_ansi_file_is_decoded_at_the_configured_codepage_not_by_assumption() {
    // "caf" + 0xE9 + " notes": 0xE9 is e-acute in CP1252 and is not valid UTF-8,
    // so by the time detection runs this really is an ANSI file. No trailing
    // newline either, so the same fixture pins the unterminated case of §4.5.
    let e_acute = char::from_u32(0xE9).expect("e-acute");
    let bytes: Vec<u8> = [b"caf".as_slice(), &[0xE9u8], b" notes"].concat();

    let mut app = Harness::with_settings(Settings {
        codepage: Some(1252),
        ..Settings::default()
    });
    let native = app.file("fr.notes", &bytes);
    let (text, meta) = app.open(&native);
    assert_eq!(
        meta.encoding,
        Encoding::Ansi(1252),
        "read at the given page"
    );
    assert_eq!(text, format!("caf{e_acute} notes"), "decoded, not replaced");
    assert_eq!(app.bytes(&native), bytes, "and reading changed nothing");
    assert!(!meta.trailing_newline);

    let mut other = Harness::with_settings(Settings {
        codepage: Some(932),
        ..Settings::default()
    });
    let same = other.file("fr.notes", &bytes);
    other.open_generation(&same.clone());
    let event = other.until("a verdict on the ANSI file", |ev| {
        matches!(ev, Event::LoadFailed { path, .. } | Event::Loaded { path, .. } if path == &same)
    });
    match event {
        Event::LoadFailed { reason, .. } => {
            assert!(
                !reason.to_string().trim().is_empty(),
                "the refusal must be renderable"
            );
            assert_eq!(
                app.bytes(&native),
                bytes,
                "a refused code page must not have rewritten anything"
            );
        }
        loaded => panic!("a page core cannot write back must not decode silently: {loaded:?}"),
    }
}

/// B1, the measured data-loss path. A file over the D9 guard must be REFUSED at
/// open - not announced as an empty UTF-8/LF document nobody read - and the
/// refused document must then be un-savable, because the buffer on screen is not
/// that file. Before this fix, Save As wrote 0 bytes over it and reported Saved.
#[test]
fn an_oversize_file_is_refused_from_the_stat_and_cannot_be_overwritten() {
    let mut app = Harness::new();
    let big = app.root.join("big.notes");
    let size = 8_usize * 1024 * 1024;
    fs::write(&big, vec![b'A'; size]).expect("write the 8 MiB fixture");

    app.open_generation(&big.clone());
    match app.until(
        "LoadFailed(TooLarge)",
        |ev| matches!(ev, Event::LoadFailed { path, .. } if path == &big),
    ) {
        Event::LoadFailed { reason, .. } => {
            assert_eq!(
                reason,
                notes_api::LoadError::TooLarge,
                "the byte guard's own reason"
            );
            assert_eq!(
                reason.to_string(),
                "file is too large to open",
                "pinned copy"
            );
        }
        other => panic!("expected LoadFailed, got {other:?}"),
    }
    // No Loaded, no Rebound, and no 4 KiB-worth of invented FileMeta anywhere.
    assert!(matches!(
        app.rx.try_recv(),
        Err(_) | Ok(Event::RecentsUpdated(_))
    ));

    app.send(Command::SaveAs {
        path: big.clone(),
        text: "the stale buffer".to_string(),
        revision: 1,
    });
    let event = app.until(
        "SaveFailed",
        |ev| matches!(ev, Event::SaveFailed { path, .. } if path == &big),
    );
    match event {
        Event::SaveFailed { reason, .. } => {
            assert!(
                matches!(reason, notes_api::SaveError::NoTarget),
                "refused, not invented: {reason:?}"
            );
        }
        other => panic!("expected SaveFailed, got {other:?}"),
    }
    let on_disk = app.bytes(&big);
    assert_eq!(
        on_disk.len(),
        size,
        "the 8 MiB file is byte-for-byte intact"
    );
    assert!(!on_disk.is_empty(), "and above all, not zero bytes");
}

/// D69: a brand-new note that has never been saved gets a REAL scratch file
/// instead of being dropped with a skip - the first-run user types, autosave
/// fires, and the text must land on disk. The path is core's
/// (scratch_note_path), the write is the Save As machinery, the events are
/// Saved THEN Rebound, the document is rebound to the scratch, and the scratch
/// joins the recents like any other file.
#[test]
fn an_untitled_note_is_written_to_a_scratch_file_instead_of_dropped() {
    let app = Harness::new();
    let scratch = notes_core::paths::scratch_note_path(&StateDir(app.root.clone()));

    app.send(Command::Flush {
        // A real buffer ends with a newline; the port preserves it verbatim
        // (4.5 - it never normalises), so the fixture types it too.
        text: "the first draft\n".to_string(),
        revision: 1,
        epoch: 0,
    });

    // Event order: Saved THEN Rebound, both naming the scratch.
    let mut saved_at = None;
    let mut rebound_at = None;
    let mut recents_after = None;
    let mut index = 0usize;
    let deadline = Instant::now() + ANSWER;
    while saved_at.is_none() || rebound_at.is_none() || recents_after.is_none() {
        assert!(
            Instant::now() < deadline,
            "Saved/Rebound/recents never all arrived (saved={saved_at:?} rebound={rebound_at:?})"
        );
        let event = app
            .rx
            .recv_timeout(ANSWER)
            .expect("the engine owed an event");
        match event {
            Event::Saved { path, .. } if path == scratch => saved_at = Some(index),
            Event::Rebound { path, .. } if path == scratch => rebound_at = Some(index),
            Event::RecentsUpdated(entries) if entries.len() == 1 => {
                assert_eq!(entries[0].path, scratch, "the scratch joins the recents");
                recents_after = Some(index);
            }
            _ => {}
        }
        index += 1;
    }
    assert!(
        saved_at.expect("saved") < rebound_at.expect("rebound"),
        "Saved announces the bytes BEFORE Rebound announces the new identity"
    );

    // The bytes on disk: UTF-8, LF, and the trailing newline the new-file
    // rules give - read back, not assumed.
    let bytes = fs::read(&scratch).expect("the scratch file must exist");
    assert_eq!(bytes, b"the first draft\n", "the typed text, verbatim");

    // The session now names the scratch, so a relaunch reopens the same note.
    // The session write rides the 750 ms tick behind the events, so poll the
    // file (bounded) rather than assuming it exists yet.
    let deadline = Instant::now() + ANSWER;
    while !app.root.join(FILE_NAME).exists() {
        assert!(Instant::now() < deadline, "session.json never landed");
        std::thread::sleep(Duration::from_millis(20));
    }
    let session = read_session(&app.root).expect("session readable");
    assert_eq!(
        session.path,
        Some(scratch.clone()),
        "session.path must name the scratch so a relaunch reopens it"
    );
}

/// M4: a Save As observed at an OLD revision must not rewind the gate, or the
/// flushes between it and the last save get admitted a second time.
#[test]
fn a_stale_save_as_cannot_rewind_the_revision_gate() {
    let mut app = Harness::new();
    let notes = app.file("idea.notes", b"v1");
    app.open(&notes);
    assert_eq!(app.flush(&notes, "v9", 9), 9);
    let target = app.root.join("kept.notes");
    assert_eq!(
        app.save_as(&target, "v9", 3),
        3,
        "Save As reports the revision it was given"
    );
    // Flushes 4..=8 were already superseded by the save at 9. If the gate had been
    // rewound to 3, this would write and answer Saved.
    assert_eq!(app.flush_skipped("v5", 5), SkipReason::Clean);
    assert!(
        !app.bytes(&target).starts_with(b"v5"),
        "nothing was written at the stale revision"
    );
}

/// B2 / D27, the other half: with NO code page supplied the port must refuse an
/// ANSI file rather than guess CP1252.
#[test]
fn without_a_codepage_an_ansi_file_is_refused_rather_than_guessed() {
    let bytes: Vec<u8> = [b"caf".as_slice(), &[0xE9u8], b" notes"].concat();
    let mut app = Harness::with_settings(Settings {
        codepage: None,
        ..Settings::default()
    });
    let path = app.file("fr.notes", &bytes);
    app.open_generation(&path.clone());
    match app.until(
        "LoadFailed",
        |ev| matches!(ev, Event::LoadFailed { path: got, .. } if got == &path),
    ) {
        Event::LoadFailed { reason, .. } => {
            assert!(
                !reason.to_string().trim().is_empty(),
                "renderable either way"
            );
            assert_eq!(
                app.bytes(&path),
                bytes,
                "a refusal must not rewrite the file"
            );
        }
        other => panic!("an unknown code page must not decode silently: {other:?}"),
    }
}

/// Items 5 and 6 together, played as a restart: the autosave toggle and the
/// recents list come back from settings.toml (D10's second home), and a recent
/// whose file vanished comes back GREYED rather than deleted.
#[test]
fn the_toggle_and_the_recents_list_survive_a_restart_and_a_vanished_file() {
    let dir = tempfile::tempdir().expect("a temp state dir");
    let root = dir.path().to_path_buf();
    let doomed = root.join("doomed.notes");
    fs::write(&doomed, b"draft").expect("write doomed");

    {
        let mut first = Harness::at(root.clone(), Settings::default());
        first.send(Command::SetAutosave(false));
        first.open(&doomed);
        assert_eq!(
            first.flush_skipped("edited", 1),
            SkipReason::AutosaveDisabled
        );
        first.send(Command::Shutdown);
        first.drain_events();
    }
    fs::remove_file(&doomed).expect("delete the note between runs");

    let mut again = Harness::at(root.clone(), Settings::default());
    let other = again.file("second.notes", b"second");
    // Opened by hand rather than through the `open` helper: that helper waits for
    // `Loaded` and DISCARDS everything else on the way, and the list this test
    // asserts on arrives in the same burst. Every later helper call would eat it.
    again.open_generation(&other.clone());
    let list = match again.until(
        "RecentsUpdated with both files",
        |ev| matches!(ev, Event::RecentsUpdated(entries) if entries.len() >= 2),
    ) {
        Event::RecentsUpdated(entries) => entries,
        other => panic!("expected RecentsUpdated, got {other:?}"),
    };
    assert_eq!(
        list[0].path, other,
        "most recent first, from the fresh open"
    );
    let vanished = list
        .iter()
        .find(|entry| entry.path == doomed)
        .expect("a vanished recent is kept, not silently deleted (features.md 4.4)");
    assert!(!vanished.exists, "and it is greyed out, not dropped");
    assert!(list.len() <= 10, "D13 cap");
    // The toggle has to survive too, and it is checked last on purpose: this is the
    // same engine and the same one read, so a skip here can only have come from
    // settings.toml - the caller passed the default, which is autosave ON.
    assert_eq!(
        again.flush_skipped("typed", 2),
        SkipReason::AutosaveDisabled,
        "settings.toml owned the toggle across the restart, not the caller's default"
    );
}

/// D54 case (c), the one the SECOND smoke run measured: the state directory
/// already exists and is fine, and the write STILL fails - because
/// session.json itself refuses it (read-only attribute, an ACL, another
/// process's lock, or something that is not a file). The latch on the write
/// path reports that once, but by then the bridge may be tearing the window
/// down, and an unrendered report is the same silence as no report. So the
/// port probes at startup and emits StateDirUnusable BEFORE the first frame.
// Clearing the read-only attribute is the fixture's cleanup half, and clippy
// is right that the bit is not portable on Unix - this is the Windows ACL
// story, so the asymmetry is named rather than papered over.
#[allow(clippy::permissions_set_readonly_false)]
#[test]
fn an_unwritable_session_file_is_reported_at_startup_not_at_shutdown() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let session_path = root.join(FILE_NAME);
    write_session(&root, &Session::default()).expect("seed a writable session");
    let mut perms = fs::metadata(&session_path).expect("metadata").permissions();
    perms.set_readonly(true);
    fs::set_permissions(&session_path, perms).expect("make the seed read-only");

    let mut app = Harness::at(root, Settings::default());
    // No StateDirUnusable: the directory itself is fine (core's judgement);
    // the target FILE is the problem, and it fails at the write, latched.
    assert!(
        !matches!(app.rx.try_recv(), Ok(Event::StateDirUnusable { .. })),
        "the directory is writable; the target file is the problem"
    );

    app.send(Command::SetPinned(true));
    match app.until("the read-only refusal", |ev| {
        matches!(ev, Event::StateWriteFailed { .. })
    }) {
        Event::StateWriteFailed { file, reason } => {
            assert_eq!(file, StateFile::Session);
            assert!(!reason.trim().is_empty(), "renderable: core's sentence");
        }
        other => panic!("expected StateWriteFailed, got {other:?}"),
    }
    // LATCHED: two tick periods of silence, not a flood.
    let quiet = Instant::now() + ticks(2);
    while let Ok(event) = app
        .rx
        .recv_timeout(quiet.saturating_duration_since(Instant::now()))
    {
        assert!(
            !matches!(event, Event::StateWriteFailed { .. }),
            "the read-only refusal was reported more than once: {event:?}"
        );
    }
    drop(app);
    // Restore writability so the TempDir can clean up after itself.
    let mut perms = fs::metadata(&session_path).expect("metadata").permissions();
    perms.set_readonly(false);
    fs::set_permissions(&session_path, perms).expect("restore the seed");
}

/// The cold-start budget is a documented product constraint, so core's
/// ensure_state_dir (which the port now calls at startup) is MEASURED here
/// against a real, already-created directory - the warm path every launch
/// after the first takes.
#[test]
fn core_state_dir_ensure_cost_is_bounded() {
    let dir = tempfile::tempdir().expect("tempdir");
    // The first call creates; every later call is the warm path.
    notes_core::session::ensure_state_dir(dir.path())
        .expect("the first ensure creates the directory");

    const RUNS: u32 = 200;
    let started = Instant::now();
    for _ in 0..RUNS {
        notes_core::session::ensure_state_dir(dir.path()).expect("the warm path must stay healthy");
    }
    let per_call = started.elapsed().as_micros() as u64 / RUNS as u64;
    // A real-disk number; print it so the run records it.
    println!("core ensure_state_dir: {per_call} us per call ({RUNS} runs measured)");
    assert!(
        per_call < 25_000,
        "the state-dir check must not become a cold-start budget item: {per_call} us"
    );
}

/// The menu-label rule is core's (features.md 4.4); the port's only job is to
/// CALL it. Two recents that share a basename in different folders must not
/// reach the bridge as the same string - without `display_labels` in
/// `emit_recent` they both render as the bare name and the menu cannot tell
/// the user which is which. A label is a property of the LIST, which is why
/// the stored entry keeps the plain basename as its fact and the rendering
/// happens on the way out.
#[test]
fn two_recents_with_the_same_basename_do_not_share_a_menu_label() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = Harness::new();
    let a_dir = dir.path().join("project-a");
    let b_dir = dir.path().join("project-b");
    fs::create_dir(&a_dir).expect("folder a");
    fs::create_dir(&b_dir).expect("folder b");
    let a = a_dir.join("readme.notes");
    let b = b_dir.join("readme.notes");
    fs::write(&a, b"from a\n").expect("write a");
    fs::write(&b, b"from b\n").expect("write b");

    app.open_generation(&a);
    app.until("the first Loaded", |ev| matches!(ev, Event::Loaded { .. }));
    app.open_generation(&b.clone());
    let entries = match app.until(
        "a two-entry recent list",
        |ev| matches!(ev, Event::RecentsUpdated(list) if list.len() == 2),
    ) {
        Event::RecentsUpdated(list) => list,
        other => panic!("expected a two-entry RecentsUpdated, got {other:?}"),
    };

    let labels: Vec<&str> = entries.iter().map(|e| e.display.as_str()).collect();
    let unique: std::collections::HashSet<&&str> = labels.iter().collect();
    assert_eq!(
        unique.len(),
        2,
        "colliding recents must not render as one menu string: {labels:?}"
    );
    // The SHAPE is core's call, not the port's: it grows parent folders until
    // the labels differ (here "…\project-b\readme.notes"), which the test only
    // checks as far as the contract goes - the name is still the tail of the
    // label, and the two entries are distinguishable.
    for label in &labels {
        assert!(
            label.ends_with("readme.notes"),
            "the file name stays the tail of the label: {label}"
        );
        assert!(
            label.contains("project-a") || label.contains("project-b"),
            "a parent folder disambiguates the collision: {label}"
        );
    }
    // And the label is RENDERED, not persisted: the file core reads back still
    // holds the plain basename, so a later list containing only ONE of these
    // files labels plainly again instead of carrying the other's suffix.
    let _ = b;
}

/// M5/D54, third case of the same disease: settings.toml rode
/// [`Event::SaveFailed`] with a `revision: 0` the file does not have - a
/// settings file HAS no revision - and would have flooded one event per tick
/// exactly as the session write did. Now it reports ONE
/// StateWriteFailed(Settings) per failure, latched, cleared by the next
/// success. The block is the same shape the session test uses: a DIRECTORY
/// sitting where settings.toml belongs, which the atomic write's rename
/// refuses with an OS sentence.
#[test]
fn a_failing_settings_write_is_reported_once_until_it_succeeds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let settings_path = root.join("settings.toml");
    let mut app = Harness::at(root, Settings::default());

    let assert_one_failure = |app: &mut Harness, want: &str| {
        match app.until(want, |ev| {
            matches!(
                ev,
                Event::StateWriteFailed {
                    file: StateFile::Settings,
                    ..
                }
            )
        }) {
            Event::StateWriteFailed { file, reason } => {
                assert_eq!(file, StateFile::Settings, "the SUBJECT must be nameable");
                assert!(!reason.trim().is_empty(), "renderable: core's sentence");
            }
            other => panic!("expected StateWriteFailed(Settings) ({want}), got {other:?}"),
        }
        let quiet = Instant::now() + ticks(2);
        while let Ok(event) = app
            .rx
            .recv_timeout(quiet.saturating_duration_since(Instant::now()))
        {
            assert!(
                !matches!(
                    event,
                    Event::StateWriteFailed {
                        file: StateFile::Settings,
                        ..
                    }
                ),
                "the settings failure was reported more than once: {event:?}"
            );
        }
    };

    // The toggle CHANGES, so the write is queued (an equal value queues
    // nothing).
    fs::create_dir(&settings_path).expect("block settings.toml with a directory");
    app.send(Command::SetAutosave(false));
    assert_one_failure(&mut app, "the first settings failure");

    // Success clears the latch - silently, and observably: the retry lands a
    // real settings.toml where the directory was.
    fs::remove_dir(&settings_path).expect("unblock");
    app.send(Command::SetAutosave(true));
    std::thread::sleep(ticks(2));
    assert!(
        settings_path.is_file(),
        "the unblocked write must have produced settings.toml"
    );

    fs::remove_file(&settings_path).expect("remove the succeeded file");
    fs::create_dir(&settings_path).expect("block again");
    app.send(Command::SetAutosave(false));
    assert_one_failure(&mut app, "the second settings failure");
}

/// D69's fallback: when the scratch place cannot be made, NeedsPath is the
/// honest answer ("we could not make a file for it") and NO file is written.
/// The startup StateDirUnusable event has usually already said why.
#[test]
fn an_unusable_scratch_location_still_skips_and_writes_nothing() {
    let mut app = Harness::new();
    // A FILE sitting where the notes directory belongs: ensure_scratch_dir
    // refuses, and the flush falls back to the skip.
    fs::write(app.root.join("notes"), b"not a directory").expect("block");

    assert_eq!(
        app.flush_skipped("the first draft", 1),
        SkipReason::NeedsPath,
        "the fallback skip, with its real new meaning"
    );
    assert!(
        !notes_core::paths::scratch_note_path(&StateDir(app.root.clone())).exists(),
        "no scratch file may be written when the place cannot be made"
    );
}

/// D69 edge: once the note HAS a path (Save As), later edits write THERE and
/// the abandoned scratch keeps its stale text forever. That is accepted
/// behaviour - call the leftover a DRAFT the user can still open, not a leak
/// to clean up; nothing in the vocabulary says "delete user content".
#[test]
fn a_note_saved_away_stops_using_the_scratch_and_the_draft_is_kept() {
    let mut app = Harness::new();
    let scratch = notes_core::paths::scratch_note_path(&StateDir(app.root.clone()));

    app.send(Command::Flush {
        text: "draft one\n".to_string(),
        revision: 1,
        epoch: 0,
    });
    let deadline = Instant::now() + ANSWER;
    while !scratch.exists() {
        assert!(Instant::now() < deadline, "the scratch write never landed");
        std::thread::sleep(Duration::from_millis(20));
    }
    let draft_bytes = fs::read(&scratch).expect("read the draft");

    let elsewhere = app.root.join("named.notes");
    app.send(Command::SaveAs {
        path: elsewhere.clone(),
        text: "named now\n".to_string(),
        revision: 2,
    });
    // The Save As rebinds: a new generation. The next edit is stamped with it.
    app.epoch += 1;
    app.send(Command::Flush {
        text: "named now, edited\n".to_string(),
        revision: 3,
        epoch: app.epoch,
    });
    let deadline = Instant::now() + ANSWER;
    loop {
        if let Ok(session) = read_session(&app.root) {
            if session.path.as_deref() == Some(elsewhere.as_path()) {
                break;
            }
        }
        assert!(Instant::now() < deadline, "the Save As never landed");
        std::thread::sleep(Duration::from_millis(20));
    }
    // Wait out one full flush cycle: the edit must go to the NAMED file, and
    // the scratch must not gain a second copy of the text.
    std::thread::sleep(Duration::from_millis(750 * 2 + 100));
    assert_eq!(
        fs::read(&scratch).expect("read the draft again"),
        draft_bytes,
        "the abandoned scratch keeps its stale draft - a draft, not a leak"
    );
    assert_eq!(
        fs::read(&elsewhere).expect("read the named file"),
        b"named now, edited\n",
        "edits after Save As land in the named file"
    );
}

/// The scratch name is DETERMINISTIC (core's scratch_note_path is pure and
/// lexical): a second untitled buffer in the same state dir - the next launch,
/// or another instance - reuses the SAME path rather than inventing
/// untitled-2.notes. Two writers on one scratch resolve last-writer-wins by
/// the atomic rename, whole file or nothing, exactly like session.json.
#[test]
fn a_second_untitled_note_reuses_the_same_scratch_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let scratch = notes_core::paths::scratch_note_path(&StateDir(dir.path().to_path_buf()));

    // First launch: type, flush, land the draft.
    let (first, _rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        Settings::default(),
        None,
        None,
    );
    first
        .send(Command::Flush {
            text: "launch one\n".to_string(),
            revision: 1,
            epoch: 0,
        })
        .expect("queued");
    let deadline = Instant::now() + ANSWER;
    while fs::read(&scratch).unwrap_or_default() != b"launch one\n" {
        assert!(Instant::now() < deadline, "the first launch never wrote");
        std::thread::sleep(Duration::from_millis(20));
    }
    first.close().expect("shutdown joins the engine");

    // Second launch on the SAME state dir: an untitled buffer again, and the
    // same path answers - no untitled-2.notes, no new name scheme.
    let (second, _rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        Settings::default(),
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
    let deadline = Instant::now() + ANSWER;
    loop {
        if fs::read(&scratch).unwrap_or_default() == b"launch two\n" {
            break;
        }
        assert!(Instant::now() < deadline, "the second launch never wrote");
        std::thread::sleep(Duration::from_millis(20));
    }
    second.close().expect("shutdown joins the engine");

    // Last writer wins, whole file: the only scratch that exists is the one
    // deterministic path, and no numbered sibling was invented.
    let notes_dir = dir.path().join("notes");
    let names: Vec<String> = fs::read_dir(&notes_dir)
        .expect("list the notes dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        vec!["untitled.notes".to_string()],
        "exactly one deterministic scratch, no invented siblings"
    );
}

/// FINDING 2: the stale flush. The user types in A (the bridge debounces
/// 250 ms), hits Ctrl+O for B inside that window, and the ALREADY-QUEUED
/// Flush{A-text} arrives after the rebind. Without the epoch guard it wrote A's
/// text into B atomically and reported Saved (RED before). With it, the stale
/// Flush is DISCARDED and says so, B stays byte-identical, and the CURRENT
/// document still saves normally afterwards.
#[test]
fn a_stale_flush_never_lands_in_the_file_that_replaced_its_document() {
    let dir = tempfile::tempdir().expect("tempdir");
    let a = dir.path().join("a.notes");
    let b = dir.path().join("b.notes");
    fs::write(&a, b"doc A\n").expect("seed a");
    fs::write(&b, b"doc B\n").expect("seed b");
    let mut app = Harness::at(dir.path().to_path_buf(), Settings::default());
    app._dir = Some(dir);

    app.open(&a); // generation 1
    app.flush(&a, "edited A\n", 2); // lands in A
    app.open_generation(&b); // generation 2: the document is now B

    // The STALE flush: buffered while A was open, delivered after the rebind.
    app.gateway
        .send(Command::Flush {
            text: "edited A\n".to_string(),
            revision: 3,
            epoch: 1,
        })
        .expect("queued");
    let reason = loop {
        match app.rx.recv_timeout(ANSWER) {
            Ok(Event::AutosaveSkipped { reason }) => break reason,
            Ok(_) => {}
            Err(_) => panic!("the discard was silent - a dropped edit must say so"),
        }
    };
    assert_eq!(
        reason,
        SkipReason::Superseded,
        "the discard names itself instead of vanishing"
    );
    assert_eq!(
        fs::read(&b).expect("read b"),
        b"doc B\n",
        "B is byte-identical: the stale text never landed"
    );

    // And the CURRENT document still saves normally afterwards.
    assert_eq!(app.flush(&b, "edited B\n", 1), 1);
    assert_eq!(fs::read(&b).expect("read b again"), b"edited B\n");
}
