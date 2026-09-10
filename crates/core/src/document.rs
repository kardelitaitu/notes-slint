//! The document state machine — pure, no I/O.
//!
//! The buffer (the text) never lives in core; the bridge owns the editor.
//! Core owns identity (the path), a monotonic revision (D11: a counter, NOT a
//! content hash), the dirty flag, and the per-document autosave arming state
//! (ADR-0001 option B). The save engine does the actual writing; these
//! transitions record what the save engine did and decide what it should do.

use std::path::{Path, PathBuf};

/// Autosave skip decisions: why a debounced autosave did not run.
// NOTE(api): the PUBLIC vocabulary is api::SkipReason, written by the api
// worker in this same wave; its variant names are identical to this enum's,
// so the later dto/re-export join is mechanical. core must not depend on
// notes-api (the core-is-pure arch rule), which is why the local mirror
// exists here at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    AutosaveDisabled,
    ForeignFileNotArmed,
    Clean,
    ReadOnly,
    Oversize,
}

/// Extension classification of a document (ADR-0001 option B): the native
/// .notes format arms autosave on open; anything else (.md, .txt, ...) is
/// foreign and opens DISARMED. The caller derives the kind from the file
/// extension — a lexical check, no I/O — and passes it here, so core never
/// inspects extensions itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// Native .notes format: autosave armed from the start.
    Notes,
    /// A foreign text file: autosave disarmed until an explicit save or a
    /// Save As arms it.
    Foreign,
}

/// The per-document state. Armed is PER DOCUMENT (ADR-0001 requirement 2):
/// one Document never inherits another's arming — it is set only by the
/// constructor, an explicit save, or a Save As.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    path: Option<PathBuf>,
    /// Monotonic edit counter owned by core (D11). Every mark_dirty/apply_edit
    /// bumps it by exactly one; it is never derived from content.
    revision: u64,
    /// The revision as of the last recorded save; flushes at or below it are
    /// stale and skip as Clean.
    saved_revision: u64,
    dirty: bool,
    armed: bool,
    read_only: bool,
    oversize: bool,
}

impl Document {
    /// A brand-new untitled note: no path yet, native format by definition
    /// (an untitled note is a .notes document that has no file behind it
    /// yet), autosave armed, not read-only, not oversized.
    pub fn new() -> Self {
        Document {
            path: None,
            revision: 0,
            saved_revision: 0,
            dirty: false,
            armed: true,
            read_only: false,
            oversize: false,
        }
    }

    /// Opens a document. kind is the extension classification (see
    /// FileKind): .notes arms autosave, foreign files open disarmed
    /// (ADR-0001 option B). read_only and oversize arrive as plain bools
    /// from the caller's file metadata — core invents no FileMeta type;
    /// that vocabulary lives in api.
    pub fn open(path: &Path, kind: FileKind, read_only: bool, oversize: bool) -> Self {
        Document {
            path: Some(path.to_path_buf()),
            revision: 0,
            saved_revision: 0,
            dirty: false,
            armed: kind == FileKind::Notes,
            read_only,
            oversize,
        }
    }

    /// The open document's path, if it has one.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The current monotonic revision (D11).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether there are unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Whether autosave is currently armed for this document.
    pub fn is_armed(&self) -> bool {
        self.armed
    }

    /// Records an edit: bumps the monotonic revision and marks the document
    /// dirty. Saturates at u64::MAX rather than panicking — unreachable in
    /// practice, but core never panics.
    pub fn apply_edit(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.dirty = true;
    }

    /// Records a non-text change that must be persisted (D10's pin bit, for
    /// example). Same revision bump as apply_edit.
    pub fn mark_dirty(&mut self) {
        self.apply_edit();
    }

    /// Records a completed explicit save (the bridge did the I/O). Arms the
    /// document (ADR-0001 requirement 3: an explicit save arms) and anchors
    /// the saved revision, so stale flushes skip as Clean.
    pub fn mark_saved(&mut self) {
        self.dirty = false;
        self.saved_revision = self.revision;
        self.armed = true;
    }

    /// Records a completed Save As to path (the bridge did the I/O): the NEW
    /// path is armed (ADR-0001 requirement 4) and the document is clean at
    /// the current revision.
    pub fn save_as(&mut self, path: &Path) {
        self.path = Some(path.to_path_buf());
        self.mark_saved();
    }

    /// The autosave decision, with a FIXED check order so the rendered skip
    /// reason is deterministic: AutosaveDisabled, ReadOnly, Oversize,
    /// ForeignFileNotArmed, Clean. None means autosave should proceed.
    pub fn should_autosave(&self, autosave_enabled: bool) -> Option<Skip> {
        if !autosave_enabled {
            return Some(Skip::AutosaveDisabled);
        }
        if self.read_only {
            return Some(Skip::ReadOnly);
        }
        if self.oversize {
            return Some(Skip::Oversize);
        }
        if !self.armed {
            return Some(Skip::ForeignFileNotArmed);
        }
        if !self.dirty {
            return Some(Skip::Clean);
        }
        None
    }

