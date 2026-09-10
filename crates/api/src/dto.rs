//! The types the UI may see.
//!
//! `api` owns none of these. They are defined one layer down in `notes-core` and
//! re-exported here so a bridge can name them without importing a crate it is
//! forbidden to import (AGENTS.md: "a bridge may import `api` and its own
//! toolkit. Nothing else in this repo"). That reach-through is not a style
//! preference — `bridge-sees-only-api` fails the build on it, which is why this
//! module exists at all: it is the one place where `core` becomes visible to a
//! UI.
//!
//! The rule for adding here: re-export a `core` type only when a
//! [`Command`](crate::Command) or an [`Event`](crate::Event) has to carry it.
//! Everything else stays private to `core`. A UI type is never added here —
//! rule 2 of the crate docs.

/// A window rect in FRAME pixels (the Win32 GetWindowRect space), owned by
/// `notes-core`, carried by [`Command::GeometryChanged`](crate::Command::GeometryChanged)
/// and stored verbatim.
///
/// Clamping, scaling and intersection are methods on this type in `core`, not
/// rules in the port: `api` routes, it does not decide (§5.2 rule 5).
pub use notes_core::geometry::Rect;

/// The directory the app persists state into — portable `<exe_dir>\data` or
/// installed `%APPDATA%\notes-gpui` (docs/architecture.md §5).
///
/// `api` RECEIVES a `StateDir`; it never resolves one. D-STATE keeps
/// `notes_core::resolve_state_dir` pure, so the single `<exe_dir>\data`
/// existence probe belongs to the caller that starts the gateway. That is why the
/// function is deliberately NOT re-exported here while the type is: re-exporting
/// it would hand the port a decision it is not allowed to make, and a portable
/// deployment is signalled to `api` by which `StateDir` arrives, never by
/// `api` probing the filesystem itself.
pub use notes_core::paths::StateDir;

/// The persisted window session: rect (frame pixels), monitor id, scale,
/// maximised, pinned, and the document that was last open. Owned by `notes-core`,
/// re-exported because [`Gateway::initial_state`](crate::Gateway::initial_state)
/// hands it to a bridge before the window exists, so the bridge has to be able to
/// NAME the type it is being handed (docs/architecture.md §5.5 step 1).
///
/// Neither `Copy` nor `Eq`: it carries an f32 scale factor and a PathBuf, so it
/// rides as a clone. That is also why it is not inside `FileMeta`, which is
/// deliberately a small `Copy` struct.
///
/// Storage is core's (session.json, written atomically); restoring it onto a real
/// window is the bridge's. The port moves the value and interprets nothing - and
/// note that the pin bit lives HERE, one home, per D10.
pub use notes_core::session::Session;
