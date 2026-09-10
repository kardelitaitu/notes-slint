//! The Gateway: the port's only handle, and the only synchronous call in the app.
//!
//! Three things live here and nowhere else: the two channels, the one engine
//! thread, and the pre-window read of session.json. Everything else in this crate
//! is data.
//!
//! The lifecycle is deliberately asymmetric, and both halves are tested in
//! tests/reentrancy.rs:
//!
//! * [`Command::Shutdown`] is **DRAIN AND EXIT** - the engine handles every
//!   command already queued behind it, emits their events, then breaks. It works
//!   while the caller still holds a live Sender, which is why a UI callback can
//!   send it and then be dropped itself.
//! * [`Gateway::drop`] is **ABORT** - the last Sender goes with it, the engine
//!   sees Disconnected and stops. Only this path joins, because join blocks and
//!   drop is the one place where a caller has already said "I am done with this
//!   port". Dropping a Gateway that was also told to Shutdown is harmless: the
//!   join just waits for an exit that is already under way.
//!
//! Disk I/O is deliberately SPLIT: the pre-window reads of the two state files
//! happen here, on the calling thread (see [`Gateway::startup_state`] for why
//! that is the whole synchronous surface), while every write - session,
//! settings, documents - happens on the engine thread, from which the bounded
//! join protects shutdown.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use notes_core::session::read_session_or_default;
use notes_core::settings::read_settings;
use notes_core::{Session, Settings, StateDir};
use notes_platform::{HostFacts, WindowBackend};

use crate::command::Command;
use crate::engine::{Engine, on_engine_thread};
use crate::event::Event;

/// The event half of the channel pair [`Gateway::start`] hands back.
///
/// Named for what the caller does with it: this is the receiving end, and the
/// caller owns it. The bridge drains it with a try_recv, on wake only, and never
/// blocks on it inside a frame - rule 4 of the crate docs.
pub type EventRx = Receiver<Event>;

/// The event Sender the engine is handed, so [`Engine::new`]'s signature names
/// the role instead of repeating a bare type.
pub(crate) type EventTx = Sender<Event>;

// The engine's starting preferences are [`Settings`], and this slice stopped
// defining it: the type belongs to notes_core::settings (settings.toml) and
// reaches a bridge through dto.rs. The parameter keeps the name [`Settings`],
// which is what the placeholder's CONTRACT comment promised, so
// [`Gateway::start`]'s signature is unchanged in shape. Plain comments rather
// than doc comments: there is no item left below to document - the stopgap struct
// this sat on is gone, and a doc comment on nothing is a hard error.

/// What the bridge needs BEFORE it creates the window (docs/architecture.md §5.5
/// step 1), in one read that costs one file.
///
/// A snapshot taken by [`Gateway::start`], not a live query: it is read once,
/// on the thread about to call CreateWindow, at a moment when no Events exist
/// yet. `pinned` is a field here rather than something the bridge infers because
/// topmost must be applied as the window appears - a window that pops up unpinned
/// and pins itself 200 ms later is a visible lie.
#[derive(Debug, Clone, PartialEq)]
pub struct InitialState {
    /// Where the window was: rect (frame pixels), monitor, scale, maximised,
    /// pinned, and the document that was open. Defaults on a first launch or a
    /// corrupt session.json - startup never fails on that file.
    pub session: Session,
    /// The global auto-save toggle, so the menu's check mark is right before the
    /// first frame.
    pub autosave_enabled: bool,
    /// D10: read straight out of the session, the one home of the pin bit.
    pub pinned: bool,
}

/// The handle on one engine thread.
///
/// `Gateway` is Send and its command Sender is Clone + Send, which is how
/// several GPUI callbacks each get a way to send without sharing a Gateway
/// reference. None of them may be called from the engine thread - the assertions
/// in [`Gateway::send`] and [`Gateway::startup_state`] are what catch it.
pub struct Gateway {
    /// Option because Drop takes it: removing the last Sender IS the abort
    /// signal, and it has to happen before the join.
    cmd_tx: Option<Sender<Command>>,
    /// Option for the same reason: join once, never twice.
    engine: Option<JoinHandle<()>>,
    /// Option because startup_state takes it: the snapshot is true exactly once.
    initial: Option<InitialState>,
}

