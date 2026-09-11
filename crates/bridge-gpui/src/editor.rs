//! The editor model, its painting, and the eight methods the platform IME calls on it.
//! Slices S1 (model) and S2 (paint, focus, keys, geometry).
//!
//! UNTIL S6 THIS WIDGET IS A DECORATION. It paints, it takes the keyboard, it answers
//! the IME, and NONE of that reaches the port: no Flush, no autosave, no Loaded text,
//! no file. Nobody should read M2 as done because text appears on screen - the text
//! goes nowhere. That is the line S2 stops at and the line S6 erases.
//!
//! What S1 bought, and S2 now stands on, is the one fact that has quietly broken
//! later slice stands on and the one thing that has quietly broken every hand-rolled
//! editor attempt: **every range in `gpui::InputHandler` is in UTF-16 CODE UNITS, and
//! a UTF-16 code unit index is not a byte index.** Handing `self.content[range]` a
//! range converted wrongly is not a wrong character, it is a panic in the middle of a
//! keystroke, and only on text a Latin-only test never types - Chinese, emoji (a
//! single pictograph is TWO UTF-16 units), or an `e` plus a combining acute.
//!
//! The shape of the model, the field list, and the conversion helpers are taken from
//! the only reference implementation in the box, gpui-0.2.2 `examples/input.rs`
//! (`TextInput` at :88-386, the UTF-16 helpers at :197-233, `Focusable` at :603-607).
//! Where the example is wrong, this file says so at the line and does the right thing
//! - see `replace_and_mark_text_in_range`.
//!
//! What is NOT here, by slice: painting and layout (S2), the line model (S3), the
//! mouse (S4), the clipboard (S5), the flush to the port (S6), caret blink and
//! read-only (S7), undo (S8).

use std::ops::Range;

use gpui_kit::prelude::*;
use gpui_kit::{
    App, Bounds, ClipboardItem, Context, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    Render, ScrollDelta, ScrollWheelEvent, ShapedLine, SharedString, Style, TextAlign, TextRun,
    UTF16Selection, UnderlineStyle, Window, actions, div, fill, point, px, relative, rgb, size,
};
use unicode_segmentation::UnicodeSegmentation;
/// A development probe, silent unless `NOTES_S2_PROBE` is set in the environment.
///
/// It exists because part of S2 is a claim about geometry - the rectangle handed to
/// the IME tracks the caret - that is invisible in a screenshot and unreachable from
/// a unit test without a window, because the caller is the text-services framework,
/// not our code. With the variable set, what each geometry method answered and the x
/// it was derived from go to stderr, the only channel a `windows_subsystem="windows"`
/// binary has. Gated rather than permanent because an unconditional `eprintln!` in a
/// paint path is a measurement artifact, not a feature; S3 deletes it with the layout
/// rework.
fn probe(what: impl AsRef<str>) {
    if std::env::var_os("NOTES_S2_PROBE").is_some() {
        eprintln!("notes-gpui: [s2] {}", what.as_ref());
    }
}

// ---------------------------------------------------------------------------
// UTF-16 <-> UTF-8, the whole reason this file exists
// ---------------------------------------------------------------------------

/// UTF-16 code-unit offset -> UTF-8 byte offset. Copied from
/// `examples/input.rs:197-211` with `self.content` turned into a parameter, which is
/// what makes it testable without a window.
///
/// Note what the loop does when `offset` lands INSIDE a surrogate pair (an odd offset
/// into an emoji): it has already added the whole character's UTF-8 length when
/// `utf16_count` first reaches or passes `offset`, so it returns the byte offset AFTER
/// the character. That is the conservative direction - it can never split a UTF-8
/// sequence, and that is the property the slicing below depends on.
pub(crate) fn offset_from_utf16(content: &str, offset: usize) -> usize {
    let mut utf8_offset = 0;
    let mut utf16_count = 0;

    for ch in content.chars() {
        if utf16_count >= offset {
            break;
        }
        utf16_count += ch.len_utf16();
        utf8_offset += ch.len_utf8();
    }

    utf8_offset
}

/// UTF-8 byte offset -> UTF-16 code-unit offset (`examples/input.rs:213-226`).
pub(crate) fn offset_to_utf16(content: &str, offset: usize) -> usize {
    let mut utf16_offset = 0;
    let mut utf8_count = 0;

    for ch in content.chars() {
        if utf8_count >= offset {
            break;
        }
        utf8_count += ch.len_utf8();
        utf16_offset += ch.len_utf16();
    }

    utf16_offset
}

/// A byte range as UTF-16 units. `examples/input.rs:228-230`.
pub(crate) fn range_to_utf16(content: &str, range: &Range<usize>) -> Range<usize> {
    offset_to_utf16(content, range.start)..offset_to_utf16(content, range.end)
}

/// A UTF-16 range as bytes. `examples/input.rs:232-234`.
pub(crate) fn range_from_utf16(content: &str, range_utf16: &Range<usize>) -> Range<usize> {
    offset_from_utf16(content, range_utf16.start)..offset_from_utf16(content, range_utf16.end)
}

/// The byte span of the character that OCCUPIES a given UTF-16 unit, or `None` when
/// the unit sits exactly on a boundary between characters (or past the end).
fn char_span_at_unit(content: &str, unit: usize) -> Option<(usize, usize)> {
    let mut utf8_offset = 0;
    let mut utf16_count = 0;
    for ch in content.chars() {
        let next_units = utf16_count + ch.len_utf16();
        if unit >= utf16_count && unit < next_units {
            return Some((utf8_offset, utf8_offset + ch.len_utf8()));
        }
        utf8_offset += ch.len_utf8();
        utf16_count = next_units;
    }
    None
}

/// Like `offset_from_utf16`, except that a unit in the MIDDLE of a surrogate pair
/// resolves to the START of that character rather than its end.
pub(crate) fn offset_from_utf16_floor(content: &str, unit: usize) -> usize {
    match char_span_at_unit(content, unit) {
        Some((start, _)) => start,
        None => offset_from_utf16(content, unit),
    }
}

/// The smallest byte offset whose UTF-16 index is >= unit: round a unit index UP to
/// the next boundary, so a unit that sits exactly between two characters does not drag
/// the following one into the range as well. That is why this is not the end of
/// `char_span_at_unit`, and why the read path pairs it with the floor above.
pub(crate) fn offset_from_utf16_ceil(content: &str, unit: usize) -> usize {
    let mut utf8_offset = 0;
    let mut utf16_count = 0;
    for ch in content.chars() {
        if utf16_count >= unit {
            return utf8_offset;
        }
        utf8_offset += ch.len_utf8();
        utf16_count += ch.len_utf16();
    }
    utf8_offset
}

/// A UTF-16 range widened outwards to whole characters, which is what a READER wants:
/// asked for the second half of an emoji, hand over the whole emoji and say so
/// through `adjusted_range`, rather than handing back an empty string that the
/// platform will build a candidate list out of. A WRITER does not use this - a caret
/// the platform put mid-pair should land after the character, not swallow it - so
/// only `text_for_range` goes through here. This is a deliberate difference from the
/// example, whose `text_for_range` (examples/input.rs:268-272) reads through the
/// plain conversion and so answers a mid-pair request with "", reporting an empty
/// adjusted range back to the IME.
pub(crate) fn range_from_utf16_outward(content: &str, range_utf16: &Range<usize>) -> Range<usize> {
    offset_from_utf16_floor(content, range_utf16.start)
        ..offset_from_utf16_ceil(content, range_utf16.end)
}

/// The byte offset of the first grapheme boundary at or after `byte`.
pub(crate) fn grapheme_boundary_after(content: &str, byte: usize) -> usize {
    let byte = byte.min(content.len());
    content
        .grapheme_indices(true)
        .map(|(index, _)| index)
        .find(|index| *index >= byte)
        .unwrap_or(content.len())
}

/// The byte offset of the last grapheme boundary at or before `byte`.
pub(crate) fn grapheme_boundary_before(content: &str, byte: usize) -> usize {
    let byte = byte.min(content.len());
    let mut last = 0usize;
    for (index, _) in content.grapheme_indices(true) {
        if index > byte {
            break;
        }
        last = index;
    }
    last
}

/// Clamp a caller-supplied byte range onto GRAPHEME CLUSTER boundaries.
///
/// S1 shipped a hand-written version of this that could only find CHARACTER
/// boundaries, because `unicode-segmentation` was not in the workspace template. The
/// FCR was granted, so the tables are real now - and the test that recorded what the
/// weaker version cost (`e` separated from its U+0301) is kept, renamed, and now
/// asserts the corrected claim instead.
///
/// Two rules, and the difference between them is the point:
///
/// * a NON-empty range widens outwards, so a replacement always consumes whole
///   clusters. Narrowing it instead would strand a combining mark with nothing to sit
///   on: valid UTF-8, and a character the user can neither type nor delete.
/// * a zero-width range - an insertion, a caret - snaps FORWARD to the next boundary.
///   Widening a caret would turn it into a deletion, so one press of the right arrow
///   inside `e` + U+0301 moves past the whole cluster rather than eating it.
///
/// Every grapheme boundary is also a character boundary, so no slice can panic.
pub(crate) fn clamp_range(content: &str, range: &Range<usize>) -> Range<usize> {
    if range.start == range.end {
        let at = grapheme_boundary_after(content, range.start);
        at..at
    } else {
        let start = grapheme_boundary_before(content, range.start);
        let end = grapheme_boundary_after(content, range.end.max(range.start));
        start..end.max(start)
    }
}

/// The splice, as a pure function, so that "it cannot panic" is a claim a test can
/// check for every range rather than one we hope the UI never produces.
/// The visual lines of a buffer, as byte ranges, each without its line feed. A buffer
/// ending in a newline gets a final empty line, which is where Enter at the end puts
/// the caret. LF only: CRLF is normalised on the way in, see the paste handler.
pub(crate) fn line_ranges(content: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (index, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            out.push(start..index);
            start = index + 1;
        }
    }
    out.push(start..content.len());
    out
}

pub(crate) fn splice(content: &str, range: &Range<usize>, text: &str) -> String {
    let range = clamp_range(content, range);
    let mut out = String::with_capacity(content.len() + text.len());
    out.push_str(&content[..range.start]);
    out.push_str(text);
    out.push_str(&content[range.end..]);
    out
}

// ---------------------------------------------------------------------------
// The state: content, selection, and the marked (uncommitted IME) range
// ---------------------------------------------------------------------------

/// Everything the input handler stores, with NO GPUI in it.
///
/// The four fields are exactly the four that `examples/input.rs` keeps on
/// `TextInput`; they live here rather than on `Editor` itself because a
/// `FocusHandle` cannot be constructed outside an `App` (`FocusHandle::new` is
/// `pub(crate)`, gpui src/window.rs:282, and there is no `Default`), and the whole
/// point of S1 is that the byte arithmetic is PROVEN by tests, not by a screen.
/// Slicing the state out is what the brief's escape clause asks for: a test that
/// needs the GPU to prove string arithmetic is a design smell in the code.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct TextState {
    /// UTF-8, always. The platform speaks UTF-16 only at the boundary of this struct.
    pub(crate) content: String,
    /// The caret or selection, in BYTES, always on character boundaries, always
    /// ordered (`start <= end`). Every writer here guarantees both.
    pub(crate) selected_range: Range<usize>,
    /// Whether the head of the selection is at its start (shift-selection with a
    /// mouse or a keyboard, reported to the platform through `UTF16Selection`).
    pub(crate) selection_reversed: bool,
    /// The IME composition range: text the platform has put on screen but the user
    /// has NOT committed. Bytes, not UTF-16 units, like everything else in here.
    /// THE MARKED RANGE - and the invariant that lives with it, because this field caused
    /// two bugs and can cause more.
    ///
    /// INVARIANT: every door that moves the CARET or the SELECTION away from where the
    /// composition sits MUST commit the mark (set it to None) on its way through - no
    /// exceptions. The reason is not tidiness: `replacement_range` prefers this range over
    /// the selection, so a stale mark makes the next keystroke overwrite a fragment of an
    /// abandoned composition; and `is_composing()` reads this field, so a stale mark makes
    /// D51 refuse to flush text the user has visibly moved on from. That is data loss with
    /// no error anywhere.
    ///
    /// THE DOORS, enumerated rather than remembered: `selected_range` is written in exactly
    /// seven places - `replace`, `replace_and_mark` (which SETS the mark), `move_to`,
    /// `select_to`, `select_range`, `select_all` and `clear_selection`. The first six clear.
    /// `clear_selection` is the ONE DELIBERATE EXCEPTION (Escape): cancelling a selection is
    /// not cancelling an IME session, and it does not move the caret, so the mark still
    /// describes where the composition is. Pinned by name in
    /// tests/ime_seam.rs::every_motion_door_clears_an_open_composition and
    /// ::a_pure_composition_update_does_not_clear_the_mark.
    ///
    /// WHY A NEW DOOR CANNOT BE CAUGHT BY A TEST ALONE: the field is `pub(crate)`, so a
    /// function that assigns `selected_range` directly - as the test helpers do - never
    /// passes through any door and no table can see it. Adding an eighth mover means adding
    /// it to the list above, to the table in that test, and to the exception note if it
    /// deliberately keeps the mark. A comment on the field is the only place all three can
    /// be said at once.
    pub(crate) marked_range: Option<Range<usize>>,
    /// The grapheme column up and down arrows remember, so a caret walking a ragged
    /// paragraph does not slide to the margin and stay there. `None` until the first
    /// horizontal move; never updated by `move_vertical`, which would destroy it.
    desired_column: Option<usize>,
    /// Bumped by every mutation of the TEXT, and by nothing else - not a caret move,
    /// not a scroll, not a selection. This is what the bridge watches instead of
    /// re-reading the buffer, because comparing a 2000-line string on every wake is a
    /// cost the wire must not pay (crates/bridge-gpui/src/main.rs, `Wire`).
    edits: u64,
}

