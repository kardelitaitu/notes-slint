//! The paint cost of the shape rebuild, measured with NO WINDOW and NO GPU.
//!
//! Same arrangement as `ime_seam.rs`: the bridge crate has no lib target, so `editor.rs`
//! is compiled into THIS test crate by path and everything `pub(crate)` in it is
//! reachable. What that buys is the whole point of the file: the loop that decides the
//! paint cost is a free function over a payload type, so it can be run, counted and timed
//! in CI, where a window cannot be - and an instrument nobody can read in CI is a
//! decoration, whatever it prints on a screen.
//!
//! The load-bearing assertion is NOT the microseconds, which belong to the machine that
//! ran them. It is `lines_examined(200) == lines_examined(2000)`: a rebuild that walks the
//! whole buffer is O(note) whatever the wall clock says. RED on the version that threw the
//! pool away and moved every shaped line back into it once per frame (200 vs 2,000, and
//! 11,631 us a frame at 2,000 lines with ZERO lines re-shaped, which is the proof that
//! shaping was never the cost).
#![allow(dead_code)]
#![allow(unexpected_cfgs)]
#![allow(unused_crate_dependencies)]

#[path = "../src/editor.rs"]
mod editor;

use editor::{LineGeometry, PaintStats, ShapeCache, line_ranges, rebuild_shapes, retained_window};
use gpui_kit::{ShapedLine, px};
use std::hint::black_box;
use std::ops::Range;
use std::time::Instant;

/// A stand-in for `ShapedLine` with the SAME byte size, which is the only property that
/// matters for what is measured here: the cost being examined is moving whole shaped
/// lines, and a `ShapedLine` is big because of gpui's inline
/// `SmallVec<[DecorationRun; 32]>`. The size is pinned to the real type by a test rather
/// than to a number copied out of a transcript, so a kit generation that changes
/// `ShapedLine` changes this file with it.
struct FakeLine {
    _pad: [u8; 2984],
}

const fn fake_line() -> FakeLine {
    FakeLine { _pad: [0; 2984] }
}

#[test]
fn the_stand_in_is_the_thing_it_is_standing_in_for() {
    // If this ever fails, every microsecond reported by this file is a number about the
    // wrong struct. It is a size assertion, not a performance one, so it holds in any
    // profile on any machine.
    assert_eq!(
        std::mem::size_of::<FakeLine>(),
        std::mem::size_of::<ShapedLine>(),
        "FakeLine must be exactly as expensive to move as the line it replaces"
    );
}

/// A note with `lines` lines of plausible text, distinct per line: a note whose lines
/// were all identical would let a cache hit on a key that is not the text.
fn buffer(lines: usize) -> String {
    let mut out = String::new();
    for index in 0..lines {
        if index > 0 {
            out.push('\n');
        }
        out.push_str("the quick brown fox jumps over the lazy dog ");
        out.push_str(&index.to_string());
    }
    out
}

/// Rows a 700-pixel viewport at a 24-pixel line height can show - the working set of a
/// frame, and the number the rebuild must not exceed however long the note is.
const VIEWPORT_ROWS: usize = 29;

/// Drive `frames` rebuilds of the SAME cache over the SAME buffer, with the viewport on
/// the first rows. Returns the last frame's counters and the mean microseconds a frame
/// spent in the loop. `edits` is what the caller claims the mutation counter is; the
/// first call always rebuilds because the cache starts empty.
fn drive(content: &str, edits: u64, frames: usize) -> (PaintStats, PaintStats, u128) {
    let mut cache: ShapeCache<FakeLine> = ShapeCache::default();
    cache.align(content, edits);
    let lines = cache.lines();
    assert_eq!(
        lines,
        line_ranges(content).len(),
        "the cache must cut the same lines"
    );
    let window = 0..lines.min(VIEWPORT_ROWS);
    let retention = retained_window(&cache, &window, content.len(), 0, &None, &(0..0));
    let mut stats = PaintStats::default();
    let mut warm = PaintStats::default();
    let mut us = 0u128;
    for turn in 0..frames {
        if turn == 1 {
            warm = stats;
        }
        let mut frame = PaintStats::default();
        let began = Instant::now();
        rebuild_shapes(
            black_box(content),
            None,
            &retention,
            &mut cache,
            &mut frame,
            &mut |_text: &str, _mark| fake_line(),
        );
        us += began.elapsed().as_micros();
        stats = frame;
    }
    (warm, stats, us / frames.max(1) as u128)
}

