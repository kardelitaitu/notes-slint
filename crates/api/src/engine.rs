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
//! [`EventRx`](crate::EventRx) and never runs engine code. The queues are
//! unbounded (D24), so no command is dropped and no event is back-pressured -
//! but THAT is not what keeps a 400 ms save from blocking a frame, and saying
//! it was, was FALSE: the engine also calls user32 synchronously, and a window
//! op from another thread SENDS to the window's owner and blocks until that
//! owner pumps. Two things carry the no-frame-blocking guarantee now, and both
//! are load-bearing: the platform crate's ASYNC window ops
//! (SWP_ASYNCWINDOWPOS - the call returns before the move lands, so the engine
//! never waits on the owner), and the BOUNDED JOIN in gateway.rs, so that even
//! an engine stalled on some future blocking seam cannot hang shutdown. Do not
//! remove either and do not add a synchronous window call without an async
//! route.

use std::cell::Cell;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use notes_core::geometry::Rect;
use notes_core::save::IoStep;
use notes_core::session::write_session;
use notes_core::settings::write_settings;
use notes_core::{
    DecodeError, Detected, Document, FileKind, LineEnding as CoreLineEnding, NoteParts,
    PathVerdict, SaveError as CoreSaveError, Session, Settings, Skip, StateDir, TextEncoding,
    classify_io_error, clear as clear_recents, decode, detect, ensure_scratch_dir, identity_key,
    is_notes_path, is_oversize, mark_missing, path_policy, push as push_recent, rebuild,
    save_document_revision, split,
};

use crate::command::{Command, WindowHandle};
use crate::event::{
    Encoding, Event, FileMeta, LineEnding, LoadError, RecentEntry, SaveError, SkipReason, StateFile,
};
use notes_platform::{FrameRect, HostFacts, PinOutcome, WindowBackend};

use crate::gateway::EventTx;

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

/// Which state file is dirty. Two bits, one field, one tick - see
/// [`Engine::pending`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Pending {
    session: bool,
    settings: bool,
}

/// The argument to [`Engine::queue`]: which of the two files changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Session,
    Settings,
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
    /// The persisted preferences, in core's type: the global auto-save toggle, the
    /// ANSI code page, and the recents list. One field on purpose - these three
    /// used to be three fields, which meant three things that could drift from the
    /// file they are persisted to (D10's one-home discipline, applied to the
    /// settings.toml half of state; session.json owns the window half).
    ///
    /// Codepage resolution happens at DETECT time, not here (D27): what the file
    /// persisted is the user's choice, the gap nobody chose is filled by the
    /// host's ANSI code page through the [`HostFacts`] seam this crate owns, and
    /// when both are silent an ANSI file is REFUSED, never guessed. See
    /// [`Engine::resolved_codepage`].
    settings: Settings,
    /// 5.5 step 3: the handle the bridge registered. Now USED: the first
    /// registration is the moment the port restores the window and applies the
    /// pin, through its own host seams (D46) - see [`Engine::restore_and_pin`].
    window: Option<WindowHandle>,
    /// The two host seams, constructed by the port (D46) so a bridge never names a
    /// platform type. [`None`] only where notes-platform has no implementation to
    /// offer - its Win32 module is [`cfg(windows)`] - and then the port places
    /// nothing and pins nothing, quietly. That is a build-time fact about a host this
    /// app does not ship to, not a refused action, so it is the one case allowed to
    /// stay silent; every refusal by a REAL seam becomes
    /// [`Event::GeometryNotRestored`].
    backend: Option<Box<dyn WindowBackend>>,
    facts: Option<Box<dyn HostFacts>>,
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
    /// THE DOCUMENT GENERATION, and this struct is its only writer. It moves
    /// exactly once per rebind and ONLY at an emit that announces it - a `Loaded`
    /// or a `Rebound` - so a caller that stores the number it is handed can never
    /// be out of step, which is what the bridge's mirrored counter could not
    /// promise. The scratch bind inside [`Engine::flush`] is the deliberate
    /// exception: it acquires a FILE for the buffer the window already shows, so
    /// it moves nothing and carries the unchanged number. A Flush stamped with an
    /// older generation is a buffered edit for a document that no longer exists -
    /// it is discarded, not written into the file that replaced it (the
    /// stale-flush-overwrites-B bug).
    epoch: u64,
    /// The path whose bytes the most recent refused [`Command::Open`] declined
    /// to read, or None. Save As consults it for exactly ONE question - "is the
    /// TARGET the file the app refused to read?" - because that is the measured
    /// 0-byte overwrite the guard exists for. It is a PATH, not the bool it
    /// replaces, and the difference is c92494f3: a refusal does not rebind the
    /// buffer, so the document behind it is the one the user was looking at, and
    /// a session-global flag made THAT unsavable (the deleted scratch failed a
    /// load, the flag went true, and Save As answered NoTarget for a real note;
    /// measured again with the read gate: a stream refusal held a loaded note
    /// hostage and stranded the untitled buffer). Cleared by a successful
    /// install and by the scratch restore; identity is core's `identity_key`,
    /// the same answer is_scratch asks.
    load_refused_for: Option<PathBuf>,
    /// Coalesced "a state file needs writing": ONE field, ONE tick, ONE deadline -
    /// never a second timer and never a second thread. It carries two bits because
    /// there are two files with two owners (D10: session.json holds window state,
    /// settings.toml holds autosave and recents), and writing the wrong one because
    /// a single bool could not say which changed is both wasted I/O and a chance to
    /// clobber. Each bit clears only when ITS write succeeds, so a failure retries
    /// on the next tick instead of losing the change.
    pending: Pending,
    deadline: Instant,
    /// M5: a failed SESSION write is reported once, then latched until a write
    /// succeeds - see [`Engine::flush_state`]. Documents are different: one
    /// event per failed attempt is the contract there (ADR-0001). The session
    /// has no user action to attach to, and the tick retries it forever, so
    /// per-tick reporting is an unbounded stream of toasts.
    session_failure_latched: bool,
    /// The settings.toml twin of [`Self::session_failure_latched`] (M5/D54):
    /// same per-tick retry, same flood without a latch, same clear-on-success.
    settings_failure_latched: bool,
    /// MAJOR 4: set while the shutdown drain runs, so side-effecting host work
    /// (the restore move and the pin) is skipped for commands that were queued
    /// behind Shutdown - the one moment the UI thread is already waiting on
    /// this thread's exit. State writes still happen; only window ops skip.
    draining: bool,
    /// MAJOR 6: the flush-tick re-measure can fail forever (an IsWindow
    /// refusal does not heal), and an unlatched report is one event per tick.
    /// Same discipline as the state-file latches: one report per failure
    /// failure, cleared by the NEXT SUCCESS - "episode" means exactly that,
    /// nothing more: one failure, silence until a write succeeds, then a new
    /// failure is news again.
    measure_failure_latched: bool,
    /// MAJOR 5: the instant the most recent async move was ISSUED. The move
    /// has not necessarily landed when the call returns, so for two idle
    /// periods afterwards a measured rect may be the PRE-MOVE or MID-DRAG
    /// position - true of the screen for a moment, poison for the file. The
    /// flush refuses to store a rect measured inside that window.
    last_move_issued: Option<Instant>,
}

