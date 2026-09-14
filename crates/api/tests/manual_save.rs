//! THE HAND-TRIGGERED SAVE, proved from outside the port: a bridge sends
//! [`Command::Save`] and the engine answers - EXACTLY ONCE, with one of
//! [`Event::Saved`] (then [`Event::Rebound`]), [`Event::SaveFailed`], or
//! [`Event::AutosaveSkipped`] - and the answer is true about the BYTES ON DISK.
//! Nothing here reaches into the engine or calls core in place of the app: every
//! step is a [`Gateway::send`], an [`Event`], and what fs reports afterwards,
//! because the three claims this file exists for are only visible that way:
//!
//!   1. COUNT, not just kind. A refusal is an answer, so a refused Save may not
//!      be silent; and a Save may not be answered twice (the M5 latch lesson,
//!      moved from the state files to the document path).
//!   2. THE ARMING REACHES THE UI. An in-place Save of a foreign file answers
//!      Saved THEN Rebound with the SAME path and the SAME epoch. A bridge that
//!      read Rebound as "reload the text" would clobber the buffer at the exact
//!      moment the file became armed, so the unchanged pair IS the contract, and
//!      it is pinned here rather than only in a doc comment.
//!   3. NOTHING IS WRITTEN WHEN NOTHING SHOULD BE. Every refusal is checked
//!      against the file, including the case where the disk moved under the
//!      engine - which is how "skipped because Clean" is told apart from "wrote
//!      the stale buffer it was handed".
//!
//! The harness shape follows tests/session.rs (same echo discipline, byte slices
//! compared as bytes). The save_as flow is deliberately NOT copied: SaveAs
//! rebinds identity and moves the epoch, Save does neither, and the event shapes
//! differ on purpose.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use notes_api::{
    Command, Event, FileMeta, Gateway, SaveError, Settings, SkipReason, StateDir, WindowHandle,
};

// thiserror and notes-platform are dependencies of notes-api, not of this test
// target: naming them keeps the unused-crate-dependencies lint honest.
use notes_platform as _;
use thiserror as _;

/// Long enough that a real disk round trip cannot time out on a loaded machine,
/// short enough that a hung engine fails the run instead of hanging the suite.
const ANSWER: Duration = Duration::from_secs(5);

/// The quiet period that ENDS an answer window, deliberately LONGER than the
/// engine idle tick (AUTOSAVE_IDLE, mirrored in tests/session.rs as TICK =
/// 750 ms): a second answer to one Save, or a refusal repeated on every tick,
/// has to arrive INSIDE the window for the count assertion to see it.
const QUIET: Duration = Duration::from_millis(900);

struct Harness {
    gateway: Option<Gateway>,
    rx: Receiver<Event>,
    root: PathBuf,
    /// The generation the engine last announced, mirrored off the wire exactly
    /// as a bridge pump stores it - only from a Loaded or a Rebound, never at
    /// send time. A Save carrying any other number tests a caller that lies.
    epoch: u64,
    _dir: Option<tempfile::TempDir>,
}

impl Harness {
    fn new() -> Self {
        Harness::with_settings(Settings::default())
    }

    fn with_settings(settings: Settings) -> Self {
        let dir = tempfile::tempdir().expect("a temp state dir");
        let root = dir.path().to_path_buf();
        let mut app = Harness::at(root, settings);
        app._dir = Some(dir);
        app
    }

    /// No host, as in session.rs: none of these scenarios is about the window,
    /// and the port owns the platform object now (D46).
    fn at(root: PathBuf, settings: Settings) -> Self {
        let (gateway, rx) = Gateway::start_with_host(StateDir(root.clone()), settings, None, None);
        let _ = gateway.send(Command::RegisterWindow {
            handle: WindowHandle(0x100),
        });
        Harness {
            gateway: Some(gateway),
            rx,
            root,
            epoch: 0,
            _dir: None,
        }
    }