/// THE LOAD-BEARING ASSERTION: a steady frame - the buffer has not moved, the cache is
/// warm - examines a number of lines that does not depend on the size of the note, and
/// rebuilds no pool at all.
#[test]
fn a_steady_frame_examines_the_same_number_of_lines_whatever_the_buffer() {
    let (small_warm, small, _) = drive(&buffer(200), 0, 3);
    let (large_warm, large, _) = drive(&buffer(2000), 0, 3);
    // THE CLAIM, in both directions. The settled frame - the buffer unmoved, the view
    // unmoved - walks nothing at all, at any size. The frame that does walk, walks the
    // screen and not the note.
    assert_eq!(
        small.lines_examined, large.lines_examined,
        "a settled frame must not examine more lines because the note got longer"
    );
    assert_eq!(small.lines_examined, 0, "a settled frame walks NOTHING");
    assert_eq!(
        small_warm.lines_examined, large_warm.lines_examined,
        "the first frame must not walk more lines because the note got longer"
    );
    assert_eq!(
        large_warm.lines_examined, VIEWPORT_ROWS,
        "and it is the viewport"
    );
    assert_eq!(small.lines_shaped, 0, "a warm cache shapes nothing");
    assert_eq!(
        small.pool_rebuilds + large.pool_rebuilds,
        0,
        "the pool is reused, never rebuilt"
    );
}

/// The case the index alone cannot answer: Enter at the top of a note moves every line
/// below it down one slot without changing a byte of any of them. The rebuild must still
/// cost the viewport, and must re-shape the ONE line that is new.
#[test]
fn a_newline_at_the_top_shifts_the_note_without_re_shaping_it() {
    let before = buffer(2000);
    let mut cache: ShapeCache<FakeLine> = ShapeCache::default();
    cache.align(&before, 0);
    let window = 0..cache.lines().min(VIEWPORT_ROWS);
    let retention = retained_window(&cache, &window, before.len(), 0, &None, &(0..0));
    let mut warm = PaintStats::default();
    rebuild_shapes(
        &before,
        None,
        &retention,
        &mut cache,
        &mut warm,
        &mut |_t, _m| fake_line(),
    );
    // Press Enter: the buffer gains a line at the top and the mutation counter moves.
    let after = "\n".to_string() + &before;
    cache.align(&after, 1);
    let window = 0..cache.lines().min(VIEWPORT_ROWS);
    let retention = retained_window(&cache, &window, after.len(), 0, &None, &(0..0));
    let mut shifted = PaintStats::default();
    let began = Instant::now();
    rebuild_shapes(
        &after,
        None,
        &retention,
        &mut cache,
        &mut shifted,
        &mut |_t, _m| fake_line(),
    );
    let us = began.elapsed().as_micros();
    assert_eq!(shifted.lines_examined, VIEWPORT_ROWS, "viewport, not note");
    assert_eq!(
        shifted.lines_shaped, 1,
        "only the line that did not exist is shaped; the shift is answered by the pool"
    );
    assert_eq!(shifted.pool_rebuilds, 0);
    println!("SHIFT one Enter at the top of 2,000 lines: {us} us, {shifted:?}",);
}