/// The state directory: created if absent (case a), accepted as-is if it is
/// The bounded wait on the engine thread, and WHY 3 s - WITH THE HONEST WORST
/// CASE: the drain executes Flush and SaveAs as FULL document saves on this
/// thread, so a shutdown carrying a large SaveAs can legitimately take longer
/// than 3 s and be ABANDONED here while perfectly healthy - the consequence is
/// a spurious Err(Shutdown) from close() and an engine that finishes its own
/// exit later (the bridge waits for it; that trade was accepted). A healthy
/// IDLE shutdown is tens of milliseconds; 3 s is four idle ticks. The only
/// thing observed to block indefinitely is the engine inside a synchronous
/// window op whose owner is parked (12 s timed, >15 s reproduced), and the
/// deadline turns that from a hang into an abandonment. It cannot be cheaply
/// widened by queue inspection: mpsc cannot be peeked without consuming, so
/// "the queue still holds a Flush/SaveAs" is unknowable without a redesign.
const JOIN_DEADLINE: Duration = Duration::from_secs(3);

/// The std shape for a bounded join: the actual join moves to a helper thread
/// and the caller waits on a completion channel with recv_timeout. On timeout
/// the engine thread is ABANDONED, not stopped - Rust cannot cancel a thread,
/// and our send() is unbounded so it never blocks, which means a permanently
/// stuck engine can never be joined by definition; hanging shutdown on it is
/// the same bug wearing a different hat. The helper detaches and finishes
/// whenever the engine does (the engine unblocks when its owner next pumps
/// and completes its own exit, final flush included).
fn join_bounded(handle: JoinHandle<()>, deadline: Duration) -> Option<Duration> {
    let (done_tx, done_rx) = mpsc::channel::<()>();
    thread::spawn(move || {
        let _ = handle.join();
        let _ = done_tx.send(());
    });
    let started = Instant::now();
    match done_rx.recv_timeout(deadline) {
        Ok(()) => None,
        Err(_) => Some(started.elapsed()),
    }
}

/// The two host seams the port owns, built or absent together (D46). Named
/// because it is a CONCEPT the port hands its engine, not a type to spell out.
type PlatformHost = (Option<Box<dyn WindowBackend>>, Option<Box<dyn HostFacts>>);

/// The host this process runs on, as the two seams the port owns (D46). One
/// Win32 surface serves both traits, so the same unit struct is boxed twice.
/// The cfg is a build fact, not a decision: on a host notes-platform has no
/// implementation for, the port owns no host, and placement and pinning do
/// nothing - allowed to be silent because it can never be a refusal on a
/// machine this app ships to. A REAL seam's refusal never is (see
/// [`Event::GeometryNotRestored`]).
#[cfg(windows)]
fn platform_host() -> PlatformHost {
    (
        Some(Box::new(notes_platform::windows::Backend)),
        Some(Box::new(notes_platform::windows::Backend)),
    )
}

/// See [`platform_host`].
#[cfg(not(windows))]
fn platform_host() -> PlatformHost {
    (None, None)
}

impl Gateway {
    /// Starts the engine; returns the handle and the event channel the caller now
    /// owns.
    ///
    /// session.json and settings.toml are read HERE, on the caller's thread, once -
    /// the only way
    /// [`Gateway::startup_state`] can be answered before a window exists without a
    /// request/response round trip through the queue. The read never fails
    /// ([`read_session_or_default`]), so a file the user mangled cannot stop
    /// startup. Cold start: that is one small file read and nothing else. The
    /// note file is NOT touched (tests/reentrancy.rs proves it), because parsing
    /// a document before the window exists is how a 2 MB note becomes a slow
    /// title bar.
    ///
    /// Panics only if the OS refuses a thread - a process already failing. The
    /// signature is infallible by contract, and a half-built Gateway is harder to
    /// handle than no process.
    #[must_use = "dropping the Gateway aborts the engine thread"]
    pub fn start(state_dir: StateDir, settings: Settings) -> (Gateway, EventRx) {
        // D46: the port constructs the host itself, so a bridge never names a
        // notes-platform type. The cfg split is a build fact, not a decision:
        // where platform has no Win32 module to offer, the port owns no host.
        let (backend, facts) = platform_host();
        Self::start_with_host(state_dir, settings, backend, facts)
    }

