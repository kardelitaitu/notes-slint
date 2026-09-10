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

use gpui::prelude::*;
use gpui::{
    App, Bounds, ClipboardItem, Context, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, PaintQuad, Pixels, Point, Render, ShapedLine, SharedString, Style, TextRun,
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
    pub(crate) marked_range: Option<Range<usize>>,
}

impl TextState {
    pub(crate) fn new(content: String) -> Self {
        let len = content.len();
        Self {
            content,
            selected_range: len..len,
            selection_reversed: false,
            marked_range: None,
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

    /// Collapse the selection to a caret at `offset`, snapped forward onto a grapheme
    /// boundary so a caller can hand us an index from anywhere and we cannot end up
    /// holding a position that would split a cluster if it became an insertion.
    pub(crate) fn move_to(&mut self, offset: usize) {
        let at = grapheme_boundary_after(&self.content, offset);
        self.selected_range = at..at;
        self.selection_reversed = false;
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
    }

    /// Ctrl+A.
    pub(crate) fn select_all(&mut self) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
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

    /// The end of the first line: the only part S2 shapes. Multiline is S3.
    pub(crate) fn first_line_end(&self) -> usize {
        match self.content.find('\n') {
            // Exclude the newline itself: gpui shape_line debug-asserts that its input
            // has none (src/text_system.rs:372), and a shaped line would draw a box.
            Some(index) => index,
            None => self.content.len(),
        }
    }

    /// The exact text that was shaped, so a cached layout can be checked against the
    /// buffer it came from. This is a SUBSTRING from byte 0, which is what keeps the
    /// byte indices S3 and S4 hand to `x_for_index` in the buffers own space.
    pub(crate) fn first_line(&self) -> &str {
        let end = self.first_line_end();
        &self.content[..end]
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
    last_layout: Option<ShapedLine>,
    /// The text that `last_layout` was shaped from. Compared by value rather than
    /// assumed equal, because the shaped text is a first-line SUBSTRING of the buffer
    /// - see `display_desynced`.
    last_layout_text: String,
    last_bounds: Option<Bounds<Pixels>>,
}

impl Editor {
    /// The context's `focus_handle()` is `App::focus_handle` (gpui src/app.rs:2029)
    /// reached through `Context`'s deref; the example builds a `TextInput` the same way
    /// (examples/input.rs:703-704).
    /// No font, no shaping, no layout: S1/S2 construct cheap and shape on first paint
    /// (the cold-start budget, whitepaper section 2). This is the whole reason
    /// `last_layout` starts empty.
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            state: TextState::default(),
            focus_handle: cx.focus_handle(),
            last_layout: None,
            last_layout_text: String::new(),
            last_bounds: None,
        }
    }

    #[allow(dead_code)] // S3 onward: the buffer arrives from the port in S6
    pub(crate) fn with_content(content: String, cx: &mut Context<Self>) -> Self {
        Self {
            state: TextState::new(content),
            focus_handle: cx.focus_handle(),
            last_layout: None,
            last_layout_text: String::new(),
            last_bounds: None,
        }
    }

    /// True when the cached layout no longer describes what we would paint. A stale
    /// layout is not a crash waiting to happen, it is a WRONG IME RECT waiting to
    /// happen, so both geometry methods check this instead of trusting the cache.
    fn display_desynced(&self) -> bool {
        self.last_layout_text != self.state.first_line()
    }

    /// What S6 will refuse to flush while this is `Some`: an uncommitted composition
    /// is not the user's text yet.
    #[allow(dead_code)] // S2 onward
    pub(crate) fn text(&self) -> &str {
        &self.state.content
    }

    #[allow(dead_code)] // S2 onward
    pub(crate) fn is_composing(&self) -> bool {
        self.state.marked_range.is_some()
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
    /// WHERE THE IME CANDIDATE WINDOW GOES. Real from S2, and the reason the layout is
    /// cached in paint: the platform asks this between frames, so the only honest
    /// source is the line that was actually drawn.
    ///
    /// The x coordinates come from the cached layout (byte indices, which is what
    /// `x_for_index` takes - the buffer and the shaped line agree on them because the
    /// shaped text is a substring from byte 0). The frame comes from `element_bounds`,
    /// the callers own idea of where the field is, rather than from the cached bounds,
    /// so a resize in flight cannot make the candidate list lag the window.
    ///
    /// If the cache does not describe the current buffer, the answer is `None`: a
    /// rectangle built from a stale layout is not a rough guess, it is a lie the
    /// candidate window will sit on top of.
    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        if self.display_desynced() {
            probe("bounds_for_range: no answer, the cached layout is stale");
            return None;
        }
        let line_len = layout.text.len();
        let range = clamp_range(
            &self.state.content,
            &range_from_utf16(&self.state.content, &range_utf16),
        );
        let start = range.start.min(line_len);
        let end = range.end.min(line_len).max(start);
        let left = layout.x_for_index(start);
        let right = layout.x_for_index(end);
        probe(format!(
            "bounds_for_range units {range_utf16:?} -> bytes {start}..{end}, x {:.1}..{:.1}",
            f32::from(left),
            f32::from(right)
        ));
        Some(Bounds::from_corners(
            point(element_bounds.left() + left, element_bounds.top()),
            point(element_bounds.left() + right, element_bounds.bottom()),
        ))
    }

    /// WHICH CHARACTER IS UNDER THIS PIXEL, used by drag-to-position. Real from S2,
    /// and this is where the assert the plan decided to keep lives - see the comment
    /// on the check below.
    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let line_point = self.last_bounds?.localize(&point)?;
        let layout = self.last_layout.as_ref()?;
        // examples/input.rs:383 has `assert_eq!(last_layout.text, self.content)` here
        // - a plain assert, in the path a keystroke takes. The invariant is right and
        // is kept: a cached layout that does not describe the buffer would return a
        // character index for text that is no longer there. The FAILURE MODE is what
        // changed. A notes app must not abort because a frame was stale, so a release
        // build returns None (the platform loses drag-to-position for that one call)
        // and a debug build still panics, loudly, with both strings named, at the
        // exact frame the cache and the buffer diverged.
        debug_assert!(
            !self.display_desynced(),
            "editor: cached layout {:?} does not describe the buffer's first line {:?}",
            self.last_layout_text,
            self.state.first_line()
        );
        if self.display_desynced() {
            probe("character_index_for_point: no answer, the cached layout is stale");
            return None;
        }
        let utf8 = layout.index_for_x(point.x - line_point.x)?;
        let units = offset_to_utf16(&self.state.content, utf8);
        probe(format!(
            "character_index_for_point x={:.1} -> byte {utf8} -> unit {units}",
            f32::from(point.x)
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
        self.state.move_to(0);
        cx.notify();
    }

    fn end(&mut self, _: &End, _window: &mut Window, cx: &mut Context<Self>) {
        let len = self.state.content.len();
        self.state.move_to(len);
        cx.notify();
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
    /// guarantees. Note the flatten below - it is a SINGLE-LINE field being honest,
    /// and S3 has to delete it. Paste also goes through `replace_text_in_range`, so a
    /// paste cannot land mid-cluster by construction.
    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let one_line = text.replace('\n', " ");
            self.replace_text_in_range(None, &one_line, window, cx);
        }
    }

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
// The paint: one shaped line, the caret, and the composition underlined
// ---------------------------------------------------------------------------

/// What prepaint worked out and paint draws, so nothing is shaped twice and the
/// quads are painted from the SAME line the caret position came from.
pub(crate) struct PrepaintState {
    line: Option<ShapedLine>,
    /// The text `line` was shaped from, carried to `paint` so it can be cached on the
    /// editor next to the layout.
    shaped: String,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

/// The editor element. Copy of the examples shape (examples/input.rs:388-562):
/// request a full-width, one-line-high box, shape in prepaint, and in paint hand the
/// bounds to the IME, draw selection, glyphs, caret, then cache.
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
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
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
        let input = self.input.read(cx);
        let display = input.state.first_line().to_string();
        let selected_range = input.state.selected_range.clone();
        let cursor = input.state.cursor_offset();
        let marked_range = input.state.marked_range.clone();
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());

        // Runs split at the composition boundaries so the marked text can be
        // underlined: before it, the marked part, after it. Empty runs are filtered
        // out, because a zero-length run is what makes gpui draw a stray underline.
        let run = TextRun {
            len: display.len(),
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match marked_range {
            Some(marked) => {
                let marked = clamp_range(&display, &marked);
                vec![
                    TextRun {
                        len: marked.start,
                        ..run.clone()
                    },
                    TextRun {
                        len: marked.end - marked.start,
                        underline: Some(UnderlineStyle {
                            color: Some(run.color),
                            thickness: px(1.0),
                            wavy: false,
                        }),
                        ..run.clone()
                    },
                    TextRun {
                        len: display.len() - marked.end,
                        ..run
                    },
                ]
                .into_iter()
                .filter(|run| run.len > 0)
                .collect()
            }
            None => vec![run],
        };

        let line = window.text_system().shape_line(
            SharedString::from(display.clone()),
            font_size,
            &runs,
            None,
        );

        let cursor_pos = line.x_for_index(cursor.min(display.len()));
        let (selection, cursor_quad) = if selected_range.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + cursor_pos, bounds.top()),
                        size(px(2.), bounds.bottom() - bounds.top()),
                    ),
                    rgb(0x0033_99ff),
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left()
                                + line.x_for_index(selected_range.start.min(display.len())),
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + line.x_for_index(selected_range.end.min(display.len())),
                            bounds.bottom(),
                        ),
                    ),
                    rgb(0x2d_4a_6b),
                )),
                None,
            )
        };
        PrepaintState {
            line: Some(line),
            shaped: display,
            cursor: cursor_quad,
            selection,
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
        // This call is what makes the two geometry methods above answerable at all:
        // it registers this element as the windows input target for the entity.
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection);
        }
        let Some(line) = prepaint.line.take() else {
            return;
        };
        // A failed paint is a blank line for one frame, not an aborted editor.
        let _ = line.paint(bounds.origin, window.line_height(), window, cx);
        if focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }
        let shaped = std::mem::take(&mut prepaint.shaped);
        let caret = self.input.read(cx).state.cursor_offset();
        // The same expression bounds_for_range evaluates, from the same line object,
        // so the probe is evidence about the geometry answer and not about a copy of
        // the arithmetic.
        probe(format!(
            "paint shaped={:?} caret_byte={caret} caret_x={:.1}",
            shaped,
            f32::from(line.x_for_index(caret.min(shaped.len())))
        ));
        // And here the SAME method the platform calls, called with the arguments the
        // platform would give it, from inside a real frame. Nothing else available
        // from a script proves this: without a composition in progress Windows never
        // asks, so a live run shows zero calls. The caret x in the line above and the
        // rect x here come from one layout object, which is the claim being made.
        let units = offset_to_utf16(&self.input.read(cx).state.content, caret);
        let answered = self.input.update(cx, |editor, cx| {
            editor.bounds_for_range(units..units, bounds, window, cx)
        });
        match answered {
            Some(rect) => probe(format!(
                "bounds_for_range(0-width at unit {units}) -> x={:.1} width={:.1} height={:.1}",
                f32::from(rect.origin.x),
                f32::from(rect.size.width),
                f32::from(rect.size.height)
            )),
            None => probe(format!("bounds_for_range(0-width at unit {units}) -> None")),
        }
        self.input.update(cx, |input, _cx| {
            input.last_bounds = Some(bounds);
            input.last_layout_text = shaped;
            input.last_layout = Some(line);
        });
    }
}

impl Render for Editor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("NotesEditor")
            .track_focus(&self.focus_handle(cx))
            .size_full()
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
}
