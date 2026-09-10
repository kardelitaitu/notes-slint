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
    Command, Encoding, Event, FileMeta, Gateway, LineEnding, Rect, SaveError, Settings, SkipReason,
    StateDir, WindowHandle,
};
// The session file is read back with core's own parser: what is being asserted is
// that the engine wrote a file CORE can read, not one whose bytes we guessed.
use notes_core::session::{FILE_NAME, read_session};

// thiserror is a dependency of notes-api, not of this test target; naming it
// keeps the unused-crate-dependencies lint honest about that.
use thiserror as _;

/// Long enough that a real disk round trip cannot time out on a loaded machine,
/// short enough that a hung engine fails the run instead of hanging it forever.
const ANSWER: Duration = Duration::from_secs(5);

/// A running engine, its events, and the directory it writes into.
///
/// The TempDir is a field rather than a local so the files outlive every
/// assertion, and each test gets its own so no run can see another session.json.
struct Harness {
    gateway: Gateway,
    rx: Receiver<Event>,
    root: PathBuf,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temp state dir");
        let root = dir.path().to_path_buf();
        let (gateway, rx) = Gateway::start(StateDir(root.clone()), Settings::default());
        // 5.5 step 3, once per scenario: the window exists before anything is
        // asked of it, so the engine is in the state a bridge would leave it in.
        gateway.send(Command::RegisterWindow {
            handle: WindowHandle(0x100),
        });
        Harness {
            gateway,
            rx,
            root,
            _dir: dir,
        }
    }

    /// Writes a fixture file into the state directory and returns its path.
    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join(name);
        fs::write(&path, bytes).expect("write the fixture file");
        path
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

    fn open(&mut self, path: &Path) -> (String, FileMeta) {
        self.gateway.send(Command::Open {
            path: path.to_path_buf(),
        });
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
        self.gateway.send(Command::Flush {
            text: text.to_string(),
            revision,
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
        self.gateway.send(Command::Flush {
            text: text.to_string(),
            revision,
        });
        match self.until("a skip", |ev| {
            matches!(ev, Event::AutosaveSkipped { .. } | Event::Saved { .. })
        }) {
            Event::AutosaveSkipped { reason } => reason,
            other => panic!("expected AutosaveSkipped, got {other:?}"),
        }
    }

    fn save_as(&mut self, path: &Path) -> u64 {
        self.gateway.send(Command::SaveAs {
            path: path.to_path_buf(),
        });
        match self.until("Saved", |ev| match ev {
            Event::Saved { path: p, .. } => p == path,
            Event::SaveFailed { path: p, .. } => p == path,
            _ => false,
        }) {
            Event::Saved { revision, .. } => revision,
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
        app.save_as(&target),
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
    app.gateway.send(Command::SaveAs {
        path: nowhere.clone(),
    });
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
    let mut app = Harness::new();
    let rect = Rect::new(40, 24, 1024, 700);
    app.gateway.send(Command::SetPinned(true));
    app.gateway.send(Command::GeometryChanged { rect });
    // Shutdown drains, and the final session write happens INSIDE the drain -
    // which is what makes the assertions below possible at all.
    app.gateway.send(Command::Shutdown);
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
    let (again, _rx) = Gateway::start(StateDir(app.root.clone()), Settings::default());
    let initial = again.initial_state();
    assert_eq!(initial.session.rect, rect);
    assert!(
        initial.pinned,
        "the bridge applies topmost from this read, not from guesswork"
    );
    drop(again);
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

    app.gateway.send(Command::ClearRecents);
    app.until(
        "an empty RecentsUpdated",
        |ev| matches!(ev, Event::RecentsUpdated(entries) if entries.is_empty()),
    );

    // Opening a file that vanished fails with renderable copy: the variant this
    // slice added, because the frozen vocabulary had nowhere honest for it.
    app.gateway.send(Command::Open { path: gone.clone() });
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