/// The wall clock, printed rather than asserted: the machine this runs on is not the
/// machine that ships, and a threshold on a debug build is a flake waiting to happen.
/// What the numbers ARE for is the before/after table in the commit report - same
/// profile, same harness, same stand-in, only the rebuild different.
#[test]
fn rebuild_cost_per_frame_by_buffer_size() {
    for lines in [200usize, 2000, 20000] {
        let content = buffer(lines);
        let (_, stats, us) = drive(&content, 0, 3);
        println!(
            "PAINT lines={lines:>5} us_per_frame={us:>8} examined={:>6} shaped={:>6} pool_rebuilds={}",
            stats.lines_examined, stats.lines_shaped, stats.pool_rebuilds,
        );
        let shifted = drive(&("\n".to_string() + &content), 1, 3);
        println!(
            "PAINT lines={lines:>5} shifted us_per_frame={:>8} examined={:>6} shaped={:>6} pool_rebuilds={}",
            shifted.2, shifted.1.lines_examined, shifted.1.lines_shaped, shifted.1.pool_rebuilds,
        );
    }
}

// ---------------------------------------------------------------------------
// THE BOUNDARY OF THE RETENTION SET
//
// The interior case cannot fail: a line inside the viewport is retained by the viewport,
// so a test that only covers the interior is decoration. Everything worth asserting here
// is on or outside the edge. And the consequence being tested is the real one: a line
// with no slot makes `Editor::bounds_for_range` answer `None`, and gpui-pre propagates
// that with `?` in `retrieve_caret_position`
// (gpui-pre-windows-0.3.4/src/events.rs:647-655), so `handle_ime_position` (:658-662)
// skips `update_ime_position` entirely. Nothing is moved to the origin - the toolkit
// declines to move - and the composition and candidate windows FREEZE at the last
// position they were given, over text the user has since scrolled past. Silent wrongness
// is why this survived a rewrite and a review; an origin jump would have been reported.
// ---------------------------------------------------------------------------

/// A 700-pixel box, 24-pixel rows, scrolled so the first row it shows is `first`.
fn scrolled(lines: usize, first: usize) -> LineGeometry {
    LineGeometry {
        top: px(0.0),
        left: px(0.0),
        right: px(700.0),
        row: px(24.0),
        scroll_y: px((first.min(lines.saturating_sub(1)) * 24) as f32),
        viewport_h: px(700.0),
    }
}

/// A warm cache for a note, plus the geometry showing rows `first..first + viewport`.
fn warm(content: &str, first: usize) -> (ShapeCache<FakeLine>, LineGeometry) {
    let mut cache: ShapeCache<FakeLine> = ShapeCache::default();
    cache.align(content, 0);
    let geom = scrolled(cache.lines(), first);
    let visible = geom.visible_lines(cache.lines());
    let retention = retained_window(&cache, &visible, content.len(), first, &None, &(0..0));
    let mut stats = PaintStats::default();
    rebuild_shapes(
        content,
        None,
        &retention,
        &mut cache,
        &mut stats,
        &mut |_t, _m| fake_line(),
    );
    (cache, geom)
}

/// The precondition `Editor::bounds_for_range` needs before it can hand the OS a rect:
/// the line the range sits on has a slot, and the geometry can place it. Asserting THIS
/// is asserting "the IME gets SOME bounds" - the rest of that method is arithmetic over
/// the slot, and it returns `None` at exactly the point this fails.
fn answerable(cache: &ShapeCache<FakeLine>, geom: &LineGeometry, index: usize) -> bool {
    cache.slot(index).is_some() && f32::from(geom.bounds_of(index).size.height) > 0.0
}

/// Compose on a line, then wheel that line off the top of the screen. The mark is still
/// live and the OS is still asking, so retention - not the viewport - is the only thing
/// that can answer for it.
#[test]
fn a_mark_outside_the_viewport_still_gets_some_bounds() {
    let content = buffer(2000);
    let (mut cache, geom) = warm(&content, 900);
    let mark_line = 3usize;
    let bytes = cache.bytes_of(&content, mark_line).unwrap();
    let marked = Some(bytes.start + 1..bytes.end.max(bytes.start + 2));
    let visible = geom.visible_lines(cache.lines());
    assert!(!visible.contains(&mark_line), "off screen by construction");
    let retention = retained_window(&cache, &visible, content.len(), mark_line, &marked, &(0..0));
    let mut stats = PaintStats::default();
    rebuild_shapes(
        &content,
        marked.clone(),
        &retention,
        &mut cache,
        &mut stats,
        &mut |_t, _m| fake_line(),
    );
    assert!(
        answerable(&cache, &geom, mark_line),
        "a live composition scrolled off screen must still have a rect to be answered with"
    );
    assert!(
        stats.lines_examined <= VIEWPORT_ROWS + 4,
        "bounded: {stats:?}"
    );
    // The rect is the LINE, not a default: row 3 under a 900-row scroll sits 897 rows
    // above the top of the box.
    assert_eq!(
        f32::from(geom.bounds_of(mark_line).top()),
        3.0 * 24.0 - 900.0 * 24.0
    );
}

