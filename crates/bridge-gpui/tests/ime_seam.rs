//! The IME seam, pinned against the kit generation (gpui-kit 0.6.1 / gpui-pre 0.3.4,
//! commit 33e24dd7). The bridge crate has no lib target (Cargo.toml declares only a
//! bin, "notes-gpui"), so the editor is included by path: nothing under src/ is
//! modified, and everything pub(crate) in it is visible here because it is compiled
//! into THIS test crate. editor.rs carries no crate:: or notes_api references, so the
//! include is self-contained; its own #[cfg(test)] unit tests also activate here and
//! run in this binary.
//!
//! What each group proves, against the trait as it exists in 0.3.4
//! (gpui-pre-0.3.4/src/input.rs, read in the registry cache):
//!
//! 1. Compile fact: Editor: EntityInputHandler with NONE of the eight methods the
//!    trait grew - so the impl only compiles because the defaults exist. The two the
//!    migration report worried about are NOT delegations in 0.3.4: text_length_utf16
//!    is the constant None (input.rs:94-100) and text_input_editable_range is the
//!    constant None (input.rs:117-123). set_selected_text_range's default is an EMPTY
//!    BODY (input.rs:85-91) - a silent no-op, not a forward to our methods.
//!    prefers_ime_for_printable_keys does delegate to accepts_text_input
//!    (input.rs:262-265), whose default is true (input.rs:103-105), and paste forwards
//!    to our replace_text_in_range (input.rs:42-46). The runtime value of those
//!    defaults is asserted in the window_driven module below, which only compiles with
//!    the test-support feature (the two manifest lines are in the report) - until then
//!    they are NOT evidence and are not claimed as any.
//! 2. On Windows the IME consumer surface is narrow (gpui-pre-windows-0.3.4/src/
//!    events.rs): the commit is replace_text_in_range(None, ...) (:744), a composition
//!    update is replace_and_mark_text_in_range (:760), the candidate rect is
//!    selected_text_range + bounds_for_range (:648-649), and IME-vs-key routing is
//!    query_accepts_text_input (:697). The Windows backend never calls
//!    text_length_utf16, text_input_editable_range or set_selected_text_range.
//! 3. The model takes UTF-16 units at its boundary and stores bytes (editor.rs:258-268,
//!    295-304), so the UTF-16/UTF-8 and commit-order pins below run headless.

// The included editor.rs carries items only main.rs reaches (text, is_composing, edits,
// load); as a module of this test crate they are dead code, which would otherwise fail
// the workspace clippy gate. Scoped here, at the crate root of the TEST crate only.
#![allow(dead_code)]
// TWO MORE, both caused by this file being a test crate that includes editor.rs by path
// rather than linking the bin (ac6fc37b landed with them; they are properties of the
// arrangement, not of the select-all bug this file was written for):
//  * `unexpected_cfgs`: the window_driven module below is gated on gpui-kit's
//    `test-support` feature, which is a FEATURE OF THAT CRATE and so is not a cfg value
//    this package declares. Narrowly allowed, because the gate is the point - without it
//    the module compiles against a TestAppContext that cannot open a window.
//  * `unused_crate_dependencies`: the package's `notes_api` and `raw_window_handle` are
//    used by main.rs, which is NOT part of this test crate. Same reason the bridge's
//    editor-only unit tests do not need them.
#![allow(unexpected_cfgs)]
#![allow(unused_crate_dependencies)]

#[path = "../src/editor.rs"]
mod editor;

use editor::TextState;
use gpui_kit::EntityInputHandler;
use unicode_segmentation::UnicodeSegmentation;

/// A line holding every shape the IME can be asked to index: e + U+0301 (a combining
/// sequence), U+2611 + U+FE0F (an emoji with a variation selector) and a CJK run.
/// 21 UTF-8 bytes, 10 UTF-16 code units, 8 grapheme clusters - the 21-vs-10 gap is
/// exactly the difference a UTF-8-for-UTF-16 mistake makes, and it lands mid-word.
const LINE: &str = "e\u{301}\u{2611}\u{FE0F} 日本語 x";

