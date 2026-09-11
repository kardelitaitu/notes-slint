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

use editor::{PaintStats, ShapeCache, line_ranges, rebuild_shapes};
use gpui_kit::ShapedLine;
use std::hint::black_box;
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
fn drive(content: &str, edits: u64, frames: usize) -> (PaintStats, u128) {
    let mut cache: ShapeCache<FakeLine> = ShapeCache::default();
    cache.align(content, edits);
    let lines = cache.lines();
    assert_eq!(
        lines,
        line_ranges(content).len(),
        "the cache must cut the same lines"
    );
    let window = 0..lines.min(VIEWPORT_ROWS);
    let mut stats = PaintStats::default();
    let mut us = 0u128;
    for _ in 0..frames {
        let mut frame = PaintStats::default();
        let began = Instant::now();
        rebuild_shapes(
            black_box(content),
            None,
            window.clone(),
            &mut cache,
            &mut frame,
            &mut |_text: &str, _mark| fake_line(),
        );
        us += began.elapsed().as_micros();
        stats = frame;
    }
    (stats, us / frames.max(1) as u128)
}

/// THE LOAD-BEARING ASSERTION: a steady frame - the buffer has not moved, the cache is
/// warm - examines a number of lines that does not depend on the size of the note, and
/// rebuilds no pool at all.
#[test]
fn a_steady_frame_examines_the_same_number_of_lines_whatever_the_buffer() {
    let small = drive(&buffer(200), 0, 3).0;
    let large = drive(&buffer(2000), 0, 3).0;
    assert_eq!(
        small.lines_examined, large.lines_examined,
        "a steady frame must not examine more lines because the note got longer"
    );
    assert_eq!(
        small.lines_examined, VIEWPORT_ROWS,
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
    let mut warm = PaintStats::default();
    rebuild_shapes(
        &before,
        None,
        window.clone(),
        &mut cache,
        &mut warm,
        &mut |_t, _m| fake_line(),
    );
    // Press Enter: the buffer gains a line at the top and the mutation counter moves.
    let after = "\n".to_string() + &before;
    cache.align(&after, 1);
    let window = 0..cache.lines().min(VIEWPORT_ROWS);
    let mut shifted = PaintStats::default();
    let began = Instant::now();
    rebuild_shapes(
        &after,
        None,
        window,
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
        let (stats, us) = drive(&content, 0, 3);
        println!(
            "PAINT lines={lines:>5} us_per_frame={us:>8} examined={:>6} shaped={:>6} pool_rebuilds={}",
            stats.lines_examined, stats.lines_shaped, stats.pool_rebuilds,
        );
        let shifted = drive(&("\n".to_string() + &content), 1, 3);
        println!(
            "PAINT lines={lines:>5} shifted us_per_frame={:>8} examined={:>6} shaped={:>6} pool_rebuilds={}",
            shifted.1, shifted.0.lines_examined, shifted.0.lines_shaped, shifted.0.pool_rebuilds,
        );
    }
}
