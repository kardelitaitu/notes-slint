//! The engine loop: one thread, one receiver, and the disk behind both.
//!
//! This file owns the loop, the drain/abort asymmetry, the reentrancy trap, the
//! deadline seam, and the three file arms (Open, SaveAs, Flush). It is still not a
//! domain layer: every rule it applies lives in notes-core, and what happens here
//! is CALLING - detect/decode for the do-no-harm round trip (D14, §4.5),
//! [`Document`] for armed/dirty/read-only and the fixed skip order (ADR-0001,
//! D11), [`save_document_revision`] for the atomic write (D12),
//! [`write_session`] for session.json, [`notes_core::recent`] for the MRU
//! (D13), [`notes_core::format`] for .notes frontmatter. Where a decision would
//! have needed core and core has no function for it yet, the code says so at the
//! site (see the revision-space note in [`Engine::flush`]).
//!
//! Why a thread at all: autosave fires seconds after any UI call, so it cannot be
//! answered by the call that triggered it (§5.4). One engine thread owns the state
//! and performs every read and write; the UI thread owns an
//! [`EventRx`](crate::EventRx) and never runs engine code. Because both queues
//! are unbounded (D24), a 400 ms save cannot block a frame in either direction.

use std::cell::Cell;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use notes_core::session::write_session;
use notes_core::{
    DecodeError, Detected, Document, FileKind, LineEnding as CoreLineEnding, NoteParts,
    SaveError as CoreSaveError, Session, SessionError, Skip, StateDir, TextEncoding,
    classify_io_error, clear as clear_recents, decode, detect, is_notes_path, is_oversize,
    push as push_recent, rebuild, save_document_revision, split,
};

use crate::command::{Command, WindowHandle};
use crate::event::{
    Encoding, Event, FileMeta, LineEnding, LoadError, RecentEntry, SaveError, SkipReason,
};
use crate::gateway::{EventTx, Settings};

/// The autosave idle cadence, in its temporary home.
///
/// It lives HERE rather than in a bridge timer on purpose: a UI timer dies with
/// the window, and then save cadence has quietly become a UI concern. M4 changes
/// the POLICY inside the tick arm - when to flush - not this structure, and a
/// settings-loaded interval replaces this constant once settings.rs lands. Until
/// then the cadence is fixed and the tick's job is the session write.
const AUTOSAVE_IDLE: Duration = Duration::from_millis(750);

/// How many commands the shutdown drain will process, at most. Why the budget is
/// a count rather than a duration, and what happens when it runs out, is written
/// down at [`Engine::drain`].
const MAX_DRAIN: usize = 4_096;

/// The format of a file that does not exist yet: nothing to preserve, so the
/// format the product writes by default - UTF-8, no BOM, LF, and a trailing
/// newline, because a text file whose last line is unterminated earns a warning
/// in every other editor.
fn new_file_detected() -> Detected {
    Detected {
        encoding: TextEncoding::Utf8,
        line_ending: CoreLineEnding::Lf,
        trailing_newline: true,
        bom_present: false,
    }
}

thread_local! {
    /// Set when an engine loop starts, on that thread only. See
    /// [`on_engine_thread`].
    static ENGINE_THREAD: Cell<bool> = const { Cell::new(false) };
}

/// True when the calling thread IS an engine thread.
///
/// [`Gateway::send`],
/// [`Gateway::startup_state`] and [`Gateway::drop`] assert against it:
/// port would queue a command behind itself and then wait on its own queue. Now
/// that this thread does I/O, there is a second reason: a slow disk makes the
/// deadlock intermittent rather than immediate.
pub(crate) fn on_engine_thread() -> bool {
    ENGINE_THREAD.with(|flag| flag.get())
}

/// Arms the trap on the current thread; disarms when the returned guard drops.
///
/// Hidden from the docs, and pub for one reason: a trap that cannot be observed
/// firing is indistinguishable from one that was never wired, and
/// tests/reentrancy.rs is a separate crate that has to stand on an engine thread to
/// prove the assert is live. Only [`Engine::run`] and that test call this.
#[must_use = "the trap is armed only while the returned guard is alive"]
#[doc(hidden)]
pub fn mark_current_thread_as_engine() -> EngineGuard {
    EngineGuard(ENGINE_THREAD.with(|flag| flag.replace(true)))
}

/// Restores the previous engine-thread marking.
#[doc(hidden)]
pub struct EngineGuard(bool);

impl Drop for EngineGuard {
    fn drop(&mut self) {
        ENGINE_THREAD.with(|flag| flag.set(self.0));
    }
}

/// Observation point for [`Engine::run`]'s arming line. Test-only, because what
/// it proves is unobservable from outside: thread-locals are not inherited, so no
/// other thread can ask an engine thread whether it is marked, and a test that
/// arms the flag on a thread IT spawned proves the mechanism but not the wiring.
/// [`Engine::run`] records what its own thread sees; the test joins first.
#[cfg(test)]
pub(crate) mod latch_probe {
    use std::sync::atomic::{AtomicBool, Ordering};

    static ARMED: AtomicBool = AtomicBool::new(false);

    pub(crate) fn record(armed: bool) {
        ARMED.store(armed, Ordering::SeqCst);
    }

    /// Reads and resets, so a second run() on any thread has to re-arm to pass.
    pub(crate) fn take() -> bool {
        ARMED.swap(false, Ordering::SeqCst)
    }
}

