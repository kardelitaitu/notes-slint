//! The engine loop: one thread, one receiver, no disk I/O yet.
//!
//! Lifecycle only in this slice. This file owns the loop, the drain/abort
//! asymmetry, the reentrancy trap and the deadline seam. The command ARMS that
//! need a file are deliberately incomplete and say so out loud - see the single
//! dispatch site marked TODO(W5). notes-core's encoding and save engines are
//! being written in parallel and land next slice.
//!
//! Why a thread at all: autosave fires seconds after any UI call, so it cannot
//! be answered by the call that triggered it (docs/architecture.md §5.4). One
//! engine thread owns the state; the UI thread owns an
//! [`EventRx`](crate::EventRx) of [`Event`](crate::Event)s and never runs
//! engine code.

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use notes_core::{Session, StateDir};

use crate::command::{Command, WindowHandle};
use crate::event::{Event, SaveError};
use crate::gateway::{EventTx, Settings};

/// The autosave idle cadence, in its temporary home.
///
/// It lives HERE rather than in a bridge timer on purpose: a UI timer dies with
/// the window, and then save cadence has quietly become a UI concern. M4 changes
/// the POLICY inside the tick arm - when to flush - not this structure, and a
/// settings-loaded interval replaces this constant once notes-core grows
/// settings.rs. Until then the deadline is fixed, so the tick arm fires about
/// every 750 ms and does nothing except keep the seam honest.
const AUTOSAVE_IDLE: Duration = Duration::from_millis(750);

/// The one reason a not-yet-wired command reports, so the next slice deletes a
/// constant instead of hunting strings.
const NOT_WIRED: &str = "the save path is not wired in this build: notes-core's encoding and save engines land in the next slice";

/// How many commands the shutdown drain will process, at most. Why the budget is
/// a count rather than a duration, and what happens when it runs out, is written
/// down at [`Engine::drain`].
const MAX_DRAIN: usize = 4_096;

thread_local! {
    /// Set when an engine loop starts, on that thread only. See
    /// [`on_engine_thread`].
    static ENGINE_THREAD: Cell<bool> = const { Cell::new(false) };
}

/// True when the calling thread IS an engine thread.
///
/// This is the reentrancy trap's whole mechanism. [`Gateway::send`] and
/// [`Gateway::initial_state`] assert against it, because engine code that
/// calls back into the port queues a command behind itself and then blocks
/// waiting on its own queue - a self-deadlock, and exactly the failure AGENTS.md
/// says only shows up under load.
pub(crate) fn on_engine_thread() -> bool {
    ENGINE_THREAD.with(|flag| flag.get())
}

/// Arms the trap on the current thread; disarms when the returned guard drops.
///
/// Hidden from the docs, and pub for one reason: a trap that cannot be observed
/// firing is indistinguishable from a trap that was never wired, and
/// tests/reentrancy.rs is a separate crate that has to stand on an engine thread
/// to prove the assert is live. Only [`Engine::run`] and that test call this.
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

/// Whether the loop keeps going after handling a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Continue,
    Exit,
}