/// The same for a selection straddling the window edge - the case the shipped rule missed
/// entirely, because the selection was not in the set at all.
#[test]
fn a_selection_straddling_the_window_edge_keeps_both_ends_answerable() {
    let content = buffer(2000);
    let (mut cache, geom) = warm(&content, 900);
    let near = cache.bytes_of(&content, 910).unwrap();
    let far = cache.bytes_of(&content, 1500).unwrap();
    let selection = near.start..far.end;
    let visible = geom.visible_lines(cache.lines());
    assert!(!visible.contains(&1500), "the far end is off screen");
    let retention = retained_window(&cache, &visible, content.len(), 910, &None, &selection);
    let mut stats = PaintStats::default();
    rebuild_shapes(
        &content,
        None,
        &retention,
        &mut cache,
        &mut stats,
        &mut |_t, _m| fake_line(),
    );
    assert!(
        answerable(&cache, &geom, 1500),
        "the far end of a straddling selection must be answerable; a missing slot is a None rect and a candidate window frozen where it last stood"
    );
    assert!(
        stats.lines_examined <= VIEWPORT_ROWS + 4,
        "bounded: {stats:?}"
    );
}

/// THE NUMBER THAT MUST NOT MOVE. Worst-case anchors: caret at the top, a mark at the
/// top, and a selection over the WHOLE note - select-all, then scroll to the bottom.
///
/// DO NOT SIMPLIFY THIS FIXTURE BACK. The version of the flatness test that shipped with
/// 8404b566 built its own `0..29` window and handed it straight to `rebuild_shapes`, so it
/// measured the LOOP while claiming to measure the design - it passed at 200 and 2,000
/// because a window the test supplied is flat by construction, whatever the rule that would
/// have produced one. This one drives the production predicate, `retained_window`, which is
/// the only thing in the file that decides what a frame looks at. `foil_the_rule_as_shipped
/// at_8404b566` below is the proof the assertion bites: same fixture, old rule, 20,000
/// lines examined at 20,000 lines.
#[test]
fn retention_stays_flat_with_the_worst_case_anchors_at_every_size() {
    let mut seen = Vec::new();
    for lines in [200usize, 2000, 20000] {
        let content = buffer(lines);
        let (mut cache, geom) = warm(&content, lines - 30);
        let head = cache.bytes_of(&content, 0).unwrap();
        let marked = Some(head.start..head.end.max(head.start + 1));
        let selection = 0..content.len();
        let visible = geom.visible_lines(cache.lines());
        let retention = retained_window(&cache, &visible, content.len(), 0, &marked, &selection);
        let mut stats = PaintStats::default();
        let began = Instant::now();
        rebuild_shapes(
            &content,
            marked,
            &retention,
            &mut cache,
            &mut stats,
            &mut |_t, _m| fake_line(),
        );
        // PRINT FIRST: this is the measurement the whole change is judged on, and it is
        // worth reading even on the frame that fails it.
        println!(
            "FLAT lines={lines:>5} worst-case us={:>7} examined={:>5} rebuilt={:>5} shaped={:>5}",
            began.elapsed().as_micros(),
            stats.lines_examined,
            stats.window_rebuilds,
            stats.lines_shaped
        );
        assert!(
            answerable(&cache, &geom, 0),
            "{lines} lines, line 0 must answer"
        );
        assert!(
            stats.lines_examined <= VIEWPORT_ROWS + 4,
            "{lines} lines examined {} - retention widened past the screen",
            stats.lines_examined
        );
        seen.push(stats.lines_examined);
    }
    assert_eq!(seen, vec![seen[0]; 3], "flat at every size: {seen:?}");
}