    /// Stop the engine and JOIN it, so a queued state write has landed before a
    /// test reads session.json back off the disk.
    fn shutdown(&mut self) {
        if let Some(gateway) = self.gateway.take() {
            let _ = gateway.close();
        }
        while self.rx.recv_timeout(Duration::from_millis(10)).is_ok() {}
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join(name);
        fs::write(&path, bytes).expect("write the fixture");
        path
    }

    fn bytes(&self, path: &Path) -> Vec<u8> {
        fs::read(path).unwrap_or_default()
    }

    fn send(&self, command: Command) {
        let gateway = self.gateway.as_ref().expect("the engine is running");
        assert!(
            gateway.send(command.clone()).is_ok(),
            "the engine must still be accepting commands"
        );
    }

    fn open(&mut self, path: &Path) -> (String, FileMeta) {
        self.send(Command::Open {
            path: path.to_path_buf(),
        });
        let event = self.until(
            "Loaded",
            |ev| matches!(ev, Event::Loaded { path: p, .. } if p == path),
        );
        match event {
            Event::Loaded {
                text, meta, epoch, ..
            } => {
                self.epoch = epoch;
                (text, meta)
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    /// THE COMMAND UNDER TEST, echoed at the generation the engine announced.
    /// Returns every event between the send and the next quiet period.
    fn save(&mut self, text: &str, revision: u64) -> Vec<Event> {
        let epoch = self.epoch;
        self.save_at_epoch(epoch, text, revision)
    }

    /// ... or at a generation of the test choosing, which is how a stale Save is
    /// played: a bridge that buffered the text under the document before last.
    fn save_at_epoch(&mut self, epoch: u64, text: &str, revision: u64) -> Vec<Event> {
        self.send(Command::Save {
            text: text.to_string(),
            revision,
            epoch,
        });
        self.answer_window()
    }

    /// A debounced flush, for the half of the contract that is about what happens
    /// AFTER the save that armed the file.
    fn flush(&mut self, text: &str, revision: u64) -> Vec<Event> {
        let epoch = self.epoch;
        self.send(Command::Flush {
            text: text.to_string(),
            revision,
            epoch,
        });
        self.answer_window()
    }

    fn answer_window(&mut self) -> Vec<Event> {
        let mut seen = Vec::new();
        loop {
            match self.rx.recv_timeout(QUIET) {
                Ok(event) => {
                    if let Event::Loaded { epoch, .. } | Event::Rebound { epoch, .. } = &event {
                        self.epoch = *epoch;
                    }
                    seen.push(event);
                }
                Err(_) => return seen,
            }
        }
    }

    /// Wait out one idle tick and throw away whatever it announces, so an Open,
    /// its RecentsUpdated and the settings write it armed have ALL landed before
    /// a Save is asked about. Without this, a late event from the open would be
    /// counted against the save.
    fn wait_a_tick(&mut self) {
        std::thread::sleep(QUIET);
        while self.rx.recv_timeout(Duration::from_millis(10)).is_ok() {}
    }

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
                    if let Event::Loaded { epoch, .. } | Event::Rebound { epoch, .. } = &event {
                        self.epoch = *epoch;
                    }
                    if f(&event) {
                        return event;
                    }
                    seen.push(event);
                }
                Err(err) => panic!("never saw {want}; saw {seen:?} first ({err:?})"),
            }
        }
    }
}

/// The three shapes that ARE an answer. Rebound is not one of them - it is the
/// companion of a Saved, asserted separately - which is what makes the count
/// below a claim about verdicts rather than about event traffic.
fn answers(events: &[Event]) -> Vec<&Event> {
    events
        .iter()
        .filter(|ev| {
            matches!(
                ev,
                Event::Saved { .. } | Event::SaveFailed { .. } | Event::AutosaveSkipped { .. }
            )
        })
        .collect()
}