/// Whether the loop keeps going after handling a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Continue,
    Exit,
}

/// The engine thread's entire state.
///
/// Moved into one thread and never handed back, so nothing here is shared and
/// nothing is locked. That is what makes it survivable to do file I/O here: the
/// engine can be slow, but it cannot contend, and it cannot be called back into.
pub(crate) struct Engine {
    cmd_rx: mpsc::Receiver<Command>,
    /// None means "nobody is listening any more" - see [`Self::emit`].
    event_tx: Option<Sender<Event>>,
    /// Where session.json lives. D-STATE: the caller resolved it, this thread only
    /// writes into it.
    state_dir: StateDir,
    /// The in-memory session, seeded from session.json by the spawner. Geometry and
    /// the pin bit are written HERE and nowhere else (D10), and the last-open path
    /// travels with them because §5.5 restores the window's document.
    session: Session,
    /// The global auto-save toggle (settings.toml in the shipped app). NOT
    /// per-document arming, which is [`Document`]'s field - D10's discipline of
    /// one home per bit, applied to a second flag.
    autosave_enabled: bool,
    /// §5.5 step 3: the handle the bridge registered. Stored, never used -
    /// topmost goes through notes-platform, which this crate may not import.
    window: Option<WindowHandle>,
    /// Revision, dirty, armed, read-only, oversize and the fixed order of the skip
    /// reasons: all of it core's judgement. This engine supplies the facts the
    /// bridge sent and records what happened.
    doc: Document,
    /// What the file's own bytes said they were, which IS the write-back
    /// instruction (§4.5, D14). The text alone cannot reproduce the file.
    detected: Detected,
    /// The frontmatter block lifted off the text at load, put back at save.
    /// [`Self::body_for_ui`] and [`Self::text_for_disk`] are its only readers.
    frontmatter: Option<String>,
    /// NO TEXT IS HELD HERE. The bridge owns the buffer (§5.5), and both commands
    /// that write carry the text they mean. An engine-side snapshot was a second,
    /// lagging copy of the document: a Flush is debounced, so Save As against it
    /// could write stale text to a new path while the editor showed something
    /// else. That is data loss with a nicer name, so the command vocabulary grew
    /// the missing fields (D30) and the fields below disappeared with them.
    /// The last revision that reached disk. D11's rule in one u64: a Flush at or
    /// below it is Clean and no write is attempted.
    last_saved_revision: u64,
    /// Coalesced "session.json needs writing": a bool, not a queue, so 65 geometry
    /// updates before the next tick cost ONE write (D11's spirit - no disk churn).
    pending_session_write: bool,
    /// D13's MRU, most-recent-first and capped by [`notes_core::recent`] itself.
    /// Held in core's entry type so the identity rule is never re-implemented
    /// here; it becomes a port type only at [`Self::emit_recent`].
    recents: Vec<notes_core::RecentEntry>,
    deadline: Instant,
}

impl Engine {
    /// Builds the loop's state from what [`Gateway::start`] already read.
    pub(crate) fn new(
        cmd_rx: mpsc::Receiver<Command>,
        event_tx: EventTx,
        state_dir: StateDir,
        session: Session,
        settings: Settings,
    ) -> Self {
        // The session's last document is RESTORED as state, not read from disk:
        // no bytes are touched here, and whether to Open it is the bridge's call
        // (§5.5). Recents start empty because their store is settings.toml, which
        // the settings slice owns; the first Open or Save As repopulates the list
        // this build reports.
        Engine {
            cmd_rx,
            event_tx: Some(event_tx),
            state_dir,
            autosave_enabled: settings.autosave_enabled,
            window: None,
            doc: match session.path.as_ref() {
                Some(path) => Document::open(path, file_kind(path), false, false),
                None => Document::new(),
            },
            detected: new_file_detected(),
            frontmatter: None,
            last_saved_revision: 0,
            pending_session_write: false,
            recents: Vec::new(),
            deadline: Instant::now() + AUTOSAVE_IDLE,
            session,
        }
    }

