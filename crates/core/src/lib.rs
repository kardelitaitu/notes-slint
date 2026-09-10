//! notes-core invariant: pure Rust only — no gpui, no windows crate, no
//! platform crate, no unsafe.
//!
//! The headless engine of notes-gpui. It holds the persisted window geometry
//! type (geometry::Rect), the state-directory rule (paths::resolve_state_dir)
//! and the persisted session (session::Session); the document state machine
//! and the save engine land in later milestones. No UI or OS types; failures
//! on I/O-reachable paths are typed (CoreError), never panics.

pub mod geometry;
pub mod paths;
pub mod session;

pub use geometry::Rect;
pub use paths::{StateDir, resolve_state_dir};
pub use session::{Session, SessionError};

/// Crate-wide error for I/O-reachable operations. The UI has to be able to
/// render the reason (AGENTS.md), so failures cross the seam as typed errors
/// and are never unwrapped away.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