/// EXACTLY ONE answer: the count, not only the kind. Zero is the silence
/// ADR-0001 forbids; two is a verdict the status line would render twice.
fn one_answer(events: &[Event]) -> &Event {
    let found = answers(events);
    assert_eq!(
        found.len(),
        1,
        "one Save, one answer; the window held {events:?}"
    );
    found[0]
}

fn skip(events: &[Event]) -> SkipReason {
    match one_answer(events) {
        Event::AutosaveSkipped { reason } => *reason,
        other => panic!("expected AutosaveSkipped, got {other:?}"),
    }
}

/// A successful Save, asserted as the PAIR it must be: one answer, a Saved
/// naming this path and revision, and exactly one Rebound after it. Returns the
/// Rebound, because what it carries is half the contract.
fn assert_saved<'a>(events: &'a [Event], path: &Path, revision: u64) -> &'a Event {
    match one_answer(events) {
        Event::Saved {
            path: p,
            revision: r,
        } => {
            assert_eq!(p.as_path(), path, "Saved names the file it wrote");
            assert_eq!(*r, revision, "Saved names the revision the command carried");
        }
        other => panic!("expected exactly one Saved, got {other:?} in {events:?}"),
    }
    let mut rebounds = events
        .iter()
        .filter(|ev| matches!(ev, Event::Rebound { .. }));
    let reb = rebounds.next().expect(
        "a successful Save must also answer Rebound, or the arming never reaches the bridge",
    );
    assert!(rebounds.next().is_none(), "one Saved, one Rebound, no more");
    match reb {
        Event::Rebound {
            path: p,
            revision: r,
            ..
        } => {
            assert_eq!(p.as_path(), path, "Rebound names the same file");
            assert_eq!(*r, revision, "Saved and Rebound agree on the revision");
        }
        _ => unreachable!("filtered to Rebound"),
    }
    reb
}

fn set_read_only(path: &Path, on: bool) {
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_readonly(on);
    fs::set_permissions(path, perms).expect("toggle the read-only bit");
}

#[test]
fn a_note_the_user_asked_to_save_is_saved_even_with_autosave_switched_off() {
    let settings = Settings {
        autosave_enabled: false,
        ..Settings::default()
    };
    let mut app = Harness::with_settings(settings);
    let note = app.file("idea.notes", b"the first draft");
    let (_text, meta) = app.open(&note);
    assert!(meta.armed, "a native note opens armed");
    app.wait_a_tick();

    let events = app.save("typed while autosave was off", 1);
    assert_saved(&events, &note, 1);
    assert_eq!(
        app.bytes(&note),
        b"typed while autosave was off",
        "the answer is Saved and the bytes agree with it"
    );
}

#[test]
fn an_explicit_save_arms_a_foreign_file_in_place_and_the_bridge_is_told() {
    let mut app = Harness::new();
    let md = app.file("readme.md", b"a foreign file");
    let (_text, meta) = app.open(&md);
    assert!(!meta.armed, "ADR-0001: a foreign file opens disarmed");
    let generation = app.epoch;
    app.wait_a_tick();

    let events = app.save("a foreign file, saved by hand", 1);
    let reb = assert_saved(&events, &md, 1);
    match reb {
        Event::Rebound {
            meta, epoch, path, ..
        } => {
            assert_eq!(path.as_path(), &md, "the path did NOT move");
            assert!(
                meta.armed,
                "the save armed it, and this is the only channel by which that fact can reach the status line"
            );
            assert_eq!(
                *epoch, generation,
                "an in-place save is not a new generation: a bridge that reloaded on Rebound would clobber the buffer it just armed"
            );
        }
        _ => unreachable!("assert_saved filtered to Rebound"),
    }
    assert_eq!(app.bytes(&md), b"a foreign file, saved by hand");

    let later = app.flush("autosave writes the next edit without asking", 2);
    assert!(
        matches!(one_answer(&later), Event::Saved { .. }),
        "armed now, so the debounced path owns the file: {later:?}"
    );
    assert_eq!(
        app.bytes(&md),
        b"autosave writes the next edit without asking",
        "and the next edit lands without anyone asking again"
    );
}

