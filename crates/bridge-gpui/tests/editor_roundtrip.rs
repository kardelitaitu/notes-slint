//! THE BRIDGE LEG of the do-no-harm rule (whitepaper 4.5, AGENTS.md "Do no harm"),
//! proven on the REAL Editor - the view, not a stand-in for it.
//!
//! Why this file could not exist before now: the buffer the wire reads is
//! Editor::text(), Command::Flush carries it verbatim (main.rs, flush_command), and the
//! only way to OWN an Editor is Editor::with_content(content, cx), whose single build
//! asks a GPUI App for a FocusHandle (editor.rs:1405-1429). No App, no Editor - which is
//! why tests/paint_cost.rs and the headless half of tests/ime_seam.rs work on TextState
//! instead, and why editor.rs:3294 says outright that Editor::load "cannot be reached
//! from a headless test". This file reaches it, through the test-support pass-through in
//! Cargo.toml, exactly the way ime_seam.rs's window_driven module does.
//!
//! ARRANGEMENT, copied from ime_seam.rs:50-51 and paint_cost.rs:20-21: the bridge crate
//! has no lib target, so editor.rs is compiled INTO this test crate by path. Nothing
//! under src/ is touched, everything pub(crate) in it is visible here, and the include
//! is self-contained (editor.rs carries no crate:: and no notes_api reference).
//!
//! WHY THE GATE IS THE WHOLE FILE: the #![cfg(feature = "test-support")] below is an
//! INNER attribute on the crate root, so with the feature off this test crate is EMPTY -
//! it cannot start a window, cannot slow the default "cargo test -p
//! notes-bridge-gpui", and cannot be mistaken for evidence that ran. The two allows sit
//! ABOVE the cfg on purpose: inner attributes apply before the cfg prunes the body, and
//! even a pruned crate needs unused_crate_dependencies (this test crate does not use
//! notes_api or raw-window-handle, which only main.rs does).
//!
//! THE THESIS, in one sentence: between the bytes core decoded and the string the bridge
//! hands the port, the editor must be a glass jar. with_content and load store what they
//! are given - not a trimmed copy, not an EOL-normalised copy, not a copy with the
//! trailing line feed eaten - and they move NO counter while doing it, because a load
//! that bumped edits would wake an autosave for text nobody typed.
//!
//! AND THE ONE THING THAT IS NOT A VIOLATION, stated here because every row below insists
//! on byte-identity and so reads as a contradiction of the paste rule: a CRLF file
//! arriving in the buffer WITH its carriage returns is correct. Normalising to LF is the
//! EDIT path's job (editor.rs:2026 runs the clipboard string through replace("\\r\\n",
//! "\\n").replace('\\r', "\\n") on the way in), never the load's, because core keeps LF
//! internally and restores the file's own ending at the SAVE layer: save.rs:116 calls
//! encoding::encode(text, detected), whose contract is the theorem written at
//! encoding.rs:5-8, the theorem `encode(decode(bytes, detect(bytes, cp)),
//! detect(bytes, cp)) == bytes`, and it is there that the detected EOL, the BOM and the
//! trailing newline are applied. So a buffer that normalised would be harmless for a
//! newline and fatal for a BOM; the rule that covers both is "the bridge never edits
//! the bytes it was given".

// Both are properties of the arrangement, not of what is asserted; see the docs above.
#![allow(dead_code)]
#![allow(unused_crate_dependencies)]
#![cfg(feature = "test-support")]

#[path = "../src/editor.rs"]
mod editor;

use editor::{Editor, line_ranges};
use gpui_kit::{EntityInputHandler, TestAppContext};

/// One corpus row: the text core's decoder hands the bridge, the shape it stands for,
/// and how many lines the buffer must cut into. There is no expected field on purpose:
/// the claim is identity, so the expectation IS the input.
struct Row {
    shape: &'static str,
    text: &'static str,
    lines: usize,
}