    /// The one engine thread. Arms the reentrancy trap for as long as it runs.
    pub(crate) fn run(mut self) {
        let _armed = mark_current_thread_as_engine();
        // This line is the probe's ONLY writer, and it runs on the loop's own
        // thread, so a true reading can only mean the real engine thread armed the
        // latch. Delete the marking call above and
        // the_real_engine_thread_arms_the_latch fails - which is the point of it.
        #[cfg(test)]
        latch_probe::record(on_engine_thread());
        loop {
            match self.receive() {
                Ok(command) => {
                    if self.handle(command) == Flow::Exit {
                        break;
                    }
                }
                // The tick arm: the session write's other home, and where M4's
                // periodic-flush policy will go.
                Err(RecvTimeoutError::Timeout) => self.on_tick(),
                // The ABORT path: Gateway::drop took the last Sender. recv hands
                // over everything already queued before reporting Disconnected,
                // so no accepted command - and no accepted write - is stranded.
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        // Dropping self.event_tx is how the caller's EventRx learns the engine is
        // finished: buffered events stay readable, then Disconnected.
    }

    /// Blocks for the next command, but never past [`Self::next_deadline`].
    fn receive(&self) -> Result<Command, RecvTimeoutError> {
        match self.next_deadline() {
            Some(at) => {
                let now = Instant::now();
                // A deadline at or behind now would spin recv_timeout at 100% CPU;
                // on_tick always moves it forward.
                if at <= now {
                    return Err(RecvTimeoutError::Timeout);
                }
                self.cmd_rx.recv_timeout(at - now)
            }
            None => self
                .cmd_rx
                .recv()
                .map_err(|_| RecvTimeoutError::Disconnected),
        }
    }

    /// When the tick arm should next run; None means "wait for a command forever".
    ///
    /// Fixed today, and a method because M4 makes it depend on the document: a
    /// dirty buffer, the debounce and a loaded interval all move it.
    fn next_deadline(&self) -> Option<Instant> {
        Some(self.deadline)
    }

    fn on_tick(&mut self) {
        self.flush_session();
        self.deadline = Instant::now() + AUTOSAVE_IDLE;
    }

    /// Handles one command. Only [`Command::Shutdown`] returns [`Flow::Exit`].
    fn handle(&mut self, command: Command) -> Flow {
        match command {
            Command::RegisterWindow { handle } => {
                // §5.5 step 3. Stored, and nothing emitted: no UI is waiting to
                // hear its own registration back, and there is no platform call
                // this crate is allowed to make with the handle.
                self.window = Some(handle);
            }
            Command::GeometryChanged { rect } => {
                if self.session.rect != rect {
                    self.session.rect = rect;
                    self.queue_session_write();
                }
                // The same rect twice changes nothing, so it queues nothing:
                // pinned by repeated_geometry_at_one_rect_queues_one_update.
            }
            Command::SetAutosave(on) => {
                // The global toggle, answered by the menu's own check mark. The
                // per-document half is ADR-0001's and lives on Document.
                self.autosave_enabled = on;
            }
            Command::SetPinned(on) => {
                if self.session.pinned != on {
                    self.session.pinned = on;
                    self.queue_session_write();
                }
            }
            Command::ClearRecents => {
                // core's own clear, so the cap and the entry type stay core's
                // business even for the empty case.
                self.recents = clear_recents();
                self.emit_recent();
            }
            Command::Open { path } => self.open(&path),
            Command::SaveAs {
                path,
                text,
                revision,
            } => self.save_as(&path, &text, revision),
            Command::Flush { text, revision } => self.flush(text, revision),
            Command::Shutdown => {
                self.drain();
                return Flow::Exit;
            }
        }
        Flow::Continue
    }

    /// Open: stat first, read second, decode last - and never at all when the
    /// file is over the guard.
    ///
    /// The order is the cold-start rule (whitepaper §2): [`is_oversize`] answers
    /// from the length, so a 9 MiB file is decided on the stat without reading,
    /// allocating or decoding anything. Per D9 nothing is refused outright - the
    /// document still opens, and [`FileMeta::oversize`] plus
    /// [`FileMeta::read_only`] carry the verdict, which is what
    /// [`LoadError::TooLarge`] is for on the paths that cannot present an empty
    /// read-only buffer at all.
    fn open(&mut self, path: &Path) {
        let on_disk = match fs::metadata(path) {
            Ok(meta) => meta,
            Err(err) => {
                self.fail_load(path, &err);
                return;
            }
        };
        let oversize = is_oversize(usize::try_from(on_disk.len()).unwrap_or(usize::MAX));
        if oversize {
            // D9: the file is opened, not refused, and the text is deliberately
            // not decoded - so the buffer is empty and read-only rather than
            // half-loaded. A half-loaded buffer is the dangerous option: the next
            // autosave would write a truncated file over the user's real one.
            self.doc = Document::open(path, file_kind(path), true, true);
            self.frontmatter = None;
            self.last_saved_revision = 0;
            self.emit(Event::Loaded {
                path: path.to_path_buf(),
                text: String::new(),
                meta: FileMeta {
                    encoding: Encoding::Utf8,
                    line_ending: LineEnding::Lf,
                    trailing_newline: false,
                    read_only: true,
                    oversize: true,
                    armed: self.doc.is_armed(),
                },
            });
            self.remember(path);
            return;
        }
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) => {
                self.fail_load(path, &err);
                return;
            }
        };

        // The ANSI default is None on purpose: the system codepage is a
        // GetACP-style platform call and api may not import notes-platform. The
        // settings slice replaces this with a loaded codepage.
        let detected = detect(&bytes, None);
        let raw = match decode(&bytes, detected) {
            Ok(text) => text,
            Err(err) => {
                self.emit(Event::LoadFailed {
                    path: path.to_path_buf(),
                    reason: LoadError::Undecodable {
                        byte_offset: decode_offset(&err),
                        encoding_hint: Some(api_encoding(detected.encoding)),
                    },
                });
                return;
            }
        };

        // §5.5's do-no-harm chain, all of it core's: detect once, strip
        // frontmatter for the UI, remember it for the write-back.
        self.doc = Document::open(
            path,
            file_kind(path),
            on_disk.permissions().readonly(),
            false,
        );
        self.detected = detected;
        let body = self.body_for_ui(&raw);
        self.last_saved_revision = 0;
        self.emit(Event::Loaded {
            path: path.to_path_buf(),
            text: body,
            meta: FileMeta {
                encoding: api_encoding(detected.encoding),
                line_ending: api_line_ending(detected.line_ending),
                trailing_newline: detected.trailing_newline,
                read_only: on_disk.permissions().readonly(),
                oversize: false,
                // ADR-0001, read from the one place that decides it: a .notes file
                // arms on open, a foreign file does not, and a freshly opened
                // document never inherits the arming of the one before it.
                armed: self.doc.is_armed(),
            },
        });
        self.remember(path);
    }

