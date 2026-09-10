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
    /// Held so the directory outlives every assertion; None when the harness is
    /// resuming a directory that some other owner already keeps alive.
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
        let (gateway, rx) = Gateway::start(StateDir(root.clone()), settings);
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

    fn open(&mut self, path: &Path) -> (String, FileMeta) {
        self.send(Command::Open {
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
        self.send(Command::Flush {
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
        self.send(Command::Flush {
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

    /// Save As carries the text (D30): the engine holds no buffer of its own, so
    /// this is the moment the command's own text field earns its keep. Also waits
    /// for [`Event::Rebound`], which is the point of the variant.
    fn save_as(&mut self, path: &Path, text: &str, revision: u64) -> u64 {
        self.send(Command::SaveAs {
            path: path.to_path_buf(),
            text: text.to_string(),
            revision,
        });
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
    app.send(Command::SetPinned(true));
    app.send(Command::GeometryChanged { rect });
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
    app.send(Command::Open { path: gone.clone() });
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
    other.send(Command::Open { path: same.clone() });
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

    app.send(Command::Open { path: big.clone() });
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

/// M9: a brand-new note that has never been saved is not an error.
#[test]
fn a_note_without_a_path_skips_instead_of_reporting_an_invented_error() {
    let mut app = Harness::new();
    assert_eq!(
        app.flush_skipped("the first draft", 1),
        SkipReason::NeedsPath,
        "the user's next move is Save As, so the reason must say that"
    );
    // SkipReason carries no copy by design, so what M9 pins is that the engine
    // answered with a named variant instead of a string it invented at the call site.
    assert!(format!("{:?}", notes_api::SkipReason::NeedsPath).contains("NeedsPath"));
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
    app.send(Command::Open { path: path.clone() });
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
    again.open(&other);
    assert_eq!(
        again.flush_skipped("typed", 1),
        SkipReason::AutosaveDisabled,
        "settings.toml owned the toggle across the restart, not the caller's default"
    );
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
    assert!(!vanished.exists, "and it is greyed out");
    assert!(list.len() <= 10, "D13 cap");
}