fn composing_state(content: &str, composition: &str) -> TextState {
    let mut state = TextState::new(content.to_string());
    state.move_to(content.len());
    state.replace_and_mark(None, composition, None);
    state
}

// ---------------------------------------------------------------------------
// 1. The compile fact: the impl stands on the eight defaults
// ---------------------------------------------------------------------------

#[test]
fn editor_satisfies_entity_input_handler_so_the_eight_defaults_exist() {
    // This assertion carries the whole migration question at compile time: the impl
    // (editor.rs:979-1126) overrides only the eight pre-0.3.4 methods, so if a future
    // generation turns one of the defaults into a required method - or into a
    // delegation to a method this Editor does not have - this test stops compiling and
    // the seam has moved, before any window exists to lie about it.
    fn assert_impl<T: EntityInputHandler>() {}
    assert_impl::<editor::Editor>();
}

// ---------------------------------------------------------------------------
// 2. What the platform is told about length, in both unit systems
// ---------------------------------------------------------------------------

#[test]
fn the_ime_line_measures_21_utf8_bytes_and_10_utf16_units() {
    let state = TextState::new(LINE.to_string());

    // The numbers every conversion in editor.rs has to agree with. A length answered
    // in the wrong column here is how a composition lands in the middle of a word.
    let bytes = state.content.len();
    let units = editor::offset_to_utf16(&state.content, bytes);
    println!("LINE: {LINE:?}");
    println!("utf8 bytes = {bytes}, utf16 units = {units}");

    assert_eq!(bytes, 21, "UTF-8 bytes of the IME line");
    assert_eq!(units, 10, "UTF-16 code units of the same line");
    assert_eq!(LINE.graphemes(true).count(), 8, "grapheme clusters");

    // A platform naming the boundary between the emoji and the space speaks in units.
    assert_eq!(
        editor::offset_from_utf16(&state.content, 4),
        9,
        "unit 4 is the space: UTF-16 4 -> byte 9"
    );

    // A unit that lands INSIDE a grapheme is legal UTF-16 but splits the cluster. The
    // plain conversion returns the byte offset after the base character (editor.rs
    // doc at :62-66): between U+2611 and its variation selector here.
    let mid_grapheme = editor::offset_from_utf16(&state.content, 3);
    println!("unit 3 (inside the variation-selector emoji) -> byte {mid_grapheme}");
    assert_eq!(mid_grapheme, 6, "conservative: after the base character");

    // The marked-range report round-trips through UTF-16: compose over 本 (unit 6,
    // bytes 13..16) and the platform reads the mark back in ITS units.
    let mut state = TextState::new(LINE.to_string());
    state.replace_and_mark(Some(6..7), "好", None);
    assert_eq!(state.marked_range, Some(13..16), "stored in bytes");
    assert_eq!(
        state.marked_text_range(),
        Some(6..7),
        "reported to the platform in UTF-16 units"
    );
    assert_eq!(state.content, "e\u{301}\u{2611}\u{FE0F} 日好語 x");
}

#[test]
fn a_unit_inside_a_surrogate_pair_never_splits_the_emoji_on_commit() {
    // The D52 killer in one line: unit 2 of "a😀b" is the LOW SURROGATE.
    let state = TextState::new("a😀b".to_string());
    let floor = editor::offset_from_utf16_floor(&state.content, 2);
    let commit_at = editor::offset_from_utf16(&state.content, 2);
    println!("unit 2 of a-emoji-b: floor -> byte {floor}, commit conversion -> byte {commit_at}");

    assert_eq!(floor, 1, "floor puts the caret ON the emoji");
    assert_eq!(commit_at, 5, "the commit conversion lands AFTER the emoji");

    // A platform that commits at the mid-surrogate unit therefore inserts after the
    // pictograph, never inside its UTF-8 sequence and never as a broken pair.
    let mut state = TextState::new("a😀b".to_string());
    state.replace(Some(2..2), "X");
    assert_eq!(state.content, "a😀Xb");
}