    /// A load that never got as far as decoding. The Windows error-code knowledge
    /// is core's ([`classify_io_error`] is what separates a sharing violation
    /// from a denied ACL); the port only shapes it for the UI.
    fn fail_load(&mut self, path: &Path, err: &std::io::Error) {
        self.emit(Event::LoadFailed {
            path: path.to_path_buf(),
            reason: load_error_from_io(err),
        });
    }

    /// Save As: write the snapshot at the chosen path, then rebind AND arm.
    ///
    /// Requirement 4 of ADR-0001 is [`Document::save_as`]'s job; this function
    /// calls it and reports. Choosing a location is an explicit act, so a second
    /// explicit save would be pedantic - which is also why the armed flag flips
    /// Save As: write THE TEXT IT WAS GIVEN at the chosen path, then rebind and
    /// arm.
    ///
    /// Requirement 4 of ADR-0001 is [`Document::save_as`]'s job; this function
    /// calls it and reports. Choosing a location is an explicit act, so a second
    /// explicit save would be pedantic - which is also why the armed flag flips
    /// here and nowhere else in the port.
    ///
    /// Two events, in this order: [`Event::Saved`] because the write happened at
    /// a revision (D11's stream must not gain a hole), then [`Event::Rebound`]
    /// because the file the UI describes is now a different one - new path, new
    /// arming, and the TARGET's encoding rather than the source's. A failed Save
    /// As emits only [`Event::SaveFailed`]: nothing was rebound.
    fn save_as(&mut self, path: &Path, text: &str, revision: u64) {
        // If the target already exists, ITS bytes win: overwriting a UTF-16 file
        // with the source's UTF-8 is exactly the silent change §4.5 forbids.
        let detected = existing_detected(path);
        let disk_text = self.text_for_disk(text);
        match self.write(path, &disk_text, detected, revision) {
            Ok(()) => {
                self.doc.save_as(path);
                self.detected = detected;
                self.last_saved_revision = revision;
                let read_only = fs::metadata(path).is_ok_and(|meta| meta.permissions().readonly());
                self.emit(Event::Saved {
                    path: path.to_path_buf(),
                    revision,
                });
                self.emit(Event::Rebound {
                    path: path.to_path_buf(),
                    meta: FileMeta {
                        encoding: api_encoding(detected.encoding),
                        line_ending: api_line_ending(detected.line_ending),
                        trailing_newline: detected.trailing_newline,
                        read_only,
                        oversize: false,
                        // Read back from Document, never copied from the previous
                        // file: save_as just armed it (ADR-0001 requirement 4).
                        armed: self.doc.is_armed(),
                    },
                    revision,
                });
                self.remember(path);
            }
            Err(reason) => self.emit(Event::SaveFailed {
                path: path.to_path_buf(),
                revision,
                reason,
            }),
        }
    }

    /// Flush: an explicit save or an autosave trigger, gated by core's order.
    ///
    /// The stale check runs FIRST, before Document is consulted at all, because
    /// "this flush is old" outranks every other reason (D11): a stale autosave on
    /// a read-only file must report Clean, not ReadOnly, or the status line
    /// explains a write that was never needed. Everything after that is
    /// [`Document::should_autosave`]'s fixed order, so the rendered reason is
    /// deterministic and the rule stays in core.
    ///
    /// Why should_flush is not the gate: it compares the flush revision against
    /// core's own counter, and the two are different spaces while the bridge owns
    /// the buffer (§10.4, decision (ii)) - core has no setter for it. So the port
    /// carries the counter it was handed ([`Self::last_saved_revision`]) and asks
    /// core the questions core can answer. When core owns the buffer, this line
    /// becomes [`Document::should_flush`] and the field disappears.
    fn flush(&mut self, text: String, revision: u64) {
        if revision <= self.last_saved_revision {
            self.emit(Event::AutosaveSkipped {
                reason: SkipReason::Clean,
            });
            return;
        }
        let Some(path) = self.doc.path().map(Path::to_path_buf) else {
            // Nothing to write to. Reported, not swallowed: a flush the engine
            // cannot honour is a fact the status line needs.
            self.emit(Event::SaveFailed {
                path: PathBuf::new(),
                revision,
                reason: SaveError::Other("no document is open to save".to_string()),
            });
            return;
        };
        // A revision above the last saved one IS the change signal (D11): no
        // stored text is needed to know the buffer moved.
        self.doc.mark_dirty();
        if let Some(skip) = self.doc.should_autosave(self.autosave_enabled) {
            self.emit(Event::AutosaveSkipped {
                reason: api_skip(skip),
            });
            return;
        }

        let detected = self.detected;
        let disk_text = self.text_for_disk(&text);
        match self.write(&path, &disk_text, detected, revision) {
            Ok(()) => {
                self.doc.mark_saved();
                self.last_saved_revision = revision;
                self.emit(Event::Saved { path, revision });
            }
            Err(reason) => self.emit(Event::SaveFailed {
                path,
                revision,
                reason,
            }),
        }
    }