impl TextState {
    pub(crate) fn new(content: String) -> Self {
        let len = content.len();
        Self {
            content,
            selected_range: len..len,
            selection_reversed: false,
            marked_range: None,
            desired_column: None,
            edits: 0,
        }
    }

    /// The range a replacement would act on: an explicit platform range, else the
    /// marked composition, else the selection. (`examples/input.rs:307-311`.)
    fn replacement_range(&self, range_utf16: Option<&Range<usize>>) -> Range<usize> {
        let raw = match range_utf16 {
            Some(range) => range_from_utf16(&self.content, range),
            None => self
                .marked_range
                .clone()
                .unwrap_or_else(|| self.selected_range.clone()),
        };
        clamp_range(&self.content, &raw)
    }

    /// The plain commit: replace, caret after the inserted text, composition over.
    pub(crate) fn replace(&mut self, range_utf16: Option<Range<usize>>, text: &str) {
        let range = self.replacement_range(range_utf16.as_ref());
        self.content = splice(&self.content, &range, text);
        let caret = range.start + text.len();
        self.selected_range = caret..caret;
        self.selection_reversed = false;
        self.marked_range = None;
        // THE COUNTER the wire watches. Forgetting this line is not a crash and no test
        // of the model would notice: the buffer changes, the screen changes, and the
        // autosave never fires because the bridge is told nothing happened. Measured live
        // exactly that way - `final flush: nothing outstanding` while 26 bytes sat unsaved.
        self.edits += 1;
    }

    /// The composition replace: replace, and keep `text` marked as uncommitted.
    ///
    /// The example computes the resulting selection as
    /// `new_range.start + range.start .. new_range.end + range.end`
    /// (examples/input.rs:343-347). Two things are wrong with that for any
    /// non-empty replacement, and both are pinned by tests below: the right-hand term
    /// adds the END of the old range, so the selection runs past the caret by the
    /// width of what was replaced (on a 5-byte composition replaced by a 3-byte
    /// character it yields `0..8` in a 3-byte document, which panics the next slice),
    /// and the units it converts are document-wide while the offset added is the
    /// insertions, which double-counts everywhere except byte 0.
    pub(crate) fn replace_and_mark(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
    ) {
        let range = self.replacement_range(range_utf16.as_ref());
        self.content = splice(&self.content, &range, text);
        self.marked_range = if text.is_empty() {
            None
        } else {
            Some(range.start..range.start + text.len())
        };
        self.selected_range = match new_selected_range_utf16 {
            Some(utf16) => {
                // `new_selected_range` is in terms of UTF-16 characters
                // (gpui src/platform.rs:1042) and, as in the API it mirrors
                // (`setMarkedText:selectedRange:replacementRange:`), it is measured WITHIN
                // `new_text`, not from the start of the document. Converting it against the
                // whole content and then adding `range.start` - what the example does
                // - double-counts the offset and is right only when the composition began
                // at byte 0. So: convert inside `text`, clamp inside `text`, shift once.
                let inner = clamp_range(text, &range_from_utf16(text, &utf16));
                let start = range.start + inner.start;
                let end = range.start + inner.end;
                start..end
            }
            None => {
                let caret = range.start + text.len();
                caret..caret
            }
        };
        self.selection_reversed = false;
        self.edits += 1;
    }

    /// The byte range that is currently marked, in UTF-16 units, for the platform.
    pub(crate) fn marked_text_range(&self) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| range_to_utf16(&self.content, range))
    }

    pub(crate) fn unmark_text(&mut self) {
        self.marked_range = None;
    }

    pub(crate) fn selected_text_range(&self) -> UTF16Selection {
        UTF16Selection {
            range: range_to_utf16(&self.content, &self.selected_range),
            reversed: self.selection_reversed,
        }
    }

    /// Text for a UTF-16 range, reporting back what was actually read. A platform
    /// that asks for `2..3` of a single emoji is asking for half a surrogate pair;
    /// `actual_range` is how it learns the answer was the whole character.
    /// Where the caret is: the end of the selection, unless it was dragged backwards.
    pub(crate) fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    /// Put the caret at `offset` WITHOUT touching the remembered column - the door
    /// `move_vertical` uses, because a vertical move that overwrote the column would
    /// collapse the caret to the margin after one keystroke and stay there.
    fn caret_to(&mut self, offset: usize) {
        let at = grapheme_boundary_after(&self.content, offset);
        self.selected_range = at..at;
        self.selection_reversed = false;
        // Any caret MOTION commits the composition in the only sense this model has: the
        // characters stay, the "uncommitted" flag goes. Without this, click away from an
        // unfinished IME session leaves `marked_range` pointing at text the user has
        // left behind, the next keystroke replaces THAT instead of inserting at the
        // caret, and S6 would refuse to flush on a composition that no longer exists.
        self.marked_range = None;
    }

    /// Collapse the selection to a caret at `offset`, snapped forward onto a grapheme
    /// boundary so a caller can hand us an index from anywhere and we cannot end up
    /// holding a position that would split a cluster if it became an insertion.
    pub(crate) fn move_to(&mut self, offset: usize) {
        self.caret_to(offset);
        self.desired_column = Some(self.column_of(self.selected_range.start));
    }

    /// Grow the selection to `offset`, flipping which end is the head if the user
    /// crossed over. Byte-for-byte the example's rule (examples/input.rs:186-196),
    /// with the boundary snap added.
    pub(crate) fn select_to(&mut self, offset: usize) {
        let at = grapheme_boundary_after(&self.content, offset);
        if self.selection_reversed {
            self.selected_range.start = at;
        } else {
            self.selected_range.end = at;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.desired_column = Some(self.column_of(self.cursor_offset()));
        // Same rule as `caret_to`: a selection by mouse or shift-key commits the mark.
        self.marked_range = None;
    }

    /// The word a byte sits in, for a double click. Clusters, not chars and not bytes,
    /// because a word boundary that falls inside `e` + U+0301 is the S1 bug class and
    /// this is the one place in the file that walks forward cluster by cluster.
    ///
    /// A word is a run of clusters that are all whitespace, or all alphanumeric, or all
    /// neither; a line feed is whitespace and so always ends the run, which is what
    /// makes a double click never reach past its own line. Empty input yields the empty
    /// range at 0 rather than a panic, because this is called from a click handler.
    pub(crate) fn word_range_at(&self, byte: usize) -> Range<usize> {
        let clusters: Vec<(usize, &str)> = self.content.grapheme_indices(true).collect();
        if clusters.is_empty() {
            return 0..0;
        }
        let class = |s: &str| -> u8 {
            match s.chars().next() {
                None => 2,
                Some(c) if c.is_whitespace() => 2,
                Some(c) if c.is_alphanumeric() => 1,
                Some(_) => 3,
            }
        };
        let mut at = 0usize;
        for (index, (start, _)) in clusters.iter().enumerate() {
            if *start > byte {
                break;
            }
            at = index;
        }
        let wanted = class(clusters[at].1);
        let mut first = at;
        while first > 0 && class(clusters[first - 1].1) == wanted {
            first -= 1;
        }
        let mut last = at;
        while last + 1 < clusters.len() && class(clusters[last + 1].1) == wanted {
            last += 1;
        }
        let start = clusters[first].0;
        let end = start
            + clusters[first..=last]
                .iter()
                .map(|(_, s)| s.len())
                .sum::<usize>();
        start..end
    }

    /// Select a range and put the head at its end - the double and triple click door,
    /// which must leave a caret that extends in the right direction if the user then
    /// shift-arrows.
    pub(crate) fn select_range(&mut self, range: Range<usize>) {
        let range = clamp_range(&self.content, &range);
        self.selected_range = range.clone();
        self.selection_reversed = false;
        self.desired_column = Some(self.column_of(range.end));
        self.marked_range = None;
    }

    /// Ctrl+A.
    ///
    /// THE SIXTH DOOR, and the one the S5 test missed: selecting everything is motion away
    /// from an uncommitted composition, exactly as a click or an arrow is. Left marked, the
    /// buffer keeps `is_composing()` true - so D51 refuses to flush text the user has moved
    /// on from - and the next plain keystroke lands in `replacement_range`, which prefers a
    /// stale mark over the selection the user just made. Ctrl+A then typing would overwrite
    /// a fragment of an abandoned composition instead of replacing everything.
    /// `ime_seam.rs` (ac6fc37b) is the red this closes.
    pub(crate) fn select_all(&mut self) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        self.marked_range = None;
    }

    /// Escape: drop the selection, keep the caret where the head was, and leave the
    /// composition ALONE - cancelling a selection is not cancelling an IME session.
    pub(crate) fn clear_selection(&mut self) {
        let caret = self.cursor_offset();
        self.selected_range = caret..caret;
        self.selection_reversed = false;
    }

    /// The boundary the LEFT arrow goes to: the last cluster start STRICTLY before
    /// `byte`. Strictly, because `byte` is usually already on a boundary - and the
    /// clamping helper that snaps a range must be inclusive, so the two rules cannot
    /// share one function. An inclusive left-arrow would be a key that does nothing.
    pub(crate) fn previous_boundary(&self, byte: usize) -> usize {
        let mut last = 0usize;
        for (index, _) in self.content.grapheme_indices(true) {
            if index >= byte {
                break;
            }
            last = index;
        }
        last
    }

    /// The boundary the RIGHT arrow goes to: the first cluster start STRICTLY after
    /// `byte`, or the end of the buffer.
    pub(crate) fn next_boundary(&self, byte: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .find(|index| *index > byte)
            .unwrap_or(self.content.len())
    }

    /// Which visual line a byte offset is on.
    pub(crate) fn line_index_at(&self, byte: usize) -> usize {
        let byte = byte.min(self.content.len());
        self.content.as_bytes()[..byte]
            .iter()
            .filter(|b| **b == b'\n')
            .count()
    }

    /// The byte range of the line under `byte`, WITHOUT its newline (a line never
    /// contains one: gpui `shape_line` debug-asserts that its input has none,
    /// src/text_system.rs:372, and the newline is the separator, not content to draw).
    pub(crate) fn line_range_at(&self, byte: usize) -> Range<usize> {
        line_ranges(&self.content)
            .into_iter()
            .nth(self.line_index_at(byte))
            .unwrap_or(0..self.content.len())
    }

    /// The grapheme column of a byte offset within its line - the sticky target up and
    /// down arrows remember. Counted in clusters, not bytes or units, so a column means
    /// the same thing on `e`+U+0301 as on `e`.
    pub(crate) fn column_of(&self, byte: usize) -> usize {
        let line = self.line_range_at(byte);
        self.content[line.start..byte.min(line.end)]
            .graphemes(true)
            .count()
    }

    /// The byte offset of grapheme column `column` on the line starting at `start`,
    /// clamped to that line's end. This clamp is the whole of `end` on a short line.
    pub(crate) fn byte_at_column(&self, start: usize, column: usize) -> usize {
        let line = self.line_range_at(start);
        self.content[line.start..line.end]
            .grapheme_indices(true)
            .map(|(index, _)| line.start + index)
            .nth(column)
            .unwrap_or(line.end)
            .min(line.end)
    }

    /// Up and down. The caret goes to the remembered column of the neighbouring line,
    /// clamped to that lines length, which is what makes walking a paragraph of ragged
    /// lines feel right instead of sliding to the margin and staying there. Column
    /// rather than pixel because it needs no layout, so it is testable without a
    /// window - the same reason S1 split the state out.
    /// Where up/down should LAND, without moving anything: one rule shared by the
    /// caret motion and the selection motion, so the keyboard cannot disagree with
    /// itself about what a line is. Clamped to the neighbour line length, which is the
    /// whole of a short line and the remembered column on a long one.
    pub(crate) fn vertical_target(&self, delta: isize) -> usize {
        let lines = line_ranges(&self.content);
        if lines.is_empty() {
            return 0;
        }
        let here = self
            .line_index_at(self.cursor_offset())
            .min(lines.len() - 1);
        let there = if delta < 0 {
            here.saturating_sub(delta.unsigned_abs())
        } else {
            here.saturating_add(delta as usize).min(lines.len() - 1)
        };
        if there == here {
            return if delta < 0 {
                lines[here].start
            } else {
                lines[here].end
            };
        }
        let wanted = self
            .desired_column
            .unwrap_or_else(|| self.column_of(self.cursor_offset()));
        self.byte_at_column(lines[there].start, wanted)
    }

    pub(crate) fn move_vertical(&mut self, delta: isize) {
        let byte = self.vertical_target(delta);
        self.caret_to(byte);
    }

    /// Shift+up/down. Same target, the other door: ONE selection model, two input
    /// routes, which is the only reason the mouse in S4 and the keyboard agree at all.
    pub(crate) fn select_vertical(&mut self, delta: isize) {
        let byte = self.vertical_target(delta);
        // The column is restored after the move, exactly as `caret_to` protects it for
        // the motion door: `select_to` records the column it landed on, and after a
        // clamp to a short line that would be the SHORT lines column - so the second
        // shift-down would walk down the margin instead of back to column five. Found
        // by the test, in code I had just written.
        let kept = self.desired_column;
        self.select_to(byte);
        self.desired_column = kept;
    }

    pub(crate) fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
    ) -> Option<String> {
        // Outward, so a mid-pair request gets the whole character and is told so.
        let widened = range_from_utf16_outward(&self.content, &range_utf16);
        let range = clamp_range(&self.content, &widened);
        *adjusted_range = Some(range_to_utf16(&self.content, &range));
        Some(self.content[range].to_string())
    }
}