// ---------------------------------------------------------------------------
// 3. The motion doors and the mark: the clearing half, the inverse, the red
// ---------------------------------------------------------------------------

#[test]
fn every_motion_door_clears_an_open_composition() {
    // move_to is the caret-move and click door (editor.rs:401-419: caret_to commits),
    // select_to the drag, select_range the double click - and unmark_text and
    // replace(None, ...) are the platform's own cancel (:736) and commit (:744), the
    // latter being the door Editor::paste funnels into (editor.rs:1300-1305).
    let doors = [
        "move_to",
        "select_to",
        "select_range",
        "select_all",
        "unmark_text",
        "replace",
    ];
    for door in doors {
        let mut state = TextState::new(String::new());
        state.replace_and_mark(None, "nihao", None);
        assert_eq!(
            state.marked_range,
            Some(0..5),
            "{door}: marked while composing"
        );
        match door {
            "move_to" => state.move_to(2),
            "select_to" => state.select_to(1),
            "select_range" => state.select_range(0..2),
            // The sixth door, added when ac6fc37b went red: a table that lists it is how a
            // SEVENTH gets noticed. See the invariant note at editor.rs:268.
            "select_all" => state.select_all(),
            "unmark_text" => state.unmark_text(),
            _ => state.replace(None, "P"),
        }
        assert_eq!(state.marked_range, None, "{door}: the mark goes");
        let expected = if door == "replace" { "P" } else { "nihao" };
        assert_eq!(
            state.content, expected,
            "{door}: committing is not deleting"
        );
    }
}

#[test]
fn a_pure_composition_update_does_not_clear_the_mark() {
    // THE INVERSE HALF. A door test alone would pass on an implementation that clears
    // the mark on every event - an editor that feels like working IME from the outside
    // while the candidate window can never attach to anything.
    let mut state = TextState::new(String::new());
    state.replace_and_mark(None, "ni", None);
    assert_eq!(state.marked_range, Some(0..2));
    state.replace_and_mark(None, "nihao", None);
    assert_eq!(
        state.marked_range,
        Some(0..5),
        "the second composition update REPLACES the mark, it does not clear it"
    );
    assert_eq!(state.content, "nihao", "still composing, still marked");

    // The deliberate exception, named at editor.rs:503-505: Escape collapses the
    // selection and leaves the IME session alone.
    state.select_range(0..5);
    state.replace_and_mark(None, "nihao", None);
    state.clear_selection();
    assert_eq!(
        state.marked_range,
        Some(0..5),
        "cancelling a selection is not cancelling a composition"
    );
}

#[test]
fn select_all_commits_an_open_composition_like_every_other_motion_door() {
    // CLOSED (this commit). Every other motion door cleared the mark
    // (caret_to :410, select_to :437, select_range :494) and the flush-gate doc
    // (editor.rs:936-941) states the invariant "every motion path clears the mark, so
    // this can only be true while the user is actually mid-IME". select_all
    // (editor.rs:498-501) does not, and the Ctrl+A action (:1205-1208) calls only
    // select_all - so with an open composition, Ctrl+A leaves is_composing() true
    // (D51 then refuses to flush a buffer the user has visibly moved on from) and the
    // next plain keystroke, whose replacement_range prefers the mark over the
    // selection (editor.rs:295-304), replaces the stale composition text instead of
    // the selection. The in-file S5 test (any_motion_commits_an_open_composition,
    // editor.rs:2461) covers move_to, select_to and select_range but not this door.
    let mut state = composing_state("日本語 test", "nihao");
    assert_eq!(state.marked_range, Some(14..19), "composing at the caret");
    state.select_all();
    assert_eq!(
        state.marked_range, None,
        "Ctrl+A is a motion: the composition commits, the bytes stay"
    );
    assert_eq!(
        state.content, "日本語 testnihao",
        "the committed bytes are not deleted"
    );
}

// ---------------------------------------------------------------------------
// 4. The commit order: land at the UTF-16 offset the platform named
// ---------------------------------------------------------------------------