    /// The seam under [`Gateway::start`], for tests: the same engine with a
    /// caller-chosen host instead of the real one. Deliberately doc(hidden) -
    /// this is a seam for the test suite, not a feature; a bridge never sees
    /// this name and never supplies a host (D46/D47).
    #[doc(hidden)]
    pub fn start_with_host(
        state_dir: StateDir,
        settings: Settings,
        backend: Option<Box<dyn WindowBackend>>,
        facts: Option<Box<dyn HostFacts>>,
    ) -> (Gateway, EventRx) {
        // Both reads happen here, on the caller's thread, before the thread exists,
        // so the engine and the InitialState come from the same bytes and cannot
        // disagree. The channels exist first because a corrupt settings file is
        // REPORTED, not defaulted over, and the queue is the only output the
        // port has (see the match below).
        let session = read_session_or_default(&state_dir.0);
        // D10's second half: settings.toml owns the autosave toggle and the recents
        // list, session.json owns the window. Reading the settings file is not
        // eager work the cold-start budget could defer - the menu cannot render a
        // check mark it has not read, and recents are the first thing a second
        // launch opens - so it belongs in this same pre-window slot rather than a
        // second round trip after the window exists. Startup still never fails on
        // a state file.
        //
        // The code page is NOT overridden here, and the old rule that said it was
        // - "the caller wins, because the code page is a machine fact" - is dead:
        // D46 gave the port its own HostFacts, so the machine is asked at detect
        // time through [`Settings::resolved_codepage`], not by a GetACP in the
        // bridge, which may not import notes-platform at all. What the FILE says
        // is the user's own choice and wins once persisted; the gap nobody chose
        // is filled by the host; when both are silent, ANSI files are refused,
        // never guessed (D27).
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
        let (event_tx, event_rx) = mpsc::channel::<Event>();
        // D54, and the port owns it because the port is the code that writes
        // into this directory. The first smoke run (d8fc569) proved the
        // fresh-install half: nothing created the directory, every write
        // returned NotFound, and the app silently never remembered anything.
        // Why 55 green tests missed that: every headless test hands the Gateway
        // a TempDir that ALREADY EXISTS - only a start from a non-existent
        // directory can see the gap, so the suite now has exactly that test.
        // THREE cases, and the middle smoke run proved they are not one:
        // (a) the directory is absent -> create it; (b) it exists with files
        // in it -> the normal case; (c) it exists and we still cannot persist
        // into it. CORE OWNS all three now (914e53a): ensure_state_dir creates
        // (idempotently), judges (NotADirectory / Symlink via the SAME
        // predicate the save path refuses on), and probes for writability -
        // the port keeps only the REPORTING, because the moment to tell the
        // user is HERE, before the engine thread exists and while the window
        // the bridge is about to create can still render it. An unrendered
        // report is the same silence as no report - which is how D54 shipped.
        if let Err(err) = notes_core::session::ensure_state_dir(&state_dir.0) {
            // Core wrote the three sentences (NotADirectory / Symlink /
            // NotWritable) as renderable copy; the port passes them verbatim.
            let _ = event_tx.send(Event::StateDirUnusable {
                reason: err.to_string(),
            });
        }
        let settings = match read_settings(&state_dir) {
            // Exactly what was written, machine locale included: it is the
            // user's own persisted choice.
            Ok(Some(persisted)) => persisted,
            // NO FILE is an absent fact, and the caller decides what a first
            // launch starts on - that is what the `settings` parameter is.
            Ok(None) => settings,
            // CORRUPT is a present fact about present bytes, and D12 forbids
            // both rewriting them and swallowing them: the launch proceeds on
            // FACTORY settings and the fact is rendered as
            // [`Event::SettingsCorrupt`], queued before the engine thread
            // exists, so it is the first thing the caller drains.
            Err(err) => {
                let _ = event_tx.send(Event::SettingsCorrupt {
                    reason: err.to_string(),
                });
                Settings::default()
            }
        };
        let initial = InitialState {
            pinned: session.pinned,
            autosave_enabled: settings.autosave_enabled,
            session: session.clone(),
        };

        let engine = Engine::new(
            cmd_rx, event_tx, state_dir, session, settings, backend, facts,
        );
        let spawned = thread::Builder::new()
            .name("notes-engine".to_string())
            .spawn(move || engine.run());

        let gateway = Gateway {
            cmd_tx: Some(cmd_tx),
            // If no thread could be started there is nothing to join. send() then
            // becomes a no-op and EventRx closes at once: the caller learns the
            // engine is gone from the only channel that still reaches them,
            // instead of this constructor unwinding.
            engine: spawned.ok(),
            initial: Some(initial),
        };
        (gateway, event_rx)
    }