/// CONVERGENCE. A frame of an unchanged view examines NOTHING; a mark on an offscreen
/// line must not make the predicate oscillate.
#[test]
fn a_settled_frame_examines_nothing_and_an_offscreen_mark_does_not_oscillate() {
    let content = buffer(2000);
    let (mut cache, geom) = warm(&content, 900);
    let bytes = cache.bytes_of(&content, 3).unwrap();
    let marked = Some(bytes.start + 1..bytes.end.max(bytes.start + 2));
    let visible = geom.visible_lines(cache.lines());
    let retention = retained_window(&cache, &visible, content.len(), 3, &marked, &(0..0));
    let mut first = PaintStats::default();
    rebuild_shapes(
        &content,
        marked.clone(),
        &retention,
        &mut cache,
        &mut first,
        &mut |_t, _m| fake_line(),
    );
    let mut second = PaintStats::default();
    rebuild_shapes(
        &content,
        marked.clone(),
        &retention,
        &mut cache,
        &mut second,
        &mut |_t, _m| fake_line(),
    );
    assert_eq!(
        second.lines_examined, 0,
        "a settled frame walks nothing: {second:?}"
    );
    assert_eq!(second.window_rebuilds, 0);
    let mut trail = Vec::new();
    for step in 0..8 {
        let geom = scrolled(cache.lines(), 900 + step % 2);
        let visible = geom.visible_lines(cache.lines());
        let retention = retained_window(&cache, &visible, content.len(), 3, &marked, &(0..0));
        let mut frame = PaintStats::default();
        rebuild_shapes(
            &content,
            marked.clone(),
            &retention,
            &mut cache,
            &mut frame,
            &mut |_t, _m| fake_line(),
        );
        trail.push((frame.lines_examined, frame.window_rebuilds));
    }
    println!("CONVERGE 8 frames of a one-row scroll, mark pinned offscreen: {trail:?}");
    assert!(
        trail.iter().all(|(e, _)| *e <= VIEWPORT_ROWS + 4),
        "{trail:?}"
    );
    assert!(
        trail.iter().all(|(_, r)| *r <= 2),
        "a one-row scroll exposes one row; an offscreen pin must not re-walk it: {trail:?}"
    );
}

/// MEASUREMENT, not an assertion: what `TextState::line_index_at` costs now that it is a
/// `partition_point` over the lazy line-start cache in `editor.rs`, and the paint path does
/// not call it. Printed so the number is on the record instead of argued about. The loose
/// ceiling only says "a keystroke is not a second of input" - it is not the claim.
#[test]
fn measure_the_caret_path_that_still_scans_bytes() {
    use editor::TextState;
    use std::hint::black_box;
    for lines in [200usize, 2000, 20000] {
        let content = buffer(lines);
        let mut state = TextState::new(content.clone());
        state.move_to(content.len());
        let calls = 200usize;
        let began = Instant::now();
        for _ in 0..calls {
            black_box(state.line_index_at(black_box(content.len())));
        }
        let scan = began.elapsed().as_micros() as usize / calls;
        let began = Instant::now();
        for _ in 0..calls {
            black_box(state.line_range_at(black_box(content.len())));
        }
        let range = began.elapsed().as_micros() as usize / calls;
        println!(
            "MEASURE lines={lines:>5} line_index_at={scan:>5}us/call line_range_at={range:>6}us/call",
        );
        // TWO bounds, two different amounts of ownership.
        //
        // `line_range_at` (word selection, backspace-to-line-start, the caret's own line) is
        // pinned tight because this file owns it. It used to count the whole prefix AND build
        // the full line index to `nth` it - measured at 138 / 1,415 / 17,456 us per call at
        // 200 / 2,000 / 20,000 lines, i.e. one keystroke eating a whole 60 Hz frame. It now
        // goes through the same line-start index as `line_index_at` - a search, not a scan -
        // and measures 0 us at every size.
        //
        // `line_index_at` is printed rather than pinned: the fix was not this file's - it
        // landed in `editor.rs`, which keeps a lazy `line_starts` cache (526d73e6), and
        // `line_index_at` is a `partition_point` over it, O(log lines), reached once per
        // Up/Down through `vertical_target`. The ceiling below stays a tripwire against
        // "seconds per arrow key", not a claim about a constant: microseconds per call are
        // wall-clock and machine-dependent, so the assert pins only the order of magnitude
        // and the printed number keeps the measurement on the record.
        assert!(
            range < 200,
            "line_range_at must be O(line), not O(note): {range}us"
        );
        assert!(scan < 20_000, "a keystroke must not take seconds: {scan}us");
    }
}