#[test]
fn a_committed_insert_lands_at_the_utf16_offset_the_platform_named() {
    // "日本語 test": byte 9 / unit 3 is the caret after 語. The composition inserts the
    // raw keystrokes there and marks them; the commit replaces the marked range with
    // the chosen characters and parks the caret after them.
    let mut state = TextState::new("日本語 test".to_string());
    state.move_to(9);
    assert_eq!(state.selected_range, 9..9, "caret after 語, in bytes");

    state.replace_and_mark(None, "nihao", Some(0..5));
    println!("composing: content = {:?}", state.content);
    println!(
        "  marked  bytes {:?} = units {:?}",
        state.marked_range,
        state.marked_text_range()
    );
    println!(
        "  selected bytes {:?} = units {:?}",
        state.selected_range,
        state.selected_text_range().range
    );
    assert_eq!(state.content, "日本語nihao test");
    assert_eq!(state.marked_range, Some(9..14));
    assert_eq!(
        state.marked_text_range(),
        Some(3..8),
        "the mark in platform units"
    );
    assert_eq!(
        state.selected_range,
        9..14,
        "new_selected_range is measured inside the composition"
    );

    // The commit exactly as Windows delivers it: GCS_RESULTSTR arrives as
    // replace_text_in_range(None, ...) (gpui-pre-windows events.rs:744) and the marked
    // range is what gets replaced.
    state.replace(None, "你好");
    let bytes = state.content.len();
    let units = editor::offset_to_utf16(&state.content, bytes);
    let caret = state.cursor_offset();
    let clusters_before_caret = state.content[..caret].graphemes(true).count();
    println!(
        "committed: content = {:?}, bytes = {bytes}, units = {units}",
        state.content
    );
    println!(
        "  caret byte {caret} = unit {} = after cluster #{clusters_before_caret}",
        editor::offset_to_utf16(&state.content, caret)
    );
    assert_eq!(state.content, "日本語你好 test");
    assert_eq!(state.marked_range, None, "a commit clears the mark");
    assert_eq!(caret, 15, "caret after 你好, in bytes");
    assert_eq!(
        editor::offset_to_utf16(&state.content, caret),
        5,
        "the same caret in UTF-16 units"
    );
    assert_eq!(clusters_before_caret, 5, "and in grapheme clusters");

    // The same commit with the platform NAMING the range in UTF-16 units lands
    // identically - replacement_range converts units 3..8 to the same bytes 9..14.
    let mut state = TextState::new("日本語 test".to_string());
    state.move_to(9);
    state.replace_and_mark(None, "nihao", Some(0..5));
    state.replace(Some(3..8), "你好");
    assert_eq!(state.content, "日本語你好 test");
    assert_eq!(state.cursor_offset(), 15);
}

// ---------------------------------------------------------------------------
// 5. The defaults' runtime values - feature-gated, NOT evidence until enabled
// ---------------------------------------------------------------------------

// These drive the trait the way the platform adapter does (gpui-pre input.rs:145-284
// forwards every platform call into a view method taking &mut Window and the view's
// Context). A window cannot exist without the test-support harness, which the bridge
// does not enable; the two manifest lines that switch this on are in the report.
// Nothing in this module is claimed as a proof until
// "cargo test -p notes-bridge-gpui --features test-support --test ime_seam" runs it.
#[cfg(feature = "test-support")]
mod window_driven {
    use super::*;
    use gpui_kit::{ClipboardItem, TestAppContext};

    fn editor_window(cx: &mut TestAppContext) -> gpui_kit::WindowHandle<editor::Editor> {
        cx.add_window({
            let line = LINE.to_string();
            move |_, cx| editor::Editor::with_content(line, cx)
        })
    }