    /// Hands a command to the engine and returns immediately.
    ///
    /// [`Ok`] means QUEUED, not done: the answer is an [`Event`], later, on the
    /// channel the caller owns (rule 4). A queued command is a promise the engine
    /// keeps - [`Command::Shutdown`] drains everything queued behind it, and the
    /// drain's bound (see `Engine::drain`) is the one case where the promise is
    /// knowingly broken, which is why the bound is 4 096 deep.
    ///
    /// [`Err`] hands the command BACK and means it was never accepted: the engine
    /// had already exited, or this Gateway was closed. It is a [`Result`] rather
    /// than nothing because "the engine is gone" used to be invisible - a dropped
    /// Save As with no event and no return value, which is the one failure mode
    /// AGENTS.md forbids (silence). Callers may still ignore it with [`let _`];
    /// they may not be unable to check.
    ///
    /// Panics in debug builds if called from the engine thread (see
    /// `crate::engine::on_engine_thread`).
    pub fn send(&self, command: Command) -> Result<(), Command> {
        debug_assert!(
            !on_engine_thread(),
            "engine thread re-entered the port: engine code must never hold or call a Gateway"
        );
        match &self.cmd_tx {
            Some(tx) => tx.send(command).map_err(|err| err.0),
            // Closed or dropped: the Sender went with it, so nothing can accept
            // this and no Event will ever describe it.
            None => Err(command),
        }
    }

    /// The pre-window read (§5.5 step 1): the one synchronous call on the port.
    ///
    /// It returns the snapshot taken in [`Gateway::start`] - no round trip to the
    /// engine, no block on a channel, no second file read. That is what makes it
    /// safe on the thread about to create the window, before the first try_recv
    /// could have run.
    ///
    /// CONSUME-ONCE, and [`None`] afterwards, on purpose. This is NOT a live
    /// query: [`Command::SetPinned`] and [`Command::SetAutosave`] change engine
    /// state that this snapshot cannot see, so a second call would hand back a
    /// start-of-day truth that has quietly become a lie. Taking the value makes
    /// "which is it" impossible to ask by accident. A live query needs a
    /// request/response pair in the vocabulary - it does not exist, and guessing
    /// from a stale copy is what this signature now prevents.
    #[must_use]
    pub fn startup_state(&mut self) -> Option<InitialState> {
        debug_assert!(
            !on_engine_thread(),
            "engine thread re-entered the port: startup_state is a pre-window read for the UI thread"
        );
        self.initial.take()
    }

