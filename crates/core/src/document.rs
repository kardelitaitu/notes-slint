//! The document state machine — pure, no I/O.
//!
//! The buffer (the text) never lives in core; the bridge owns the editor.
//! Core owns identity (the path), a monotonic revision (D11: a counter, NOT a
//! content hash), the dirty flag, and the per-document autosave arming state
//! (ADR-0001 option B). The save engine does the actual writing; these
//! transitions record what the save engine did and decide what it should do.

use std::path::{Path, PathBuf};

/// Autosave skip decisions: why a debounced autosave did not run.
///
/// The SAME type is the refusal shape for a hand-triggered save
/// ([`Document::should_save_manual`]): one vocabulary, so the port maps one
/// enum to `SkipReason` and the bridge renders one list of reasons — whether
/// nothing saved because a toggle is off or because the file is read-only is
/// the user's question either way.
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
        // Skip::Oversize is UNREACHABLE THROUGH THE ENGINE TODAY, and that is a
        // fact about the caller, not a dead branch here: no engine path ever sets
        // this flag. All three `Document::open` calls hard-code `false` for it
        // (api/engine.rs: `Engine::with_host`'s session restore, `Engine::open`,
        // and `Engine::restore_missing_scratch`), and an over-guard open is
        // refused at the STAT before any Document exists —
        // `LoadFailed { reason: LoadError::TooLarge }`, api/engine.rs,
        // `Engine::open`'s `if oversize` arm, pinned by
        // `an_oversize_file_is_refused_from_the_stat_and_cannot_be_overwritten`,
        // api/tests/session.rs.
        //
        // It stays because the guarantee is CORE's and not the engine's: a document
        // that carries the oversize verdict never autosaves, whoever constructed it
        // and whatever the port's policy does next. If the "open read-only anyway"
        // policy ever returns (see the field comment on api's FileMeta::oversize),
        // this arm is the one thing standing between a 9 MiB file and a rewrite on a
        // debounce. Its POSITION is load-bearing too: checked BEFORE
        // ForeignFileNotArmed and BEFORE Clean, so an oversize document is never
        // misreported as merely un-armed or merely saved. Removing it would be safe
        // against today's engine and fatal against tomorrow's, which is the wrong
        // trade to make in the crate whose whole job is do-no-harm.
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

    /// THE hand-triggered save rule: may a save the USER asked for (a Save menu
    /// row, a key chord) proceed? Same question as [`Self::should_flush`], same
    /// typed answer — `Some(reason)` is a refusal the caller MUST render
    /// (ADR-0001 requirement 1: every save answers Saved, SaveFailed, or a
    /// refusal; never silence), `None` means write it.
    ///
    /// It differs from the debounced path by EXACTLY TWO of that one's gates,
    /// and only those two:
    ///
    /// * [`Skip::AutosaveDisabled`] is BYPASSED. The toggle is a standing
    ///   instruction about UNATTENDED writes — "do not touch my file when I did
    ///   not ask". A save the user just asked for is the case the toggle was
    ///   never about, and refusing it would make "autosave off" mean "no
    ///   saving". That is the exact situation the Save row and chord are being
    ///   built for: THE PORT HAS NO PLAIN SAVE COMMAND YET (api's Command has
    ///   SaveAs and Flush only), and this method is the core half of the one a
    ///   later slice routes here — written now so the routing adds no rule.
    /// * [`Skip::ForeignFileNotArmed`] is BYPASSED, and by name rather than by
    ///   accident: ADR-0001 arms a foreign file "after the user performs one
    ///   explicit save". A gate that refused the save until the file was armed
    ///   would make its own arming act unreachable — the deadlock ADR-0001
    ///   exists to avoid. An explicit Save IS the arming act.
    ///
    /// Arming costs nothing and is NOT re-implemented here: this is a pure
    /// query. The success path already ends at [`Self::mark_saved`], the one
    /// site that sets `armed = true` and clears dirty, so this method opens no
    /// second arming or dirty-clearing door.
    ///
    /// What survives the bypass is the do-no-harm pair, in [`Self::should_autosave`]'s
    /// order so the rendered reason stays deterministic: [`Skip::ReadOnly`] —
    /// consent to save is not the write permission the filesystem withheld, and
    /// Save As to a new name is the way out (which arms, per ADR-0001
    /// requirement 4); [`Skip::Oversize`] — same reason it has in
    /// [`Self::should_autosave`]: the guarantee is core's, and a save the user
    /// asked for must not rewrite bytes the app refused to read. [`Self::should_flush`]'s
    /// D11 staleness gate needs no twin here — it exists to drop a DEBOUNCED
    /// write a newer save already covered, and a hand-triggered save carries no
    /// in-flight queue to go stale. The ordinary dirty gate below takes its
    /// place: nothing unsaved is nothing to write, and answering Clean keeps an
    /// accidental keystroke from churning the file — and, for an empty untitled
    /// buffer, from minting a scratch file for nothing (the D69 empty-buffer
    /// rule holds on this path too).
    ///
    /// NOT A DOOR PAST THE PORT. The refused-load guard that stops the measured
    /// 0-byte overwrite — api's `load_refused_for`, which refuses to write a
    /// buffer into the file whose OPEN was refused — is about a PATH, and core
    /// holds no such state: it decides only from what it was handed. The caller
    /// applies that guard before writing, and nothing here lets a manual save
    /// talk its way past it.
    ///
    /// CALLER CONTRACT (the routing is slice 3's; the anchor rule is the part a
    /// debug-only assert cannot carry): [`Self::mark_saved`]'s
    /// `debug_assert!(revision <= noted)` compiles out in release, so it is a
    /// tripwire, not the contract. A manual save must [`Self::note_revision`]
    /// the revision it is anchoring BEFORE [`Self::mark_saved`] — exactly the
    /// order `Engine::flush` uses — or a release build quietly anchors an
    /// un-noted revision and the next real flush reads as Clean and is skipped.
    /// And if the command's revision is ABOVE core's counter, the buffer moved
    /// without core being told, so the caller marks it dirty first (`flush`
    /// does, with [`Self::apply_edit`]): asking this method without that mark
    /// is how a real save gets refused as Clean.
    pub fn should_save_manual(&self) -> Option<Skip> {
        // NO `autosave_enabled` argument, and that absence IS the decision: the
        // toggle gates unattended writes only. `armed` is deliberately unread
        // here — an explicit save is what SETS it, so it cannot be the reason to
        // refuse one. `dirty` is asked instead of `revision > saved_revision`: a
        // hand-triggered save has no queue to be stale, and dirty is the state
        // that means "the file does not have this text yet".
        if self.read_only {
            return Some(Skip::ReadOnly);
        }
        if self.oversize {
            return Some(Skip::Oversize);
        }
        if !self.dirty {
            return Some(Skip::Clean);
        }
        None
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

    /// THE EMPTY-BUFFER RULE (D69 companion), proved where the state lives:
    /// a brand-new, unmodified, never-saved document is never worth a file —
    /// both the debounced flush and autosave refuse it, so the port cannot
    /// create the scratch file "for nothing" by asking core honestly. The
    /// moment text exists (one revision), both proceed. The queries are
    /// &self: asking can never bump the revision that makes it dirty.
    #[test]
    fn an_empty_untitled_document_is_never_worth_a_file_until_text_exists() {
        let mut d = Document::new();
        assert_eq!(d.revision(), 0, "asking about the buffer never edits it");
        assert!(!d.is_dirty());
        assert_eq!(
            d.should_autosave(true),
            Some(Skip::Clean),
            "empty and unmodified: autosave skips"
        );
        assert_eq!(
            d.should_flush(0, true),
            Some(Skip::Clean),
            "empty and unmodified: the flush skips too"
        );
        d.apply_edit(); // the first character exists
        assert_eq!(
            d.should_autosave(true),
            None,
            "text exists: autosave proceeds"
        );
        assert_eq!(
            d.should_flush(1, true),
            None,
            "text exists: the flush proceeds"
        );
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

    // --- THE HAND-TRIGGERED SAVE: Document::should_save_manual ---

    /// The whole state space both save rules read — native/foreign x
    /// read-only x oversize x clean/dirty — labelled, so the claim that a
    /// manual save bypasses EXACTLY the two gates the contract names is
    /// checked over every state rather than sampled by hand.
    fn every_state() -> Vec<(String, Document)> {
        let mut out = Vec::new();
        for native in [true, false] {
            for read_only in [true, false] {
                for oversize in [true, false] {
                    for dirty in [true, false] {
                        let kind = if native {
                            FileKind::Notes
                        } else {
                            FileKind::Foreign
                        };
                        let name = if native { "C:/n/a.notes" } else { "C:/n/a.md" };
                        let mut d = Document::open(Path::new(name), kind, read_only, oversize);
                        if dirty {
                            d.apply_edit();
                        }
                        out.push((
                            format!(
                                "{name} ro={read_only} oversize={oversize} dirty={dirty}",
                                name = if native { "notes" } else { "foreign" }
                            ),
                            d,
                        ));
                    }
                }
            }
        }
        out
    }

    #[test]
    fn an_autosave_disabled_document_still_saves_when_the_user_asks() {
        let mut d = native();
        d.apply_edit();
        // The debounced path is off, and owns that: the toggle refuses it.
        assert_eq!(
            d.should_flush(d.revision(), false),
            Some(Skip::AutosaveDisabled),
            "autosave stays off"
        );
        // A save the user asked for is not a debounced write.
        assert_eq!(
            d.should_save_manual(),
            None,
            "the toggle may not refuse a save the user asked for"
        );
        // One accepted Save does not switch autosave back on.
        d.note_revision(d.revision());
        d.mark_saved();
        d.apply_edit();
        assert_eq!(
            d.should_flush(d.revision(), false),
            Some(Skip::AutosaveDisabled),
            "the toggle is still the toggle for the next debounce"
        );
        assert_eq!(
            d.should_save_manual(),
            None,
            "and the next ask is still honoured"
        );
    }

    #[test]
    fn a_foreign_documents_first_explicit_save_arms_it_and_autosave_takes_over() {
        let mut d = foreign();
        d.apply_edit(); // revision 1
        // Autosave may not touch a foreign file yet (ADR-0001's disarm)...
        assert_eq!(d.should_flush(1, true), Some(Skip::ForeignFileNotArmed));
        // ...and that disarm may not be the reason to refuse the act that lifts it.
        assert_eq!(
            d.should_save_manual(),
            None,
            "an explicit Save IS the arming act"
        );
        // The port wrote it. Anchor per the caller contract: note BEFORE mark,
        // because the D11 assert is debug-only and so is not the contract.
        d.note_revision(1);
        d.mark_saved();
        // Arming was FREE — mark_saved is the only site that set it.
        assert!(d.is_armed(), "the explicit save armed the foreign file");
        assert!(!d.is_dirty());
        assert_eq!(d.saved_revision, d.revision());
        assert_eq!(
            d.should_save_manual(),
            Some(Skip::Clean),
            "nothing left to write"
        );
        // And the debounced path is live from here on, which was the point.
        d.apply_edit();
        assert_eq!(
            d.should_flush(d.revision(), true),
            None,
            "autosave runs on its own after the one save"
        );
    }

    #[test]
    fn a_read_only_document_refuses_even_an_explicit_save() {
        let mut d = Document::open(Path::new("C:/n/locked.notes"), FileKind::Notes, true, false);
        d.apply_edit();
        assert_eq!(
            d.should_save_manual(),
            Some(Skip::ReadOnly),
            "asking louder does not grant the write permission the filesystem withheld"
        );
        // Un-armed as well: the answer is still the permission, not the arming.
        let mut foreign_ro =
            Document::open(Path::new("C:/n/locked.md"), FileKind::Foreign, true, false);
        foreign_ro.apply_edit();
        assert_eq!(foreign_ro.should_save_manual(), Some(Skip::ReadOnly));
    }

    #[test]
    fn an_oversize_document_refuses_even_an_explicit_save() {
        let mut d = Document::open(Path::new("C:/n/huge.md"), FileKind::Foreign, false, true);
        d.apply_edit();
        // Both bypassed gates are true of this document, and neither is the answer:
        assert_eq!(d.should_autosave(false), Some(Skip::AutosaveDisabled));
        assert_eq!(d.should_autosave(true), Some(Skip::Oversize));
        assert_eq!(
            d.should_save_manual(),
            Some(Skip::Oversize),
            "consent to save is not consent to rewrite bytes the app refused to read"
        );
    }

    #[test]
    fn a_manual_save_answers_read_only_before_oversize() {
        // Fixed order, mirrored from should_autosave with the two bypassed arms
        // simply gone — so the reason the user sees is deterministic.
        let mut d = Document::open(
            Path::new("C:/n/locked-huge.md"),
            FileKind::Foreign,
            true,
            true,
        );
        d.apply_edit();
        assert_eq!(d.should_save_manual(), Some(Skip::ReadOnly));
    }

    #[test]
    fn an_untouched_document_has_nothing_for_an_explicit_save_to_write() {
        let blank = Document::new();
        assert_eq!(
            blank.should_save_manual(),
            Some(Skip::Clean),
            "the D69 empty-buffer rule holds here too: a stray keystroke may not mint a scratch file"
        );
        let mut d = native();
        d.apply_edit();
        d.note_revision(d.revision());
        d.mark_saved();
        assert_eq!(
            d.should_save_manual(),
            Some(Skip::Clean),
            "already on disk: nothing to write, and the answer is a reason, not silence"
        );
        assert_eq!(d.revision(), 1);
        assert!(!d.is_dirty());
    }

    #[test]
    fn an_untitled_note_is_answered_by_the_ports_path_rule_not_a_core_refusal() {
        // core adds NO new gate here: should_flush never reads the path field
        // either, and D69's scratch machinery in the port owns where a nameless
        // buffer's bytes land. A dirty untitled note is therefore a save that
        // proceeds; the empty one above is the Clean refusal.
        let mut d = Document::new();
        d.apply_edit();
        assert_eq!(d.path(), None);
        assert_eq!(d.should_save_manual(), None);
    }

    #[test]
    fn asking_whether_to_save_never_saves_anything() {
        let mut d = foreign();
        d.apply_edit();
        let before = d.clone();
        for _ in 0..3 {
            assert_eq!(d.should_save_manual(), None);
            assert_eq!(d.should_autosave(false), Some(Skip::AutosaveDisabled));
        }
        assert_eq!(d, before, "the query moved no state");
        assert!(!d.is_armed(), "asking must not arm — only the save arms");
    }

    #[test]
    fn the_documented_save_order_anchors_where_the_bridge_noted() {
        // The CALLER CONTRACT, run end to end: note_revision BEFORE mark_saved.
        // debug_assert! is the tripwire; this is the proof the order the doc
        // names is the order that works, and that the anchor lands exactly on
        // the revision the port wrote.
        let mut d = foreign();
        d.apply_edit();
        d.apply_edit(); // revision 2
        assert_eq!(d.should_save_manual(), None);
        d.note_revision(2);
        d.mark_saved(); // 2 <= noted 2: no trip
        assert_eq!(d.saved_revision, 2);
        assert!(d.is_armed());
        assert_eq!(d.should_save_manual(), Some(Skip::Clean));
    }

    #[test]
    fn an_explicit_save_bypasses_exactly_two_gates_and_no_other_verdict() {
        for (label, d) in every_state() {
            for enabled in [false, true] {
                let autosave = d.should_autosave(enabled);
                let manual = d.should_save_manual();
                // 1. A hand-triggered save is NEVER refused for either bypassed
                //    reason, in any state, toggle either way.
                assert!(
                    !matches!(
                        manual,
                        Some(Skip::AutosaveDisabled) | Some(Skip::ForeignFileNotArmed)
                    ),
                    "{label} enabled={enabled}: a Save refused for {manual:?}"
                );
                // 2. It is never STRICTER than the autosave it replaces.
                if autosave.is_none() {
                    assert_eq!(
                        manual, None,
                        "{label} enabled={enabled}: autosave would have written"
                    );
                }
                // 3. The ONLY saves it grants over autosave are the two gates the
                //    contract names — nothing else in the order ever differs.
                if manual.is_none() {
                    if let Some(refusal) = autosave {
                        assert!(
                            matches!(refusal, Skip::AutosaveDisabled | Skip::ForeignFileNotArmed),
                            "{label} enabled={enabled}: manual overruled {refusal:?}"
                        );
                    }
                }
            }
        }
    }
}