    /// The one write path: encode with the DETECTED settings (never a normalised
    /// one) and let core do the atomic dance - sibling temp, fsync, rename, sweep
    /// (D12). Encoding happens before any filesystem touch, so an unencodable
    /// character leaves the previous good file byte-for-byte intact.
    fn write(
        &self,
        path: &Path,
        text: &str,
        detected: Detected,
        revision: u64,
    ) -> Result<(), SaveError> {
        match save_document_revision(path, text, detected, revision) {
            Ok(_) => Ok(()),
            Err(err) => Err(api_save_error(err, detected.encoding)),
        }
    }

    /// DRAIN AND EXIT, ITERATIVELY AND BOUNDED: every command already queued
    /// behind Shutdown is handled, and its events emitted, before the loop breaks.
    /// Deliberately the opposite of [`Gateway::drop`](crate::Gateway), which
    /// removes the last Sender so the engine stops at Disconnected instead. Both
    /// paths are tested; only the drop path joins, and it ends with
    /// [`Self::flush_session`] so a clean shutdown never loses geometry or the
    /// pin bit.
    ///
    /// Two properties this loop has to keep, both learned from a mutation test
    /// rather than from reading it:
    ///
    /// * **No recursion.** A Shutdown found INSIDE the queue is the instruction
    ///   already being carried out, so here it is a no-op. Handing it back to
    ///   [`Self::handle`] called drain() again, and thousands of queued
    ///   Shutdowns overflowed the engine thread's stack and killed the process -
    ///   not a panic, so nothing unwinds and nothing joins. Pinned by
    ///   drain_never_recurses_on_a_queued_shutdown.
    /// * **A bound.** try_recv succeeds for as long as anyone keeps sending, so the
    ///   drain stops at [`MAX_DRAIN`] and the excess dies with the thread. A
    ///   truncated exit loses events; a hung one loses the process, because
    ///   Gateway::drop waits in join() with no timeout anywhere on that path.
    ///   Pinned by drain_stops_at_its_budget_instead_of_hanging_shutdown.
    fn drain(&mut self) {
        for _ in 0..MAX_DRAIN {
            match self.cmd_rx.try_recv() {
                // Already carrying this one out: not a re-entry, not an event.
                Ok(Command::Shutdown) => continue,
                Ok(command) => {
                    let _ = self.handle(command);
                }
                // Nothing left in the queue. This is the normal exit.
                Err(mpsc::TryRecvError::Empty) | Err(mpsc::TryRecvError::Disconnected) => {
                    self.flush_session();
                    return;
                }
            }
        }
        self.flush_session();
    }

    /// Writes session.json when something changed. Atomicity and the temp sweep
    /// are core's (D12); this function only decides WHEN, keeps the flag set so a
    /// failure retries on the next tick, and reports the failure rather than
    /// hiding it. The path names the file that failed, so the copy never reads as
    /// if the user's note was the thing lost.
    fn flush_session(&mut self) {
        if !self.pending_session_write {
            return;
        }
        match write_session(&self.state_dir.0, &self.session) {
            Ok(()) => self.pending_session_write = false,
            Err(err) => self.emit(Event::SaveFailed {
                path: self.state_dir.0.join(notes_core::session::FILE_NAME),
                revision: 0,
                reason: api_session_error(&err),
            }),
        }
    }

    /// Queues the one coalesced session write. True when this call added the flag,
    /// false when one was already outstanding.
    fn queue_session_write(&mut self) -> bool {
        let queued = !self.pending_session_write;
        self.pending_session_write = true;
        queued
    }

    /// Records a file as opened: into the session (so the next launch restores it,
    /// which is a stored path and not an automatic Open), into the MRU, and into
    /// the menu via an event.
    fn remember(&mut self, path: &Path) {
        self.session.path = Some(path.to_path_buf());
        self.queue_session_write();
        let display = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        // D13 in full - identity, case preservation, the cap, the no-duplicates
        // move-to-top - is core's function, not a re-statement of it here.
        self.recents = push_recent(
            std::mem::take(&mut self.recents),
            path.to_path_buf(),
            &display,
        );
        self.emit_recent();
    }

    /// core's entries to the port's, at the one place they become UI data.
    fn emit_recent(&mut self) {
        let list = self
            .recents
            .iter()
            .map(|entry| RecentEntry {
                path: entry.path.clone(),
                display: entry.display.clone(),
                exists: entry.exists,
            })
            .collect();
        self.emit(Event::RecentsUpdated(list));
    }

    /// THE .notes seam: frontmatter comes off for the editor, so the user never
    /// types into a --- block, and goes back on for the file, byte-for-byte.
    ///
    /// Both halves are [`notes_core::format`], landed as contracted:
    /// [`split`] returns [`NoteParts`] whose frontmatter span includes both
    /// --- lines and their terminators, which is why rebuild is lossless and why
    /// a foreign file (no frontmatter) round-trips as its own bytes. There is no
    /// parser in this crate and there must never be one (AGENTS.md rule 5).
    fn body_for_ui(&mut self, raw: &str) -> String {
        let NoteParts { frontmatter, body } = split(raw);
        self.frontmatter = frontmatter.map(str::to_owned);
        body.to_owned()
    }

    /// The write side of [`Self::body_for_ui`]. A file that never had
    /// frontmatter is rebuilt as exactly what the UI holds, so §4.5's
    /// byte-identical round trip survives the port.
    fn text_for_disk(&self, ui: &str) -> String {
        rebuild(self.frontmatter.as_deref(), ui)
    }