/// The engine thread's entire state.
///
/// Moved into one thread and never handed back, so nothing here is shared and
/// nothing here is locked: the channels are the only synchronisation, which is
/// what makes the unbounded-queue decision safe rather than merely convenient.
pub(crate) struct Engine {
    cmd_rx: mpsc::Receiver<Command>,
    /// None means "nobody is listening any more" - see [`Self::emit`].
    event_tx: Option<Sender<Event>>,
    /// Where session.json lives. Read once by [`Gateway::start`] before this
    /// thread existed; writing it is the next slice's job, and the write happens
    /// on THIS thread, so the directory belongs here. Not read yet, and the
    /// reason is spelled out rather than left as a bare allow: W5's persistence
    /// arm is the reader, and it must not grow a second StateDir to find it.
    #[allow(dead_code, reason = "read by W5's session-persistence arm")]
    state_dir: StateDir,
    /// The in-memory session, seeded from session.json by whoever spawned us.
    /// Geometry and the pin bit are written HERE and nowhere else: D10 gives pin
    /// state one home, and this field is it.
    session: Session,
    /// The global auto-save toggle (settings.toml in the shipped app). Not the
    /// same thing as per-document arming, which is ADR-0001 and lives on the
    /// document - see [`FileMeta::armed`](crate::FileMeta::armed).
    autosave_enabled: bool,
    /// §5.5 step 3: the handle the bridge registered. Stored, never used yet -
    /// applying topmost goes through notes-platform, which this crate is
    /// forbidden to import, so this slice cannot even try by accident.
    window: Option<WindowHandle>,
    /// Coalesced "session.json needs writing": one bool, not a queue. Two
    /// geometry updates before the next flush is ONE write of the newest state,
    /// never two (D11's spirit - no disk churn). Nothing clears it in this
    /// build, because persistence is next slice's work: the flag records intent,
    /// not a completed write.
    pending_session_write: bool,
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
        Engine {
            cmd_rx,
            event_tx: Some(event_tx),
            state_dir,
            session,
            autosave_enabled: settings.autosave_enabled,
            window: None,
            pending_session_write: false,
            deadline: Instant::now() + AUTOSAVE_IDLE,
        }
    }

    /// The one engine thread. Arms the reentrancy trap for as long as it runs.
    pub(crate) fn run(mut self) {
        let _armed = mark_current_thread_as_engine();
        loop {
            match self.receive() {
                Ok(command) => {
                    if self.handle(command) == Flow::Exit {
                        break;
                    }
                }
                // The tick arm. M4 puts the autosave policy in here.
                Err(RecvTimeoutError::Timeout) => self.on_tick(),
                // The ABORT path: Gateway::drop took the last Sender. recv hands
                // over everything already queued before it reports Disconnected,
                // and nothing further can arrive, so exiting here cannot strand a
                // command the port accepted.
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        // Dropping self.event_tx here is what tells the caller's EventRx that the
        // engine is finished: their buffered events stay readable, then report
        // Disconnected. That is how a test proves this thread joined.
    }

    /// Blocks for the next command, but never past [`Self::next_deadline`].
    fn receive(&self) -> Result<Command, RecvTimeoutError> {
        match self.next_deadline() {
            Some(at) => {
                let now = Instant::now();
                // A deadline at or behind now would make recv_timeout return
                // immediately forever: a 100% CPU spin. on_tick always moves it
                // forward, so this branch costs one extra trip, not a busy loop.
                if at <= now {
                    return Err(RecvTimeoutError::Timeout);
                }
                self.cmd_rx.recv_timeout(at - now)
            }
            // No pending timer: wait for a command indefinitely.
            None => self
                .cmd_rx
                .recv()
                .map_err(|_| RecvTimeoutError::Disconnected),
        }
    }

    /// When the tick arm should next run; None means "no timer, wait for a
    /// command forever".
    ///
    /// Fixed today (see [`AUTOSAVE_IDLE`]), and a method rather than a constant
    /// because M4 makes it depend on the document: a dirty buffer, the debounce
    /// and a loaded interval all move it. The Option is what lets the engine
    /// sleep properly when there is nothing to save.
    fn next_deadline(&self) -> Option<Instant> {
        Some(self.deadline)
    }

    fn on_tick(&mut self) {
        self.deadline = Instant::now() + AUTOSAVE_IDLE;
    }

    /// Handles one command. Only [`Command::Shutdown`] returns [`Flow::Exit`].
    fn handle(&mut self, command: Command) -> Flow {
        match command {
            Command::RegisterWindow { handle } => {
                // §5.5 step 3. Stored, and nothing emitted: no UI is waiting to
                // hear its own registration back, and inventing an event for it
                // would be noise on the channel.
                self.window = Some(handle);
            }
            Command::GeometryChanged { rect } => {
                if self.session.rect != rect {
                    self.session.rect = rect;
                    self.queue_session_write();
                }
                // The same rect twice changes nothing, so it queues nothing.
            }
            Command::SetAutosave(on) => {
                // The global toggle. No event: the menu's own checked state is
                // the answer, and the consequence for a document surfaces later
                // as SkipReason::AutosaveDisabled.
                self.autosave_enabled = on;
            }
            Command::SetPinned(on) => {
                if self.session.pinned != on {
                    self.session.pinned = on;
                    self.queue_session_write();
                }
            }
            Command::ClearRecents => {
                // The recent list is notes-core's (recent.rs) and is not held
                // here yet. Answering with an EMPTY RecentsUpdated would claim a
                // list was cleared that this engine never had, so it reports as
                // unwired like the file arms instead of going silent.
                self.unwired("ClearRecents", 0);
            }
            Command::Shutdown => {
                self.drain();
                return Flow::Exit;
            }
            // TODO(W5): everything below needs file engines that are still
            // landing in notes-core, and this slice promises no disk I/O. They
            // ANSWER rather than sit silent (AGENTS.md: silence is forbidden),
            // and they never claim success - Saved is not emitted for work that
            // did not happen. Open is the awkward one: the frozen vocabulary has
            // no load-failure variant, so it borrows SaveFailed with the missing
            // piece named in the text. The honest fix is Event::LoadFailed, which
            // is a vocabulary change, not an engine one.
            Command::Open { path } => self.emit(Event::SaveFailed {
                path,
                revision: 0,
                reason: SaveError::Other(format!("{NOT_WIRED}: Open")),
            }),
            Command::SaveAs { path } => {
                // The path the user just chose is the one to name in the failure.
                self.emit(Event::SaveFailed {
                    path,
                    revision: self.last_revision(),
                    reason: SaveError::Other(format!("{NOT_WIRED}: SaveAs")),
                })
            }
            Command::Flush { revision, .. } => {
                // The revision is echoed back so the bridge can reconcile its
                // dirty state (D11): a failed save must not leave the status line
                // looking like a saved buffer.
                self.emit(Event::SaveFailed {
                    path: self.document_path(),
                    revision,
                    reason: SaveError::Other(format!("{NOT_WIRED}: Flush")),
                })
            }
        }
        Flow::Continue
    }

    /// DRAIN AND EXIT: every command already queued behind Shutdown is handled,
    /// and its events emitted, before the loop breaks. Deliberately the opposite
    /// of [`Gateway::drop`](crate::Gateway), which removes the last Sender so
    /// the engine stops at Disconnected instead. Both paths are tested in
    /// tests/reentrancy.rs; only the drop path joins.
    /// Two properties this loop has to keep, both learned from a mutation test
    /// rather than from reading it:
    ///
    /// * **No recursion.** A Shutdown found INSIDE the queue is the instruction
    ///   already being carried out, so here it is a no-op. Handing it back to
    ///   [`Self::handle`] called drain() again, and thousands of queued Shutdowns
    ///   overflowed the engine thread's stack and killed the process - not a panic, so
    ///   nothing unwinds, nothing joins, and the "clean exit" this function promises
    ///   becomes a dead process. Pinned by
    ///   [`drain_never_recurses_on_a_queued_shutdown`].
    /// * **A bound.** try_recv succeeds for as long as anyone keeps sending, so the
    ///   drain stops at [`MAX_DRAIN`] and the excess dies with the thread. A
    ///   truncated exit loses events; a hung one loses the process, because
    ///   [`Gateway::drop`](crate::Gateway) waits in join() with no timeout. Pinned by
    ///   [`drain_stops_at_its_budget_instead_of_hanging_shutdown`].
    ///
    /// [`Flow::Exit`] is never returned from in here: the caller is the one already
    /// exiting, so a nested exit is the case handled above, not a second exit.
    fn drain(&mut self) {
        for _ in 0..MAX_DRAIN {
            match self.cmd_rx.try_recv() {
                // Already carrying this one out: not a re-entry, not an event.
                Ok(Command::Shutdown) => continue,
                Ok(command) => {
                    let _ = self.handle(command);
                }
                // Nothing left in the queue. This is the normal exit.
                Err(mpsc::TryRecvError::Empty) | Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }
    }

    /// Reports a command this build cannot serve, naming the missing variant so
    /// the message is a diagnosis and not a shrug.
    fn unwired(&mut self, what: &str, revision: u64) {
        self.emit(Event::SaveFailed {
            path: self.document_path(),
            revision,
            reason: SaveError::Other(format!("{NOT_WIRED}: {what}")),
        });
    }

    /// Queues the one coalesced session write. True when this call added the
    /// pending flag, false when one was already outstanding.
    fn queue_session_write(&mut self) -> bool {
        let queued = !self.pending_session_write;
        self.pending_session_write = true;
        queued
    }

    /// The document this engine believes is open. Empty when none is - which in
    /// this build is always, because Open is unwired.
    fn document_path(&self) -> PathBuf {
        self.session.path.clone().unwrap_or_default()
    }

    /// The last revision this engine saw. api holds no buffer (the bridge owns
    /// it), so 0 until core's Document is wired in here.
    fn last_revision(&self) -> u64 {
        0
    }

    /// Emits one event, and stops emitting once the listener is gone.
    ///
    /// A closed [`EventRx`](crate::EventRx) is a signal, not an error: the UI is
    /// gone and nobody can hear anything. The engine keeps serving commands until
    /// Disconnected - shutting down is the caller's decision, not ours - and it
    /// neither panics nor spams a log at a dead window.
    fn emit(&mut self, event: Event) {
        let Some(tx) = self.event_tx.as_ref() else {
            return;
        };
        if tx.send(event).is_err() {
            self.event_tx = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notes_core::Rect;

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
        for revision in 0..excess as u64 {
            cmd_tx.send(flush(revision)).expect("unbounded");
        }

        assert_eq!(engine.handle(Command::Shutdown), Flow::Exit);
        let answered = emitted(&events);
        assert_eq!(
            answered.len(),
            MAX_DRAIN,
            "the drain must stop at exactly its budget",
        );
        assert_eq!(answered.first().copied(), Some(0));
        assert_eq!(answered.last().copied(), Some(MAX_DRAIN as u64 - 1));
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