#[test]
fn a_read_only_note_refuses_an_explicit_save_by_name_and_changes_no_bytes() {
    let mut app = Harness::new();
    let locked = app.file("locked.md", b"the file as it stands");
    set_read_only(&locked, true);
    let (_text, meta) = app.open(&locked);
    assert!(
        meta.read_only,
        "Loaded has to say it, or the refusal is luck"
    );
    let before = app.bytes(&locked);
    app.wait_a_tick();

    let events = app.save("an edit the disk will not accept", 1);
    assert_eq!(skip(&events), SkipReason::ReadOnly, "its own named verdict");
    assert_eq!(before, app.bytes(&locked), "a refusal wrote nothing");
    set_read_only(&locked, false);
}

#[test]
fn an_oversize_file_is_refused_at_open_so_no_save_can_rewrite_it() {
    // Skip::Oversize is a refusal core keeps on this path too (its own test is
    // an_oversize_document_refuses_even_an_explicit_save), and the port-level
    // truth is the stronger one in front of it: an over-guard file never becomes
    // a Document at all, so a Save cannot even be aimed at it. Asserted here
    // because a Save that DID land on the refused name would be the measured
    // 0-byte overwrite all over again.
    let mut app = Harness::new();
    let big = app.root.join("big.notes");
    let size = 8_usize * 1024 * 1024;
    fs::write(&big, vec![b'A'; size + 1]).expect("write the over-guard fixture");
    app.wait_a_tick();

    app.send(Command::Open { path: big.clone() });
    app.until(
        "LoadFailed(TooLarge)",
        |ev| matches!(ev, Event::LoadFailed { path, .. } if path == &big),
    );

    let events = app.save("text from the note that is actually open", 1);
    let scratch = notes_core::paths::scratch_note_path(&StateDir(app.root.clone()));
    assert_saved(&events, &scratch, 1);
    let bytes = app.bytes(&big);
    assert_eq!(bytes.len(), size + 1, "the refused file kept every byte");
    assert!(
        bytes.iter().all(|b| *b == b'A'),
        "and not one of them changed"
    );
}

#[test]
fn an_untouched_buffer_is_refused_clean_and_no_file_appears() {
    // The D69 empty-buffer rule holds on this path too: an accidental Ctrl+S on
    // a note nobody has typed into must not mint a scratch file.
    let mut app = Harness::new();
    let scratch = notes_core::paths::scratch_note_path(&StateDir(app.root.clone()));
    assert!(!scratch.exists());

    let events = app.save("", 0);
    assert_eq!(
        skip(&events),
        SkipReason::Clean,
        "nothing unsaved is nothing to write"
    );
    assert!(
        !scratch.exists(),
        "and the refusal created no file behind its own back"
    );
}

#[test]
fn an_unchanged_second_save_writes_nothing_even_after_an_outside_edit() {
    let mut app = Harness::new();
    let md = app.file("notes.md", b"first bytes");
    app.open(&md);
    app.wait_a_tick();
    let first = app.save("edited once, saved once", 1);
    assert_saved(&first, &md, 1);

    // Somebody else edits the file behind the engine back. The BUFFER has not
    // moved since the save, so a Save now must refuse: the anchor is what tells
    // "nothing to write" from "write the buffer anyway", and writing here would
    // silently discard the other edit.
    fs::write(&md, b"somebody else edited this file").expect("outside edit");
    let events = app.save("edited once, saved once", 1);
    assert_eq!(skip(&events), SkipReason::Clean);
    assert_eq!(
        app.bytes(&md),
        b"somebody else edited this file",
        "a refused Save wrote NOTHING, not even the buffer it was handed"
    );
}