// ---------------------------------------------------------------------------
// The view: the state, plus the focus handle the toolkit needs
// ---------------------------------------------------------------------------

/// One shaped visual line and where it was drawn. Owned by the Editor, written in
/// `paint`, read by the two geometry methods and by the next frame's shape cache.
pub(crate) struct LineFrame {
    /// The bytes of the buffer this line shapes, newline excluded.
    pub(crate) bytes: Range<usize>,
    /// The text that was handed to `shape_line`, kept by value so a cache hit can be
    /// checked rather than assumed.
    pub(crate) text: String,
    /// Where the composition underline sits WITHIN this line, in the line own bytes.
    /// Part of the cache key: reuse by text alone would hand back an underline that
    /// belongs to another frame.
    pub(crate) mark: Option<Range<usize>>,
    pub(crate) line: ShapedLine,
    /// The rect this line occupies after scrolling, in window coordinates. Its bottom
    /// is the baseline the caret and the IME rect are measured from.
    pub(crate) bounds: Bounds<Pixels>,
}

/// The editor widget's model. In S1 it is never rendered; S2 paints what is in here.
pub(crate) struct Editor {
    /// Content, selection and composition. Read it as the five fields the plan names:
    /// `content`, `selected_range`, `selection_reversed`, `marked_range` - plus the
    /// `focus_handle` below, which is why it is a separate struct (`TextState`).
    state: TextState,
    focus_handle: FocusHandle,
    /// The shaped line from the LAST PAINT, cached in `paint` and read by the two
    /// geometry methods. Caching it in paint rather than shaping it on demand is what
    /// keeps the IME answer and the pixels in agreement, and it is what the example
    /// does (examples/input.rs:555-559, reading :404-415).
    /// One entry PER VISUAL LINE, in line order: the shaped line, the bytes of the
    /// buffer it shapes, and the bounds it was actually drawn at, already scroll
    /// offset. Line-ADDRESSED is the point - y has to choose a line before x can
    /// choose a character - and writing the drawn bounds back is what keeps the IME
    /// rect and the pixels from ever disagreeing (examples/input.rs:555-559 reads the
    /// cache written at :404-415; here both are per line).
    frames: Vec<LineFrame>,
    /// The WHOLE buffer this frame set was shaped from, compared by value. Never
    /// against one line of it: the examples single-line assert cannot hold once a
    /// newline exists, because shape_line refuses to take one.
    frame_text: String,
    /// Pixels of the buffer scrolled above the top of the element. The rule is the
    /// boring one every text widget uses: the caret is always visible. No animation,
    /// no scrollbar, no horizontal scroll (word wrap is out of scope for the whole
    /// project - a note wider than the window is a note you resize the window for).
    scroll_y: Pixels,
    /// What the last frames shaping cost, in microseconds, so the typing budget is a
    /// number from the binary rather than an argument about the source.
    shape_us: u128,
    /// The height of the element box at the last paint, i.e. the viewport the scroll is
    /// clamped against. The model needs it because the wheel listener sits on the Div
    /// and the clamp rule lives here.
    viewport_h: Pixels,
    /// The caret line the last painted frame showed. The caret rule compares against
    /// this; the wheel never writes it, which is the whole reason a scroll survives.
    caret_line_shown: Option<usize>,
    /// Set by a button-down, cleared by the matching up. The drag is the model's idea,
    /// not gpui's: the Div only reports that the mouse moved while this was true.
    dragging: bool,
}

impl Editor {
    /// The context's `focus_handle()` is `App::focus_handle` (gpui src/app.rs:2029)
    /// reached through `Context`'s deref; the example builds a `TextInput` the same way
    /// (examples/input.rs:703-704).
    /// No font, no shaping, no layout: S1/S2 construct cheap and shape on first paint
    /// (the cold-start budget, whitepaper section 2). This is the whole reason
    /// `frames` starts empty.
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            state: TextState::default(),
            focus_handle: cx.focus_handle(),
            frames: Vec::new(),
            frame_text: String::new(),
            scroll_y: px(0.0),
            shape_us: 0,
            viewport_h: px(0.0),
            caret_line_shown: None,
            dragging: false,
        }
    }

    #[allow(dead_code)] // S3 onward: the buffer arrives from the port in S6
    pub(crate) fn with_content(content: String, cx: &mut Context<Self>) -> Self {
        Self {
            state: TextState::new(content),
            focus_handle: cx.focus_handle(),
            frames: Vec::new(),
            frame_text: String::new(),
            scroll_y: px(0.0),
            shape_us: 0,
            viewport_h: px(0.0),
            caret_line_shown: None,
            dragging: false,
        }
    }

    /// True when the cached layout no longer describes what we would paint. A stale
    /// layout is not a crash waiting to happen, it is a WRONG IME RECT waiting to
    /// happen, so both geometry methods check this instead of trusting the cache.
    fn display_desynced(&self) -> bool {
        self.frame_text != self.state.content
    }

    /// The line a byte offset belongs to, for the geometry methods. A byte at the very - the caret after the last character, which is where typing
    /// leaves it - has no line that CONTAINS it, so it falls to the last line.
    fn frame_for(&self, byte: usize) -> Option<&LineFrame> {
        self.frames
            .iter()
            .find(|frame| frame.bytes.contains(&byte))
            .or_else(|| self.frames.last())
    }

    /// The line a PIXEL y falls on, with the clamp S4 was asked for: below the last line
    /// is the last line, above the first is the first line, and `None` is left for the
    /// one genuinely unresolvable case - no layout yet, which is only true before the
    /// first paint. This is what makes "click in the empty space under a short note" put
    /// the caret at the END of the text instead of doing nothing, which is how every
    /// text control on Windows behaves.
    fn frame_at(&self, y: Pixels) -> Option<&LineFrame> {
        let mut chosen = self.frames.first()?;
        for frame in &self.frames {
            if frame.bounds.top() <= y {
                chosen = frame;
            } else {
                break;
            }
        }
        Some(chosen)
    }

    /// WHEEL AND TRACKPAD. gpui routes `PlatformInput::ScrollWheel` to any `Div` with a
    /// listener (the fluent `on_scroll_wheel` at src/elements/div.rs:827-834, the
    /// platform input at src/window.rs:3615-3618), so no port seam is needed for a delta
    /// that is pure UI state - and the port must not own one, because api routes and
    /// translates and decides nothing (AGENTS.md).
    ///
    /// A notch of the wheel is worth this many LINES. The Windows default is three wheel
    /// lines per notch and that is the number a user feels on every other window on the
    /// machine, so it is the number this app uses - not the 120 units / 5 rows gpui
    /// reports here (measured: `wheel raw=-120.0` against `row=24.0`).
    ///
    /// The real value lives in `SystemParametersInfo(SPI_GETWHEELSCROLLLINES)`, which
    /// this crate cannot read without a platform seam, and the seam is deliberately not
    /// added for one constant: a per-user setting that only the wheel honours is not
    /// worth widening the port. This is the line to change when that seam exists.
    ///
    /// A `Lines` delta is taken as notches on the same scale, which is an assumption on
    /// the platform we do not ship; named rather than hidden.
    fn scroll_by(&mut self, delta: &ScrollDelta, cx: &mut Context<Self>) {
        const WHEEL_LINES_PER_NOTCH: f32 = 3.0;
        const WHEEL_UNITS_PER_NOTCH: f32 = 120.0;
        let row = self
            .frames
            .first()
            .map_or(0.0, |frame| f32::from(frame.bounds.size.height));
        let lines = match delta {
            ScrollDelta::Lines(point) => point.y * WHEEL_LINES_PER_NOTCH,
            ScrollDelta::Pixels(point) => {
                f32::from(point.y) / WHEEL_UNITS_PER_NOTCH * WHEEL_LINES_PER_NOTCH
            }
        };
        let raw = lines * row;
        let content = row * self.frames.len().max(1) as f32;
        let max = (content - f32::from(self.viewport_h)).max(0.0);
        // The platform reports scroll-down as positive y and the offset is measured the
        // other way, so the sign flips exactly once, here.
        let next = (f32::from(self.scroll_y) + raw).clamp(0.0, max);
        probe(format!(
            "wheel raw={raw:.1} lines={lines:.1} offset_before={:.1} offset_after={next:.1} row={row:.1}",
            f32::from(self.scroll_y)
        ));
        if next != f32::from(self.scroll_y) {
            self.scroll_y = px(next);
            cx.notify();
        }
    }

    /// The byte under a pixel point, with the two clamps S4 established: below the last
    /// line is the last line, past the end of a line is that line's end.
    fn byte_at(&self, point: Point<Pixels>) -> Option<usize> {
        let frame = self.frame_at(point.y)?;
        let x = point.x - frame.bounds.left();
        let utf8 = frame.line.index_for_x(x).unwrap_or(frame.text.len());
        Some((frame.bytes.start + utf8).min(frame.bytes.end))
    }

    /// CLICK, SHIFT-CLICK, DOUBLE, TRIPLE, and the DRAG.
    ///
    /// `click_count` is gpui's own, not a guess: `MouseDownEvent::click_count` at
    /// src/interactive.rs:104, and on this platform it is maintained by gpui's click
    /// state machine at src/platform/windows/events.rs:459-467 (`click_state.update(
    /// button, physical_point)` feeding the event), because the Windows backend does not
    /// forward WM_LBUTTONDBLCLK as a separate kind - it counts.
    ///
    /// Everything still leaves through `move_to`/`select_to`/`select_range`, so the snap
    /// onto a grapheme boundary is in one place and a click cannot land mid-cluster.
    fn position_caret(
        &mut self,
        point: Point<Pixels>,
        shift: bool,
        click_count: usize,
        cx: &mut Context<Self>,
    ) {
        probe(format!(
            "caret_point x={:.1} y={:.1} shift={shift} clicks={click_count}",
            f32::from(point.x),
            f32::from(point.y)
        ));
        let Some(byte) = self.byte_at(point) else {
            return;
        };
        if shift {
            // A shift-click extends whatever the last gesture made, including a word.
            self.state.select_to(byte);
        } else if click_count >= 3 {
            let line = self.state.line_range_at(byte);
            self.state.select_range(line);
        } else if click_count == 2 {
            let word = self.state.word_range_at(byte);
            self.state.select_range(word);
        } else {
            self.state.move_to(byte);
        }
        cx.notify();
    }

    /// DRAG SELECT. A button-down already carries the point, so all the drag needs on
    /// top of `position_caret` is one bool: gpui gives `on_mouse_move` to the Div while
    /// the button is held, and the model decides what that means. Direction comes free
    /// from `select_to`, which flips `selection_reversed` when the head crosses the
    /// anchor - so dragging up leaves the caret at the TOP, which is the half of
    /// direction-awareness the clipboard reads.
    fn begin_drag(
        &mut self,
        point: Point<Pixels>,
        shift: bool,
        click_count: usize,
        cx: &mut Context<Self>,
    ) {
        self.dragging = true;
        self.position_caret(point, shift, click_count, cx);
    }

    fn extend_drag(&mut self, point: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.dragging {
            return;
        }
        let Some(byte) = self.byte_at(point) else {
            return;
        };
        if byte != self.state.cursor_offset() {
            self.state.select_to(byte);
            cx.notify();
        }
    }

    fn end_drag(&mut self) {
        self.dragging = false;
    }

    /// PAGE MOTION. A page is the element's own height in rows, not a magic line count,
    /// so it follows the window when the window is resized and needs no constant.
    fn page(&mut self, pages: isize, shift: bool, cx: &mut Context<Self>) {
        let row = f32::from(
            self.frames
                .first()
                .map_or(px(0.0), |frame| frame.bounds.size.height),
        );
        if row <= 0.0 {
            return;
        }
        let rows = ((f32::from(self.viewport_h) / row).floor() as isize).max(1);
        let delta = rows * pages;
        if shift {
            self.state.select_vertical(delta);
        } else {
            self.state.move_vertical(delta);
        }
        cx.notify();
    }

    /// The buffer, as the wire reads it. LF inside, always - the file's own ending is
    /// restored at the SAVE layer by core, which is the only place that knows what the
    /// file was (section 4.5).
    pub(crate) fn text(&self) -> &str {
        &self.state.content
    }

    /// Composition in progress. `Flush` is refused while this is true (D51), because
    /// uncommitted text is not the user's words yet - and every motion path clears the
    /// mark (S5), so this can only be true while the user is actually mid-IME.
    pub(crate) fn is_composing(&self) -> bool {
        self.state.marked_range.is_some()
    }

    /// The mutation counter the bridge watches instead of the buffer. Watching `text()`
    /// for a change would mean cloning a 2000-line string on every 8 ms wake; this is a
    /// u64 compare, and it counts only TEXT changes - a click, a scroll, a resize and a
    /// selection leave it alone, so none of them can trigger a save.
    pub(crate) fn edits(&self) -> u64 {
        self.state.edits
    }

    /// LOAD: put a document into the editor and RESET THE VIEW STATE. Every field here
    /// is a bug if it survives, and `caret_line_shown` is the subtle one: the
    /// bring-into-view rule fires only when the caret LINE changed, so leaving the old
    /// document's value in place can mean the new note opens at the old note's offset
    /// and never corrects itself. The caret goes to the START of the new document, not
    /// the end: opening a note and finding yourself at the bottom is not where you left
    /// it, it is where the file ended.
    pub(crate) fn load(&mut self, content: String, cx: &mut Context<Self>) {
        self.state = TextState::new(content);
        self.state.move_to(0);
        self.frames.clear();
        self.frame_text.clear();
        self.scroll_y = px(0.0);
        self.caret_line_shown = None;
        self.dragging = false;
        cx.notify();
    }
}