    /// Asks the engine to shut down and WAITS for it to finish: the explicit,
    /// joinable alternative to [`impl Drop for Gateway`].
    ///
    /// [`Command::Shutdown`] is DRAIN AND EXIT, so close() is the path that
    /// guarantees every accepted command - and the final session write inside the
    /// drain - has been carried out before the caller proceeds. It returns the
    /// command back if the engine was already gone (the same [`Self::send`]
    /// contract), and it blocks, which is precisely why it is a named method and
    /// not something a destructor does silently.
    ///
    /// Never call it from the engine thread: it joins the caller's own thread.
    /// The debug_assert in [`Self::send`] fires first if a Gateway was ever
    /// reachable from engine code, and no clone of the command Sender escapes this
    /// struct to make that possible.
    pub fn close(mut self) -> Result<(), Command> {
        let result = self.send(Command::Shutdown);
        if let Some(handle) = self.engine.take() {
            // BOUNDED, never a bare join: the engine can be blocked in a
            // synchronous window op whose owner is PARKED - and the thread
            // calling close() from the window's own close callback is the one
            // thread that owner needs in order to pump. A bare join here was
            // the measured permanent hang (12 s timed, >15 s reproduced, no
            // panic, no log, no event).
            if let Some(waited) = join_bounded(handle, JOIN_DEADLINE) {
                // The report is the TYPED RESULT, not an Event: the only Event
                // sender lives in the engine, and a clone held by the Gateway
                // would delay EventRx's Disconnected past close() - breaking
                // the pinned "Disconnected means the engine is gone" contract
                // (pinned by a_slow_consumer_never_blocks_the_producer). The
                // abandoned engine's own later events still arrive on the
                // channel, and it finishes its exit when its owner next pumps.
                let _ = waited;
                return Err(Command::Shutdown);
            }
        }
        result
    }

    /// True once this Gateway cannot accept anything: [`Self::close`] ran, it was
    /// dropped, or the engine thread finished.
    ///
    /// [`Self::send`] handing the command back is the fact; this is the
    /// explanation a caller can ask for without firing a probe command, and it is
    /// what the M2 bridge should show next to a failed action.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.cmd_tx.is_none() || self.engine.as_ref().is_some_and(|h| h.is_finished())
    }

    /// True while the engine thread is still running. A test seam worth keeping:
    /// the honest alternative is polling EventRx until it goes Disconnected.
    #[must_use]
    pub fn engine_is_alive(&self) -> bool {
        self.engine
            .as_ref()
            .is_some_and(|handle| !handle.is_finished())
    }
}