#[test]
fn an_untitled_note_is_bound_to_its_scratch_by_one_save_without_moving_the_epoch() {
    let mut app = Harness::new();
    let scratch = notes_core::paths::scratch_note_path(&StateDir(app.root.clone()));
    assert!(
        !scratch.exists(),
        "the scenario starts with no scratch file"
    );
    let generation = app.epoch;
    app.wait_a_tick();

    let events = app.save("the first thing typed in this session", 1);
    let reb = assert_saved(&events, &scratch, 1);
    match reb {
        Event::Rebound { meta, epoch, .. } => {
            assert_eq!(
                *epoch, generation,
                "acquiring a file is not a new generation"
            );
            assert!(
                meta.armed,
                "the note is armed for autosave from this moment"
            );
        }
        _ => unreachable!("assert_saved filtered to Rebound"),
    }
    let saved_bytes = app.bytes(&scratch);
    let mut expected = b"the first thing typed in this session".to_vec();
    expected.push(10_u8); // and the trailing LF the new-file rule adds
    assert_eq!(
        saved_bytes, expected,
        "the buffer is whole; the LF is the new-file rule, not a dropped byte"
    );
    // The claim ADR-0007 actually makes is that api owns the no-path branch BY
    // REUSING that arm - so the check is not a byte literal copied from this
    // scenario but the two binds compared, which goes red the day the
    // hand-triggered path grows its own version of the scratch answer.
    let mut debounced = Harness::new();
    let other = notes_core::paths::scratch_note_path(&StateDir(debounced.root.clone()));
    let via_flush = debounced.flush("the first thing typed in this session", 1);
    assert!(
        matches!(one_answer(&via_flush), Event::Saved { .. }),
        "the debounced path binds the scratch too: {via_flush:?}"
    );
    assert_eq!(
        saved_bytes,
        debounced.bytes(&other),
        "one scratch rule, one byte shape: Save and Flush may not drift"
    );
    debounced.shutdown();
    assert!(
        !events
            .iter()
            .any(|ev| matches!(ev, Event::RecentsUpdated(entries) if entries
                .iter()
                .any(|e| e.path == scratch))),
        "the scratch is bound for restore, not remembered as a chosen file"
    );

    app.shutdown();
    let session = notes_core::session::read_session(&app.root).expect("the shutdown write landed");
    assert_eq!(
        session.path.as_deref(),
        Some(scratch.as_path()),
        "and the next launch reopens this same note"
    );
}

#[test]
fn a_stale_save_is_discarded_as_superseded_and_writes_nothing() {
    // Save carries Flush shape FOR this case: it writes the CURRENT path with
    // the buffer it was handed, so the echoed generation is the only stale-write
    // defense on the path (SaveAs could not go stale the same way - it rebinds,
    // and the rebind is itself the answer).
    let mut app = Harness::new();
    let a = app.file("a.md", b"the first document");
    let b = app.file("b.md", b"the second document");
    app.open(&a);
    let stale = app.epoch;
    app.open(&b);
    assert_ne!(app.epoch, stale, "opening a document moves the generation");
    let before_b = app.bytes(&b);
    app.wait_a_tick();

    let events = app.save_at_epoch(stale, "a buffer from the document before this one", 5);
    assert_eq!(
        skip(&events),
        SkipReason::Superseded,
        "discarded, and said out loud rather than dropped"
    );
    assert_eq!(
        before_b,
        app.bytes(&b),
        "the current file did not get the stale text"
    );
    assert_eq!(
        app.bytes(&a),
        b"the first document",
        "nor did the file the stale buffer actually belongs to"
    );
}