impl Focusable for Editor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// All eight methods, in the traits order, delegating to the tested state. The
/// `window` and `cx` parameters are the traits, not ours: `EntityInputHandler`
/// (gpui src/input.rs:10-73) has exactly these eight and no more.
impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        self.state.text_for_range(range, adjusted_range)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        // The example also always answers Some: a disabled or read-only field is S7,
        // and until it exists `ignore_disabled_input` has nothing to ignore.
        Some(self.state.selected_text_range())
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.state.marked_text_range()
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.state.unmark_text();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.replace(range, text);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state
            .replace_and_mark(range, new_text, new_selected_range);
        cx.notify();
    }

    /// WHERE THE IME CANDIDATE WINDOW GOES. The x comes from the shaped line that was
    /// actually drawn, the y from that line's bounds AFTER scrolling - a candidate list
    /// for line 4 has to sit at line 4, which is the whole reason the cache is
    /// line-addressed. The left edge still comes from `element_bounds`, the caller's own
    /// idea of where the field is, so a resize in flight cannot make the list lag the
    /// window. A range spanning several lines answers with the FIRST line's segment:
    /// a candidate window belongs to one line.
    ///
    /// If the cache does not describe the current buffer the answer is `None`. A
    /// rectangle built from a stale layout is not a rough guess, it is a lie the
    /// candidate window will sit on top of.
    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        if self.display_desynced() {
            probe("bounds_for_range: no answer, the cached layout is stale");
            return None;
        }
        let range = clamp_range(
            &self.state.content,
            &range_from_utf16(&self.state.content, &range_utf16),
        );
        let frame = self.frame_for(range.start)?;
        let len = frame.text.len();
        let start = range.start.saturating_sub(frame.bytes.start).min(len);
        let end = range
            .end
            .saturating_sub(frame.bytes.start)
            .clamp(start, len);
        let left = frame.line.x_for_index(start);
        let right = frame.line.x_for_index(end);
        probe(format!(
            "bounds_for_range units {range_utf16:?} -> line {} local {start}..{end}, x {:.1}..{:.1} y {:.1}",
            self.state.line_index_at(range.start),
            f32::from(left),
            f32::from(right),
            f32::from(frame.bounds.top())
        ));
        Some(Bounds::from_corners(
            point(element_bounds.left() + left, frame.bounds.top()),
            point(element_bounds.left() + right, frame.bounds.bottom()),
        ))
    }

    /// WHICH CHARACTER IS UNDER THIS PIXEL. y first, then x - the order the single-line
    /// version could not use, because with one line there was nothing to choose. A point
    /// in the empty space below the last line is no answer at all, not a clamp: the
    /// caller is asking where the pointer was.
    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        // examples/input.rs:383 asserts `last_layout.text == self.content`, comparing
        // ONE shaped line against the WHOLE buffer - which cannot hold once a newline
        // exists, because shape_line will not take one. The invariant is kept,
        // restated as the pair that CAN be equal, and per S2 as a checked return rather
        // than an abort: a notes app must not die because a frame was stale. A debug
        // build still panics loudly at the frame the cache and the buffer diverged.
        debug_assert!(
            !self.display_desynced(),
            "editor: cached frames describe {:?} but the buffer is {:?}",
            self.frame_text,
            self.state.content
        );
        if self.display_desynced() {
            probe("character_index_for_point: no answer, the cached layout is stale");
            return None;
        }
        let frame = self
            .frames
            .iter()
            .find(|frame| frame.bounds.contains(&point))?;
        let local = frame.bounds.localize(&point)?;
        let utf8 = frame.line.index_for_x(local.x)?;
        let byte = (frame.bytes.start + utf8).min(frame.bytes.end);
        let units = offset_to_utf16(&self.state.content, byte);
        probe(format!(
            "character_index_for_point x={:.1} y={:.1} -> byte {byte} -> unit {units}",
            f32::from(point.x),
            f32::from(point.y)
        ));
        Some(units)
    }
}

// ---------------------------------------------------------------------------
// Keys: the actions a single-line editor needs, and the handlers for them
// ---------------------------------------------------------------------------

// Local action names, in the examples own style (examples/input.rs:13-30). These are
// UI verbs, not port vocabulary: nothing here crosses the seam.
actions!(
    notes_editor,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        EscapeSelection,
        SelectUp,
        SelectDown,
        PageUp,
        PageDown,
        SelectPageUp,
        SelectPageDown,
        Newline,
        Up,
        Down,
        Copy,
        Cut,
        Paste,
    ]
);

/// Word motion is NOT bound: the example has no Ctrl+Left/Right either, so there is no
/// reference for where a word boundary is once combining marks and emoji sequences are
/// involved, and a word motion guessed here would be the one thing in this file that
/// quietly disagrees with the caret. It needs the same `grapheme_*` treatment plus a
/// letter/extended set; that is a slice of its own.
impl Editor {
    /// Every handler ends in the SAME door the IME uses - `TextState::replace` -
    /// rather than a second splice path. Two ways to change the buffer is two ways to
    /// disagree about the selection.
    fn left(&mut self, _: &Left, _window: &mut Window, cx: &mut Context<Self>) {
        if self.state.selected_range.is_empty() {
            let at = self.state.previous_boundary(self.state.cursor_offset());
            self.state.move_to(at);
        } else {
            let at = self.state.selected_range.start;
            self.state.move_to(at);
        }
        cx.notify();
    }

    fn right(&mut self, _: &Right, _window: &mut Window, cx: &mut Context<Self>) {
        if self.state.selected_range.is_empty() {
            let at = self.state.next_boundary(self.state.cursor_offset());
            self.state.move_to(at);
        } else {
            let at = self.state.selected_range.end;
            self.state.move_to(at);
        }
        cx.notify();
    }

    fn select_left(&mut self, _: &SelectLeft, _window: &mut Window, cx: &mut Context<Self>) {
        let at = self.state.previous_boundary(self.state.cursor_offset());
        self.state.select_to(at);
        cx.notify();
    }

    fn select_right(&mut self, _: &SelectRight, _window: &mut Window, cx: &mut Context<Self>) {
        let at = self.state.next_boundary(self.state.cursor_offset());
        self.state.select_to(at);
        cx.notify();
    }

    fn select_all(&mut self, _: &SelectAll, _window: &mut Window, cx: &mut Context<Self>) {
        self.state.select_all();
        cx.notify();
    }

    fn home(&mut self, _: &Home, _window: &mut Window, cx: &mut Context<Self>) {
        // The VISUAL line end, not the buffer end. Home on line 7 of a note must not
        // teleport to the start of the document - that difference is the whole reason a
        // text editor is not a text field.
        let line = self.state.line_range_at(self.state.cursor_offset());
        self.state.move_to(line.start);
        cx.notify();
    }

    fn end(&mut self, _: &End, _window: &mut Window, cx: &mut Context<Self>) {
        let line = self.state.line_range_at(self.state.cursor_offset());
        self.state.move_to(line.end);
        cx.notify();
    }

    fn up(&mut self, _: &Up, _window: &mut Window, cx: &mut Context<Self>) {
        self.state.move_vertical(-1);
        cx.notify();
    }

    fn down(&mut self, _: &Down, _window: &mut Window, cx: &mut Context<Self>) {
        self.state.move_vertical(1);
        cx.notify();
    }

    fn select_up(&mut self, _: &SelectUp, _window: &mut Window, cx: &mut Context<Self>) {
        self.state.select_vertical(-1);
        cx.notify();
    }

    fn select_down(&mut self, _: &SelectDown, _window: &mut Window, cx: &mut Context<Self>) {
        self.state.select_vertical(1);
        cx.notify();
    }

    /// A page is the element's own height in rows, so `page` is the only place that
    /// knows what a page is - no magic line count, and it follows a resized window.
    fn page_up(&mut self, _: &PageUp, _window: &mut Window, cx: &mut Context<Self>) {
        self.page(-1, false, cx);
    }

    fn page_down(&mut self, _: &PageDown, _window: &mut Window, cx: &mut Context<Self>) {
        self.page(1, false, cx);
    }

    fn select_page_up(&mut self, _: &SelectPageUp, _window: &mut Window, cx: &mut Context<Self>) {
        self.page(-1, true, cx);
    }

    fn select_page_down(
        &mut self,
        _: &SelectPageDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.page(1, true, cx);
    }

