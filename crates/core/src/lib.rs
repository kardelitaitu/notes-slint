//! notes-core invariant: pure Rust only — no gpui, no windows crate, no
//! platform crate, no unsafe.
//!
//! The headless engine of notes-gpui. It holds the persisted window geometry
//! type (geometry::Rect), the state-directory rule (paths::resolve_state_dir),
//! the persisted session (session::Session), the document state machine
//! (document::Document) and the byte-exact text-encoding layer
//! (encoding::detect/decode/encode) and the atomic save engine (save).
//! No UI or OS types; failures on I/O-reachable paths are typed (CoreError),
//! never panics.

pub mod document;
pub mod encoding;
pub mod format;
pub mod geometry;
pub mod path_policy;
pub mod paths;
pub mod recent;
pub mod save;
pub mod session;
pub mod settings;

pub use document::{Document, FileKind, Skip};
pub use encoding::{
    DecodeError, Detected, EncodeError, LineEnding, MAX_TEXT_BYTES, SUPPORTED_ANSI_CODEPAGES,
    TextEncoding, decode, detect, encode, is_oversize, round_trip,
};
pub use format::{NoteParts, is_notes_path, rebuild, split};
pub use geometry::Rect;
pub use path_policy::{PathVerdict, path_policy};
pub use paths::{StateDir, resolve_state_dir};
pub use recent::{MAX_RECENTS, RecentEntry, RecentList, clear, identity_key, mark_missing, push};
pub use save::{
    SaveError, SaveOutcome, classify_io_error, save_document, save_document_revision,
    save_session_bytes,
};
pub use session::{Session, SessionError};
pub use settings::{Settings, SettingsError, read_settings, write_settings};

/// Crate-wide error for I/O-reachable operations. The UI has to be able to
/// render the reason (AGENTS.md), so failures cross the seam as typed errors
/// and are never unwrapped away.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
