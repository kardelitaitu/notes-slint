//! The editor model, and the eight methods the platform IME calls on it. Slice S1.
//!
//! S1 is deliberately invisible: nothing here paints, nothing here talks to the port,
//! and `main` does not construct an `Editor` yet. What S1 buys is the one thing every
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
#![allow(dead_code)] // S1 has no painter and main.rs has no editor widget yet; S2 removes this line.

use std::ops::Range;

use gpui::UTF16Selection;
use gpui::{
    App, Bounds, Context, EntityInputHandler, FocusHandle, Focusable, Pixels, Point, Window,
};

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

/// The largest byte offset `<= byte` that is a UTF-8 character boundary.
///
/// THIS IS THE HAND-WRITTEN HALF OF WHAT `unicode-segmentation` WOULD DO, and it is
/// deliberately weaker: it finds CHARACTER boundaries, not GRAPHEME CLUSTER
/// boundaries, because a cluster boundary needs the Unicode property tables and this
/// crate has no dependency that carries them (see the FCR in the slice report: the
/// workspace template does not list `unicode-segmentation`, and gpui has it only as a
/// DEV dependency, so `examples/input.rs:11` can use it and we cannot).
///
/// Why that is enough for S1 and not enough for S4: slicing is the only place a wrong
/// answer PANICS, and every slice in this file goes through a character boundary.
/// Putting the caret between `e` and U+0301 does not panic - it looks wrong on screen
/// and the user cannot see why - so the cluster snap belongs to the slice that moves
/// the caret to a mouse point, where the grapheme table (or an explicit range list)
/// must be added. It is one function, `snap_to_char_boundary`, and the note below is
/// the seam.
pub(crate) fn snap_to_char_boundary(content: &str, byte: usize) -> usize {
    let byte = byte.min(content.len());
    if content.is_char_boundary(byte) {
        return byte;
    }
    let mut candidate = byte;
    while !content.is_char_boundary(candidate) {
        // A continuation byte is 0b10xx_xxxx, so at most three steps back reaches the
        // head of the sequence, and `is_char_boundary(0)` is always true - no underflow.
        candidate -= 1;
    }
    candidate
}

/// Clamp a caller-supplied byte range into the content on character boundaries and
/// make it ordered. Every index that arrives from the platform passes through here.
pub(crate) fn clamp_range(content: &str, range: &Range<usize>) -> Range<usize> {
    let start = snap_to_char_boundary(content, range.start);
    let end = snap_to_char_boundary(content, range.end.max(range.start));
    start..end
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
}

impl Editor {
    /// The context's `focus_handle()` is `App::focus_handle` (gpui src/app.rs:2029)
    /// reached through `Context`'s deref; the example builds a `TextInput` the same way
    /// (examples/input.rs:703-704).
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            state: TextState::default(),
            focus_handle: cx.focus_handle(),
        }
    }

    #[allow(dead_code)] // S2 onward
    pub(crate) fn with_content(content: String, cx: &mut Context<Self>) -> Self {
        Self {
            state: TextState::new(content),
            focus_handle: cx.focus_handle(),
        }
    }

    pub(crate) fn state(&self) -> &TextState {
        &self.state
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

    /// PLACEHOLDER UNTIL S2 HAS LAYOUT, and deliberately returning `None`.
    ///
    /// The platform asks where a range of text is so it can put the IME candidate
    /// window there. The example answers from `self.last_layout` (examples/input.rs
    /// :352-371), which is the shaped line its `paint` recorded; there is no layout in
    /// S1, so the honest answer is "I do not know yet", which lets the OS place the
    /// window where it always puts one. A fabricated rectangle would be worse than
    /// useless: the candidate list would sit over the wrong text and it would look like
    /// a layout bug in S2 rather than a lie told here. When `TextLayout` lands, this
    /// becomes: convert with `range_from_utf16`, then corners from the layouts x at the
    /// two indices, inside `element_bounds`.
    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        None
    }

    /// PLACEHOLDER UNTIL S2 HAS LAYOUT, same reasoning: without a shaped line there is
    /// no honest answer to "which character is at this pixel". Returns `None` (the
    /// platform treats it as "no drag-to-position information") rather than guessing a
    /// byte index from the x coordinate, which for a CJK or emoji buffer is a cursor in
    /// the wrong place. Becomes: localize the point, `index_for_x`, `offset_to_utf16`.
    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
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
    /// an emoji and its variation selector. It cannot panic and cannot produce invalid
    /// UTF-8. Note honestly what it does instead: it separates the cluster, because a
    /// cluster boundary needs Unicode tables this crate does not have (see the
    /// unicode-segmentation FCR). snap_to_char_boundary is the one function that
    /// changes when that dependency lands, and S4 is where it starts being called.
    #[test]
    fn a_replacement_that_splits_a_grapheme_cluster_is_valid_utf8_and_says_so() {
        let mut state = TextState::new("e\u{0301}".to_string());
        state.replace(Some(1..1), "-X-");
        assert_eq!(state.content, "e-X-\u{0301}", "the mark ended up detached");
        assert!(std::str::from_utf8(state.content.as_bytes()).is_ok());

        let mut state = TextState::new("\u{1F600}\u{FE0F}".to_string());
        state.replace(Some(2..2), "_X_");
        assert!(state.content.starts_with('\u{1F600}'));
        assert!(std::str::from_utf8(state.content.as_bytes()).is_ok());
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