    /// Enter is an edit like every other one: through the same splice, which is why it
    /// cannot split a cluster and why a marked composition is handled here exactly as it
    /// is everywhere else.
    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, "\n", window, cx);
    }

    fn escape(&mut self, _: &EscapeSelection, _window: &mut Window, cx: &mut Context<Self>) {
        self.state.clear_selection();
        cx.notify();
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.selected_range.is_empty() {
            let at = self.state.previous_boundary(self.state.cursor_offset());
            self.state.select_to(at);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.selected_range.is_empty() {
            let at = self.state.next_boundary(self.state.cursor_offset());
            self.state.select_to(at);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    /// Clipboard, by the examples route (examples/input.rs:131-153): three handlers,
    /// nothing else. S2 takes them because they are that separable; S5 owns the
    /// guarantees. THE NEWLINE IS KEPT: the example flattens \n at examples/input.rs:133 because its field is one line, and inheriting that into a notes app is a fork bug - a pasted paragraph arrives as one long line. CRLF collapses to LF because core keeps LF internally and the do-no-harm rule restores the file own ending at the save layer. Paste also goes through `replace_text_in_range`, so a
    /// paste cannot land mid-cluster by construction.
    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let normalised = text.replace("\r\n", "\n").replace('\r', "\n");
            self.replace_text_in_range(None, &normalised, window, cx);
        }
    }

    /// Copy puts the buffer's OWN bytes on the clipboard: LF between the lines, no
    /// carriage return, no flatten. That is the half of the do-no-harm rule (whitepaper
    /// section 4.5) that lives here - the file's original ending, CRLF or lone CR, is
    /// restored at the SAVE layer by core, which is the only place that knows what the
    /// file was. The bridge never sees an encoding and never adds a `\r`.
    fn copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.state.selected_range.is_empty() {
            let selected = self.state.content[self.state.selected_range.clone()].to_string();
            cx.write_to_clipboard(ClipboardItem::new_string(selected));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.state.selected_range.is_empty() {
            let selected = self.state.content[self.state.selected_range.clone()].to_string();
            cx.write_to_clipboard(ClipboardItem::new_string(selected));
            self.replace_text_in_range(None, "", window, cx);
        }
    }
}

// ---------------------------------------------------------------------------
// The paint: one shaped line per visual line, the caret, the composition
// underlined, and the scroll that keeps the caret on screen
// ---------------------------------------------------------------------------

/// One line on its way from text to pixels: its bytes in the buffer, the text itself,
/// the underline if a composition sits inside it, and the shape. Named because clippy is
/// right that this tuple is doing four jobs at once, and prepaint is not the place to
/// read a 60-character type.
type ShapedEntry = (Range<usize>, String, Option<Range<usize>>, ShapedLine);

/// What prepaint worked out and paint draws, so nothing is shaped twice and the
/// quads come from the SAME lines the geometry methods will answer with.
pub(crate) struct PrepaintState {
    /// One per visual line, already offset by `scroll_y`.
    frames: Vec<LineFrame>,
    cursor: Option<PaintQuad>,
    /// ONE QUAD PER SPANNED LINE. The example's single selection rect cannot describe
    /// a selection across a line break, so this is a list. What S3 draws is stated in
    /// the acceptance report; per-line rects, not one bounding box.
    selections: Vec<PaintQuad>,
    /// Carried to `paint`, which writes the model's cache in ONE assignment: frames,
    /// the text they describe, the offset they were drawn at and the shaping cost. A
    /// half-updated cache is exactly the stale-rect bug the desync check exists for.
    scroll_y: Pixels,
    frame_text: String,
    shape_us: u128,
    /// The caret line this frame intends to have shown; written to the model only by
    /// paint, because only paint proves the frame reached the screen.
    shown_line: Option<usize>,
}

/// Whether the bring-into-view rule should fire for a caret on `caret_line`, given
/// the line the last painted frame showed. `None` means "nothing has been shown yet",
/// which is exactly the state `Editor::load` puts the view back into: a loaded note
/// whose caret lands on line 0 while the OLD document's `caret_line_shown` still says
/// 1999 would otherwise be treated as no change, and the note would open scrolled to
/// the previous document's offset and never correct itself.
fn follow_required(shown: Option<usize>, caret_line: usize) -> bool {
    shown != Some(caret_line)
}