    /// Emits one event, and stops emitting once the listener is gone.
    ///
    /// A closed [`EventRx`](crate::EventRx) is a signal, not an error: the UI is
    /// gone. The engine keeps serving commands - including the file arms, which is
    /// the point: a window the user closed must not cost them a save - until
    /// Disconnected, with no panic and no log spam at a dead window.
    fn emit(&mut self, event: Event) {
        let Some(tx) = self.event_tx.as_ref() else {
            return;
        };
        if tx.send(event).is_err() {
            self.event_tx = None;
        }
    }
}

/// FileKind is core's extension classification (ADR-0001 option B), and
/// [`is_notes_path`] is core's - one owner of "what counts as ours", shared with
/// the frontmatter rules that have to agree with it. api asks, never decides.
fn file_kind(path: &Path) -> FileKind {
    if is_notes_path(path) {
        FileKind::Notes
    } else {
        FileKind::Foreign
    }
}

/// core's fixed skip order, named as the port names it. Total, and the
/// exhaustiveness is what catches a new core variant at compile time.
fn api_skip(skip: Skip) -> SkipReason {
    match skip {
        Skip::AutosaveDisabled => SkipReason::AutosaveDisabled,
        Skip::ForeignFileNotArmed => SkipReason::ForeignFileNotArmed,
        Skip::Clean => SkipReason::Clean,
        Skip::ReadOnly => SkipReason::ReadOnly,
        Skip::Oversize => SkipReason::Oversize,
    }
}

fn api_encoding(encoding: TextEncoding) -> Encoding {
    match encoding {
        TextEncoding::Utf8 => Encoding::Utf8,
        TextEncoding::Utf8Bom => Encoding::Utf8Bom,
        TextEncoding::Utf16Le => Encoding::Utf16Le,
        TextEncoding::Utf16Be => Encoding::Utf16Be,
        TextEncoding::Ansi(codepage) => Encoding::Ansi(codepage),
    }
}

fn api_line_ending(ending: CoreLineEnding) -> LineEnding {
    match ending {
        CoreLineEnding::Lf => LineEnding::Lf,
        CoreLineEnding::CrLf => LineEnding::CrLf,
    }
}

/// core's save failure to the port's, adding the ENCODING the file asked for -
/// core's Unencodable carries none, and the port's copy names it. Every other
/// case keeps core's rendered text inside [`SaveError::Other`], so the port never
/// invents UI copy.
fn api_save_error(err: CoreSaveError, encoding: TextEncoding) -> SaveError {
    match err {
        CoreSaveError::ReadOnly => SaveError::ReadOnly,
        CoreSaveError::PermissionDenied => SaveError::PermissionDenied,
        CoreSaveError::DiskFull => SaveError::DiskFull,
        CoreSaveError::Locked => SaveError::Locked,
        CoreSaveError::NotFound => SaveError::NotFound,
        CoreSaveError::Unencodable => SaveError::Unencodable(api_encoding(encoding)),
        other => SaveError::Other(other.to_string()),
    }
}

/// session.json failing is reported through the save vocabulary because that is
/// what it is - a write that did not happen. Missing/corrupt are read-side states
/// of a file the engine is writing, so they arrive as Other with core's copy.
fn api_session_error(err: &SessionError) -> SaveError {
    match err {
        SessionError::Io(io_err) => api_save_error(classify_io_error(io_err), TextEncoding::Utf8),
        other => SaveError::Other(other.to_string()),
    }
}

/// A read failure, shaped for the UI. NotFound is separated out because a missing
/// file is not a failed operation and the copy is different.
fn load_error_from_io(err: &std::io::Error) -> LoadError {
    match err.kind() {
        ErrorKind::NotFound => LoadError::NotFound,
        ErrorKind::PermissionDenied => LoadError::PermissionDenied,
        _ => match classify_io_error(err) {
            CoreSaveError::Locked => LoadError::Locked,
            CoreSaveError::NotFound => LoadError::NotFound,
            CoreSaveError::PermissionDenied => LoadError::PermissionDenied,
            other => LoadError::Other(other.to_string()),
        },
    }
}

/// Where decoding broke, or 0 when no single byte is the culprit - an unsupported
/// codepage is a verdict about the whole file, not about an offset.
fn decode_offset(err: &DecodeError) -> usize {
    match err {
        DecodeError::MalformedUtf8 { byte_offset }
        | DecodeError::UnterminatedUtf16 { byte_offset } => *byte_offset,
        DecodeError::UnsupportedCodepage(_) => 0,
    }
}

/// What an existing target's own bytes say they are, so Save As preserves the
/// TARGET's format rather than imposing the source's (§4.5). Unreadable, empty or
/// oversize targets fall back to the product default: replacing something we
/// cannot read is the user's explicit choice, made in a dialog.
fn existing_detected(path: &Path) -> Detected {
    let Ok(meta) = fs::metadata(path) else {
        return new_file_detected();
    };
    if meta.len() == 0 || is_oversize(usize::try_from(meta.len()).unwrap_or(usize::MAX)) {
        return new_file_detected();
    }
    let Ok(bytes) = fs::read(path) else {
        return new_file_detected();
    };
    detect(&bytes, None)
}
#[cfg(test)]
mod tests {
    use super::*;
    use notes_core::Rect;

