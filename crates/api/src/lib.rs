//! notes-api invariant: the port — commands in, events out; no UI types, and it
//! knows no bridge exists.
//!
//! One gateway, one vocabulary in each direction, nothing else. A bridge imports
//! this crate and its own toolkit and may import no other crate in this repo;
//! `api` in turn never names a bridge, a widget or a keystroke. The arrow points
//! one way: bridge → api, never api → bridge.
//!
//! Two halves now. The VOCABULARY — [`Command`], [`Event`] and the data that
//! rides on them — is FROZEN: read it, do not edit it. The LIFECYCLE —
//! [`Gateway`] and its one engine thread — is what this slice wires, and it
//! carries no disk I/O: the file arms answer with a [`SaveError`] naming the
//! engine they are waiting on, and the next slice (W5) replaces exactly those
//! arms. `What exists, and what W5 adds` below, instead of guessing from code.
//!
//! # The four rules
//!
//! ## 1. No business logic — `api` routes and translates
//!
//! A rule that lands here instead of in `notes-core` turns the gateway into a
//! fourth domain layer and quietly destroys the headless testability of `core`
//! (AGENTS.md; docs/architecture.md §5.2 rule 5). So every value in this crate is
//! a fact somebody ELSE decided: [`FileMeta::oversize`] is the size guard's
//! verdict, not a length check the port performs (D9);
//! [`Command::GeometryChanged`] carries a rect the engine stores without
//! interpreting, because clamping and scaling are methods on `Rect` in `core`;
//! [`SaveError`] carries the copy for a failure, not a retry policy. Even
//! [`SkipReason`] reports a decision (`core::autosave` made it) rather than
//! making one.
//!
//! ## 2. No UI types
//!
//! No widget, colour, font, key event — and no keystroke at all. The bridge owns
//! the editor and the window, so the largest unit of text that crosses here is a
//! [`Command::Flush`] snapshot (docs/architecture.md §5.5: "`api` never sees a
//! keystroke, only `Flush { text }`"). The machine check is
//! `cargo tree -p notes-api -i gpui` — empty, and enforced; it is not a
//! preference. [`WindowHandle`] is what this rule costs: a window handle crosses
//! as an `i64`, because a real `HWND` would drag a platform type through the
//! port — and two versions of the `windows` crate coexist in this dependency
//! graph (0.57 through gpui, 0.61 through platform), so the two ends of the port
//! could not even agree on the type.
//!
//! ## 3. It does not know that any bridge exists
//!
//! No `#[cfg(feature = "gpui")]`, no `trait Bridge`, no UI-registered callback,
//! no branching on which toolkit is attached. It must compile with zero UI crates
//! present (docs/architecture.md §5.2 rule 4): if `api` starts asking who is
//! listening, the seam is gone.
//!
//! Related trap: do not design a `trait Bridge` while GPUI is the only
//! implementation. The seam is already "api has no UI types", and an interface
//! guessed from one implementation encodes that implementation's shape into the
//! thing you call generic (AGENTS.md; whitepaper §8, R12).
//!
//! ## 4. The threading rule
//!
//! The bridge drains events with **`try_recv`, on wake only, and NEVER blocks on
//! a channel inside a frame.** Symmetrically, nothing on the engine thread calls
//! back into the bridge, and `api` is never called from the save worker. Both
//! directions deadlock, and both fail only under load — which is exactly why this
//! is a crate rule and not a comment in one file (docs/architecture.md §5.4;
//! AGENTS.md, "never block on a channel inside a GPUI frame; never call into
//! `api` from the save worker thread").
//!
//! Rule 4 is also why `Command` has no return value and why
//! [`Event::SaveFailed`] exists. Autosave fires seconds after any UI call, on
//! another thread, and can fail on a read-only file, a permission denial, a full
//! disk or a OneDrive lock. There is no caller waiting for that `Err`, so the
//! failure crosses the port as data. Never add a synchronous `Result`-returning
//! save path "just because it is convenient" (AGENTS.md).
//!
//! **The one named exception, and why it is not this rule.** [`arm_file_drop`] is
//! synchronous and DOES answer with a `Result`. It is not a convenience: OLE binds
//! a drop target to the thread that registers it and pumps that window, so routing
//! the act through the queue would run it on the engine thread, where nothing
//! pumps — Explorer then freezes mid-drag, in the sender's process, with no error
//! anywhere in ours. So it crosses by call, never by channel: it touches no
//! `Sender`, no `Receiver` and no engine state, cannot block a frame behind a
//! queue, and its `Result` belongs to a caller standing right there with the
//! window. Its return value is also not a verdict about the user's data — it is
//! whether a registration happened — which is why it may live in this crate at all
//! (rule 1) and why the file it lands in owns no decision about a file. The rule
//! still forbids what it always forbade: a second synchronous call that answers
//! about a document.
//!
//! # Bound contract — where each decision is encoded
//!
//! * **D9** the size guard is 8 MiB and its verdict is *opened read-only*.
//!   Nothing is ever refused: a refused open is a lost document. Both verdicts
//!   are [`FileMeta`] fields (`oversize`, `read_only`), never a `Command`
//!   outcome or an error.
//! * **D10** pin state has ONE home — `session.json`. [`FileMeta::armed`] is
//!   ADR-0001 autosave arming, per document. They are different questions with
//!   different answers and must never be merged into one flag.
//! * **D11** `revision` is a monotonic `u64`, not a content hash. A `Flush` at
//!   or below the last saved revision means [`SkipReason::Clean`].
//! * **D13** recents: cap 10; identity is canonicalised + `\\?\`-stripped +
//!   case-insensitive; [`RecentEntry::display`] keeps the original case; a
//!   vanished path stays in the list with `exists: false` and STILL counts toward
//!   the cap.
//! * **D14** [`Encoding::Ansi`] carries a Windows codepage id (CP1252 = 1252), and
//!   an ANSI file writes back in that same codepage — because there is no
//!   encoding that is the inverse of "some 8-bit page" without it.
//! * **ADR-0001** [`Event::AutosaveSkipped`] is mandatory, not optional. Silence
//!   is forbidden: it must be RENDERED in the bridge's status line, not merely
//!   defined here.
//!
//! [`SaveError`]'s `Display` text IS the user-visible copy, so those strings are
//! a product surface: renaming a variant is a refactor, editing its text is a
//! decision. `event.rs` pins every string verbatim in a test for that reason.
//!
//! # What exists, and what the next slices add
//!
//! Wired now, exactly as the last slice recorded it:
//!
//! ```text
//! ```text
//! Gateway::start(state_dir: StateDir, settings: Settings) -> (Gateway, EventRx)
//! Gateway::startup_state(&mut self) -> Option<InitialState>   // once, then None
//! Gateway::send(&self, Command) -> Result<(), Command>        // Err = never accepted
//! Gateway::close(self) -> Result<(), Exit>                    // Shutdown + bounded join
//! Gateway::engine_panicked(&mut self) -> bool   // ask BEFORE close(): did it unwind?
//! Gateway::engine_is_alive(&self) -> bool       // a test seam, see below
//! pub type EventRx = std::sync::mpsc::Receiver<Event>;
//! ```
//!
//! Wired by this slice, with every rule in notes-core and only the routing here:
//! [`Command::Open`] stats first — so D9's guard answers from the length
//! without reading or decoding 8 MiB — then reads, detects and decodes, and answers
//! [`Event::Loaded`] or [`Event::LoadFailed`]; [`Command::SaveAs`] writes the
//! snapshot at the chosen path and ARMS the document (ADR-0001 requirement 4);
//! [`Command::Save`] is the HAND-TRIGGERED save - it routes core's
//! `Document::should_save_manual`, so neither the auto-save toggle nor a foreign
//! file's disarm refuses it while read-only and oversize still do, and every ask
//! is answered by exactly one of [`Event::Saved`] (then [`Event::Rebound`], which
//! is how the arming it just caused reaches the bridge), [`Event::SaveFailed`], or
//! [`Event::AutosaveSkipped`]; an untitled note binds the same scratch file the
//! debounced path binds, so no save on this port requires a dialog;
//! [`Command::Flush`] saves behind core's own `Document`'s fixed skip order, so a foreign
//! file refuses and says why, and a stale revision is reported Clean rather than
//! written (D11); [`Command::ClearRecents`] empties the list the engine reports;
//! and the session write that [`Command::GeometryChanged`] queues is performed on
//! the tick arm and once more inside the shutdown drain.
//!
//! The startup order this API supports, in the shape docs/architecture.md §5.5
//! fixes it, with the call each step makes:
//!
//! ```text
//! 1. query session       Gateway::startup_state() -> Option<InitialState>  (once)
//! 2. create the window   bridge, AT the rect from step 1
//! 3. register the handle Gateway::send(Command::RegisterWindow { .. })
//! 4. apply topmost       the port, on registration, from session.pinned (D46)
//!                        again on every SetPinned that names a live window
//!
//! ```
//! Step 1 is consume-once on purpose: [`Gateway::startup_state`] is the snapshot
//! [`Gateway::start`] took, not a live query - a second call would hand back a
//! pin bit and autosave toggle that [`Command::SetPinned`] has since changed. The
//! port's part of the order is that the rect and the pin arrive BEFORE the first
//! frame; step 2 is bridge work.
//!
//! **What to do with an [`Err`] from [`Gateway::send`]**: the [`Err`] carries
//! the command back, meaning the engine never took it, so no [`Event`] for it can
//! EVER arrive. Render it as a failed action and consult
//! [`Gateway::is_closed`](crate::Gateway::is_closed) - do NOT [`let _`] it,
//! because a silently dropped Flush is data loss with the label removed. [`Ok`]
//! means QUEUED, not done: the answer is still an [`Event`], later, on the
//! channel.
//!
//! WIRED (D46): the port constructs the host itself. `Gateway::start` builds
//! notes-platform's Win32 backend behind the unchanged public signature, and
//! tests inject a recording fake through `Gateway::start_with_host`, which is
//! doc(hidden) because it is a seam, not a feature - a bridge never names it.
//! notes-platform is a declared dependency, arch.rs has allowed the edge all
//! along, and the paragraph that claimed otherwise was false and is gone.
//!
//! What it unlocks, now that the frame-versus-client geometry question is settled
//! (Win32 `SetWindowPos` and `GetWindowRect` are both frame space, and
//! gpui 0.2.2 cannot express frame space at all): next slice, on
//! [`Command::RegisterWindow`] the port does the restore itself -- primary work
//! area, the monitor the rect belongs to, core's [`Rect::clamped_to`], then
//! `set_frame_rect` -- and applies topmost from the stored `pinned` bit,
//! which consumes the handle this crate only ever stored. Persistence moves to the
//! RESTORE rect, because a maximized `GetWindowRect` overhangs the monitor by
//! the invisible borders and storing it stores a lie. A failed restore is reported,
//! never silenced: [`Event::GeometryNotRestored`] (D29/D36: add the event). And
//! [`show = false`] is not a way to hide the correction -- gpui re-applies
//! stashed placement on activate and would stomp it.
//!
//! * [`Settings`] is an api-side placeholder because notes-core has no settings
//!   module yet; the [`Gateway::start`] parameter name survives the swap.
//! * The 750 ms cadence in `engine::AUTOSAVE_IDLE` is a constant until a loaded
//!   interval replaces it. M4 changes the BODY of the tick arm, not its shape.
//! * The [`.notes` frontmatter seam delegates to [`notes_core::format`], which
//!   landed as contracted, so the port parses nothing itself.
//! * A file over the D9 guard is **refused**, at the stat: [`Event::LoadFailed`]
//!   carrying [`LoadError::TooLarge`], with no `Loaded`, no buffer and no
//!   [`FileMeta`]. This rule used to read "opens read-only with an empty buffer
//!   rather than being refused, because a half-loaded buffer is the one option that
//!   could shorten the user's file", and the reasoning inverted: an EMPTY buffer is
//!   the maximally shortened file, and pointing Save As at it is how a 9 MiB note
//!   becomes zero bytes with `Saved` reported. [`LoadError::TooLarge`] is therefore
//!   not a reserve for "the paths that cannot present a buffer" - it IS the verdict,
//!   and no path in the port opens an oversize file. [`FileMeta::oversize`] is the
//!   one surviving trace of the old policy: a bit every construct site sets to
//!   `false`, kept as the seam a future "open read-only anyway" rule would hang from
//!   (its field doc is the authority; core's `Skip::Oversize` already refuses to
//!   write such a document, unreachable today and correct to keep).
//!
//! # The channels, and why unbounded is load-bearing
//!
//! D24: in rustc 1.98's std there is NO `mpsc::unbounded_channel` —
//! `mpsc::channel()` IS the unbounded pair, and `sync_channel(n)` is the bounded
//! one. Two `mpsc::channel()` pairs, one per direction:
//!
//! ```text
//! let (cmd_tx, cmd_rx): (Sender<Command>, Receiver<Command>) = mpsc::channel();
//! let (evt_tx, evt_rx): (Sender<Event>, Receiver<Event>) = mpsc::channel();
//! ```
//!
//! Unbounded in BOTH directions is what makes rule 4 possible, and it is a safety
//! argument, not a convenience. A bounded event queue lets a slow UI block the
//! engine inside `event_tx.send()` while that UI blocks inside its own
//! `cmd_tx.send()` — ABBA, a deadlock with no lock anywhere in sight. A bounded
//! command queue forces the other horn: block inside a frame, or drop a
//! [`Command::Flush`], which is data loss. Unbounded makes `the queue was full so
//! we dropped your SaveFailed` impossible by construction. Back-pressure is
//! core's debounce, never the channel's, and tests/reentrancy.rs pins that
//! argument with 10,000 unanswered events.
//!
//! # Ownership, so nobody has to wonder who joins
//!
//! * [`Gateway`] owns the command `Sender` and the `JoinHandle`.
//! * The engine owns the command `Receiver` and ONE clone of the event `Sender`,
//!   so when its loop ends [`EventRx`] reports Disconnected. That is how a caller
//!   — and a test — proves the thread finished.
//! * The caller owns [`EventRx`] and drains it on the UI thread.
//!
//! # The lifecycle asymmetry, deliberate and both tested
//!
//! [`Command::Shutdown`] is DRAIN AND EXIT: every command already queued behind
//! it is handled, and its events emitted, before the loop breaks. It works while
//! the caller still holds a live `Sender`.
//!
//! [`Gateway::drop`] is ABORT: the last `Sender` goes with it and the loop stops
//! at Disconnected. Only the drop path joins — join blocks, and drop is where a
//! caller has said it is done with the port. std mpsc still delivers
//! already-queued commands and already-buffered events before it reports
//! Disconnected, so `abort` never means `lose what you accepted`.
//!
//! Dropping the [`EventRx`] is a signal, not an error: the engine stops emitting
//! and keeps serving commands until Disconnected. No panic, no log spam at a dead
//! window.
//!
//! # The reentrancy trap - and precisely what it does NOT cover
//!
//! A thread-local marks the engine thread when its loop starts, and
//! [`Gateway::send`], [`Gateway::startup_state`] and [`Gateway::drop`] all
//! [`debug_assert!`] against it. Engine code that re-entered the port would queue
//! a command behind itself and then wait on its own queue. It is a
//! [`debug_assert!`] so the release build never pays a TLS read per send, which is
//! exactly why tests/reentrancy.rs asserts the trap FIRES and engine.rs's
//! `latch_probe` test proves the marking runs on the very thread
//! that runs the loop: an unarmed trap and correct code look identical outside.
//!
//! **The claim this slice corrects.** AGENTS.md names TWO deadlocks: never call
//! into the port from the engine, and never call into it from the SAVE WORKER
//! thread. This trap covers the FIRST ONLY. A thread-local is not inherited by a
//! thread that spawns it, so a worker would be unmarked and could hold a callback
//! that reaches [`Gateway::send`] without one assert firing - and a
//! [`std::thread::Builder`] spawn from inside the engine inherits nothing either.
//! There is no save worker in this build, so there is nothing to mark; when one
//! lands it needs its own tag on the same slot, its own assert, and its own test
//! (a second [`std::cell::Cell`] tag or one small [`enum`](std::option::Option),
//! decided with it). Until then this
//! paragraph, not a comment claiming coverage that does not exist, is what keeps
//! deadlock #2 honest.
//!
//! One ownership note left, because [`StateDir`] is what makes it awkward:
//! [`Gateway::start`] RECEIVES an already-resolved StateDir and never resolves
//! one (D-STATE) - and it is now RE-EXPORTED as `resolve_state_dir` so a
//! bridge can reach the rule without importing notes-core (FCR 2). Naming a rule
//! is not holding one: the single `<exe_dir>\data` portable probe still belongs
//! to whoever launches the app, `api` never calls the resolver, does not read
//! the environment, and does not look at the filesystem to find out where it
//! lives.
//! Which files the engine may touch after that is next slice's business; that it
//! may not choose WHERE is this slice's.
//!
//! The tripwire for whoever comes next: a second thread, a lock around engine
//! state, an `Instant` on a UI-visible type, or a `match` that decides something
//! about the user's data — any of those and this has stopped being a port.