/// The editor element: request the viewport, shape in prepaint, and in paint hand the
/// bounds to the IME, draw, then cache the frames on the model (the example's shape,
/// examples/input.rs:388-562, made line-addressed).
pub(crate) struct EditorElement {
    input: Entity<Editor>,
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // The VIEWPORT, not the content: the buffer's height is lines x line-height and
        // asking for that would make the element as tall as the note and push the status
        // line off screen. The scroll happens inside, and what is visible is decided by
        // two things now - the caret rule (only when the caret changed line) and the wheel.
        // THE BOX, and why it is a definite length instead of "fill the parent".
        // Measured on a live 200-line note, not reasoned from the source: `relative(1.)`
        // here reported h=0.0 - and `visible=0`, which is how the one-line-tall box S2 and
        // S3 shipped with went unnoticed, since glyphs paint at their origin whatever the
        // box claims; `px(300.)` reported h=300.0; and pinning this bridge's ROOT div to
        // the window's own height changed nothing, so the percentage collapses somewhere
        // above the view root, in gpui's wrapper, which is not in 0.2.2's shipped sources
        // under any name I could find (`struct Root` and `fn size_full` are both absent).
        // The one definite length available at layout time is the window's client height,
        // so the editor takes that minus the status block this bridge draws beneath it.
        // Revisit if gpui ever gives a bare Element a resolvable percentage.
        let status_box = window.line_height() * 2.0 + px(8.0);
        let viewport = (f32::from(window.window_bounds().get_bounds().size.height)
            - f32::from(status_box))
        .max(24.0);
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = px(viewport).into();
        (window.request_layout(style, [], cx), ())
    }

    /// The only shaping in the app, and it happens HERE - first paint, not startup
    /// (whitepaper section 2: cold start is a budget).
    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let text_system = window.text_system();
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let lh = f32::from(window.line_height());
        let viewport = f32::from(bounds.size.height);
        let left = bounds.left();
        let right = bounds.right();
        let mut frames: Vec<LineFrame> = Vec::new();
        let mut cursor_quad = None;
        let mut selections: Vec<PaintQuad> = Vec::new();
        let mut scroll_y = px(0.0);
        let mut shape_us = 0u128;
        let mut caret_line = 0usize;
        let mut frame_text = String::new();
        let mut shown_line: Option<usize> = None;
        let mut sel_probe = (0usize, 0usize);
        let mut focused_probe = false;
        let mut caret_probe = 0usize;
        // The whole prepaint, timed: shape_us alone cannot show the rebuild cost, which
        // is what the quadratic first draft of the cache was. This is the number the
        // wheel moves, because scrolling walks lines that were never shaped.
        // IS THIS WINDOW EVEN ACTIVE? The answer to S5's open deviation, and it was a
        // wrong QUERY, not a missing repaint: `FocusHandle::is_focused` is `window.focus
        // == Some(*self)` (src/window.rs:237-239), which is gpui's internal focus ring
        // WITHIN a window and stays true when the operating system puts another app in
        // front - measured, `focused=true` across every frame while the OS foreground
        // window was a foreign handle. What reports the OS state is
        // `Window::is_window_active` (src/window.rs:1721-1723, "focused by the operating
        // system (receiving key events)"), read once here because `window` cannot be
        // borrowed inside the update closure below.
        let active = window.is_window_active();
        let began = std::time::Instant::now();
        self.input.update(cx, |input, _cx| {
            let lines = line_ranges(&input.state.content);
            let cursor = input.state.cursor_offset();
            let selection = input.state.selected_range.clone();
            let marked = input.state.marked_range.clone();
            caret_line = input.state.line_index_at(cursor);
            // Reuse keyed by the line TEXT and its underline, never by the byte range:
            // a newline inserted at the top shifts every following line, and a key that
            // includes the offset would re-shape the whole note for one keystroke.
            // ONE PASS, not a search per line: the old frames are moved into a pool
            // keyed by the exact text, so a keystroke at the top of a 2000-line note
            // costs a linear walk instead of the quadratic scan the first draft of this
            // had (Vec::remove(position) inside a per-line loop). Composition is excluded
            // from the pool on both sides: one marked line re-shaping per frame is worth
            // never handing back an underline that belongs to another row.
            let mut pool: std::collections::HashMap<String, ShapedLine> =
                std::mem::take(&mut input.frames)
                    .into_iter()
                    .filter(|frame| frame.mark.is_none())
                    .map(|frame| (frame.text, frame.line))
                    .collect();
            let mut shaped: Vec<ShapedEntry> = Vec::new();
            for range in lines.into_iter() {
                let text = input.state.content[range.clone()].to_string();
                let mark = marked.as_ref().and_then(|m| {
                    let start = m.start.max(range.start);
                    let end = m.end.min(range.end);
                    (end > start).then_some(start - range.start..end - range.start)
                });
                let cached = mark.is_none().then(|| pool.remove(&text)).flatten();
                let line = match cached {
                    Some(line) => line,
                    None => {
                        let run = TextRun {
                            len: text.len(),
                            font: style.font(),
                            color: style.color,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let runs = match mark.clone() {
                            Some(mark) => {
                                let mark = clamp_range(&text, &mark);
                                vec![
                                    TextRun {
                                        len: mark.start,
                                        ..run.clone()
                                    },
                                    TextRun {
                                        len: mark.end - mark.start,
                                        underline: Some(UnderlineStyle {
                                            color: Some(run.color),
                                            thickness: px(1.0),
                                            wavy: false,
                                        }),
                                        ..run.clone()
                                    },
                                    TextRun {
                                        len: text.len() - mark.end,
                                        ..run
                                    },
                                ]
                                .into_iter()
                                .filter(|run| run.len > 0)
                                .collect()
                            }
                            None => vec![run],
                        };
                        let at = std::time::Instant::now();
                        let line = text_system.shape_line(
                            SharedString::from(text.clone()),
                            font_size,
                            &runs,
                            None,
                        );
                        shape_us += at.elapsed().as_micros();
                        line
                    }
                };
                shaped.push((range, text, mark, line));
            }
            // Whatever is left in `old` is a line that no longer exists; dropping it is
            // the whole invalidation story.

            // THE CARET IS VISIBLE WHEN THE CARET MOVED, not every frame. The first form
            // of this rule ran unconditionally and made a wheel delta unobservable: every
            // frame after it snapped the view back onto the caret line, so the user
            // scrolled and nothing had happened. Comparing against the line the last PAINT
            // showed - not the last prepaint, because a prepaint that never painted must
            // not claim to have shown anything - gives the rule both directions: scroll
            // away and it stays, move the caret and the view follows.
            let content_height = lh * (shaped.len() + 1) as f32;
            let caret_top = lh * caret_line as f32;
            let mut offset = if follow_required(input.caret_line_shown, caret_line) {
                let mut offset = f32::from(input.scroll_y);
                if caret_top - offset + lh > viewport {
                    offset = caret_top + lh - viewport;
                }
                if caret_top - offset < 0.0 {
                    offset = caret_top;
                }
                offset
            } else {
                // The wheel owns the offset: no line changed, so the user's scroll is left
                // exactly where they put it.
                f32::from(input.scroll_y)
            };
            // The clamp is not conditional on who moved the view: content can shrink under
            // a scroll the wheel owns, and an offset past the end is a blank page.
            offset = offset.max(0.0).min((content_height - viewport).max(0.0));
            scroll_y = px(offset);
            shown_line = Some(caret_line);
            sel_probe = (selection.start, selection.end);
            caret_probe = cursor;
            for (index, (range, text, mark, line)) in shaped.into_iter().enumerate() {
                let top = f32::from(bounds.top()) + lh * index as f32 - offset;
                let frame_bounds =
                    Bounds::from_corners(point(left, px(top)), point(right, px(top + lh)));
                frames.push(LineFrame {
                    bytes: range,
                    text,
                    mark,
                    line,
                    bounds: frame_bounds,
                });
            }
            if let Some(frame) = frames.get(caret_line).filter(|_| active) {
                let x = frame.line.x_for_index(
                    cursor
                        .saturating_sub(frame.bytes.start)
                        .min(frame.text.len()),
                );
                cursor_quad = Some(fill(
                    Bounds::new(
                        point(frame.bounds.left() + x, frame.bounds.top()),
                        size(px(2.0), frame.bounds.size.height),
                    ),
                    rgb(0x0033_99ff),
                ));
            }
            // FOCUSED OR NOT: a grey bar, not a live blue selection, and no caret at all
            // while the window is inactive - what Windows itself does, and the reason a
            // selection in a background window must not look like the thing you are
            // editing. See `active` above for why this is the window and not the handle.
            let selection_colour = if active && input.focus_handle.is_focused(window) {
                rgb(0x2d_4a_6b)
            } else {
                rgb(0x3f_3f_3f)
            };
            focused_probe = active;
            if !selection.is_empty() {
                for frame in &frames {
                    // VIEWPORT CLIP: a selection across 400 lines would draw 400 quads,
                    // and every one outside the visible rows is work nobody can see. The
                    // frames stay complete - the IME rect and the click hit-test still
                    // need a line that is scrolled off - only the QUADS are clipped.
                    if frame.bounds.bottom() <= bounds.top()
                        || frame.bounds.top() >= bounds.bottom()
                    {
                        continue;
                    }
                    let start = selection.start.max(frame.bytes.start);
                    let end = selection.end.min(frame.bytes.end);
                    if end <= start {
                        continue;
                    }
                    selections.push(fill(
                        Bounds::from_corners(
                            point(
                                frame.bounds.left()
                                    + frame.line.x_for_index(start - frame.bytes.start),
                                frame.bounds.top(),
                            ),
                            point(
                                frame.bounds.left()
                                    + frame.line.x_for_index(end - frame.bytes.start),
                                frame.bounds.bottom(),
                            ),
                        ),
                        selection_colour,
                    ));
                }
            }
            frame_text = input.state.content.clone();
        });
        let prepaint_us = began.elapsed().as_micros();
        probe(format!(
            "prepaint lines={} h={:.1} caret={caret_probe} sel={:?}..{:?} focused={focused_probe} scroll={:.1} caret_line={caret_line} shape_us={shape_us} prepaint_us={prepaint_us}",
            frames.len(),
            f32::from(bounds.size.height),
            sel_probe.0,
            sel_probe.1,
            f32::from(scroll_y)
        ));
        PrepaintState {
            frames,
            cursor: cursor_quad,
            selections,
            scroll_y,
            frame_text,
            shape_us,
            shown_line,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        // Registers this element as the Windows input target for the entity, which is
        // what makes the two geometry methods answerable at all.
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let quad_count = prepaint.selections.len();
        for selection in prepaint.selections.drain(..) {
            window.paint_quad(selection);
        }
        let frames = std::mem::take(&mut prepaint.frames);
        let mut visible = 0usize;
        for frame in &frames {
            // The same clip the selection quads use: the cache holds every line so the
            // IME and a click can be answered for a scrolled-off row, but only the rows
            // inside the box are drawn.
            if frame.bounds.bottom() <= bounds.top() || frame.bounds.top() >= bounds.bottom() {
                continue;
            }
            visible += 1;
            // A failed line is a blank row for one frame, not an aborted editor.
            //
            // THE NEW TWO ARGUMENTS, passed as the geometry IS and not as whatever
            // silences the compiler (gpui-pre-0.3.4/src/text_system/line.rs:83 - align:
            // TextAlign, align_width: Option<Pixels>): this editor does not word-wrap, every
            // line lays out left-aligned, and the box it was shaped into is `frame.bounds`.
            // So align is Left and align_width is that box's width - the width the line was
            // measured against. Left alignment adds no offset, which is also why this cannot
            // move a glyph by a pixel: it states the box, it does not re-flow it.
            let _ = frame.line.paint(
                frame.bounds.origin,
                frame.bounds.size.height,
                TextAlign::Left,
                Some(frame.bounds.size.width),
                window,
                cx,
            );
        }
        if focus_handle.is_focused(window) {
            if let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
        }
        // ONE WRITE, all four fields, so the cache the platform is answered from can
        // never be half-updated: frames, the text they describe, the offset they were
        // drawn at, and what shaping cost.
        let frame_text = std::mem::take(&mut prepaint.frame_text);
        let scroll_y = prepaint.scroll_y;
        let shape_us = prepaint.shape_us;
        let count = frames.len();
        let scroll_probe = scroll_y;
        self.input.update(cx, |input, _cx| {
            input.frames = frames;
            input.frame_text = frame_text;
            input.scroll_y = scroll_y;
            input.shape_us = shape_us;
            input.viewport_h = bounds.size.height;
            input.caret_line_shown = prepaint.shown_line.take();
        });
        let focused_out = self.input.read(cx).focus_handle.is_focused(window);
        probe(format!(
            "paint lines={count} visible={visible} quads={quad_count} focused={focused_out} scroll={:.1} shape_us={shape_us}",
            f32::from(scroll_probe)
        ));
    }
}
impl Render for Editor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let wheel_target = cx.entity();
        let click_target = cx.entity();
        let move_target = cx.entity();
        let up_target = cx.entity();
        div()
            .key_context("NotesEditor")
            .track_focus(&self.focus_handle(cx))
            .flex_1()
            .on_scroll_wheel(move |event: &ScrollWheelEvent, _window, cx| {
                wheel_target.update(cx, |editor, cx| editor.scroll_by(&event.delta, cx));
            })
            .on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, _window, cx| {
                    let shift = event.modifiers.shift;
                    let clicks = event.click_count;
                    click_target.update(cx, |editor, cx| {
                        editor.begin_drag(event.position, shift, clicks, cx);
                    });
                },
            )
            .on_mouse_move(move |event: &MouseMoveEvent, _window, cx| {
                // `pressed_button` says a button is down; the model's flag says the drag
                // BEGAN in this editor. Both, because a drag that starts in the status bar
                // and enters the text must not select, and one that leaves the window must
                // keep selecting until the button is released.
                if event.pressed_button == Some(MouseButton::Left) {
                    let position = event.position;
                    move_target.update(cx, |editor, cx| editor.extend_drag(position, cx));
                }
            })
            .on_mouse_up(
                MouseButton::Left,
                move |_event: &MouseUpEvent, _window, cx| {
                    up_target.update(cx, |editor, _cx| editor.end_drag());
                },
            )
            .child(EditorElement { input: cx.entity() })
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::escape))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::select_page_up))
            .on_action(cx.listener(Self::select_page_down))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The corpus. Each entry is a class of range the UI can produce and a Latin-only
    /// test would never reach: CJK is one UTF-16 unit in three bytes, emoji is two
    /// units in four bytes, accents come precomposed or decomposed, and there is a
    /// variation selector, a regional-indicator pair, a bare ZWJ, and the empty buffer.
    fn hostile() -> Vec<(&'static str, &'static str)> {
        vec![
            ("", "empty"),
            ("plain", "ascii only"),
            ("中文", "CJK"),
            ("\u{1F600}\u{1F600}", "two surrogate pairs"),
            ("\u{00e9}", "precomposed acute"),
            ("e\u{0301}", "base plus combining acute"),
            ("\u{1F600}\u{FE0F}", "emoji plus variation selector"),
            ("a\u{1F1E6}\u{1F1E7}z", "regional indicator pair"),
            ("\u{200d}", "ZWJ alone"),
            ("中a\u{1F600}e\u{0301}\nend", "everything at once"),
        ]
    }

    fn units(s: &str) -> usize {
        s.encode_utf16().count()
    }

    /// (a) Every boundary conversion round-trips, both directions, on every boundary
    /// of every hostile string, and the unit total agrees with str::encode_utf16 -
    /// the oracle nobody here wrote.
    #[test]
    fn utf16_offsets_round_trip_at_every_boundary_of_every_hostile_text() {
        for (text, label) in hostile() {
            assert_eq!(
                offset_to_utf16(text, text.len()),
                units(text),
                "{label}: the byte length must map to the real UTF-16 length"
            );
            for (byte, _) in text.char_indices() {
                assert_eq!(
                    offset_from_utf16(text, offset_to_utf16(text, byte)),
                    byte,
                    "{label}: char boundary {byte} did not round-trip"
                );
            }
            assert_eq!(
                offset_from_utf16(text, offset_to_utf16(text, text.len())),
                text.len(),
                "{label}: the end did not round-trip"
            );
            // Every unit index, valid or mid-pair, must land on a sliceable byte.
            for unit in 0..=units(text) {
                let byte = offset_from_utf16(text, unit);
                assert!(
                    text.is_char_boundary(byte),
                    "{label}: unit {unit} mapped to a mid-char byte {byte}"
                );
                assert!(
                    offset_to_utf16(text, byte) >= unit,
                    "{label}: unit {unit} went backwards through the round trip"
                );
            }
        }
    }

    /// Longest shared prefix, snapped back to a boundary both sides agree on.
    fn common_prefix(a: &str, b: &str) -> usize {
        let mut n = 0;
        while n < a.len() && n < b.len() && a.as_bytes()[n] == b.as_bytes()[n] {
            n += 1;
        }
        while n > 0 && (!a.is_char_boundary(n) || !b.is_char_boundary(n)) {
            n -= 1;
        }
        n
    }

    fn common_suffix(a: &str, b: &str) -> usize {
        let max = a.len().min(b.len());
        let mut n = 0;
        while n < max && a.as_bytes()[a.len() - n - 1] == b.as_bytes()[b.len() - n - 1] {
            n += 1;
        }
        while n > 0 && (!a.is_char_boundary(a.len() - n) || !b.is_char_boundary(b.len() - n)) {
            n -= 1;
        }
        n
    }

    /// (b) THE PANIC CLASS, all of it. For every UTF-16 range a platform could derive
    /// from the text - including one that starts or ends between the two halves of a
    /// surrogate pair - a replacement must leave valid UTF-8, must keep the untouched
    /// prefix and suffix exactly, and must leave a caret we can slice with. No window,
    /// no GPU, thousands of cases.
    #[test]
    fn replacing_any_utf16_range_of_any_hostile_text_never_panics_and_never_splits_a_char() {
        let mut cases = 0usize;
        for (text, label) in hostile() {
            let total = units(text);
            for start in 0..=total {
                for end in start..=total {
                    let mut state = TextState::new(text.to_string());
                    state.replace(Some(start..end), "X");
                    cases += 1;
                    assert_eq!(
                        state.content.matches('X').count(),
                        1,
                        "{label} {start}..{end}"
                    );
                    // Everything not replaced is still there, whole and in order.
                    let pre = common_prefix(text, &state.content);
                    let post = common_suffix(&text[pre..], &state.content[pre + 1..]);
                    assert_eq!(
                        pre + post + 1,
                        state.content.len(),
                        "{label} {start}..{end}: the splice invented or lost bytes"
                    );
                    assert_eq!(
                        state.content,
                        text[..pre].to_string() + "X" + &text[text.len() - post..],
                        "{label} {start}..{end}: not a clean splice"
                    );
                    let caret = state.selected_range.start;
                    assert_eq!(
                        state.selected_range,
                        caret..caret,
                        "{label} {start}..{end}: selection not a caret"
                    );
                    assert!(
                        state.content.is_char_boundary(caret),
                        "{label} {start}..{end}: caret mid-char"
                    );
                    assert_eq!(
                        caret,
                        pre + 1,
                        "{label} {start}..{end}: caret not after the X"
                    );
                    assert!(
                        state.marked_range.is_none(),
                        "{label} {start}..{end}: still marked"
                    );
                }
            }
        }
        // 159 is the real size of this table; the floor is here so a future edit that
        // quietly narrows the corpus is a failure, not a faster test.
        assert!(cases > 150, "the table only ran {cases} cases");
    }

    /// The platform can also send a range that is stale - past the end after another
    /// edit - and a stale range must not take the app down.
    #[test]
    fn a_stale_or_oversized_range_is_clamped_rather_than_panicking() {
        for (text, label) in hostile() {
            let mut state = TextState::new(text.to_string());
            state.replace(Some(units(text)..units(text) + 500), "Y");
            assert!(state.content.ends_with('Y'), "{label}: tail insert lost");
            let mut state = TextState::new(text.to_string());
            state.replace(Some(usize::MAX - 1..usize::MAX), "Z");
            assert!(
                state.content.ends_with('Z'),
                "{label}: usize::MAX panicked the clamp"
            );
        }
    }

    /// A range that falls between a base character and its combining mark, or between
    /// an emoji and its variation selector: a legal CHARACTER boundary that is not a
    /// cluster boundary. This is the case S1 could only document as its known cost, so
    /// the test stayed and the expectation moved with the fix.
    #[test]
    fn an_insertion_between_a_base_and_its_mark_lands_past_the_whole_cluster() {
        // S1 recorded the opposite of this as its known cost: with a character
        // boundary the only rule, the insertion below separated `e` from its acute -
        // valid UTF-8, and a mark belonging to nothing. The dependency is in now, so
        // the test is kept, renamed, and INVERTED: a test that records a fixed bug is
        // the regression guard for the fix, and deleting it deletes the memory.
        let mut state = TextState::new("e\u{0301}".to_string());
        state.replace(Some(1..1), "-X-");
        assert_eq!(
            state.content, "e\u{0301}-X-",
            "the mark must not be stranded"
        );
        assert_eq!(state.selected_range, 6..6, "caret after the insertion");

        // One emoji plus its variation selector is ONE cluster, and unit 2 is a legal
        // CHARACTER boundary inside it. A caret must not cut it either.
        let mut state = TextState::new("\u{1F600}\u{FE0F}".to_string());
        state.replace(Some(2..2), "_X_");
        assert_eq!(state.content, "\u{1F600}\u{FE0F}_X_");
        assert!(std::str::from_utf8(state.content.as_bytes()).is_ok());
    }

    /// The other direction of the same rule: a NON-empty range widens outwards, so
    /// replacing the base of a cluster takes the cluster. Stranding the mark is the
    /// failure, not deleting more than was asked - and the user cannot see the
    /// difference, but a buffer full of orphan accents is unrecoverable.
    #[test]
    fn replacing_the_base_of_a_cluster_takes_the_whole_cluster() {
        let mut state = TextState::new("e\u{0301}z".to_string());
        state.replace(Some(0..1), "X");
        assert_eq!(state.content, "Xz", "the acute went with its base");

        let mut state = TextState::new("\u{1F600}\u{FE0F}z".to_string());
        state.replace(Some(0..2), "X");
        assert_eq!(state.content, "Xz", "the selector went with its emoji");
    }

    /// Caret motion, by cluster. This is the rule S4 will hang the mouse on, and it is
    /// why the motion helpers are STRICT where the clamp helper is inclusive: at a
    /// boundary, an inclusive search for the previous one returns the same place, i.e.
    /// a left arrow that does nothing.
    #[test]
    fn caret_motion_steps_by_cluster_not_by_byte_or_unit() {
        // a | emoji | e+acute  =  bytes 0, 1..5, 5..8
        let state = TextState::new("a\u{1F600}e\u{0301}".to_string());
        assert_eq!(state.next_boundary(0), 1, "one ascii char");
        assert_eq!(state.next_boundary(1), 5, "the whole emoji in one step");
        assert_eq!(state.next_boundary(5), 8, "base and mark in one step");
        assert_eq!(state.next_boundary(8), 8, "and it stops at the end");
        assert_eq!(state.previous_boundary(8), 5, "back over the e+acute");
        assert_eq!(state.previous_boundary(5), 1, "back over the emoji");
        assert_eq!(
            state.previous_boundary(6),
            5,
            "from mid-cluster, back to its start"
        );
        assert_eq!(state.previous_boundary(0), 0, "and it stops at the start");

        let mut state = TextState::new("a\u{1F600}e\u{0301}".to_string());
        state.move_to(0);
        for _ in 0..4 {
            let at = state.next_boundary(state.cursor_offset());
            state.move_to(at);
        }
        assert_eq!(state.cursor_offset(), 8, "four steps walk the whole buffer");
        for _ in 0..4 {
            let at = state.previous_boundary(state.cursor_offset());
            state.move_to(at);
        }
        assert_eq!(
            state.cursor_offset(),
            0,
            "and back again, three clusters deep"
        );

        // `new` puts the caret at the END of the buffer - which is the loading case,
        // a file opens with the caret after its last character - so walk to 0 first.
        let mut state = TextState::new("e\u{0301}z".to_string());
        state.move_to(0);
        assert_eq!(state.cursor_offset(), 0, "and the snap at 0 stays at 0");
        state.select_to(state.next_boundary(0));
        assert_eq!(
            state.selected_range,
            0..3,
            "shift-right selects the whole cluster, mark included"
        );
        assert!(!state.selection_reversed, "and forward stays forward");
        assert!(state.content.is_char_boundary(state.selected_range.end));
        state.select_all();
        assert_eq!(state.selected_range, 0..4);
        state.clear_selection();
        assert_eq!(state.selected_range, 4..4, "escape drops to the head");
        assert_eq!(state.content, "e\u{0301}z", "and touches nothing");
    }

    /// (c) An IME session and a fast typist must not disagree by one byte. The
    /// composition re-marks the whole run on every keystroke (no explicit range, so
    /// each one replaces the previous mark), then commits.
    #[test]
    fn a_marked_then_committed_sequence_is_the_same_bytes_as_typing_it() {
        let sessions = [
            ("zhongwen", "\u{4e2d}\u{6587}"),
            ("emoji", "\u{1F600}"),
            ("nn", "\u{4f60}\u{597d}"),
            ("e", "\u{00e9}"),
            ("", ""),
        ];
        for (composition, final_text) in sessions {
            let mut typed = TextState::default();
            for ch in final_text.chars() {
                let mut s = String::new();
                s.push(ch);
                typed.replace(None, &s);
            }

            let mut ime = TextState::default();
            let mut shown = String::new();
            for ch in composition.chars() {
                shown.push(ch);
                ime.replace_and_mark(None, &shown, Some(0..units(&shown)));
                assert_eq!(ime.content, shown, "the composition on screen drifted");
                assert!(
                    ime.marked_range.is_some(),
                    "an open composition must stay marked"
                );
            }
            match composition.is_empty() {
                true => ime.replace(None, final_text),
                false => ime.replace(Some(0..units(&shown)), final_text),
            }
            assert_eq!(
                ime.content, typed.content,
                "{composition} to {final_text}: IME bytes differ from typed bytes"
            );
            assert_eq!(ime.selected_range, typed.selected_range, "caret differs");
            assert_eq!(ime.marked_range, None, "composition survived the commit");
            assert_eq!(
                ime.marked_text_range(),
                None,
                "and the platform must be told so"
            );
        }
    }

    /// (d) S6 refuses to flush while a composition is open - sending marked text to
    /// the port is sending a half-typed syllable as if the user had written it - so
    /// the marked range has to be visible in the units the platform speaks, in the
    /// bytes we store, and it has to go away on commit.
    #[test]
    fn marked_range_is_reported_in_utf16_units_so_s6_can_refuse_to_flush_while_it_is_some() {
        let mut state = TextState::default();
        state.replace_and_mark(None, "zhong", Some(0..5));
        assert_eq!(state.marked_range, Some(0..5), "stored in bytes");
        assert_eq!(state.marked_text_range(), Some(0..5), "reported in units");

        state.replace(Some(0..5), "\u{4e2d}");
        assert_eq!(state.marked_range, None, "a commit clears it");
        assert_eq!(state.marked_text_range(), None);

        // Fresh buffer: a mark with no explicit range lands on the CARET, which the
        // commit above left at the end of the text, so re-marking in place would have
        // appended rather than composed.
        let mut state = TextState::default();
        state.replace_and_mark(None, "\u{4e2d}", Some(0..1));
        assert_eq!(
            state.marked_range,
            Some(0..3),
            "one CJK char is three bytes"
        );
        assert_eq!(state.marked_text_range(), Some(0..1), "and one UTF-16 unit");

        let mut state = TextState::new("a".to_string());
        state.replace_and_mark(Some(1..1), "\u{1F600}", Some(0..2));
        assert_eq!(state.marked_range, Some(1..5), "one emoji is four bytes");
        assert_eq!(
            state.marked_text_range(),
            Some(1..3),
            "and two units, after the a"
        );

        // The platform cancelling a composition leaves the text alone. S6 must not
        // read that as a commit either.
        state.unmark_text();
        assert_eq!(state.marked_range, None);
        assert_eq!(
            state.content, "a\u{1F600}",
            "unmarking must not delete text"
        );
    }

    /// The correction to the examples selection formula, as a test that fails against
    /// it: replacing a five-unit composition with one CJK character while asking for
    /// the whole new text to be selected. The example would report 0..8 in a three
    /// byte buffer, and the next slice after that is a panic.
    #[test]
    fn a_marked_replacement_over_an_existing_composition_selects_the_new_text() {
        let mut state = TextState::default();
        state.replace_and_mark(None, "zhong", Some(0..5));
        state.replace_and_mark(None, "\u{4e2d}", Some(0..1));
        assert_eq!(state.content, "\u{4e2d}");
        assert_eq!(state.marked_range, Some(0..3));
        assert!(
            state.selected_range.end <= state.content.len(),
            "selection ran past the buffer: {:?} of {} bytes",
            state.selected_range,
            state.content.len()
        );
        assert_eq!(state.selected_range, 0..3, "the new character is selected");
    }

    /// The requested selection is measured within the new text (gpui
    /// src/platform.rs:1042, mirroring setMarkedText:selectedRange:replacementRange:).
    /// Read document-wide it escapes the buffer, and it only escapes when there is
    /// text BEFORE the composition - which is the normal case.
    #[test]
    fn the_requested_selection_is_relative_to_the_new_text() {
        let mut state = TextState::new("\u{4f60}ab".to_string());
        assert_eq!(state.selected_range, 5..5, "caret after 你ab");
        state.replace_and_mark(None, "\u{4e2d}", Some(0..1));
        assert_eq!(state.content, "\u{4f60}ab\u{4e2d}");
        assert_eq!(state.selected_range, 5..8, "the inserted char, not 10..13");

        let mut state = TextState::new("x".to_string());
        state.replace_and_mark(None, "\u{4e2d}\u{6587}", Some(2..2));
        assert_eq!(state.selected_range, 7..7, "caret after two CJK chars");
    }

    /// The empty buffer is the state every new window starts in, and the first thing
    /// it receives can be a surrogate pair.
    #[test]
    fn an_empty_buffer_takes_a_surrogate_pair_as_its_first_character() {
        let mut state = TextState::default();
        assert_eq!(state.selected_range, 0..0);
        assert_eq!(state.marked_text_range(), None);
        state.replace(None, "\u{1F600}");
        assert_eq!(state.content, "\u{1F600}");
        assert_eq!(state.selected_range, 4..4, "a caret, not a selection");
        assert_eq!(
            state.selected_text_range().range,
            2..2,
            "four bytes, two units"
        );
        // A mark with no requested selection is a bare caret AFTER the new text.
        state.replace_and_mark(Some(2..2), "\u{1F600}", None);
        assert_eq!(state.content, "\u{1F600}\u{1F600}");
        assert_eq!(state.selected_range, 8..8, "caret after both emoji");
        assert_eq!(state.marked_range, Some(4..8));
        // With one requested, the new emoji comes back selected in whole.
        let mut state = TextState::default();
        state.replace_and_mark(None, "\u{1F600}", Some(0..2));
        assert_eq!(state.selected_range, 0..4, "four bytes, selected");
        assert_eq!(
            state.selected_text_range().range,
            0..2,
            "two units, reported"
        );
    }

    /// text_for_range has to tell the platform what it actually gave back, because a
    /// request for half a surrogate pair cannot be honoured.
    #[test]
    fn text_for_a_half_of_a_surrogate_pair_reports_the_whole_character() {
        let mut state = TextState::new("\u{1F600}a".to_string());
        let mut adjusted = None;
        let got = state.text_for_range(1..2, &mut adjusted);
        assert_eq!(
            got.as_deref(),
            Some("\u{1F600}"),
            "the unit range asked for half a pair"
        );
        assert_eq!(
            adjusted,
            Some(0..2),
            "and it said which units it actually read"
        );

        let mut adjusted = None;
        assert_eq!(
            state.text_for_range(0..2, &mut adjusted).as_deref(),
            Some("\u{1F600}"),
            "the whole emoji"
        );
        assert_eq!(adjusted, Some(0..2));
        let mut adjusted = None;
        assert_eq!(
            state.text_for_range(2..3, &mut adjusted).as_deref(),
            Some("a")
        );
        assert_eq!(adjusted, Some(2..3));
    }
    /// S3, item 1: the line model. LF only, and a buffer ending in a newline gets the
    /// trailing empty line - that phantom line is where Enter at the end puts the caret,
    /// so a model that omits it cannot scroll or paint correctly at the end of a note.
    #[test]
    fn the_line_model_cuts_on_line_feeds_and_keeps_the_final_empty_line() {
        assert_eq!(
            line_ranges("a\nbb\n\nc"),
            vec![0..1, 2..4, 5..5, 6..7],
            "ranges exclude the line feed itself"
        );
        assert_eq!(
            line_ranges(""),
            vec![0..0],
            "an empty buffer is one empty line"
        );
        assert_eq!(line_ranges("x\n"), vec![0..1, 2..2]);
        let state = TextState::new("abcdef\nab\nabcdefghij".to_string());
        assert_eq!(state.line_index_at(5), 0);
        assert_eq!(state.line_index_at(7), 1);
        assert_eq!(state.line_range_at(15), 10..20);
    }

    /// S3, item 3: the sticky column. A caret five characters into a paragraph that
    /// walks down past a SHORT line must come back to column five on the next long one,
    /// not camp at the margin - which is what a nearest-x implementation does.
    #[test]
    fn up_and_down_remember_the_column_through_a_short_line() {
        let mut state = TextState::new("abcdef\nab\nabcdefghij".to_string());
        state.move_to(5);
        assert_eq!(state.column_of(5), 5);
        state.move_vertical(1);
        assert_eq!(state.cursor_offset(), 9, "clamped to the short line end");
        state.move_vertical(1);
        assert_eq!(
            state.cursor_offset(),
            15,
            "and back to column five on line three"
        );
        state.move_vertical(-1);
        assert_eq!(state.cursor_offset(), 9);
        state.move_vertical(-1);
        assert_eq!(
            state.cursor_offset(),
            5,
            "the column survived the whole walk"
        );
        state.move_vertical(-1);
        assert_eq!(state.cursor_offset(), 0, "the top is a no-op, not a panic");
        state.move_vertical(5);
        assert_eq!(
            state.cursor_offset(),
            15,
            "five lines down is the LAST line, still at column five - not the margin"
        );
        state.move_vertical(1);
        assert_eq!(
            state.cursor_offset(),
            20,
            "and one more down on the last line goes to its end, the classic rule"
        );
    }

    /// Columns are clusters, so a home/end/up/down walk over CJK and combining marks
    /// steps by what the user sees rather than by bytes or code units.
    #[test]
    fn columns_are_clusters_so_cjk_and_marks_count_once() {
        let state = TextState::new("\u{4e2d}\u{6587}\nabc".to_string());
        assert_eq!(state.line_range_at(0), 0..6, "two characters, six bytes");
        assert_eq!(state.column_of(3), 1);
        assert_eq!(state.byte_at_column(0, 2), 6);
        let state = TextState::new("e\u{0301}z\nq".to_string());
        assert_eq!(
            state.byte_at_column(0, 1),
            3,
            "one step is the base AND its acute, never the acute alone"
        );
    }

    /// S3, item 2: a pasted block keeps its newlines and CRLF becomes LF on the way in.
    /// This is the handler body without the window; the flatten the example ships at
    /// examples/input.rs:133 would have made one 12-character line out of this.
    #[test]
    fn a_pasted_crlf_block_becomes_real_lines() {
        let mut state = TextState::new(String::new());
        let pasted = "one\r\ntwo\rthree"
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        state.replace(None, &pasted);
        assert_eq!(state.content, "one\ntwo\nthree");
        assert_eq!(line_ranges(&state.content).len(), 3);
        assert!(!state.content.contains('\r'), "the buffer is LF-only");
        assert!(
            state.content.matches('\n').count() == 2,
            "both line feeds are still there"
        );
    }
    /// S4: shift+up and shift+down share the caret motions target exactly, so the
    /// selection can never disagree with where the arrow would have gone - one rule,
    /// two doors. And the head is the end that moves, which is what makes a second
    /// shift-down continue rather than restart.
    #[test]
    fn shift_selection_across_lines_follows_the_same_target_as_the_caret() {
        let mut state = TextState::new("abcdef\nab\nabcdefghij".to_string());
        state.move_to(5);
        state.select_vertical(1);
        assert_eq!(
            state.selected_range,
            5..9,
            "the head moved to the short line end and the anchor stayed put"
        );
        assert!(
            !state.selection_reversed,
            "growing forward leaves the head at the end; reversed means the head is at the start"
        );
        state.select_vertical(1);
        assert_eq!(
            state.selected_range,
            5..15,
            "and a second shift-down continues to column five on line three"
        );
        state.select_vertical(-1);
        assert_eq!(state.selected_range, 5..9, "back up the same rule applies");
        assert_eq!(
            state.vertical_target(1),
            15,
            "and down still means column five - one rule, two doors"
        );
    }
    /// S5, item 2: a double click selects a WORD, and the two rules that matter are that
    /// the run is made of clusters (never half of an `e` + U+0301) and that a line feed
    /// always ends it (a double click must never reach into the next line).
    #[test]
    fn a_double_click_selects_a_word_and_never_a_line_or_a_half_cluster() {
        // Bytes: `one`=0..3, space=3, `beta`=4..8, \n=8, then `t`=9 `h`=10 `r`=11 and
        // `e`+U+0301 as ONE cluster at 12..15, `s`=15..16, space=16, `x`=17. So the word
        // the mark sits in is the whole alphanumeric run - 9..17 - which is exactly what
        // "never splits a cluster" costs and why the test says so out loud.
        let state = TextState::new("one beta\nthree\u{301}s x".to_string());
        assert_eq!(
            state.word_range_at(1),
            0..3,
            "inside 'one' is the whole of 'one'"
        );
        assert_eq!(
            state.word_range_at(5),
            4..8,
            "'beta' stops at the line feed rather than running on"
        );
        assert_eq!(
            state.word_range_at(14),
            9..17,
            "a click on the combining mark takes its base, its word, and nothing more"
        );
        let run = state.word_range_at(10);
        assert_eq!(run, 9..17, "the same run from another byte inside it");
        assert_eq!(&state.content[run.clone()], "three\u{301}s");
        assert!(
            !state.content[run].contains('\n'),
            "a word never crosses a line"
        );
        assert_eq!(
            state.word_range_at(16),
            9..17,
            "byte 16 is past the 's', and the cluster to its left still owns it - the word"
        );
        assert_eq!(
            state.word_range_at(17),
            17..18,
            "a click ON the space takes the whitespace run, not the words either side"
        );
        assert_eq!(state.word_range_at(8), 8..9, "a line feed is its own run");
    }

    /// S5, item 4, the IME-integrity rule: a caret motion, a drag or a word/line
    /// selection COMMITS an open composition. The characters stay - `replace_and_mark`
    /// already spliced them in - but the uncommitted flag has to go, or the next
    /// keystroke replaces text the user clicked away from and S6 refuses to flush over a
    /// composition that no longer exists.
    #[test]
    fn any_motion_commits_an_open_composition() {
        let mut state = TextState::new(String::new());
        state.replace_and_mark(None, "nihao", None);
        assert_eq!(state.marked_range, Some(0..5), "marked while composing");
        state.move_to(2);
        assert_eq!(
            state.marked_range, None,
            "a click commits: the bytes stay, the mark goes"
        );
        assert_eq!(state.content, "nihao", "committing is not deleting");
        state.replace_and_mark(None, "x", None);
        state.select_to(1);
        assert_eq!(state.marked_range, None, "a drag commits too");
        state.replace_and_mark(None, "y", None);
        state.select_range(0..2);
        assert_eq!(state.marked_range, None, "and so does a double click");
    }

    /// S5, item 1: the drag's direction. `select_to` is the whole mechanism - it flips
    /// which end is the head when the pointer crosses the anchor - so a drag UP leaves
    /// the caret at the TOP, which is what the reversed flag in selected_text_range
    /// reports and what S6 will hand the clipboard.
    #[test]
    fn dragging_up_leaves_the_caret_at_the_top() {
        let mut down = TextState::new("abcdefghij\nklmnopqrst\nuvwxyz".to_string());
        down.move_to(3);
        down.select_to(23);
        assert_eq!(down.selected_range, 3..23, "drag down");
        assert!(!down.selection_reversed, "head at the end");
        assert_eq!(down.cursor_offset(), 23);
        let mut up = TextState::new("abcdefghij\nklmnopqrst\nuvwxyz".to_string());
        up.move_to(23);
        up.select_to(3);
        assert_eq!(up.selected_range, 3..23, "a drag up selects the same bytes");
        assert!(up.selection_reversed, "with the head at the top");
        assert_eq!(
            up.cursor_offset(),
            3,
            "and that is where the caret is - the rule the clipboard reads"
        );
        assert!(
            up.selected_text_range().reversed,
            "and it reaches the platform"
        );
    }

    /// S5, item 3: the clipboard keeps newlines in BOTH directions. Copy reads the
    /// buffer's own bytes (LF inside, always), and the platform's own read of a range
    /// spanning lines reports the line feeds too - the flatten at
    /// examples/input.rs:133 is a fork bug, not a convention.
    #[test]
    fn a_multi_line_selection_keeps_its_newlines_both_ways() {
        let mut state = TextState::new("first\nsecond\nthird".to_string());
        state.selected_range = 0..18;
        let copied = state.content[state.selected_range.clone()].to_string();
        assert_eq!(copied, "first\nsecond\nthird");
        assert_eq!(
            copied.matches('\n').count(),
            2,
            "two line feeds, not two spaces"
        );
        assert!(
            !copied.contains('\r'),
            "and no carriage return on the way out"
        );
        let utf16 = range_to_utf16(&state.content, &(0..18));
        let mut adjusted = None;
        let read = state.text_for_range(utf16, &mut adjusted);
        assert_eq!(
            read.as_deref(),
            Some("first\nsecond\nthird"),
            "the IME read keeps them as well"
        );
    }

    /// S5, item 4: with a selection there is one, the selection is what a keystroke,
    /// a paste and a delete all act on - including a right-to-left selection, which is
    /// the same range with a different head.
    #[test]
    fn typing_pasting_and_deleting_all_replace_the_selection() {
        let mut type_over = TextState::new("keep-drop-keep".to_string());
        type_over.selected_range = 5..9;
        type_over.replace(None, "X");
        assert_eq!(type_over.content, "keep-X-keep");
        assert!(type_over.marked_range.is_none());
        assert_eq!(type_over.cursor_offset(), 6, "caret after what was typed");

        let mut paste_over = TextState::new("aaa\nbbb\nccc".to_string());
        paste_over.selected_range = 4..7;
        paste_over.replace(None, "ZZ");
        assert_eq!(paste_over.content, "aaa\nZZ\nccc");

        let mut reversed = TextState::new("0123456789".to_string());
        reversed.selected_range = 2..7;
        reversed.selection_reversed = true;
        reversed.replace(None, "");
        assert_eq!(
            reversed.content, "01789",
            "backspace over a right-to-left run"
        );
        assert!(!reversed.selection_reversed, "the head is now the caret");
    }
    /// The bug this guards: `Editor::load` cannot be reached from a headless test (a
    /// `FocusHandle` needs a window, which is why `TextState` exists at all), so the
    /// one decision load makes in the view rule is tested where it can be seen - a
    /// loaded note whose caret lands on line 0 must still be followed, because the OLD
    /// document's `caret_line_shown` said 1999 and a plain equality check would call
    /// that "no change" and leave the note open at the previous document's offset.
    #[test]
    fn a_loaded_document_always_reasserts_where_the_view_is() {
        assert!(
            follow_required(None, 0),
            "nothing shown yet: the view is claimed, even for line 0"
        );
        assert!(
            follow_required(Some(1999), 0),
            "the stale line from the previous note is not the new caret line"
        );
        assert!(
            !follow_required(Some(0), 0),
            "and a caret that has not changed line does NOT yank the view back - the S4 rule"
        );
    }
    /// The counter the wire watches, pinned because its failure mode is invisible: an
    /// editor that types correctly, paints correctly and NEVER SAVES. Every text
    /// mutation raises it; a click, a drag, a selection and a scroll must not - each of
    /// those used to be able to wake a save on a document nobody changed.
    #[test]
    fn only_a_text_mutation_moves_the_counter_the_wire_reads() {
        let mut state = TextState::new("one\ntwo\nthree".to_string());
        assert_eq!(
            state.edits, 0,
            "a fresh buffer is clean, however it was made"
        );
        state.move_to(3);
        state.select_to(9);
        state.select_vertical(-1);
        assert_eq!(state.edits, 0, "motion and selection change nothing");
        state.replace(None, "X");
        assert_eq!(state.edits, 1, "one mutation, one step");
        state.replace(None, "");
        assert_eq!(state.edits, 2, "a delete is a mutation too");
        state.replace_and_mark(None, "nihao", None);
        assert_eq!(state.edits, 3, "so is an uncommitted composition");
        state.select_range(0..3);
        assert_eq!(state.edits, 3, "and a double click is not");
    }
}