impl Engine {
    /// Builds the loop's state from what [`Gateway::start`] already read.
    /// The engine with the host it was built for. Tests that must SEE a platform
    /// call pass a recording host, through
    /// [`start_with_host`](crate::Gateway::start_with_host) or [`with_host`] below.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        cmd_rx: mpsc::Receiver<Command>,
        event_tx: EventTx,
        state_dir: StateDir,
        session: Session,
        settings: Settings,
        backend: Option<Box<dyn WindowBackend>>,
        facts: Option<Box<dyn HostFacts>>,
    ) -> Self {
        Self::with_host(
            cmd_rx, event_tx, state_dir, session, settings, backend, facts,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_host(
        cmd_rx: mpsc::Receiver<Command>,
        event_tx: EventTx,
        state_dir: StateDir,
        session: Session,
        settings: Settings,
        backend: Option<Box<dyn WindowBackend>>,
        facts: Option<Box<dyn HostFacts>>,
    ) -> Self {
        // The session's last document is RESTORED as state, not read from disk:
        // no bytes are touched here, and whether to Open it is the bridge's call
        // (§5.5).
        //
        // The persisted recents come in greyed-out-first, per features.md 4.4:
        // notes_core::recent::mark_missing (pure, no probing) drops every exists
        // flag, and only entries that answer is_file() light back up. Nothing is
        // deleted here - a note on a network share that is merely offline is still
        // the note the user was writing yesterday, and a menu that silently eats
        // entries is worse than one with a greyed row. The probe is bounded at ten
        // stats, which is inside the cold-start budget precisely BECAUSE the menu
        // renders the list anyway: it is work the app has to do before it can show
        // the first frame, not work it could defer.
        let mut settings = settings;
        mark_missing(&mut settings.recents);
        for entry in settings.recents.iter_mut() {
            // A refused name is never statted: this probe runs BEFORE A WINDOW
            // EXISTS, and a stat is exactly the exposure the read gate in
            // [`Engine::open`] refuses - a stream answers is_file() == true, a
            // stripped name answers for the file it was mangled into, and a
            // drive-relative name resolves against a CWD nobody chose. The
            // entry stays, greyed: "does not exist on disk" is the honest
            // rendering of a name the app will not touch, and the menu that
            // silently eats entries is the worse bug (see above).
            if read_policy_refusal(&entry.path).is_some() {
                continue;
            }
            if entry.path.is_file() {
                entry.exists = true;
            }
        }
        Engine {
            cmd_rx,
            event_tx: Some(event_tx),
            state_dir,
            settings,
            window: None,
            doc: match session.path.as_ref() {
                Some(path) => Document::open(path, file_kind(path), false, false),
                None => Document::new(),
            },
            detected: new_file_detected(),
            frontmatter: None,
            pending: Pending::default(),
            load_refused_for: None,
            session_failure_latched: false,
            settings_failure_latched: false,
            draining: false,
            measure_failure_latched: false,
            epoch: 0,
            last_move_issued: None,
            backend,
            facts,
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
        // THE STARTUP ANNOUNCE: the recents list is the menu's opening state,
        // and "no change has happened yet" must never read as "no recents" -
        // core may hold ten entries from last launch while the bridge renders
        // an empty menu on every launch until something changes. Announced
        // ONCE, before any command is handled, and only when there is
        // something to announce: a fresh install's empty list IS the bridge's
        // default, so the no-event-before-any-command contract that
        // tests/reentrancy.rs pins on an empty state dir still holds. The
        // rendering goes through [`Self::emit_recent`], so the label rule is
        // applied in exactly one place.
        if !self.settings.recents.is_empty() {
            self.emit_recent();
        }
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
        self.flush_state();
        self.deadline = Instant::now() + AUTOSAVE_IDLE;
    }

    /// Handles one command. Only [`Command::Shutdown`] returns [`Flow::Exit`].
    fn handle(&mut self, command: Command) -> Flow {
        match command {
            Command::RegisterWindow { handle } => {
                // 5.5 steps 3 AND 4 in one arm (D46): the handle is stored, and the
                // first registration is the earliest moment anything can be done with
                // it - the window exists, and its rect is the one number that has to
                // be right before the user sees the frame.
                //
                // first-registration ONLY, deliberately: a later one (a recreate, a
                // duplicate send) must never yank a window the user has since moved
                // by hand. That guard is the difference between a restore and a theft.
                let first = self.window.is_none();
                self.window = Some(handle);
                // MAJOR 4: the shutdown drain re-runs this arm for any
                // RegisterWindow queued behind Shutdown - exactly the state in
                // which the UI thread already holds the bounded join. The
                // handle is STATE and is stored either way; the restore is a
                // SIDE EFFECT on a window the bridge is tearing down, so the
                // drain skips it while still writing everything else.
                if first && !self.draining {
                    self.restore_and_pin(handle);
                }
            }
            // MAJOR 3: the handle is a VALUE, not a lease, and nothing cleared
            // it. A destroyed HWND fails closed, but Windows RECYCLES the
            // numbers: a stale handle that passes IsWindow names a STRANGER,
            // whose normal position the port would read into session.rect and
            // then MOVE. Unregister is the bridge's "this window is gone";
            // after it, nothing touches the stored value again.
            Command::UnregisterWindow => {
                // THE HANDLE IS STILL VALID ON THIS LINE - and it is the last
                // chance to measure honestly. The bridge closes the window
                // BEFORE close(), so the drain's flush would otherwise run
                // handle-less forever: measure fails, an existing session.json
                // is deferred into staleness, and a move-then-quick-quit loses
                // the move (the README's opening promise). Measure and flush
                // NOW, once, then let the handle go.
                if self.window.is_some() {
                    // The guard lifts here for the same reason it lifts at
                    // final_flush: this read-back is the last honest one there
                    // will ever be for this window.
                    self.last_move_issued = None;
                    self.measure_rect();
                    self.flush_state();
                }
                self.window = None;
            }
            Command::GeometryChanged => {
                // A TRIGGER, nothing more (the payload was removed deliberately:
                // the only rect a bridge can produce is its own space, and the
                // live drift bug was exactly such a hint winning the write -
                // 390,278,1010,698 frame in, 398,297,1002,678 client persisted).
                // The engine answers by MEASURING once through the platform seam
                // on the flush tick that the queue arms; the write arm then
                // carries the measured frame rect or carries nothing.
                // MAJOR 3: after UnregisterWindow there is no window to
                // describe, so the measure fails and the write is deferred -
                // persisting a rect for a handle the port no longer holds is how
                // a stranger's placement gets written.
                self.queue(Target::Session);
            }
            Command::SetAutosave(on) => {
                // The global toggle, answered by the menu's own check mark. The
                // per-document half is ADR-0001's and lives on Document.
                if self.settings.autosave_enabled != on {
                    self.settings.autosave_enabled = on;
                    self.queue(Target::Settings);
                }
            }
            Command::SetPinned(on) => {
                if self.session.pinned != on {
                    self.session.pinned = on;
                    self.queue(Target::Session);
                }
            }
            Command::ClearRecents => {
                // core's own clear, so the cap and the entry type stay core's
                // business even for the empty case.
                self.settings.recents = clear_recents();
                self.emit_recent();
                self.queue(Target::Settings);
            }
            Command::Open { path } => self.open(&path),
            Command::SaveAs {
                path,
                text,
                revision,
            } => self.save_as(&path, &text, revision),
            Command::Flush {
                text,
                revision,
                epoch,
            } => self.flush(text, revision, epoch),
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
        // THE READ GATE, before the first stat: the write side has refused
        // these names in core's atomic write since path_policy landed, but a
        // refusal on write alone left the read path free to LOAD a stream's
        // second content, a stripped ghost, or a device that answers 0 bytes
        // as an empty note. Both ways of reaching this arm WITHOUT the user
        // choosing a name funnel through it - session.json's stored path is
        // re-issued as Command::Open on every launch, and the recents probe
        // below runs before a window exists. See [`read_policy_refusal`] for
        // why exactly these four verdicts, and why UnboundedNetwork is not
        // one of them. No generation bump and no buffer: nothing was read,
        // which is the same silence the comment below states for every
        // other failed load.
        if let Some(reason) = read_policy_refusal(path) {
            // Remembered AS A PATH, not as a session-wide latch: this refusal
            // did not rebind the buffer, so the document behind it - loaded
            // and typed into, or the untitled startup buffer - stays savable,
            // and only a Save As ONTO this name is held (see save_as). The
            // bool that used to sit here held the loaded note hostage.
            self.load_refused_for = Some(path.to_path_buf());
            self.emit(Event::LoadFailed {
                path: path.to_path_buf(),
                reason,
            });
            return;
        }
        // NO GENERATION BUMP HERE. The engine ISSUES the generation and the
        // bridge ECHOES it, which only works while every move is ANNOUNCED - and
        // the only announcements are `Loaded` and `Rebound`. A load that fails
        // (missing, oversize, undecodable) emits neither, so a bump up here would
        // move the engine's number with nothing to carry it and every later Flush
        // would come back Superseded: the same silence the mirror used to break
        // in, wearing the opposite hat. The bump lives at the emit sites below,
        // beside the event that states the new number.
        let on_disk = match fs::metadata(path) {
            Ok(meta) => meta,
            Err(err) => {
                // D69's RESTART half. Because the engine records the scratch in
                // the session (see [`Engine::flush`]'s `remember`), a relaunch
                // OPENS it - and the scratch is scratch: a file THIS port made,
                // which the user is free to delete (or to sweep along with the
                // whole notes/ directory) between runs. A missing scratch is not
                // a lost document, it is a scratch that has not been written
                // yet, so the ask is answered with a fresh EMPTY one rather than
                // an error banner over an empty editor. Every OTHER missing path
                // keeps [`Engine::fail_load`]'s honest refusal: that is a real
                // loss, and the refused-load guard that follows it is what stops
                // Save As overwriting a document the app never read.
                if err.kind() == ErrorKind::NotFound && self.is_scratch(path) {
                    self.restore_missing_scratch(path);
                    return;
                }
                self.fail_load(path, &err);
                return;
            }
        };
        let oversize = is_oversize(usize::try_from(on_disk.len()).unwrap_or(usize::MAX));
        if oversize {
            // B1, measured. This branch used to answer Loaded with encoding=Utf8,
            // line_ending=Lf, trailing_newline=false and read_only=true for bytes it
            // had never read - four invented facts on a status line that rule 2
            // forbids - and worse, it armed nothing but cleared the way for Save As
            // to write that empty buffer over a real 9 MiB file and report Saved.
            // The honest answer is a refusal of the LOAD, made from the stat alone,
            // before a single byte is read or decoded. D9's "nothing is ever refused"
            // protects the user's DOCUMENT; it does not require pretending to have
            // loaded bytes there is no buffer for.
            self.load_refused_for = Some(path.to_path_buf());
            self.emit(Event::LoadFailed {
                path: path.to_path_buf(),
                reason: LoadError::TooLarge,
            });
            return;
        }
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) => {
                self.fail_load(path, &err);
                return;
            }
        };

        let detected = detect(&bytes, self.resolved_codepage());
        let raw = match decode(&bytes, detected) {
            Ok(text) => text,
            Err(err) => {
                self.load_refused_for = Some(path.to_path_buf());
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
        self.load_refused_for = None;
        let body = self.body_for_ui(&raw);
        // THE BUMP, announced: this buffer is a different document than the one
        // the bridge was echoing, and the number travels in the same event that
        // replaces the text, so the two can never be observed apart.
        self.epoch += 1;
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
            epoch: self.epoch,
        });
        self.remember(path);
    }

    /// A load that never got as far as decoding. The Windows error-code knowledge
    /// is core's ([`classify_io_error`] is what separates a sharing violation
    /// from a denied ACL); the port only shapes it for the UI.
    fn fail_load(&mut self, path: &Path, err: &std::io::Error) {
        self.load_refused_for = Some(path.to_path_buf());
        self.emit(Event::LoadFailed {
            path: path.to_path_buf(),
            reason: load_error_from_io(err),
        });
    }

    /// True when `path` IS this state dir's scratch note.
    ///
    /// Identity, not bytes: Windows paths are case-insensitive but not
    /// case-preserving, so the comparison is core's [`identity_key`] - the one
    /// answer this repo has for "the same file" (canonicalise when the file
    /// exists, a pure lexical key when it does not, and never a canonicalise on
    /// a verdict that says the attempt can stall, which is what a UNC state dir
    /// would be). This branch is reached only after the stat said NotFound, so
    /// both sides take the lexical key and nothing blocks. Where the scratch
    /// lives is core's rule (`scratch_note_path`), asked and never re-derived -
    /// and the whole point of D69 is that this is the ONLY place the mapping
    /// "the session names this path" -> "that path is the scratch" exists. It
    /// does not live in the bridge, and `path: null` never means "the scratch"
    /// anywhere: a secret two crates have to keep is how an untitled note comes
    /// back empty.
    fn is_scratch(&self, path: &Path) -> bool {
        identity_key(path) == identity_key(&notes_core::paths::scratch_note_path(&self.state_dir))
    }

    /// The scratch's own answer to "the file is gone": the IDENTITY comes back
    /// (path + armed + the new-file format), the buffer is empty, and the load
    /// is NOT a refusal.
    ///
    /// What this deliberately does not do is call [`Engine::remember`]: a file
    /// that does not exist was not opened. The scratch is in no case a recents
    /// entry any more (see [`Engine::remember`]'s split) - identity for restore,
    /// absence from the list - and this path keeps that true by binding the
    /// session directly. The session's own `path` is restated because it is the
    /// fact that has to survive this launch: it is already the scratch (that is
    /// why the bridge asked), and restating + queueing repairs the case where the
    /// session file was missing or corrupt and the bridge is opening the scratch
    /// for another reason entirely.
    fn restore_missing_scratch(&mut self, scratch: &Path) {
        let detected = new_file_detected();
        self.doc = Document::open(scratch, file_kind(scratch), false, false);
        self.detected = detected;
        self.frontmatter = None;
        self.load_refused_for = None;
        // The DIRECTORY often goes with the file, and the ordinary flush write
        // path does not create parents (core's atomic write needs a directory to
        // put its sibling temp in). Best effort and SILENT: if the place cannot
        // be made, the next write says so through NeedsPath, and the startup
        // StateDirUnusable event has usually already explained why.
        let _ = ensure_scratch_dir(&self.state_dir);
        // A move of the number, ANNOUNCED by the Loaded below: the buffer this
        // window holds becomes a fresh empty scratch, which is a different
        // document than whatever was in it.
        self.epoch += 1;
        self.emit(Event::Loaded {
            path: scratch.to_path_buf(),
            text: String::new(),
            meta: FileMeta {
                encoding: api_encoding(detected.encoding),
                line_ending: api_line_ending(detected.line_ending),
                trailing_newline: detected.trailing_newline,
                read_only: false,
                oversize: false,
                // CHECKED, not assumed: the scratch is named untitled.notes, so
                // core's `is_notes_path` classifies it as ours and ADR-0001 arms
                // autosave on open. A foreign file the app did not create stays
                // disarmed; ours never does, and no bridge workaround is needed.
                armed: self.doc.is_armed(),
            },
            epoch: self.epoch,
        });
        self.session.path = Some(scratch.to_path_buf());
        self.queue(Target::Session);
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
        // NO BUMP HERE EITHER (see [`Engine::open`]): a Save As that is refused
        // or fails emits no Rebound, and an unannounced generation move is
        // exactly the silence this change exists to remove. The buffer did not
        // change hands, so its stamp does not change.
        // The measured 0-byte overwrite, scoped to the file it was ever
        // about: the TARGET is the path whose bytes a refused open declined
        // to read, so writing here would destroy a document the app never
        // read. A refusal of some OTHER name does not follow the buffer
        // around - it did not rebind anything, so the document behind it is
        // exactly the one the user was looking at, and holding THAT hostage
        // is the c92494f3 bug in reverse. See [`Engine::load_refused_for`].
        if self
            .load_refused_for
            .as_ref()
            .is_some_and(|refused| identity_key(refused) == identity_key(path))
        {
            self.emit(Self::document_save_failed(
                path.to_path_buf(),
                revision,
                SaveError::NoTarget,
            ));
            return;
        }
        // If the target already exists, ITS bytes win: overwriting a UTF-16 file
        // with the source's UTF-8 is exactly the silent change §4.5 forbids.
        let detected = existing_detected(path, self.resolved_codepage());
        let disk_text = self.text_for_disk(text);
        match self.write(path, &disk_text, detected, revision) {
            Ok(()) => {
                self.doc.save_as(path);
                self.detected = detected;
                // save_as already anchored core's saved_revision at this document
                // revision; there is no second counter left to update.
                let read_only = fs::metadata(path).is_ok_and(|meta| meta.permissions().readonly());
                self.emit(Event::Saved {
                    path: path.to_path_buf(),
                    revision,
                });
                // THE BUMP, announced: the write landed and the Rebound below is
                // the event that carries the new number, so a bridge that echoes
                // what it is told can never be caught out by a move it was not
                // told about - which is why a refused or failed Save As above
                // bumps nothing.
                self.epoch += 1;
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
                    epoch: self.epoch,
                });
                self.remember(path);
            }
            Err(reason) => self.emit(Self::document_save_failed(
                path.to_path_buf(),
                revision,
                reason,
            )),
        }
    }

    /// Flush: an explicit save or an autosave trigger, gated by core's order.
    ///
    /// [`Document::should_flush`] is the ONLY place this build decides that a
    /// Flush is stale (D11). The port keeps no second counter - the field this
    /// replaced is deleted - because two gates in two crates is how a future fix
    /// lands in one and not the other. Core's order is also the rendered order:
    /// stale outranks everything, so a stale autosave on a read-only file says
    /// Clean, which is true, rather than ReadOnly, which explains a write nobody
    /// needed.
    ///
    /// [`Document::note_revision`] is what makes that gate usable at all: the
    /// bridge owns the revision space, so core is TOLD where the document stands,
    /// monotonically. The [`Document::revision`] comparison below is NOT a second
    /// D11 gate - it never decides whether to write. It turns "a revision core has
    /// not been told about" into a dirty document, because core's only
    /// dirty-setters bump the counter, and marking dirty AFTER the gate would make
    /// a stale Flush look like an edit.
    ///
    /// Why this used to be a port-side counter, kept as the record of an API gap
    /// that core has since closed: [`should_flush`] compared the flush revision
    /// against core's own counter, and the bridge owns the revision space.
    /// "this flush is old" outranks every other reason (D11): a stale autosave on
    /// a read-only file must report Clean, not ReadOnly, or the status line
    /// explains a write that was never needed. Everything after that is
    /// [`Document::should_autosave`]'s fixed order, so the rendered reason is
    /// deterministic and the rule stays in core.
    ///
    /// Why should_flush is not the gate: it compares the flush revision against
    /// core's own counter, and the two are different spaces while the bridge owns
    /// the buffer (§10.4, decision (ii)) - core has no setter for it. So the port
    /// carries the counter it was handed (`last_saved_revision`) and asks
    /// core the questions core can answer. When core owns the buffer, this line
    /// becomes [`Document::should_flush`] and the field disappears.
    fn flush(&mut self, text: String, revision: u64, epoch: u64) {
        // THE EPOCH GUARD: a Flush carrying an echoed generation the engine has
        // already replaced must not land in the file that replaced it - the stale
        // text would overwrite the current document atomically and report Saved.
        // Discard, and say so.
        if std::env::var("N2_DEBUG").is_ok() {
            eprintln!("FLUSH epoch={} engine_epoch={}", epoch, self.epoch);
        }
        // EQUALITY, now against a number THIS crate issued and the caller only
        // echoes. That is what makes an equality test the right shape here: the
        // bridge holds no counter to drift, and a mismatch can only mean the text
        // was buffered under a document this engine has since replaced. It is
        // still not a diagnosis - one reason string covers a stamp that is too OLD
        // (legitimately superseded) and a stamp a buggy caller never re-echoed -
        // but only one side can now be wrong, and it is not the caller's guess.
        if epoch != self.epoch {
            self.emit(Event::AutosaveSkipped {
                reason: SkipReason::Superseded,
            });
            return;
        }
        if revision > self.doc.revision() {
            self.doc.apply_edit();
        }
        self.doc.note_revision(revision);
        if let Some(skip) = self
            .doc
            .should_flush(revision, self.settings.autosave_enabled)
        {
            self.emit(Event::AutosaveSkipped {
                reason: api_skip(skip),
            });
            return;
        }
        let Some(path) = self.doc.path().map(Path::to_path_buf) else {
            // D69: an untitled buffer is not an error, it is a note nobody has
            // named yet - the first-run user types, autosave fires, and the text
            // MUST land somewhere real instead of being dropped with a skip.
            // Core owns WHERE (scratch_note_path + ensure_scratch_dir, the same
            // judge and probe every other state path uses); the port owns the
            // write, through the SAME machinery Save As uses (sibling temp,
            // fsync, rename - no second writer). Then the document is rebound:
            // Saved announces the bytes, Rebound announces the new identity, and
            // the session is pointed at the scratch so the next launch restores
            // it. The scratch does NOT join the recents (see
            // [`Engine::remember`]'s split): it is a restore target, not a file
            // the user chose.
            //
            // This mirrors save_as's body minus its refused-target guard: the
            // refused-load state protects a FOREIGN file from a blind overwrite,
            // and the scratch is this port's own freshly created file - the
            // guard has no jurisdiction here. NeedsPath survives as the FALLBACK
            // (its meaning is now "we could not make a file for it": a scratch
            // directory that cannot be created, or a refused write), and the
            // startup StateDirUnusable event has usually already said why.
            //
            // An EMPTY untitled buffer is the one case D69 does not cover: there
            // is no text to lose, so creating (and re-writing) a zero-byte
            // scratch file on every flush would be pure churn. NeedsPath stays
            // its answer until the user actually types something.
            if text.is_empty() {
                self.emit(Event::AutosaveSkipped {
                    reason: SkipReason::NeedsPath,
                });
                return;
            }
            if let Err(err) = notes_core::paths::ensure_scratch_dir(&self.state_dir) {
                let _ = err; // the reason reached the user at startup, or rides the next one
                self.emit(Event::AutosaveSkipped {
                    reason: SkipReason::NeedsPath,
                });
                return;
            }
            let scratch = notes_core::paths::scratch_note_path(&self.state_dir);
            let detected = self.detected;
            let disk_text = self.text_for_disk(&text);
            match self.write(&scratch, &disk_text, detected, revision) {
                Ok(()) => {
                    // NO EPOCH BUMP HERE, and that is a load-bearing
                    // non-action, not an oversight: the bridge mirrors the
                    // engine's generation at the SEND of an Open or a Save As
                    // (Wire::rebind), and it sent neither to get here — an
                    // untitled note is bound to its scratch from the engine's
                    // own side of the seam. The guard above compares the two
                    // counters for EQUALITY, so a bump here would put the engine
                    // one generation ahead of every flush the bridge has in the
                    // debounce and silently discard the autosave of the one
                    // document this product always has. Pinned by
                    // `the_scratch_bind_does_not_bump_the_epoch` in
                    // tests/scratch_restart.rs; the two bump sites are
                    // [`Engine::open`] and [`Engine::save_as`] and nothing else.
                    self.doc.save_as(&scratch);
                    self.detected = detected;
                    let read_only =
                        fs::metadata(&scratch).is_ok_and(|meta| meta.permissions().readonly());
                    self.emit(Event::Saved {
                        path: scratch.clone(),
                        revision,
                    });
                    // Saved THEN Rebound: Rebound's contract is literally "the
                    // open document is now a DIFFERENT file", which is only
                    // true once Saved has announced the bytes.
                    self.emit(Event::Rebound {
                        path: scratch.clone(),
                        meta: FileMeta {
                            encoding: api_encoding(detected.encoding),
                            line_ending: api_line_ending(detected.line_ending),
                            trailing_newline: detected.trailing_newline,
                            // The port just wrote this file successfully; it
                            // was not read-only a moment ago.
                            read_only,
                            oversize: false,
                            // A brand-new scratch note is armed: autosave owns
                            // it from this moment (ADR-0001 requirement 4).
                            armed: self.doc.is_armed(),
                        },
                        revision,
                        // The UNCHANGED number, stated anyway: the bind is not a
                        // rebind of the buffer the bridge holds - same text, same
                        // window, the note just acquired a file - and saying so in
                        // the event is what lets a bridge echo a number it never
                        // has to predict. This is the line the mutation test reads.
                        epoch: self.epoch,
                    });
                    // Binds session.path to the scratch and stops there:
                    // `remember` keeps the scratch out of the MRU.
                    self.remember(&scratch);
                }
                Err(_) => {
                    // The write was refused (locked, ACL, full disk): the
                    // fallback skip, and NO file was created.
                    self.emit(Event::AutosaveSkipped {
                        reason: SkipReason::NeedsPath,
                    });
                }
            }
            return;
        };

        let detected = self.detected;
        let disk_text = self.text_for_disk(&text);
        match self.write(&path, &disk_text, detected, revision) {
            Ok(()) => {
                // Anchors core's saved_revision to the document revision that
                // note_revision just aligned, so the next stale flush is decided
                // in this same place.
                self.doc.mark_saved();
                self.emit(Event::Saved { path, revision });
            }
            Err(reason) => self.emit(Self::document_save_failed(path, revision, reason)),
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
    /// [`Self::flush_state`] so a clean shutdown never loses geometry or the
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
    ///   truncated exit loses events; a hung one USED to lose the process, when
    ///   Gateway::drop joined without a timeout - the bounded join in
    ///   gateway.rs now abandons the engine after JOIN_DEADLINE instead.
    ///   Pinned by drain_stops_at_its_budget_instead_of_hanging_shutdown.
    fn drain(&mut self) {
        self.draining = true;
        for _ in 0..MAX_DRAIN {
            match self.cmd_rx.try_recv() {
                // Already carrying this one out: not a re-entry, not an event.
                Ok(Command::Shutdown) => continue,
                Ok(command) => {
                    let _ = self.handle(command);
                }
                // Nothing left in the queue. This is the normal exit.
                Err(mpsc::TryRecvError::Empty) | Err(mpsc::TryRecvError::Disconnected) => {
                    self.final_flush();
                    return;
                }
            }
        }
        self.final_flush();
    }

    /// The shutdown write, and why it is not just flush_state: the move-in-
    /// flight guard defers TICK writes because a later tick can do better - at
    /// shutdown there IS no later tick. The guard is lifted, and the host's
    /// read-back NOW is the window's actual position at quit (or the best
    /// report the host gives); the hint is still barred from the file, because
    /// the measure replaces the stored rect before the write. A pre-move
    /// read-back at quit is honest: that is where the window really is.
    fn final_flush(&mut self) {
        self.last_move_issued = None;
        self.flush_state();
    }

    /// How many pixels must stay on screen after a clamp, per axis. Core takes the
    /// number per call precisely because it is product policy and not geometry, and
    /// nobody has decided it - so it is named here with a reopening condition
    /// instead of buried in a call. 32 is a title-bar grab: enough to drag the window
    /// back, few enough that a nearly-offscreen window still reads as where the user
    /// left it. THE ONE NUMBER IN THIS SLICE WITH NO OWNER.
    const MIN_VISIBLE: u32 = 32;

    /// Places the window and applies the pin, through the host seams (D46, D48).
    ///
    /// The monitor is resolved from the SAVED rect, before clamping, because the
    /// answer is per-monitor: [`work_area_for_rect`] picks the monitor with the most
    /// overlap and falls back to the NEAREST one, which is exactly the unplugged-
    /// monitor case this app promises to survive, and blind-primary is what core's
    /// geometry doc forbids. The clamp is core's rule too ([`Rect::clamped_to`]):
    /// this function computes nothing.
    ///
    /// A maximized session is not moved: the stored rect is its restore position and
    /// moving a maximized window means un-maximizing it first, a decision platform
    /// explicitly leaves above itself. It is still pinned, because the pin is
    /// orthogonal to placement.
    /// WHAT THE GEOMETRY TESTS DO AND DO NOT PROVE, so the next reader does
    /// not mistake the fake for the desktop: `tests/geometry.rs` proves the
    /// port ASKED - that the persisted rect is the one `restore_frame_rect`
    /// RETURNED (`a_fresh_install_writes_the_session_with_the_rect_the_host_reports`
    /// feeds the fake an answer different from the input rect and asserts the
    /// stored value is the answer), and that `set_frame_rect` was called ONCE
    /// with the CLAMPED rect at scale 1.0
    /// (`the_first_registration_moves_the_window_once_at_the_clamped_frame_rect`).
    /// What no fake can prove is whether `SetWindowPos` on a REAL window sticks
    /// (gpui may re-apply stashed placement, and a recording backend has no
    /// window to lie about). That half is the smoke run's on-screen check
    /// (persisted rect vs the rect the user actually sees), not this suite's.
    /// NAMED HOLES the fake cannot reach, so nobody mistakes them for covered:
    /// (1) a DPI change between `work_area_for_rect` and the move - the clamp
    /// was computed against a work area that may no longer be the truth;
    /// (2) the monitor vanishing in that same window of time - nearest-monitor
    /// fallback happens inside the seam, invisible here; (3) `scale_factor` is
    /// never refreshed after the move, and `monitor_id` only on a SUCCESSFUL
    /// move - a refused move leaves both stale for the next launch. Each is a
    /// bounded wrongness (a clamp, a hint ordinal), not a hang - but they are
    /// unproven, and this paragraph is the receipt.
    fn restore_and_pin(&mut self, handle: WindowHandle) {
        let rect = self.session.rect;
        if self.session.maximized || self.backend.is_none() || self.facts.is_none() {
            // No move to make, or no seam on this build (see [`Engine::backend`]).
            // Pinned anyway. The session is marked dirty WITHOUT measuring: the
            // fresh-install fix below, and the measurement belongs to the flush
            // tick (MEASURE LATER, in flush_state).
            self.apply_topmost(handle);
            self.queue(Target::Session);
            return;
        }
        match self
            .facts
            .as_ref()
            .expect("checked above")
            .work_area_for_rect(to_frame(rect))
        {
            Ok((work, monitor_id)) => {
                let clamped = rect.clamped_to(to_rect(work), Self::MIN_VISIBLE);
                let moved = self
                    .backend
                    .as_mut()
                    .expect("checked above")
                    .set_frame_rect(handle.0 as isize, to_frame(clamped), 1.0);
                match moved {
                    // scale 1.0 because the rect is already frame pixels in this
                    // window's space; anything else is the double conversion the seam
                    // documents against.
                    Ok(()) => {
                        // MAJOR 5: an async move has NOT landed when this
                        // returns; stamping when it was issued is what lets the
                        // flush tick refuse to persist a pre-move read-back.
                        self.last_move_issued = Some(Instant::now());
                        self.session.monitor_id = monitor_id;
                        if clamped != rect {
                            // The clamp moved it: store where it actually is, so the
                            // next launch does not have to clamp it again.
                            self.session.rect = clamped;
                            // AND SAY SO: the restore the session asked for did not
                            // happen as asked. Once per registration - a real,
                            // countable fallback, not a silent one (D42's
                            // unanswered question: how many users hit this?).
                            self.emit(Event::GeometryNotRestored {
                                rect,
                                reason: String::from(
                                    "the saved position does not fully fit any monitor; it was clamped back on screen",
                                ),
                            });
                            self.queue(Target::Session);
                        }
                    }
                    Err(err) => self.emit(Event::GeometryNotRestored {
                        rect: clamped,
                        reason: err.to_string(),
                    }),
                }
            }
            Err(err) => self.emit(Event::GeometryNotRestored {
                rect,
                reason: err.to_string(),
            }),
        }
        // THE FRESH-INSTALL DIRTY MARK, kept from the D54 fix: without it a
        // user who never drags the window never gets a session.json (the flush
        // sees nothing pending and writes nothing). Registration queues ONE
        // write; the MEASURE itself happens on the flush tick, never here -
        // an async move has not landed when the call returns, so measuring now
        // stores the pre-move rect and D48 becomes quietly false (probe5).
        self.queue(Target::Session);
        self.apply_topmost(handle);
    }

    /// 5.5 step 4, which until this slice was "stored, never used": the pin bit
    /// lives only in session.json (D10) and this is the one place that acts on it.
    /// A refusal is reported rather than swallowed, because a session that was pinned
    /// yesterday and is not today otherwise reads as a lost setting.
    fn apply_topmost(&mut self, handle: WindowHandle) {
        let on = self.session.pinned;
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        // Platform's verdict, mapped straight onto the event: the reason
        // strings are platform's own sentences, so a richer verdict flows
        // through without a second vocabulary change.
        match backend.set_topmost(handle.0 as isize, on) {
            PinOutcome::Applied => {}
            PinOutcome::Failed(err) => {
                self.emit(Event::PinFailed {
                    reason: err.to_string(),
                });
            }
            PinOutcome::NotApplied { expected, actual } => {
                self.emit(Event::PinFailed {
                    reason: format!(
                        "the pin did not stick: asked for topmost={expected}, the window read back {actual}"
                    ),
                });
            }
        }
    }

    /// Overwrites the stored rect with this window's NORMAL position (D48). No handle,
    /// or no seam: nothing to ask, and the last known rect stays - it was measured or
    /// clamped when it was written, so keeping it is not a new claim. A seam that
    /// answers with an error did refuse, and the refusal is reported: the alternative
    /// is persisting a rect known to be wrong. Returns whether a rect was actually
    /// measured: [@@false@@] is the caller's instruction to write nothing new.
    fn measure_rect(&mut self) -> bool {
        let Some(handle) = self.window else {
            return false;
        };
        let Some(backend) = self.backend.as_mut() else {
            return false;
        };
        match backend.restore_frame_rect(handle.0 as isize) {
            Ok(frame) => {
                // A success re-arms the report (MAJOR 6): the next distinct
                // failure is news again.
                self.measure_failure_latched = false;
                let rect = to_rect(frame);
                // MAJOR 5: within two idle periods of issuing an async move,
                // this read-back may be the PRE-MOVE or MID-DRAG position - the
                // call returns before the move lands. Trust it for the latch
                // bookkeeping, never for storage: the stored rect is protected
                // from the stale read, and the next GeometryChanged persists
                // the landed position.
                let move_in_flight = self
                    .last_move_issued
                    .is_some_and(|at| at.elapsed() < 2 * AUTOSAVE_IDLE);
                if !move_in_flight && rect != self.session.rect {
                    self.session.rect = rect;
                }
                true
            }
            Err(err) => {
                // MAJOR 6: this runs every flush tick while the bit stays set,
                // and an IsWindow refusal does not heal - one report per
                // failure, cleared by the next success, not one per 750 ms.
                // (Known oscillation, accepted: with a session write pending
                // every tick - a continuous drag - an intermittently failing
                // measure reports every OTHER tick, because each success re-arms
                // the report.)
                if !self.measure_failure_latched {
                    self.measure_failure_latched = true;
                    self.emit(Event::GeometryNotRestored {
                        rect: self.session.rect,
                        reason: err.to_string(),
                    });
                }
                false
            }
        }
    }

    /// The code page THIS HOST can decode ANSI with (D27): the user's persisted
    /// choice when there is one, else the machine's ANSI code page through the
    /// [`HostFacts`] seam, else [`None`] - and [`None`] means an ANSI file is
    /// refused, never guessed. The host is asked per call, not cached: GetACP is
    /// one syscall, and the seam's rule is call out, value back, decide nothing.
    fn resolved_codepage(&self) -> Option<u16> {
        let host_acp = self.facts.as_ref().map(|facts| facts.ansi_codepage());
        self.settings.resolved_codepage(host_acp)
    }

    /// Writes whichever state files are dirty. Atomicity, the temp sweep and the
    /// TOML rendering are core's ([`write_session`] and [`write_settings`]);
    /// this decides WHEN, keeps the failed bit set so the next tick retries, and
    /// reports a failure rather than hiding it - ONCE per failure, until the
    /// next success, for the session (M5: the tick would otherwise report
    /// forever), per attempt for settings and documents.
    fn flush_state(&mut self) {
        let pending = self.pending;
        if !pending.session && !pending.settings {
            return;
        }
        if pending.session {
            // One tick, one write, and the rect inside it is the one the host
            // REPORTS (D48) rather than the one a toolkit guessed - and that is
            // now a guarantee, not an aspiration: the write happens ONLY after a
            // successful measure (a failed measure defers the whole write, keep
            // the previous persisted value and retry next tick), and NEVER while
            // a move is in flight. The cost is a single GetWindowPlacement on
            // the engine thread, where blocking is allowed.
            //
            // MEASURE LATER, NEVER IMMEDIATELY, and this is the ONLY measure site:
            // with the platform's async window ops (SWP_ASYNCWINDOWPOS),
            // set_frame_rect RETURNS BEFORE THE MOVE LANDS, so a measurement at
            // registration time stores the PRE-MOVE rect and D48 becomes quietly
            // false (probe5). By the flush tick the owner has pumped many times
            // and the measured normal position is the truth. Do not move this
            // call back to restore_and_pin.
            // THE WRITE PRIORITY (the drift bug): a move in flight poisons
            // BOTH candidates - the measured read-back is the pre-move or
            // mid-drag position, and the stored rect may be the bridge's hint
            // (its own client-ish space). Persisting either walks the window
            // across the screen one chrome-height per cycle. So the whole
            // write is DEFERRED: the previous persisted value stays in the
            // file, the pending bit stays set, and a later tick - guard
            // expired, measure honest - does the write with the measured rect.
            let has_backend = self.backend.is_some();
            let move_in_flight = self
                .last_move_issued
                .is_some_and(|at| at.elapsed() < 2 * AUTOSAVE_IDLE);
            // No backend = headless (the test harness): there is no toolkit to
            // contradict the stored value, so the write proceeds. With a
            // backend, ONLY a successful measure may write.
            //
            // THE ABSENCE RULE (smoke-caught: a fresh install remembered
            // NOTHING, because every candidate write was deferred and there was
            // no previous file to fall back to). Defer protects OVERWRITE -
            // never replace a good rect with an unmeasured one - and is wrong
            // about ABSENCE: a missing file is worse than any candidate. With
            // no session.json yet, session.rect is a number the port chose
            // itself in frame space (default-or-restored, clamped at
            // registration) and CANNOT be a toolkit hint, because hints can no
            // longer reach the field. Absence persists; presence defers. Do not
            // "restore" this by removing the carve-out.
            let has_previous = self.state_dir.0.join("session.json").is_file();
            // The measure runs whenever it honestly can - it feeds the refresh
            // AND the overwrite guarantee - even on a flush whose write will be
            // deferred. Only an in-flight move (stale read-back) skips it.
            let measurable = !move_in_flight;
            let measured = if has_backend && measurable {
                self.measure_rect()
            } else {
                // Headless (no backend): nothing can contradict the stored
                // value, so the measure is vacuously good.
                true
            };
            // THE GATE, both halves:
            // * overwrite - only a successful measure may replace the stored
            //   value (a failed measure defers: previous stays, bit retries);
            // * absence - with no session.json there is nothing to protect, and
            //   a defer would be a silent no-persist (a fresh install that
            //   remembers nothing), so the port-chosen frame-space rect writes
            //   even on a tick whose measure failed or could not run. Such a
            //   write is PROVISIONAL: the pending bit stays set so the next
            //   tick - guard expired, measure honest - re-writes with the
            //   measured rect instead of trusting an unmeasured one forever.
            let write_allowed = (measurable && measured) || !has_previous;
            if !move_in_flight {
                // MAJOR 2/5: monitor identity is refreshed HERE, in the one
                // moment the port's picture of the window updates - together
                // with the rect, so it cannot half-refresh. A launch-time
                // monitor_id frozen forever is a lie the moment the window is
                // dragged to another monitor (README: 'including when a
                // monitor has been unplugged [or] scaling has changed').
                // Skipped while a move is in flight, for the same reason the
                // rect overwrite is: the read-back names the OLD monitor. No
                // queue call here: flush_state only runs with the session bit
                // already pending, and the write below clears it - a queue at
                // this spot cannot change anything.
                {
                    if let Some(facts) = self.facts.as_ref() {
                        if let Ok((_, monitor)) =
                            facts.work_area_for_rect(to_frame(self.session.rect))
                        {
                            if self.session.monitor_id != monitor {
                                self.session.monitor_id = monitor;
                            }
                        }
                        // SCALE: the same rect, resolved with the same
                        // MONITOR_DEFAULTTONEAREST rule as work_area_for_rect,
                        // so the scale and the work area name the SAME monitor
                        // even when ids renumber. Why this number can be
                        // trusted at all: gpui 0.2.2 computes its own
                        // scale_factor with the identical call and divisor
                        // (GetDpiForMonitor, MDT_EFFECTIVE_DPI / 96.0), and
                        // app.manifest declares PerMonitorV2 - so what we
                        // persist can never disagree with what the toolkit
                        // converts physical to logical with. Err means "do not
                        // refresh this field": a dead or garbage monitor must
                        // not silently become a 1.0 default - a default scale
                        // is the lie a derivation would have been.
                        if let Ok(scale) = facts.scale_for_rect(to_frame(self.session.rect)) {
                            if (self.session.scale_factor - scale).abs() > f32::EPSILON {
                                self.session.scale_factor = scale;
                            }
                        }
                    }
                }
            }
            if write_allowed {
                match write_session(&self.state_dir.0, &self.session) {
                    Ok(()) => {
                        // A measure-backed write is done; an absence write is
                        // provisional and keeps the bit armed (see above).
                        self.pending.session = !measured || move_in_flight;
                        // Success re-arms the report: the next distinct failure is
                        // news again (M5).
                        self.session_failure_latched = false;
                    }
                    Err(err) => {
                        // M5: a locked session.json or a full disk must not become
                        // one error per tick forever. Report ONCE, latch, keep the
                        // pending bit set so the tick keeps retrying, and clear on
                        // the next success. No revision rides this event - a
                        // session file HAS none, and SaveFailed's revision-0 claim
                        // was a lie the UI could render. The reason is core's own
                        // sentence (SessionError's Display), passed through
                        // untranslated, like every other platform/core refusal.
                        if !self.session_failure_latched {
                            self.session_failure_latched = true;
                            self.emit(Event::StateWriteFailed {
                                file: StateFile::Session,
                                reason: err.to_string(),
                            });
                        }
                    }
                }
            }
            // (move in flight, or the measure refused: neither the refresh nor
            // the write ran - the file keeps the previous persisted value and
            // the pending bit survives for the next tick.)
        }
        if pending.settings {
            match write_settings(&self.state_dir, &self.settings) {
                Ok(()) => {
                    self.pending.settings = false;
                    // Success re-arms the report, exactly as the session's does.
                    self.settings_failure_latched = false;
                }
                Err(err) => {
                    // The last `revision: 0` lie (engine.rs:933, the site core
                    // named): a settings file HAS no revision, and SaveFailed
                    // is a DOCUMENT event. Same latch discipline as the session
                    // arm above - one report per failure, retried every
                    // tick, cleared on success - and core's own sentence
                    // (SettingsError's Display) as the reason.
                    if !self.settings_failure_latched {
                        self.settings_failure_latched = true;
                        self.emit(Event::StateWriteFailed {
                            file: StateFile::Settings,
                            reason: err.to_string(),
                        });
                    }
                }
            }
        }
    }

    /// Queues one coalesced write of one state file. True when this call added the
    /// bit, false when it was already outstanding - which is what makes 65 geometry
    /// updates cost one write (D11's spirit, no disk churn).
    fn queue(&mut self, target: Target) -> bool {
        let bit = match target {
            Target::Session => &mut self.pending.session,
            Target::Settings => &mut self.pending.settings,
        };
        let queued = !*bit;
        *bit = true;
        queued
    }

    /// The DOCUMENT save-failure event, and the only place one is built.
    /// [`Event::SaveFailed`] carries a real path - the file the write was
    /// attempted on - and a real revision - the buffer revision that write
    /// would have anchored (D11) - and this helper is the guarantee's single
    /// point: state files have neither fact, which is exactly why they ride
    /// their own events ([`Event::SessionWriteFailed`],
    /// [`Event::SettingsWriteFailed`], [`Event::StateDirUnusable`]) instead of
    /// this one with a revision of 0 - a number the UI could render and the
    /// user could believe. Core refuses a nameless target as InvalidPath
    /// before any save outcome exists (save.rs:141), so an empty path here
    /// could only mean the port built the lie itself; the assert is the tripwire.
    fn document_save_failed(path: PathBuf, revision: u64, reason: SaveError) -> Event {
        debug_assert!(
            !path.as_os_str().is_empty(),
            "SaveFailed is a document event: the path must name a file"
        );
        Event::SaveFailed {
            path,
            revision,
            reason,
        }
    }

    /// Records a file as opened: into the session (so the next launch restores it,
    /// which is a stored path and not an automatic Open), into the MRU, and into
    /// the menu via an event.
    ///
    /// THE SPLIT (this reverses D69's "the scratch joins the recents like any
    /// other file", and the reason is the price of that behaviour, not taste):
    /// the SCRATCH is bound into the session and kept OUT of the MRU. Reopening
    /// the scratch is not a user intent - it is the state you are in when nothing
    /// else is open - so its row is noise, and the write-side push re-aged that
    /// noise to the TOP of the list on every launch where the user typed
    /// anything, permanently displacing a real file from a ten-slot menu.
    /// Identity for restore, absence from the list. An entry a scratch put in the
    /// list BEFORE this reversal is not evicted (features.md 4.4: grey out, never
    /// silently delete); it simply stops being re-aged.
    fn remember(&mut self, path: &Path) {
        self.session.path = Some(path.to_path_buf());
        self.queue(Target::Session);
        if self.is_scratch(path) {
            // Bound, not remembered: no push, no settings.toml write armed, and
            // no RecentsUpdated either - the menu must not blink for a file the
            // user never chose.
            return;
        }
        // What goes INTO the file is the bare, case-preserved name: a FACT about
        // this path, stored per entry. The menu label is not that - a label is a
        // RENDERING of the whole list (two "readme.txt" entries disambiguate each
        // other), which is exactly why core's [`display_labels`] takes a slice
        // and not an entry, and why it is called in [`Engine::emit_recent`] and
        // nowhere on this path. Storing a rendered label here would freeze one
        // list's collisions into every future list that contains only one of
        // them. (AGENTS.md rule 5: the rule lives in core; this routes.)
        let display = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        // D13 in full - identity, case preservation, the cap, the no-duplicates
        // move-to-top - is core's function, not a re-statement of it here.
        self.settings.recents = push_recent(
            std::mem::take(&mut self.settings.recents),
            path.to_path_buf(),
            &display,
        );
        // The recents live in settings.toml: without arming THAT write, a
        // restart loses every entry the session remembered (invisible until a
        // menu existed to show it).
        self.queue(Target::Settings);
        self.emit_recent();
    }

    /// core's entries to the port's, at the one place they become UI data - and
    /// the ONE place the label rule is applied: [`display_labels`] is core's
    /// (features.md 4.4), a pure function of the WHOLE list, because two entries
    /// that share a basename disambiguate each other and no per-entry field can
    /// see that. `path` and `exists` travel as the stored facts they are; only
    /// `display` is rendered. Greying a vanished entry stays the bridge's job -
    /// a label is a naming rule, `exists` is a state (D13).
    fn emit_recent(&mut self) {
        let labels = notes_core::recent::display_labels(&self.settings.recents);
        let list = self
            .settings
            .recents
            .iter()
            .zip(labels)
            .map(|(entry, label)| RecentEntry {
                path: entry.path.clone(),
                display: label,
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

/// Both rects are physical FRAME pixels with the same four fields in the same order;
/// the two types exist because one crate may not name the other's geometry (core is
/// pure, platform is the OS). Nothing is converted - no scaling, no origin shift.
fn to_frame(rect: Rect) -> FrameRect {
    FrameRect::new(rect.x, rect.y, rect.w, rect.h)
}

/// The way back. See [`to_frame`].
#[must_use]
fn to_rect(frame: FrameRect) -> Rect {
    Rect {
        x: frame.x,
        y: frame.y,
        w: frame.w,
        h: frame.h,
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
        CoreSaveError::InvalidPath(detail) => SaveError::InvalidPath(detail),
        // 0db0b69: a link is not a file location the app may replace. The reason
        // arrives as TEXT from core because it names the link and its target, which
        // are facts about this path and this moment - exactly the pair of fields the
        // rest of this enum keeps payload-free for. Handing it to Other would read as
        // "unclassified" in the UI and hide the one instruction that helps ("save to
        // the real file instead"), so the port has a variant for it now (D29/D36:
        // add the variant, do not squash it).
        CoreSaveError::ReparsePoint(detail) => SaveError::ReparsePoint(detail),
        other => SaveError::Other(other.to_string()),
    }
}

/// A read failure, shaped for the UI. NotFound is separated out because a missing
/// file is not a failed operation and the copy is different.
fn load_error_from_io(err: &std::io::Error) -> LoadError {
    match err.kind() {
        ErrorKind::NotFound => LoadError::NotFound,
        ErrorKind::PermissionDenied => LoadError::PermissionDenied,
        // A READ, so the steps that matter are Stat and Open-of-an-existing-file:
        // sharing violations and access denials on a path we only wanted to look at.
        _ => match classify_io_error(err, IoStep::Stat) {
            CoreSaveError::Locked => LoadError::Locked,
            CoreSaveError::NotFound => LoadError::NotFound,
            CoreSaveError::PermissionDenied => LoadError::PermissionDenied,
            other => LoadError::Other(other.to_string()),
        },
    }
}

/// The read-side twin of the write gate in core's `atomic_write`: the same
/// `path_policy` verdicts, judged from the NAME ALONE, refused before the
/// first stat or read. Two call sites funnel through it - [`Engine::open`]
/// and the recents probe in [`Engine::with_host`] - because both are
/// reached without the user choosing the path (session.json's stored
/// `path` is re-issued as [`Command::Open`] on every launch, and the probe
/// stats up to ten stored names before a window exists).
///
/// Exactly four verdicts refuse, because none of them can name a document
/// a user legitimately wants edited: a stream is a second content of a
/// file the app cannot round-trip; a stripped name does not exist as the
/// OS sees it; a drive-relative path resolves against a per-drive CWD the
/// user never chose; and a reserved device that answers 0 bytes reads as
/// an EMPTY NOTE rather than an error. The write side already refuses all
/// four - this is the same policy applied before the damage instead of
/// after, not a new one.
///
/// [`PathVerdict::UnboundedNetwork`] deliberately gates NOTHING: it matches
/// every \\server\share name - legitimate notes on network shares
/// included - and the right answer to the stall it warns about (an
/// unreachable host blocks the single-threaded engine) is a bounded or
/// async probe, which is platform work, not a wider refusal.
fn read_policy_refusal(path: &Path) -> Option<LoadError> {
    let verdict = path_policy(path);
    match verdict {
        PathVerdict::StreamName
        | PathVerdict::StrippedName
        | PathVerdict::ReservedDevice
        | PathVerdict::DriveRelative => Some(LoadError::Policy(verdict)),
        PathVerdict::Allowed | PathVerdict::UnboundedNetwork => None,
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
fn existing_detected(path: &Path, codepage: Option<u16>) -> Detected {
    let Ok(meta) = fs::metadata(path) else {
        return new_file_detected();
    };
    if meta.len() == 0 || is_oversize(usize::try_from(meta.len()).unwrap_or(usize::MAX)) {
        return new_file_detected();
    }
    let Ok(bytes) = fs::read(path) else {
        return new_file_detected();
    };
    detect(&bytes, codepage)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
            // No host in a unit test: these fixtures never register a window, and a
            // None seam makes any accidental call a no-op rather than a Win32 call
            // with a made-up handle.
            None,
            None,
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
            Session {
                path: Some(PathBuf::from("no-such-dir/queued.notes")),
                ..Session::default()
            },
            Settings::default(),
            // No host in a unit test: these fixtures never register a window, and a
            // None seam makes any accidental call a no-op rather than a Win32 call
            // with a made-up handle.
            None,
            None,
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
            epoch: 0,
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
        // From 1, not 0: revision 0 is stale against a fresh engine's
        // core's saved revision and answer Clean instead of echoing the flush.
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
            Session {
                path: Some(PathBuf::from("no-such-dir/queued.notes")),
                ..Session::default()
            },
            Settings::default(),
            None,
            None,
        )
    }

    /// How many session writes are outstanding: 0 or 1, never more.
    fn pending(engine: &Engine) -> usize {
        usize::from(engine.pending.session)
    }

    /// The queue is a TARGET bit, not a counter: repeated GeometryChanged -
    /// whatever the bridge's debounce hiccup - queues one update, not N.
    #[test]
    fn repeated_geometry_at_one_rect_queues_one_update() {
        let mut engine = engine();
        // MAJOR 3: geometry describes a REGISTERED window; register one first.
        // Registration itself marks the session dirty (the fresh-install fix),
        // which is ONE pending write, and the geometry updates that follow
        // join that same write instead of stacking a second.
        engine.handle(Command::RegisterWindow {
            handle: WindowHandle(1),
        });
        assert_eq!(
            pending(&engine),
            1,
            "registration marks the session dirty exactly once"
        );

        engine.handle(Command::GeometryChanged);
        assert_eq!(pending(&engine), 1);
        for _ in 0..64 {
            engine.handle(Command::GeometryChanged);
        }
        assert_eq!(
            pending(&engine),
            1,
            "65 triggers must not queue 65 disk updates"
        );
    }

    /// A DIFFERENT rect coalesces into the same single write: the newest state
    /// is what gets persisted, and only once.
    #[test]
    fn geometry_updates_coalesce_to_the_latest_rect() {
        let mut engine = engine();
        // MAJOR 3: geometry describes a REGISTERED window; register one first.
        engine.handle(Command::RegisterWindow {
            handle: WindowHandle(1),
        });
        engine.handle(Command::GeometryChanged);
        engine.handle(Command::GeometryChanged);
        assert_eq!(
            pending(&engine),
            1,
            "one coalesced write: the queue arms a\n         target, it does not stack"
        );
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
        engine.handle(Command::GeometryChanged);
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