// tempfile is a dev-dependency used by tests/reentrancy.rs. The lint is per
// TARGET, and the lib's own test target links the dev-deps without naming them
// — this is the documented opt-out, not a placeholder import.
#[cfg(test)]
use tempfile as _;

mod command;
mod dto;
mod engine;
mod event;
mod file_drop;
mod gateway;

pub use command::{Command, WindowHandle};
pub use dto::{Rect, Session, Settings, StateDir, resolve_state_dir};
pub use engine::mark_current_thread_as_engine;
pub use event::{
    Encoding, Event, FileMeta, LineEnding, LoadError, RecentEntry, SaveError, SkipReason, StateFile,
};
// The file-drop door: the port's ONLY synchronous `Result`-returning call, and the
// only one that answers its caller instead of emitting an Event (rule 4 names it).
// Three names, because a bridge has to be able to say what went wrong and to hold
// the registration open — a guard it cannot name is a guard it cannot keep.
pub use file_drop::{DropArmError, DropGuard, arm_file_drop};
// `Exit` is the third name because `close()` answers with it: a bridge must be able to tell a
// shutdown that ran its final save from one whose engine unwound, and that distinction is worth
// nothing if the type carrying it is unreachable from outside the crate.
pub use gateway::{EventRx, Exit, Gateway, InitialState};

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Rules 2 and 4, as a compile-time constraint on the entire vocabulary:
    /// every type on the port is owned data — no lifetime parameter, hence
    /// `'static` — and is `Send + Sync`, so it survives the two thread hops the
    /// engine will have without carrying a platform handle, a borrow or a UI
    /// object. A toolkit type anywhere in these enums fails to compile here.
    const fn assert_owned_crossable<T: Send + Sync + 'static + Clone>() {}

    const _: () = {
        assert_owned_crossable::<Command>();
        assert_owned_crossable::<Event>();
        assert_owned_crossable::<WindowHandle>();
        assert_owned_crossable::<FileMeta>();
        assert_owned_crossable::<Encoding>();
        assert_owned_crossable::<LineEnding>();
        assert_owned_crossable::<SkipReason>();
        assert_owned_crossable::<RecentEntry>();
        assert_owned_crossable::<SaveError>();
        assert_owned_crossable::<Rect>();
        assert_owned_crossable::<StateDir>();
    };

    /// The port closes: a command in, an event out, nothing in between. This is
    /// what "feed Commands, assert Events, no window" means (§5.4) — written here
    /// so the engine slice has to keep the same shape.
    #[test]
    fn commands_in_events_out_across_a_channel() {
        let (cmd_tx, cmd_rx): (std::sync::mpsc::Sender<Command>, _) = std::sync::mpsc::channel();
        let (evt_tx, evt_rx): (std::sync::mpsc::Sender<Event>, _) = std::sync::mpsc::channel();

        cmd_tx
            .send(Command::Flush {
                text: "a note".into(),
                revision: 41,
                epoch: 0,
            })
            .expect("an unbounded command send never blocks the caller");
        evt_tx
            .send(Event::Saved {
                path: PathBuf::from("C:/notes/a.notes"),
                revision: 41,
            })
            .expect("an unbounded event send never blocks the engine thread");

        // Drained the way rule 4 allows: on wake, without blocking.
        match cmd_rx.try_recv() {
            Ok(Command::Flush { text, revision, .. }) => {
                assert_eq!(text, "a note");
                assert_eq!(revision, 41, "D11: a monotonic u64, not a hash");
            }
            other => panic!("command did not survive the port: {other:?}"),
        }
        match evt_rx.try_recv() {
            Ok(Event::Saved { revision, .. }) => assert_eq!(revision, 41),
            other => panic!("event did not survive the port: {other:?}"),
        }
    }

    /// D-STATE: `api` receives a StateDir, it never resolves one. The next
    /// slice's `Gateway::start` argument must be usable without `api` calling
    /// `resolve_state_dir` — but it IS re-exported (FCR 2) so a bridge can obey the rule without importing core;
    /// the port still never calls it, and never probes anything itself - which is what
    /// the next two lines and tests/public_surface.rs together keep honest.
    #[test]
    fn state_dir_arrives_already_resolved() {
        let portable = StateDir(PathBuf::from("C:/apps/notes").join("data"));
        let installed = StateDir(PathBuf::from("C:/Users/u/AppData/Roaming").join("notes-gpui"));
        assert_eq!(
            portable.0,
            PathBuf::from("C:/apps/notes/data"),
            "a portable deployment reaches api as a StateDir, signalled by the caller"
        );
        assert_ne!(portable, installed);
    }
}
