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
//! No disk I/O happens on the engine thread in this slice. [`Gateway::start`]
//! reads session.json once, on the calling thread, and that read is the whole
//! synchronous surface (see [`Gateway::initial_state`] for why).

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use notes_core::session::read_session_or_default;
use notes_core::{Session, StateDir};

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

/// The deliberate preferences the engine starts with.
///
/// A placeholder with a future owner. Settings are notes-core's (settings.toml:
/// autosave on/off, interval, recents) and that module does not exist yet - core
/// is being written in parallel with this slice. What the port needs right now is
/// exactly one value, to honour [`Command::SetAutosave`], so this declares the
/// smallest thing that compiles instead of inventing a settings format inside the
/// port (rule 1).
///
/// CONTRACT for the next slice: delete this struct, re-export the core type from
/// dto.rs. [`Gateway::start`] keeps the parameter named `Settings`, so no
/// bridge code changes when the swap happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// The GLOBAL auto-save toggle - the menu item. Not per-document arming
    /// (ADR-0001, [`FileMeta::armed`](crate::FileMeta::armed)) and not the pin bit
    /// (D10, session.json).
    /// Three different states, three different homes.
    pub autosave_enabled: bool,
}

impl Default for Settings {
    /// Autosave ON. It is the product premise ("never asks you to save"), so off
    /// has to be chosen, not discovered.
    fn default() -> Self {
        Settings {
            autosave_enabled: true,
        }
    }
}

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
/// in [`Gateway::send`] and [`Gateway::initial_state`] are what catch it.
pub struct Gateway {
    /// Option because Drop takes it: removing the last Sender IS the abort
    /// signal, and it has to happen before the join.
    cmd_tx: Option<Sender<Command>>,
    /// Option for the same reason: join once, never twice.
    engine: Option<JoinHandle<()>>,
    initial: InitialState,
}

impl Gateway {
    /// Starts the engine; returns the handle and the event channel the caller now
    /// owns.
    ///
    /// session.json is read HERE, on the caller's thread, once - the only way
    /// [`Gateway::initial_state`] can be answered before a window exists without a
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
        // Read before the thread exists, so the engine and the InitialState come
        // from ONE read and cannot disagree.
        let session = read_session_or_default(&state_dir.0);
        let initial = InitialState {
            pinned: session.pinned,
            autosave_enabled: settings.autosave_enabled,
            session: session.clone(),
        };

        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
        let (event_tx, event_rx) = mpsc::channel::<Event>();
        let engine = Engine::new(cmd_rx, event_tx, state_dir, session, settings);
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
            initial,
        };
        (gateway, event_rx)
    }

    /// Hands a command to the engine and returns immediately.
    ///
    /// No return value by design: the answer is an [`Event`], later, on the
    /// channel the caller owns (rule 4). A failed send is not worth a panic - it
    /// means the engine already exited, through Shutdown or because this Gateway
    /// was cloned into a callback that outlived it.
    pub fn send(&self, command: Command) {
        debug_assert!(
            !on_engine_thread(),
            "engine thread re-entered the port: engine code must never hold or call a Gateway"
        );
        if let Some(tx) = &self.cmd_tx {
            let _ = tx.send(command);
        }
    }

    /// The pre-window read (§5.5 step 1): the one synchronous call on the port.
    ///
    /// It returns the snapshot taken in [`Gateway::start`] - no round trip to the engine,
    /// no block on a channel, no second file read. That is what makes it safe on
    /// the thread that is about to create the window, before the first try_recv
    /// could ever have run.
    #[must_use]
    pub fn initial_state(&self) -> InitialState {
        debug_assert!(
            !on_engine_thread(),
            "engine thread re-entered the port: initial_state is a pre-window read for the UI thread"
        );
        self.initial.clone()
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
    fn drop(&mut self) {
        drop(self.cmd_tx.take());
        if let Some(handle) = self.engine.take() {
            // A panicked engine is not re-raised from a destructor (panicking in
            // Drop while unwinding aborts the process). EventRx is how the caller
            // learns the engine is gone.
            let _ = handle.join();
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
            initial: InitialState {
                session: Session::default(),
                autosave_enabled: true,
                pinned: false,
            },
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
    #[test]
    fn autosave_starts_enabled() {
        assert!(Settings::default().autosave_enabled);
        let off = Settings {
            autosave_enabled: false,
        };
        assert!(!off.autosave_enabled);
    }
}