/// The subset of the fixture corpus that matters for THIS leg. core's own corpus proves
/// the byte layer; these rows prove the byte layer survives the view.
fn corpus() -> Vec<Row> {
    vec![
        // A CRLF document: the carriage returns must still be there after the load,
        // because the file had them. Four lines, the last of them empty: "a\n" IS two
        // lines, which is the trailing-newline half of do-no-harm seen from the other
        // end.
        Row {
            shape: "CRLF block",
            text: "one\r\ntwo\r\nthree\r\n",
            lines: 4,
        },
        // Lone-CR text (classic Mac, and its own Detected EOL in core). No line feed
        // anywhere, so the buffer is ONE line however many CRs it carries: the cut is on
        // LF, and the rule is about bytes, not about the display.
        Row {
            shape: "lone CR",
            text: "a\rb\r",
            lines: 1,
        },
        // Trailing newline PRESENT and ABSENT are two different files. An editor that
        // helpfully ended the note has rewritten one of them.
        Row {
            shape: "trailing newline present",
            text: "line1\nline2\n",
            lines: 3,
        },
        Row {
            shape: "trailing newline absent",
            text: "line1\nline2",
            lines: 2,
        },
        // A BOM that arrived as a CHARACTER. encoding.rs strips the real one into
        // bom_present and re-prepends it on encode, so a U+FEFF that reached the text is
        // text the file contained - and text the buffer has no right to eat.
        Row {
            shape: "BOM char U+FEFF in the text",
            text: "\u{FEFF}title\nbody",
            lines: 2,
        },
        // The IME line from ime_seam.rs:61 - 21 UTF-8 bytes, 10 UTF-16 units, 8 grapheme
        // clusters. The row that catches a UTF-16 round-trip standing in for the bytes,
        // because any of those three numbers could otherwise be the buffer's length.
        Row {
            shape: "combining e + checked box + CJK + variation selector",
            text: "e\u{301}\u{2611}\u{FE0F} \u{65e5}\u{672c}\u{8a9e} x",
            lines: 1,
        },
        // Text DECODED FROM UTF-16: a BOM character, a CJK run and a surrogate pair (one
        // character, two UTF-16 units, four UTF-8 bytes) - with a CRLF inside it, so the
        // row is the encoding seam and the newline seam at once.
        Row {
            shape: "UTF-16-decoded content",
            text: "\u{FEFF}\u{65e5}\u{672c}\u{8a9e} \u{1F600} tail\r\nsecond \u{1F600}\u{1F600} line",
            lines: 2,
        },
        // The empty document: an untitled note and a zero-byte file. The one row where an
        // accidental normalisation shows up as a LENGTH rather than as a lost character.
        Row {
            shape: "empty",
            text: "",
            lines: 1,
        },
    ]
}

/// One window, one editor, built the way main.rs builds it after Event::Loaded.
/// add_window runs the closure with a real Window and the view's Context, so what comes
/// back is the same WindowHandle<Editor> type the bridge holds in its editor slot.
fn window_with(cx: &mut TestAppContext, content: &str) -> gpui_kit::WindowHandle<Editor> {
    let text = content.to_string();
    cx.add_window(move |_, cx| Editor::with_content(text, cx))
}

/// The two things the wire reads off a windowed editor: the buffer and the counter.
/// flush_command hands the first to Command::Flush and stamps it with the second, so
/// this pair IS the bridge's whole text obligation, read through the real view.
fn read_back(
    window: &gpui_kit::WindowHandle<Editor>,
    cx: &mut TestAppContext,
) -> (String, u64, bool) {
    window
        .update(cx, |editor, _window, _cx| {
            (
                editor.text().to_string(),
                editor.edits(),
                editor.is_composing(),
            )
        })
        .expect("the window is alive: this test added it")
}

#[gpui_kit::test]
fn every_corpus_shape_loads_byte_for_byte_and_wakes_no_save(cx: &mut TestAppContext) {
    for row in corpus() {
        let window = window_with(cx, row.text);
        let (buffer, edits, composing) = read_back(&window, cx);

        // THE IDENTITY. Not "equal after normalising", not "equal ignoring the BOM": the
        // exact string, every byte, carriage returns, U+FEFF and trailing LF included.
        assert_eq!(
            buffer, row.text,
            "{}: the buffer mangled the loaded text",
            row.shape
        );
        // A load is not an edit. edits is the counter the debounce watches (main.rs,
        // Wire::note_edit), so this zero is the difference between "opened a note" and
        // "opened a note and saved it". This is the type-level leg - construction stores
        // bytes untouched and moves nothing - taken on the real Editor.
        assert_eq!(
            edits, 0,
            "{}: a load must not move the counter the wire reads",
            row.shape
        );
        // D51: a Flush is refused while a composition is open, so a fresh load must
        // start unmarked - otherwise the first save after opening is withheld silently.
        assert!(!composing, "{}: a loaded buffer is not mid-IME", row.shape);
        // And the line cut agrees with the bytes, including the two trailing-newline
        // rows, which are the same file apart from one byte.
        assert_eq!(
            line_ranges(&buffer).len(),
            row.lines,
            "{}: wrong line count for {:?}",
            row.shape,
            buffer
        );
    }
}