impl Drop for Gateway {
    /// ABORT: take the last Sender so the engine sees Disconnected, then join.
    ///
    /// The join is the point of this Drop, and it is the only blocking call the
    /// port makes. It is safe here because dropping a Gateway is a deliberate end
    /// of ownership, not something inside a frame, and once the Sender is gone
    /// nothing else can hold the engine up.
    ///
    /// What "abort" does NOT mean: std mpsc still delivers every already-queued
    /// command before reporting Disconnected, so no accepted command and no
    /// buffered event is lost. What Shutdown adds is the explicit drain of
    /// whatever arrives while the caller still holds a live Sender, and a defined
    /// exit point. That asymmetry is why both exist.
    ///
    /// THE ASSERTION BELOW AND THE PROHIBITION ARE WHAT KEEP THAT BLOCK SAFE.
    /// Joining is only safe because no thread can be holding a Gateway that the
    /// engine itself also runs on: a shared handle cloned into engine-reachable
    /// code, then released there, would drop into a join of the caller's OWN
    /// thread - a panic inside a destructor, i.e. a process abort. Thread-locals
    /// are not inherited, so this is the only place that can see it coming.
    ///
    /// Which is why nothing may vend a cloned command [`Sender`] out of this
    /// struct, and why none is vended today. A clone on another thread is a Sender
    /// this Gateway can no longer stop: Drop would take ITS copy, the engine would
    /// still see a live queue, and the join would wait on a thread with no reason
    /// to exit - an unbounded wait inside a destructor. [`Self::send`] and
    /// [`Self::close`] are the whole surface, deliberately.
    fn drop(&mut self) {
        debug_assert!(
            !on_engine_thread(),
            "a Gateway was released on the engine thread: its Drop would join the \\
             thread it is running on, panicking inside a destructor"
        );
        drop(self.cmd_tx.take());
        if let Some(handle) = self.engine.take() {
            // BOUNDED for the same reason as close(): a bare join in a
            // destructor is an unkillable hang. A panicked engine is not
            // re-raised from a destructor (panicking in Drop while unwinding
            // aborts the process); EventRx is how the caller learns the engine
            // is gone. A timeout here CANNOT be reported - the Gateway holds no
            // Event sender by contract - and it is the one silent path, chosen
            // so the Disconnected contract survives; close() is the reporting
            // shutdown path.
            let _ = join_bounded(handle, JOIN_DEADLINE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A Gateway with no engine thread must still drop cleanly: that is the shape
    /// a thread-spawn refusal leaves behind, and it is not reproducible on demand.
    #[test]
    fn a_gateway_without_an_engine_drops_cleanly() {
        let (cmd_tx, _cmd_rx) = mpsc::channel::<Command>();
        let gateway = Gateway {
            cmd_tx: Some(cmd_tx),
            engine: None,
            initial: Some(InitialState {
                session: Session::default(),
                autosave_enabled: true,
                pinned: false,
            }),
        };
        assert!(!gateway.engine_is_alive());
        drop(gateway);
    }

    /// The started engine is alive while its Gateway is held.
    #[test]
    fn a_started_engine_is_alive() {
        let (gateway, _rx) = Gateway::start(
            StateDir(PathBuf::from("target/notes-api-tests/absent")),
            Settings::default(),
        );
        assert!(gateway.engine_is_alive());
    }

    /// The product premise: autosave starts enabled.
    /// REVIEWER ITEM 1: an already-exited engine must be OBSERVABLE. A command the
    /// engine will never accept has to come back rather than vanish into a queue
    /// with no reader - the silence AGENTS.md forbids, in its send() form.
    #[test]
    fn a_command_the_engine_cannot_accept_comes_back() {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
        drop(cmd_rx); // the shape left behind when the engine thread has finished
        let gateway = Gateway {
            cmd_tx: Some(cmd_tx),
            engine: None,
            initial: None,
        };
        assert_eq!(
            gateway.send(Command::SetPinned(true)),
            Err(Command::SetPinned(true)),
            "send must return the command itself, not swallow it"
        );
        // And a Gateway already closed has no Sender at all: same answer.
        let closed = Gateway {
            cmd_tx: None,
            engine: None,
            initial: None,
        };
        assert_eq!(closed.send(Command::Shutdown), Err(Command::Shutdown));
    }

    /// REVIEWER ITEM 2: close() is the explicit, joinable shutdown, so Drop stops
    /// being the only way to wait for the engine - and the only blocking call that
    /// carried no latch assertion.
    #[test]
    fn close_shuts_the_engine_down_and_closes_the_event_channel() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (gateway, rx) = Gateway::start(StateDir(dir.path().to_path_buf()), Settings::default());
        assert!(gateway.engine_is_alive());
        assert!(
            gateway.send(Command::SetPinned(true)).is_ok(),
            "a live engine accepts commands"
        );
        gateway.close().expect("the Shutdown was accepted");
        assert!(
            rx.recv().is_err(),
            "the engine exited, so EventRx reports Disconnected"
        );
    }

    /// REVIEWER ITEM 5: startup_state() is a snapshot handed over ONCE, not a live
    /// query. SetPinned changes engine state this value cannot see, so returning it
    /// a second time would be a stale truth with a straight face.
    #[test]
    fn startup_state_is_handed_over_once_and_is_not_the_live_value() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (mut gateway, _rx) =
            Gateway::start(StateDir(dir.path().to_path_buf()), Settings::default());
        let snapshot = gateway
            .startup_state()
            .expect("the first call gets the pre-window snapshot");
        assert!(!snapshot.pinned, "a fresh state dir has nothing pinned");
        assert!(snapshot.autosave_enabled);
        assert!(
            gateway.startup_state().is_none(),
            "the snapshot is taken, not kept - a second call must not repeat it"
        );
        // The lie this prevents: the engine now disagrees with the old snapshot.
        assert!(gateway.send(Command::SetPinned(true)).is_ok());
        assert!(gateway.startup_state().is_none());
    }

    #[test]
    fn autosave_starts_enabled() {
        assert!(Settings::default().autosave_enabled);
        let off = Settings {
            autosave_enabled: false,
            ..Settings::default()
        };
        assert!(!off.autosave_enabled);
    }
}
