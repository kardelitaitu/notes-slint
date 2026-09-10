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
    /// The highest revision the bridge has NOTED (note_revision) — the
    /// ceiling the save anchor is allowed to land on. D11: an anchor above
    /// this ceiling is a revision the bridge never reached, and the
    /// stale-flush gate would then judge real work Clean against it.
    noted: u64,
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
            noted: 0,
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
            noted: 0,
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
    ///
    /// D11 ANCHOR INVARIANT: saved_revision may only land on a revision the
    /// bridge NOTED. debug_assert is the right strength here: it can fire
    /// only on a programming error in the save wiring — anchoring a save
    /// whose revision was never noted — never on bad user data, which cannot
    /// choose when core anchors a save. The consequence it names is the
    /// exact bug D11 exists to stop: a stale Flush judged Clean against an
    /// invented anchor, a real save silently skipped. (It compiles out in
    /// release; the typed gate in should_flush still holds there.)
    pub fn mark_saved(&mut self) {
        debug_assert!(
            self.revision <= self.noted,
            "D11 anchor: mark_saved anchored revision {} but the bridge only noted up to {} — an un-noted anchor makes the next stale Flush read as Clean, silently skipping a real save",
            self.revision,
            self.noted
        );
        self.dirty = false;
        self.saved_revision = self.revision;
        self.armed = true;
    }

    /// Records the caller's (bridge-owned) revision for this document: the
    /// port's Command::Flush carries the revision, and the BRIDGE owns the
    /// revision space, so this is how core is told where the document
    /// stands. Monotonicity rule (D11): a LOWER revision never rewinds the
    /// counter — the internal value only ever moves up — and it therefore
    /// never un-saves a document (saved_revision only ever moves at
    /// mark_saved, and the counter it is anchored to cannot go backwards).
    /// This is an observation, not an edit: the dirty flag is untouched.
    /// It also raises the NOTED ceiling that mark_saved's D11 anchor assert
    /// checks: an anchor is only allowed on a revision the bridge noted.
    pub fn note_revision(&mut self, revision: u64) {
        self.noted = self.noted.max(revision);
        self.revision = self.revision.max(revision);
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

    /// THE single D11 gate for debounced autosave: a flush that observed the
    /// document at revision is STALE when that revision is at or below the
    /// last saved one — Clean, no write, no error, no event spam — regardless
    /// of the other state. Otherwise the same fixed order as should_autosave
    /// applies. Pair with note_revision when the bridge owns the revision
    /// space (Command::Flush carries it).
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
        // The save carried this revision: the bridge notes before the anchor.
        d.note_revision(d.revision());
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
        // The Save As carried this revision: noted before the anchor (D11).
        d.note_revision(d.revision());
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
        d.note_revision(2); // the save carried revision 2 (D11: noted first)
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
        d.note_revision(1); // the save carried revision 1 (D11: noted first)
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
    fn note_revision_adopts_the_bridge_space_monotonically() {
        // The bridge owns the revision space (Command::Flush carries it);
        // note_revision is how core is told. It adopts upwards and never
        // rewinds on a stale note.
        let mut d = native();
        d.note_revision(10);
        assert_eq!(d.revision(), 10);
        d.note_revision(12);
        assert_eq!(d.revision(), 12);
        d.note_revision(11); // a stale, lower note
        assert_eq!(d.revision(), 12, "a lower revision never rewinds");
        // An observation, not an edit: dirtiness is untouched.
        assert!(!d.is_dirty());
        assert_eq!(d.should_autosave(true), Some(Skip::Clean));
    }

    #[test]
    fn a_rewind_attempt_cannot_unsave_a_document() {
        let mut d = native();
        d.note_revision(5);
        d.mark_saved(); // saved at the bridge's revision 5
        d.note_revision(3); // a stale note trying to drag the counter back
        assert_eq!(d.revision(), 5, "the counter never rewinds");
        assert_eq!(d.saved_revision, 5, "the saved state is not un-saved");
        // Flushes at or below the saved revision stay stale-Clean.
        assert_eq!(d.should_flush(3, true), Some(Skip::Clean));
        assert_eq!(d.should_flush(5, true), Some(Skip::Clean));
        // New work above the saved revision proceeds again.
        d.note_revision(6);
        d.apply_edit(); // revision 7, dirty
        assert_eq!(d.should_flush(7, true), None, "new work proceeds");
    }

    #[test]
    fn d33_the_stale_flush_gate_fails_if_deleted() {
        // D33: this test exists to fail if the `revision <= saved_revision`
        // early return is removed from should_flush. A DIRTY document whose
        // last saved revision is high: without the gate, should_autosave
        // would happily proceed; with it, the stale flush is Clean.
        let mut d = native();
        d.note_revision(10);
        d.mark_saved(); // saved at 10
        d.apply_edit(); // dirty at 11
        assert!(d.is_dirty());
        assert_eq!(
            d.should_flush(9, true),
            Some(Skip::Clean),
            "stale: gate required"
        );
        assert_eq!(
            d.should_flush(10, true),
            Some(Skip::Clean),
            "at-saved: stale"
        );
        assert_eq!(d.should_flush(11, true), None, "current work proceeds");
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

    /// D33: the D11 anchor assert exists to be TRIP-ABLE. The wiring bug it
    /// catches: a save anchored at a revision the bridge never noted — the
    /// next stale Flush is then judged Clean against the invented anchor
    /// and a real save is silently skipped. debug_assert! compiles out in
    /// release, so this trip test is cfg'd to debug builds (cargo test).
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "D11 anchor")]
    fn mark_saved_at_an_un_noted_revision_trips_the_d11_anchor() {
        let mut d = native();
        d.apply_edit(); // revision 1, core-local: the bridge never noted it
        d.mark_saved(); // must panic: anchoring an un-noted revision
    }
}