#[gpui_kit::test]
fn loading_a_second_note_leaves_no_trace_of_the_first(cx: &mut TestAppContext) {
    // The S6 path. A rebind - Event::Loaded for another file, or Save As onto a path the
    // user chose - goes through Editor::load, which is with_content's twin: the same
    // TextState::new plus the view reset. Bytes left behind from the old note would be
    // flushed INTO THE NEW FILE, which is the worst shape this rule can fail in.
    let window = window_with(cx, "the first note, long enough to matter\n\n\n");
    let (first, _, _) = read_back(&window, cx);
    assert_eq!(
        line_ranges(&first).len(),
        4,
        "precondition: a multi-line first note"
    );

    for row in corpus() {
        let incoming = row.text.to_string();
        window
            .update(cx, |editor, _window, cx| editor.load(incoming, cx))
            .expect("the window is alive");
        let (buffer, edits, _) = read_back(&window, cx);
        assert_eq!(
            buffer, row.text,
            "{}: load left the old note's bytes behind",
            row.shape
        );
        assert_eq!(
            edits, 0,
            "{}: load replaced the counter, it did not edit",
            row.shape
        );
    }

    // Loading an EMPTY file over a long note is the row worth naming: a buffer that kept
    // any of the old length would write the old text into the new file.
    window
        .update(cx, |editor, _window, cx| editor.load(String::new(), cx))
        .unwrap();
    let (buffer, edits, _) = read_back(&window, cx);
    assert_eq!(
        buffer, "",
        "a load onto a zero-byte file is an empty buffer"
    );
    assert_eq!(edits, 0);
}

#[gpui_kit::test]
fn the_empty_document_is_a_zero_length_buffer_that_still_has_one_line(cx: &mut TestAppContext) {
    let window = window_with(cx, "");
    let (buffer, edits, composing) = read_back(&window, cx);
    assert_eq!(
        buffer, "",
        "an empty note is an empty buffer, not a line feed"
    );
    assert_eq!(
        buffer.len(),
        0,
        "and zero BYTES, so the Flush writes back the file it read"
    );
    assert_eq!(edits, 0, "nothing was typed, so nothing may be saved");
    assert!(!composing);
    assert_eq!(
        line_ranges(&buffer).len(),
        1,
        "one empty line is still a line to draw"
    );
}

#[gpui_kit::test]
fn a_pasted_crlf_block_becomes_real_lines_and_the_buffer_survives_it(cx: &mut TestAppContext) {
    // THE OTHER HALF OF THE THESIS, and the reason the load rows above insist on keeping
    // the carriage returns: normalisation belongs to the edit path, not to the load.
    // Editor::paste (editor.rs:2024-2029) runs the clipboard string through that
    // two-step replace and hands the result to replace_text_in_range. The private action
    // is not reachable from a test crate, and its own guarantee is already pinned
    // in-file at editor.rs:3095 (a_pasted_crlf_block_becomes_real_lines). What IS
    // reachable, and what this drives, is the door the platform calls - the
    // EntityInputHandler method - with the payload already through that same expression,
    // so the claim is made on a windowed Editor and read back with text().
    let window = window_with(cx, "head\r\ntail");
    let pasted = "one\r\ntwo\rthree"
        .replace("\r\n", "\n")
        .replace('\r', "\n");

    window
        .update(cx, |editor, window, cx| {
            // range None over a caret parked at the end (TextState::new puts it there),
            // so the replace appends - which is also the only way this row can show the
            // LOADED CRLF and the PASTED LF in one buffer.
            EntityInputHandler::replace_text_in_range(editor, None, &pasted, window, cx);
        })
        .expect("the window is alive");

    let (buffer, edits, composing) = read_back(&window, cx);
    assert_eq!(
        buffer, "head\r\ntailone\ntwo\nthree",
        "the paste keeps its line feeds, the load keeps its carriage returns"
    );
    assert_eq!(
        line_ranges(&buffer).len(),
        4,
        "head / tail+one / two / three - real lines"
    );
    assert_eq!(
        edits, 1,
        "one mutation, one step: the debounce now has something to save"
    );
    assert!(
        !composing,
        "an ordinary replace leaves no composition behind"
    );
    assert_eq!(
        buffer.matches('\n').count(),
        3,
        "every pasted line feed arrived exactly once"
    );
    assert!(
        !buffer.contains("\r\r"),
        "and no carriage return was doubled: {buffer:?}"
    );
}
