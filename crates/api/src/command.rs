//! Commands in: the only vocabulary a UI is allowed to use against the engine.
//!
//! One variant per user intent, plus the two window facts the bridge reports.
//! A `Command` has no return value on purpose. The only caller is a UI event
//! handler that must never block (see the threading rule in the crate docs), so
//! everything the caller would want to learn comes back later as an
//! [`Event`](crate::Event) — including the failures, which under autosave have
//! no caller to be returned to (docs/architecture.md §5.4, AGENTS.md "Autosave
//! is asynchronous").
//!
//! This module is **data only**. It validates nothing, decides nothing and owns
//! no state: `api` routes and translates, the rule lives in `notes-core`
//! (AGENTS.md, docs/architecture.md §5.2 rule 5).

use std::path::PathBuf;

use crate::dto::Rect;

/// A window handle, crossing the port as a plain integer.
///
/// The bridge owns the window and the toolkit creates it (docs/architecture.md
/// §5.5); the engine only ever *holds* the handle and hands it to
/// `notes-platform`, which decides nothing with it.
///
/// On Windows this is an `HWND` widened from `isize` to `i64` (lossless on
/// x86_64 and arm64). `i64` rather than a platform type because **no platform or
/// UI type may ever cross this port**: two versions of the `windows` crate
/// coexist in the dependency graph — 0.57 through gpui, 0.61 through platform —
/// so a shared `HWND` would not even name the same type on both sides. And
/// `cargo tree -p notes-api -i gpui` must stay empty; that is a build gate, not
/// a preference.
///
/// `0` is the no-window value. It is deliberately not `Option<WindowHandle>`:
/// a handle that has not been registered yet is a normal state of the port, not
/// an error a caller must unwrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowHandle(pub i64);