    #[gpui_kit::test]
    fn the_defaults_the_platform_reads(cx: &mut TestAppContext) {
        let window = editor_window(cx);
        window
            .update(cx, |editor, window, cx| {
                let length = EntityInputHandler::text_length_utf16(editor, window, cx);
                let editable = EntityInputHandler::text_input_editable_range(editor, window, cx);
                let accepts = EntityInputHandler::accepts_text_input(editor, window, cx);
                let prefers =
                    EntityInputHandler::prefers_ime_for_printable_keys(editor, window, cx);
                println!("text_length_utf16 = {length:?} (the buffer is 10 units)");
                println!("text_input_editable_range = {editable:?}");
                println!(
                    "accepts_text_input = {accepts}, prefers_ime_for_printable_keys = {prefers}"
                );
                // 0.3.4 pin (input.rs:94-123): the length and editable-range answers
                // are constant None, NOT delegations to our buffer. If a future
                // generation starts answering with a number, this fails - and the
                // number had better be 10, not 21.
                assert_eq!(length, None);
                assert_eq!(editable, None);
                assert!(accepts);
                assert!(
                    prefers,
                    "delegates to accepts_text_input (input.rs:262-265)"
                );
            })
            .unwrap();
    }

    #[gpui_kit::test]
    fn set_selected_text_range_default_is_a_no_op(cx: &mut TestAppContext) {
        let window = editor_window(cx);
        window
            .update(cx, |editor, window, cx| {
                let before = EntityInputHandler::selected_text_range(editor, false, window, cx);
                EntityInputHandler::set_selected_text_range(editor, 0..1, window, cx);
                let after = EntityInputHandler::selected_text_range(editor, false, window, cx);
                println!("selection before {before:?}, after the default set_selected_text_range {after:?}");
                // The default body is empty (input.rs:85-91): the platform cannot move
                // our caret through it. Harmless today because the Windows backend has
                // no caller; a fact to know if the web backend ever drives us.
                assert_eq!(before, after);
            })
            .unwrap();
    }

    #[gpui_kit::test]
    fn paste_default_forwards_to_replace_text_in_range(cx: &mut TestAppContext) {
        let window = editor_window(cx);
        window
            .update(cx, |editor, window, cx| {
                EntityInputHandler::paste(
                    editor,
                    ClipboardItem::new_string("ZZ".to_string()),
                    window,
                    cx,
                );
                // The selection is the caret at the end, so the paste appends.
                assert_eq!(editor.text(), "e\u{301}\u{2611}\u{FE0F} 日本語 xZZ");
            })
            .unwrap();
    }
}

// ---------------------------------------------------------------------------
// 6. The only true proof of the candidate window: a hand on a real IME
// ---------------------------------------------------------------------------

#[test]
#[ignore = "needs a live window and a real CJK IME - run by hand on Windows, never counted as automated evidence"]
fn manual_pinyin_composition_over_the_live_window() {
    let recipe = [
        "MANUAL RECIPE (D52, on the kit build, Chinese Simplified Microsoft Pinyin):",
        "1. cargo run -p notes-bridge-gpui; click into the note field.",
        "2. Type nihao. EXPECT: the inline composition appears AT THE CARET and the",
        "   candidate window opens beside it (not in a screen corner) - that is",
        "   bounds_for_range answering from the cached line frames (editor.rs:1048-1083).",
        "3. Press 1 (or click the candidate 你好). EXPECT: the buffer reads 你好, the",
        "   composition underline is gone, and no copy of the pinyin remains.",
        "4. Type nihao again, then WITHOUT committing click elsewhere in the text.",
        "   EXPECT: the composition commits in place (the click door clears the mark,",
        "   editor.rs:410), the characters stay, and the next keystroke inserts at the",
        "   new caret instead of replacing the committed text.",
        "5. Type nihao a third time and press Ctrl+A before committing. Today the mark",
        "   survives (the red test above); watch whether the next keystroke replaces the",
        "   whole selection or the stale composition - that is the bug reproduced live.",
        "What on screen proves the seam: the candidate window position (step 2) and the",
        "commit landing (step 3) in a line that already contains 你好 or an emoji.",
    ];
    panic!(
        "{}
",
        recipe.join(
            "
"
        )
    );
}