/// THE FOIL: 8404b566's retention rule, transcribed. Widen ONE contiguous range to reach
/// the caret line and the line the mark STARTS on, and take nothing at all from the
/// selection. It is kept here, and run here, for one reason: an instrument that passes on
/// the broken version is decoration, and until this function exists the flatness test
/// cannot be shown to bite. It is a transcription, not the shipped code - the shipped code
/// is `git show 8404b566:crates/bridge-gpui/src/editor.rs`, and the numbers below are the
/// numbers that rule produces at every size the new rule is quoted at.
fn foil_the_rule_as_shipped_at_8404b566<Line>(
    cache: &ShapeCache<Line>,
    visible: &Range<usize>,
    caret_line: usize,
    marked: &Option<Range<usize>>,
    selection: &Range<usize>,
) -> Range<usize> {
    let mut first = visible.start;
    let mut last = visible.end;
    for edge in [
        Some(caret_line),
        marked.as_ref().and_then(|m| cache.line_index_at(m.start)),
    ]
    .into_iter()
    .flatten()
    {
        first = first.min(edge);
        last = last.max(edge + 1);
    }
    let _ = selection;
    first..last
}

/// THE ASSERTION, BITING. Feed the foil the same worst-case anchors the real test uses and
/// count what it would have walked. It is not close at any size, and it is not a constant:
/// it is the whole note, which is the exact claim `lines_examined` was put in the file to
/// watch. The new rule walks 31 at all three sizes (see the test above); the old one walks
/// 200 / 2,000 / 20,000.
#[test]
fn the_widening_rule_fails_the_flatness_assertion_at_every_size() {
    let mut old = Vec::new();
    let mut new = Vec::new();
    for lines in [200usize, 2000, 20000] {
        let content = buffer(lines);
        let (cache, geom) = warm(&content, lines - 30);
        let head = cache.bytes_of(&content, 0).unwrap();
        let marked = Some(head.start..head.end.max(head.start + 1));
        let selection = 0..content.len();
        let visible = geom.visible_lines(cache.lines());
        let widened =
            foil_the_rule_as_shipped_at_8404b566(&cache, &visible, 0, &marked, &selection);
        let pinned = retained_window(&cache, &visible, content.len(), 0, &marked, &selection);
        old.push(widened.len());
        new.push(pinned.rows(cache.lines()).len());
        // The far end of the selection is not in the old set at all, which is the OTHER
        // failure: fewer lines walked than the new rule only when the caret happens to be
        // nearby, and no answer at all when it is not.
        assert!(
            !widened.contains(&(lines - 1)) || widened.len() == lines,
            "{lines}: the foil covered the far end by widening to the whole note"
        );
    }
    println!(
        "FOIL old-rule walked={old:?} new-rule walked={new:?} (cap {})",
        VIEWPORT_ROWS + 4
    );
    assert_eq!(new, vec![31; 3], "the new rule is flat at 31");
    assert_eq!(old, vec![200, 2000, 20000], "the old rule IS the buffer");
    assert!(
        old.iter().enumerate().all(|(i, n)| *n > new[i] * 6),
        "the assertion must bite at every size: {old:?} vs {new:?}"
    );
}