/// Everything a bridge can ask the engine to do.
///
/// Fire-and-forget by construction. Adding a variant here is the honest cost of
/// a feature; reaching around the port because a variant is missing is how the
/// seam disappears (AGENTS.md).
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Open a file from disk. Answers with [`Event::Loaded`](crate::Event::Loaded).
    /// The 8 MiB size guard never refuses a file — it opens it read-only instead
    /// (D9), and `Event::Loaded` carries the verdict in `FileMeta`.
    Open { path: PathBuf },

    /// Write the document to a new path. Choosing a location is an explicit act,
    /// so this arms the document for autosave (ADR-0001 supporting requirement 4)
    /// and puts the path into the recents list.
    ///
    /// It carries the text and the revision for the same reason
    /// [`Command::Flush`] does: the bridge owns the buffer, so a Save As that
    /// named only a path would have to write whatever the LAST debounced flush
    /// happened to leave behind - a stale document at a new path while the editor
    /// shows something else. That is data loss with a nicer name, and it is why
    /// the engine holds no text of its own. Answers [`Event::Saved`](crate::Event::Saved) and then
    /// [`Event::Rebound`](crate::Event::Rebound), or [`Event::SaveFailed`](crate::Event::SaveFailed).
    SaveAs {
        path: PathBuf,
        text: String,
        revision: u64,
    },

    /// A snapshot of the buffer: `Ctrl+S` and every autosave trigger.
    ///
    /// This is the whole of the bridge's text obligation. The live buffer stays
    /// in the bridge and no keystroke ever crosses the port
    /// (docs/architecture.md §5.5).
    ///
    /// `revision` is a monotonic counter, not a content hash (D11). A `Flush`
    /// whose revision is at or below the last saved revision means *clean*: core
    /// may report [`SkipReason::Clean`](crate::SkipReason::Clean), but on a path
    /// the user triggered by hand it must not go silent.
    Flush { text: String, revision: u64 },

    /// The menu's global auto-save toggle. Per-document arming is a different
    /// thing and lives in [`FileMeta::armed`](crate::FileMeta::armed); do not
    /// merge the two (D10).
    SetAutosave(bool),

    /// Pin the window above every other window. Persisted in `session.json`,
    /// which is the one home of pin state (D10).
    SetPinned(bool),

    /// Empty the recent-files list. Answers with
    /// [`Event::RecentsUpdated`](crate::Event::RecentsUpdated).
    ClearRecents,

    /// Stop the engine: flush pending state, join the worker thread. No
    /// user-visible answer is promised.
    Shutdown,

    /// Register the window the bridge just created, so `platform` has a handle
    /// to act on. Sent once, right after window creation: geometry *restore* is
    /// bridge work, geometry *storage* is core work (AGENTS.md, startup order).
    RegisterWindow { handle: WindowHandle },

    /// Report the window's current frame rect. The engine stores it; it does not
    /// interpret, clamp or scale it — those are methods on [`Rect`](crate::Rect)
    /// in `notes-core`, and they belong there.
    GeometryChanged { rect: Rect },

    /// The window the bridge registered is GONE (destroyed, recreated). The
    /// stored handle is a VALUE, not a lease: a destroyed HWND fails closed
    /// (IsWindow refuses it), but Windows RECYCLES handle numbers, so a stale
    /// one can pass IsWindow and name a STRANGER's window - and the port would
    /// then read that stranger's normal position into session.rect and MOVE
    /// it. Unregistering clears the stored value; every host call and every
    /// GeometryChanged after it is a no-op until the next RegisterWindow.
    UnregisterWindow,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rebuilds a command by destructuring it, so the `match` is checked for
    /// exhaustiveness by the compiler: a new variant cannot land here without
    /// being covered by the round-trip test below.
    fn rebuild(command: &Command) -> Command {
        match command {
            Command::Open { path } => Command::Open { path: path.clone() },
            Command::SaveAs {
                path,
                text,
                revision,
            } => Command::SaveAs {
                path: path.clone(),
                text: text.clone(),
                revision: *revision,
            },
            Command::Flush { text, revision } => Command::Flush {
                text: text.clone(),
                revision: *revision,
            },
            Command::SetAutosave(on) => Command::SetAutosave(*on),
            Command::SetPinned(on) => Command::SetPinned(*on),
            Command::ClearRecents => Command::ClearRecents,
            Command::Shutdown => Command::Shutdown,
            Command::RegisterWindow { handle } => Command::RegisterWindow { handle: *handle },
            Command::GeometryChanged { rect } => Command::GeometryChanged { rect: *rect },
            Command::UnregisterWindow => Command::UnregisterWindow,
        }
    }

    /// The name of each variant, used to prove the fixture below really holds
    /// one instance of every variant and that no two collapse onto each other.
    fn variant(command: &Command) -> &'static str {
        match command {
            Command::Open { .. } => "Open",
            Command::SaveAs { .. } => "SaveAs",
            Command::Flush { .. } => "Flush",
            Command::SetAutosave(_) => "SetAutosave",
            Command::SetPinned(_) => "SetPinned",
            Command::ClearRecents => "ClearRecents",
            Command::Shutdown => "Shutdown",
            Command::RegisterWindow { .. } => "RegisterWindow",
            Command::GeometryChanged { .. } => "GeometryChanged",
            Command::UnregisterWindow => "UnregisterWindow",
        }
    }

    fn all_commands() -> Vec<Command> {
        vec![
            Command::Open {
                path: PathBuf::from("C:/notes/a.notes"),
            },
            Command::SaveAs {
                path: PathBuf::from("C:/notes/b.notes"),
                text: "the buffer, at the moment the dialog closed".to_string(),
                revision: 4,
            },
            Command::Flush {
                text: "hello".to_string(),
                revision: 7,
            },
            Command::SetAutosave(true),
            Command::SetPinned(false),
            Command::ClearRecents,
            Command::Shutdown,
            Command::RegisterWindow {
                handle: WindowHandle(-1_234_567_890),
            },
            Command::GeometryChanged {
                rect: Rect::new(120, 80, 900, 600),
            },
            Command::UnregisterWindow,
        ]
    }

    /// Acceptance (a): every `Command` variant is constructible, clones,
    /// compares equal to itself under `PartialEq`, and survives a
    /// match-and-rebuild round trip.
    #[test]
    fn every_command_variant_round_trips() {
        let all = all_commands();
        let names: Vec<&'static str> = all.iter().map(variant).collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            all.len(),
            "the fixture must cover each variant exactly once: {names:?}"
        );
        // Open, SaveAs, Flush, SetAutosave, SetPinned, ClearRecents, Shutdown,
        // RegisterWindow, GeometryChanged, UnregisterWindow.
        assert_eq!(all.len(), 10, "Command gained or lost a variant");

        for command in &all {
            assert_eq!(command, &command.clone(), "{command:?} clone is not equal");
            assert_eq!(&rebuild(command), command, "round trip changed {command:?}");
        }
    }

    /// `PartialEq` must discriminate payloads, not just variants — otherwise
    /// the round trip above proves too little.
    #[test]
    fn commands_compare_their_payloads() {
        let mut commands = all_commands();
        let first = commands.remove(0);
        for other in &commands {
            assert_ne!(&first, other, "distinct variants must not compare equal");
        }

        assert_eq!(
            Command::Open {
                path: PathBuf::from("a.txt")
            },
            Command::Open {
                path: PathBuf::from("a.txt")
            }
        );
        assert_ne!(
            Command::Open {
                path: PathBuf::from("a.txt")
            },
            Command::Open {
                path: PathBuf::from("b.txt")
            }
        );
        assert_ne!(
            Command::Flush {
                text: "x".into(),
                revision: 1
            },
            Command::Flush {
                text: "x".into(),
                revision: 2
            }
        );
        assert_ne!(Command::SetAutosave(true), Command::SetAutosave(false));
        assert_ne!(
            Command::RegisterWindow {
                handle: WindowHandle(1)
            },
            Command::RegisterWindow {
                handle: WindowHandle(2)
            }
        );
    }

    /// Acceptance (c), part 1: a compile-time assertion that `WindowHandle` is
    /// a plain integer and not a platform type. `Copy` plus `Send + Sync +
    /// Unpin + 'static` rules out any borrowed, interior-mutable or
    /// reference-counted handle object; `Eq + Hash` says its identity is its
    /// bits.
    const _: fn() = || {
        const fn assert_plain_handle<
            T: Copy + Eq + std::hash::Hash + Send + Sync + Unpin + 'static,
        >() {
        }
        assert_plain_handle::<WindowHandle>();
    };

    /// Acceptance (c), part 2: constructible in `const` context, which a type
    /// with drop glue or hidden state could not be.
    const NO_WINDOW: WindowHandle = WindowHandle(0);

    #[test]
    fn window_handle_is_exactly_one_integer() {
        assert_eq!(
            std::mem::size_of::<WindowHandle>(),
            std::mem::size_of::<i64>(),
            "WindowHandle must carry nothing beyond the i64"
        );
        assert_eq!(NO_WINDOW.0, 0);
        // The bridge's cast, spelled out: HWND -> isize -> i64 is lossless.
        let from_hwnd: i64 = -1isize as i64;
        assert_eq!(WindowHandle(from_hwnd).0, from_hwnd);
        let set: std::collections::HashSet<WindowHandle> =
            [WindowHandle(0), WindowHandle(1), WindowHandle(0)]
                .into_iter()
                .collect();
        assert_eq!(set.len(), 2, "identity must be the integer");
    }
}