    /// The flush decision for a debounced autosave that observed the document
    /// at revision: a flush at or below the last saved revision is STALE and
    /// skips as Clean — no write, no error — regardless of the other state.
    /// Otherwise the same fixed order as should_autosave applies.
    pub fn should_flush(&self, revision: u64, autosave_enabled: bool) -> Option<Skip> {
        if revision <= self.saved_revision {
            return Some(Skip::Clean);
        }
        self.should_autosave(autosave_enabled)
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn foreign() -> Document {
        Document::open(
            Path::new("C:/notes/readme.md"),
            FileKind::Foreign,
            false,
            false,
        )
    }

    fn native() -> Document {
        Document::open(
            Path::new("C:/notes/idea.notes"),
            FileKind::Notes,
            false,
            false,
        )
    }

    // --- the four ADR-0001 minimums, at state-machine level ---

    #[test]
    fn opening_a_foreign_file_disarms_autosave() {
        assert!(!foreign().is_armed(), ".md must open disarmed");
        assert!(native().is_armed(), ".notes must open armed");
    }

    #[test]
    fn documents_do_not_inherit_arming_from_each_other() {
        // An armed native document is followed by two fresh documents: their
        // arming must come from their own open() kind, not from the first.
        let first = native();
        assert!(first.is_armed());
        let second = foreign();
        assert!(
            !second.is_armed(),
            "must not inherit the first's armed state"
        );
        // ...and after the first is explicitly saved (armed), a third fresh
        // foreign document is still disarmed.
        let mut saved_first = first;
        saved_first.mark_saved();
        assert!(saved_first.is_armed());
        let third = foreign();
        assert!(!third.is_armed());
    }

    #[test]
    fn explicit_save_arms_a_foreign_document() {
        let mut d = foreign();
        assert!(!d.is_armed());
        d.apply_edit();
        d.mark_saved();
        assert!(d.is_armed(), "an explicit save arms the document");
        assert!(!d.is_dirty());
        assert_eq!(d.saved_revision, d.revision);
    }

    #[test]
    fn save_as_arms_the_new_path() {
        let mut d = foreign();
        assert!(!d.is_armed());
        d.apply_edit();
        d.save_as(Path::new("C:/notes/renamed.notes"));
        assert!(d.is_armed(), "Save As arms the new path");
        assert_eq!(d.path(), Some(Path::new("C:/notes/renamed.notes")));
        assert!(!d.is_dirty());
    }

    // --- skip precedence: fixed, deterministic order ---

    #[test]
    fn skip_precedence_is_autosave_disabled_first() {
        // Every skip condition at once: the FIRST check wins.
        let worst = Document::open(
            Path::new("C:/notes/huge-ro.md"),
            FileKind::Foreign,
            true,
            true,
        );
        assert_eq!(worst.should_autosave(false), Some(Skip::AutosaveDisabled));
        assert_eq!(worst.should_autosave(true), Some(Skip::ReadOnly));
    }

    #[test]
    fn skip_precedence_read_only_then_oversize() {
        let ro = Document::open(Path::new("C:/n/a.notes"), FileKind::Notes, true, true);
        assert_eq!(ro.should_autosave(true), Some(Skip::ReadOnly));
        let big = Document::open(Path::new("C:/n/a.notes"), FileKind::Notes, false, true);
        assert_eq!(big.should_autosave(true), Some(Skip::Oversize));
    }

    #[test]
    fn skip_precedence_foreign_then_clean() {
        let foreign_clean = foreign();
        assert_eq!(
            foreign_clean.should_autosave(true),
            Some(Skip::ForeignFileNotArmed)
        );
        assert_eq!(native().should_autosave(true), Some(Skip::Clean));
    }

    #[test]
    fn dirty_armed_document_proceeds_with_autosave() {
        let mut d = native();
        d.apply_edit();
        assert_eq!(d.should_autosave(true), None, "nothing skips this flush");
        // The toggle still dominates everything.
        assert_eq!(d.should_autosave(false), Some(Skip::AutosaveDisabled));
    }

    // --- revision monotonicity (D11) and stale flushes ---

    #[test]
    fn revision_is_monotonic_across_three_edits() {
        let mut d = native();
        assert_eq!(d.revision(), 0);
        d.apply_edit();
        assert_eq!(d.revision(), 1);
        d.apply_edit();
        assert_eq!(d.revision(), 2);
        d.mark_saved(); // saved at revision 2
        assert!(d.revision() > 1 && d.revision() >= 2);
        d.apply_edit(); // third edit
        assert_eq!(d.revision(), 3, "revision only ever goes up");
        assert!(d.is_dirty());
        assert_eq!(d.saved_revision, 2);
    }

    #[test]
    fn mark_dirty_and_apply_edit_both_bump_the_revision() {
        let mut d = native();
        d.mark_dirty();
        assert_eq!(d.revision(), 1);
        d.apply_edit();
        assert_eq!(d.revision(), 2);
        assert!(d.is_dirty());
    }

    #[test]
    fn flush_at_or_below_saved_revision_is_clean() {
        let mut d = native();
        d.apply_edit(); // revision 1
        d.mark_saved(); // saved at 1
        // A stale flush that observed revision 1 (or anything below): Clean,
        // no write, no error — even with autosave enabled and the doc dirty.
        d.apply_edit(); // revision 2, dirty
        assert_eq!(d.should_flush(1, true), Some(Skip::Clean));
        assert_eq!(d.should_flush(0, true), Some(Skip::Clean));
        // A current flush proceeds.
        assert_eq!(d.should_flush(2, true), None);
    }

    #[test]
    fn untitled_new_document_starts_armed_and_clean() {
        let d = Document::new();
        assert_eq!(d.path(), None);
        assert_eq!(d.revision(), 0);
        assert!(!d.is_dirty());
        assert!(d.is_armed(), "an untitled note is native by definition");
        assert_eq!(d.should_autosave(true), Some(Skip::Clean));
    }
}