#[test]
fn a_save_the_disk_refuses_answers_save_failed_naming_the_file_and_writes_nothing() {
    // The third shape, and the one that cannot be reached from core: the gate
    // said write, the OS said no. This is the half of "exactly one answer" that
    // api owns rather than core, and the reason the answer must be a
    // SaveFailed with a real path and revision instead of a toast the port
    // invented words for.
    let mut app = Harness::new();
    let md = app.file("notes.md", b"the bytes on disk");
    app.open(&md);
    app.wait_a_tick();
    set_read_only(&md, true);

    let events = app.save("an edit the write will not fit", 1);
    match one_answer(&events) {
        Event::SaveFailed {
            path,
            revision,
            reason,
        } => {
            assert_eq!(path.as_path(), &md, "it names the file it tried");
            assert_eq!(
                *revision, 1,
                "and the revision that write would have anchored"
            );
            assert!(
                !reason.to_string().is_empty(),
                "with a reason the UI can render"
            );
        }
        other => panic!("expected exactly one SaveFailed, got {other:?} in {events:?}"),
    }
    assert_eq!(
        app.bytes(&md),
        b"the bytes on disk",
        "and the file is as it was"
    );
    set_read_only(&md, false);
}

#[test]
fn an_in_place_save_does_not_re_age_the_recent_files_list() {
    // Flush parity, and it is not an oversight: an in-place Save is not an Open,
    // so it does not call remember(). The observable cost of doing it anyway is a
    // menu that blinks on every keystroke and a settings.toml rewritten each time.
    let mut app = Harness::new();
    let md = app.file("readme.md", b"a foreign file");
    app.open(&md);
    app.wait_a_tick();
    let settings_path = app.root.join("settings.toml");
    let settled = app.bytes(&settings_path);
    assert!(
        !settled.is_empty(),
        "the Open must have persisted a recents list"
    );

    let events = app.save("a foreign file, saved by hand", 1);
    assert_saved(&events, &md, 1);
    assert!(
        !events
            .iter()
            .any(|ev| matches!(ev, Event::RecentsUpdated(_))),
        "a Save that re-membered would blink the menu: {events:?}"
    );
    app.wait_a_tick();
    assert_eq!(
        app.bytes(&settings_path),
        settled,
        "and the queued settings write that would follow it never happened"
    );
}

/// THE GUARD CORE CANNOT SEE. A refused load leaves load_refused_for naming the
/// file, moves no epoch and replaces no Document - so a Save of the buffer that
/// really is on screen passes both guards a Save has AND cores rule, and the
/// write would destroy bytes the app explicitly declined to read. ADR-0007 says
/// api still applies the refused-load guard on this path; this walks the
/// sequence in the order the review named, and fails if the line is removed.
#[test]
fn a_save_cannot_write_the_file_whose_open_was_just_refused() {
    let mut app = Harness::new();
    let note = app.file("growing.md", b"a small file");
    app.open(&note);
    let generation = app.epoch;
    let first = app.save("a small file, edited by hand", 1);
    assert_saved(&first, &note, 1);

    // Somebody else fills the file in, past the D9 guard: the next Open of THAT
    // name is refused at the stat, which is how a refusal comes to name the file
    // already on screen. No epoch bump, no new Document - the state a Save must
    // not be able to write into.
    let size = 8_usize * 1024 * 1024;
    fs::write(&note, vec![b'A'; size + 1]).expect("grow the file past the guard");
    app.send(Command::Open { path: note.clone() });
    app.until(
        "LoadFailed(TooLarge)",
        |ev| matches!(ev, Event::LoadFailed { path, .. } if path == &note),
    );
    assert_eq!(app.epoch, generation, "a refusal moves no generation");
    app.wait_a_tick();

    let events = app.save("text that would replace eight megabytes nobody read", 2);
    match one_answer(&events) {
        Event::SaveFailed {
            path,
            revision,
            reason,
        } => {
            assert_eq!(
                path.as_path(),
                &note,
                "it names the file it refused to write"
            );
            assert_eq!(
                *revision, 2,
                "and the revision that write would have anchored"
            );
            assert_eq!(
                *reason,
                SaveError::NoTarget,
                "the same verdict Save As states for the same refusal"
            );
        }
        other => panic!("expected one SaveFailed, got {other:?} in {events:?}"),
    }
    let bytes = app.bytes(&note);
    assert_eq!(bytes.len(), size + 1, "the refused file kept every byte");
    assert!(
        bytes.iter().all(|b| *b == b'A'),
        "and not one of them changed"
    );
}
