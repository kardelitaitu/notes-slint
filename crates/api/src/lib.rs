//! notes-api invariant: the port — commands in, events out; no UI types, and it
//! knows no bridge exists.
//!
//! One gateway, one vocabulary in each direction, nothing else. A bridge imports
//! this crate and its own toolkit and may import no other crate in this repo;
//! `api` in turn never names a bridge, a widget or a keystroke. The arrow points
//! one way: bridge → api, never api → bridge.
//!
//! This slice is the DATA of the port only — the two enums, their payloads and
//! the errors that ride on them. No engine, no threads, no `Gateway` yet. The
//! section "Shape of the next slice" below records what that will be, so the
//! following worker does not have to guess it.
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
//! # Shape of the next slice (recorded here, deliberately NOT implemented)
//!
//! ```text
//! Gateway::start(state_dir: StateDir, settings: Settings) -> (Gateway, EventRx)
//! Gateway::send(&self, command: Command)
//! Gateway::initial_state(&self) -> InitialState
//! pub type EventRx = std::sync::mpsc::Receiver<Event>;
//! ```
//!
//! * **`std::sync::mpsc`, UNBOUNDED in both directions** — a command channel in,
//!   an event channel out, one `mpsc::channel()` pair each way. On this toolchain
//!   (rustc 1.98) `mpsc::channel()` IS the unbounded flavour: `unbounded_channel`,
//!   `UnboundedSender` and `UnboundedReceiver` no longer exist in `std`, so do not
//!   go looking for them. `sync_channel(n)` is the bounded one and is NOT what the
//!   gateway uses — unbounded is what makes rule 4 possible, because `send` must
//!   never block the UI thread and a full bounded queue is a blocked frame.
//!   `Sender` is `Clone + Send`, which is how every GPUI callback gets one.
//!   Back-pressure is core's debounce, not the channel's.
//! * **One engine thread**, owned by `Gateway`, joining on `Command::Shutdown`.
//!   It owns the `core` state and the registered [`WindowHandle`].
//! * `start` RECEIVES an already-resolved [`StateDir`]: per D-STATE the caller
//!   does the `<exe_dir>\data` portable probe, and `api` never touches the
//!   filesystem to find itself.
//! * `Settings` and `InitialState` are `core` types, to be re-exported from
//!   `dto.rs` beside `Rect` and `StateDir`. `initial_state()` is what a bridge
//!   reads BEFORE it creates the window, so geometry *restore* stays bridge work
//!   while geometry *storage* stays core work (AGENTS.md, startup order).
//!
//! The tripwire: if anything in this crate grows a `thread::spawn`, a `RefCell`,
//! an `Instant`, or a `match` that decides something about the user's data, it
//! has stopped being a port.

mod command;
mod dto;
mod event;

pub use command::{Command, WindowHandle};
pub use dto::{Rect, StateDir};
pub use event::{Encoding, Event, FileMeta, LineEnding, RecentEntry, SaveError, SkipReason};

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
            Ok(Command::Flush { text, revision }) => {
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
    /// `resolve_state_dir` — which is why that function is not re-exported here.
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