    /// REVIEWER ITEM 3: the marking call inside [`Engine::run`] is what arms the
    /// latch, on the thread that actually runs the loop.
    ///
    /// Every other test here drives [`Engine::handle`] by hand, and the
    /// reentrancy test in tests/reentrancy.rs arms the flag on a thread the TEST
    /// spawned - which proves the mechanism exists, not that run() uses it. Delete
    /// [`mark_current_thread_as_engine`] from run() and this is the test that
    /// fails, because the probe's only writer is the line right after it.
    #[test]
    fn the_real_engine_thread_arms_the_latch() {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, _event_rx) = mpsc::channel();
        let engine = Engine::new(
            cmd_rx,
            event_tx,
            StateDir(PathBuf::from("unused-in-this-test")),
            Session::default(),
            Settings::default(),
        );
        // Another test's flag must not carry over: take() clears before starting.
        latch_probe::take();
        let handle = std::thread::Builder::new()
            .name("notes-engine-under-test".to_string())
            .spawn(move || engine.run())
            .expect("spawn the engine loop");
        // The ABORT path: no Shutdown, the last Sender simply goes away.
        drop(cmd_tx);
        handle
            .join()
            .expect("the engine thread must finish cleanly");
        assert!(
            latch_probe::take(),
            "run() must arm the reentrancy latch on the thread that runs the loop"
        );
        assert!(
            !on_engine_thread(),
            "and the test thread must still be unmarked"
        );
    }

    /// An engine wired to channels the test holds both ends of, so the queue can be
    /// filled before a single command is handled. No thread: every assertion below
    /// drives the same methods `run()` calls, which is what makes the
    /// drain ordering exact rather than a race with a scheduler.
    fn wired() -> (Engine, mpsc::Sender<Command>, mpsc::Receiver<Event>) {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let engine = Engine::new(
            cmd_rx,
            event_tx,
            StateDir(PathBuf::from("unused-in-these-tests")),
            Session::default(),
            Settings::default(),
        );
        (engine, cmd_tx, event_rx)
    }

    /// Everything emitted so far, as revision numbers. In this build every
    /// unservable Flush answers with SaveFailed carrying its own revision (D11:
    /// a failed save must not look like a saved buffer), which is exactly what
    /// makes the drain order observable without touching engine internals.
    fn emitted(events: &mpsc::Receiver<Event>) -> Vec<u64> {
        let mut out = Vec::new();
        while let Ok(event) = events.try_recv() {
            match event {
                Event::SaveFailed { revision, .. } => out.push(revision),
                other => panic!("unexpected event while draining: {other:?}"),
            }
        }
        out
    }

    fn flush(revision: u64) -> Command {
        Command::Flush {
            text: String::new(),
            revision,
        }
    }

    /// BLOCKER 2, the teeth the reviewer asked for: the drain must actually run,
    /// and it must answer what was queued BEHIND the Shutdown, in order.
    ///
    /// Removing the `drain()` call from the Shutdown arm empties this
    /// assertion (revisions 3 and 4 never arrive), which is the mutation the
    /// reviewer ran. Nothing here waits on a thread, so a passing run is proof
    /// rather than a timing coincidence.
    #[test]
    fn drain_answers_commands_queued_behind_shutdown_in_order() {
        let (mut engine, cmd_tx, events) = wired();
        for revision in 1..=2u64 {
            cmd_tx.send(flush(revision)).expect("unbounded");
        }
        cmd_tx.send(Command::Shutdown).expect("unbounded");
        for revision in 3..=4u64 {
            cmd_tx.send(flush(revision)).expect("unbounded");
        }

        // What run() does when recv() hands it the Shutdown.
        assert_eq!(engine.handle(Command::Shutdown), Flow::Exit);
        assert_eq!(
            emitted(&events),
            vec![1, 2, 3, 4],
            "everything queued before AND behind the Shutdown must be answered, in order",
        );
    }

    /// BLOCKER 1: a Shutdown found inside the drain is the instruction already
    /// being carried out, not a reason to call handle() -> drain() again.
    ///
    /// The recursion was a process killer, not a panic: thousands of queued
    /// Shutdowns blew the engine thread stack, so no unwind ran and
    /// Gateway::drop never joined. 20 000 is well past the 2 000 that reproduced
    /// it, and this test only needs to RETURN to pass.
    #[test]
    fn drain_never_recurses_on_a_queued_shutdown() {
        let (mut engine, cmd_tx, _events) = wired();
        for _ in 0..20_000 {
            cmd_tx.send(Command::Shutdown).expect("unbounded");
        }
        assert_eq!(engine.handle(Command::Shutdown), Flow::Exit);
        // Iterative means bounded work: the loop cannot have consumed more than
        // its budget, and it must still have left the excess in the queue.
        assert!(engine.cmd_rx.try_recv().is_ok(), "the excess stays queued");
    }

    /// MAJOR 3: the drain is bounded, so a producer that keeps sending cannot
    /// hold shutdown open forever - and Gateway::drop, which waits in join() with
    /// no timeout anywhere on that path, cannot be hung by it.
    #[test]
    fn drain_stops_at_its_budget_instead_of_hanging_shutdown() {
        let (mut engine, cmd_tx, events) = wired();
        let excess = MAX_DRAIN + 5;
        // From 1, not 0: revision 0 would be stale against a fresh engine's
        // last_saved_revision and answer Clean instead of echoing the flush.
        for revision in 1..=excess as u64 {
            cmd_tx.send(flush(revision)).expect("unbounded");
        }

        assert_eq!(engine.handle(Command::Shutdown), Flow::Exit);
        let answered = emitted(&events);
        assert_eq!(
            answered.len(),
            MAX_DRAIN,
            "the drain must stop at exactly its budget",
        );
        assert_eq!(answered.first().copied(), Some(1));
        assert_eq!(answered.last().copied(), Some(MAX_DRAIN as u64));
        // The 5 over the budget are dropped by the exit, not panicked over, and
        // not half-written: they never reach handle() at all.
        let mut left = 0usize;
        while engine.cmd_rx.try_recv().is_ok() {
            left += 1;
        }
        assert_eq!(
            left, 5,
            "the excess is left in the queue and dies with the thread"
        );
    }

    /// A quiet engine driven by hand: no thread, no channel to babysit. The
    /// thread-level behaviour belongs to tests/reentrancy.rs; what lives here is
    /// the state that file cannot see from outside the crate.
    fn engine() -> Engine {
        let (_cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, _event_rx) = mpsc::channel();
        Engine::new(
            cmd_rx,
            event_tx,
            StateDir(PathBuf::from("unused-in-these-tests")),
            Session::default(),
            Settings::default(),
        )
    }

    /// How many session writes are outstanding: 0 or 1, never more.
    fn pending(engine: &Engine) -> usize {
        usize::from(engine.pending_session_write)
    }

    /// D11's spirit, asserted where it is implemented: repeated GeometryChanged
    /// at the SAME rect queues one update, not N.
    #[test]
    fn repeated_geometry_at_one_rect_queues_one_update() {
        let mut engine = engine();
        let rect = Rect::new(10, 20, 300, 200);
        assert_eq!(pending(&engine), 0, "nothing is queued before a change");

        engine.handle(Command::GeometryChanged { rect });
        assert_eq!(pending(&engine), 1);
        for _ in 0..64 {
            engine.handle(Command::GeometryChanged { rect });
        }
        assert_eq!(engine.session.rect, rect);
        assert_eq!(
            pending(&engine),
            1,
            "the same rect 65 times must not queue 65 disk updates"
        );
    }

    /// A DIFFERENT rect coalesces into the same single write: the newest state
    /// is what gets persisted, and only once.
    #[test]
    fn geometry_updates_coalesce_to_the_latest_rect() {
        let mut engine = engine();
        engine.handle(Command::GeometryChanged {
            rect: Rect::new(1, 1, 100, 100),
        });
        engine.handle(Command::GeometryChanged {
            rect: Rect::new(2, 2, 200, 200),
        });
        assert_eq!(pending(&engine), 1, "one coalesced write");
        assert_eq!(engine.session.rect, Rect::new(2, 2, 200, 200));
    }

    /// D10: the pin bit has exactly one home. SetPinned writes the session and
    /// there is no second copy in this struct to drift out of sync.
    #[test]
    fn pin_state_lives_in_the_session_alone() {
        let mut engine = engine();
        assert!(!engine.session.pinned);
        engine.handle(Command::SetPinned(true));
        assert!(engine.session.pinned);
        assert_eq!(pending(&engine), 1, "a pin change needs persisting");
        assert_eq!(engine.window, None, "pinning is not a window operation");
    }

    /// RegisterWindow stores the handle and emits nothing (see the §5.5 steps).
    #[test]
    fn register_window_stores_the_handle_and_says_nothing() {
        let mut engine = engine();
        engine.handle(Command::RegisterWindow {
            handle: WindowHandle(0x1234),
        });
        assert_eq!(engine.window, Some(WindowHandle(0x1234)));
        assert!(
            engine.event_tx.is_some(),
            "registration is neither an event nor a failure"
        );
    }

    /// The tick arm must not spin: it moves the deadline forward, so an idle
    /// engine costs one wake-up per cadence, not one per poll.
    #[test]
    fn a_tick_moves_the_deadline_forward() {
        let mut engine = engine();
        engine.deadline = Instant::now();
        engine.on_tick();
        let next = engine.next_deadline().expect("the idle cadence is on");
        assert!(next > Instant::now());
        assert!(next - Instant::now() <= AUTOSAVE_IDLE);
    }

    /// The engine keeps the directory it was handed, so W5's persistence arm
    /// writes where the CALLER resolved (D-STATE) instead of re-deriving one.
    #[test]
    fn the_engine_holds_the_state_dir_it_was_given() {
        let mut engine = engine();
        engine.handle(Command::GeometryChanged {
            rect: Rect::new(0, 0, 10, 10),
        });
        assert_eq!(
            engine.state_dir.0,
            PathBuf::from("unused-in-these-tests"),
            "the StateDir survives the move into the engine untouched"
        );
    }

    /// The reentrancy trap is per-thread: armed by the loop, invisible elsewhere.
    #[test]
    fn the_engine_thread_flag_is_per_thread() {
        assert!(
            !on_engine_thread(),
            "a test thread is not the engine thread"
        );
        let armed = mark_current_thread_as_engine();
        assert!(on_engine_thread());
        let child = std::thread::spawn(on_engine_thread);
        assert!(!child.join().expect("child thread"), "arming must not leak");
        drop(armed);
        assert!(!on_engine_thread());
    }
}
