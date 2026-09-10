//! Window geometry primitives that persist across launches.
//!
//! Rect is a window rect in FRAME pixels — the Win32 GetWindowRect space. It
//! is never client space: the chrome deltas are asymmetric and DPI-dependent
//! (measured 8/19/8/20 px at one configuration), so persisting client-space
//! rects drifts ~8 px left and ~19 px up on every launch (whitepaper §12.2).
//! Frame-to-client conversion happens at window-open time in the bridge, not
//! here.

/// Persisted window rect in FRAME pixels (Win32 GetWindowRect space).
///
/// NEVER client space: chrome deltas are asymmetric and DPI-dependent
/// (8/19/8/20 px at one config), so client-space persistence drifts ~8 px
/// left + 19 px up per launch. Conversion happens at window-open time in the
/// bridge, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    /// Builds a rect from a frame-space origin and size.
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Rect { x, y, w, h }
    }

    /// Exclusive right edge. i64 so the full i32/u32 input ranges cannot
    /// overflow.
    fn right(self) -> i64 {
        i64::from(self.x) + i64::from(self.w)
    }

    /// Exclusive bottom edge, same overflow reasoning as Rect::right.
    fn bottom(self) -> i64 {
        i64::from(self.y) + i64::from(self.h)
    }

    /// True when either dimension is zero.
    pub fn is_empty(self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// Area in frame pixels. u64 so u32::MAX * u32::MAX cannot overflow.
    pub fn area(self) -> u64 {
        u64::from(self.w) * u64::from(self.h)
    }

    /// True when the two rects share at least one pixel. Edges are exclusive
    /// (x + w is not part of the rect), so rects that merely touch do not
    /// intersect, and an empty rect intersects nothing.
    pub fn intersects(self, other: Rect) -> bool {
        i64::from(self.x) < other.right()
            && i64::from(other.x) < self.right()
            && i64::from(self.y) < other.bottom()
            && i64::from(other.y) < self.bottom()
    }

    /// The shared region, or None when the rects do not intersect.
    pub fn intersection(self, other: Rect) -> Option<Rect> {
        if !self.intersects(other) {
            return None;
        }
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        // The intersects() guard makes both extents strictly positive, and
        // each is bounded by the inputs, so these casts are lossless.
        Some(Rect {
            x,
            y,
            w: (right - i64::from(x)) as u32,
            h: (bottom - i64::from(y)) as u32,
        })
    }

    /// Restore-safe clamping of a saved rect into work_area, which may have
    /// changed since the rect was saved (monitor unplugged, resolution or
    /// taskbar changed). The rect never changes size — only its position,
    /// and only when it is not already visible enough:
    ///
    /// * Per axis, the position is kept when at least min_visible pixels
    ///   (and at least one pixel) of the rect lie inside the work area. This
    ///   makes the function idempotent: clamping an already placed rect is a
    ///   no-op, so a visible window never jumps between launches.
    /// * Otherwise the rect is pulled fully inside the work area from its
    ///   nearest side; a rect larger than the work area is pinned to the
    ///   work-area origin (its top-left region is the visible part).
    /// * min_visible is capped at the rect and work-area extents, so it can
    ///   never move a fully visible rect, and the result is never empty for
    ///   a non-empty input.
    ///
    /// Callers: pass the work area of the monitor the rect belongs to (a
    /// nearest-monitor lookup by overlap), never blindly the primary — a
    /// rect sitting fully on a still-present SECONDARY monitor has zero
    /// overlap with the primary and would be yanked across screens. When no
    /// monitor matches (the saved one is truly gone), the nearest monitor's
    /// work area is the fallback.
    pub fn clamped_to(self, work_area: Rect, min_visible: u32) -> Rect {
        let x = clamp_axis(
            i64::from(self.x),
            u64::from(self.w),
            i64::from(work_area.x),
            u64::from(work_area.w),
            min_visible,
        );
        let y = clamp_axis(
            i64::from(self.y),
            u64::from(self.h),
            i64::from(work_area.y),
            u64::from(work_area.h),
            min_visible,
        );
        Rect {
            x: saturate_pos(x),
            y: saturate_pos(y),
            w: self.w,
            h: self.h,
        }
    }

    /// Scales the rect by factor — for example a DPI change between launches.
    /// Returns None when factor is zero, negative, or not finite.
    ///
    /// Rounding: each coordinate is scaled in f64 and rounded half away from
    /// zero (Rust's f64::round), then saturated into the field's range
    /// instead of overflowing. For example x = -3 and w = 3 at 0.5 become -2
    /// and 2 (the .5 rounds away from zero).
    ///
    /// Scale then un-scale: exact for power-of-two factors at ANY magnitude
    /// (x2 then x0.5 are pure binary exponent shifts — nothing to round), and
    /// exact for other factors only while magnitudes stay well under ~8
    /// million px, because an f32 reciprocal carries ~6e-8 relative error.
    /// Beyond that the un-scale is approximately correct with the documented
    /// rounding (measured: x = -134217728 drifts 4 px through x3 then x1/3).
    pub fn scaled(self, factor: f32) -> Option<Rect> {
        if !factor.is_finite() || factor <= 0.0 {
            return None;
        }
        let f = f64::from(factor);
        Some(Rect {
            x: saturate_i32((f64::from(self.x) * f).round()),
            y: saturate_i32((f64::from(self.y) * f).round()),
            w: saturate_u32((f64::from(self.w) * f).round()),
            h: saturate_u32((f64::from(self.h) * f).round()),
        })
    }
}

/// Clamps one axis of a rect span (pos .. pos + size) into a work-area span
/// (area_pos .. area_pos + area_size), keeping at least min_visible pixels
/// visible. See Rect::clamped_to for the rule.
fn clamp_axis(pos: i64, size: u64, area_pos: i64, area_size: u64, min_visible: u32) -> i64 {
    if size == 0 || area_size == 0 {
        // Nothing can be made visible; keep the position stable.
        return pos;
    }
    let required = u64::from(min_visible.max(1)).min(size).min(area_size);
    let end = pos + size as i64;
    let area_end = area_pos + area_size as i64;
    let overlap = (end.min(area_end) - pos.max(area_pos)).max(0) as u64;
    if overlap >= required {
        return pos;
    }
    if size <= area_size {
        // Pull fully inside from the nearest side. A rect already fully
        // inside never reaches here (its overlap equals its size).
        if pos < area_pos {
            area_pos
        } else {
            area_end - size as i64
        }
    } else {
        // Larger than the work area: pin to its origin.
        area_pos
    }
}

/// Saturates an axis position back into i32. Only work areas beyond the i32
/// coordinate range — impossible for real monitors — can hit the bounds;
/// saturating keeps the function total (no panic) for pathological input.
fn saturate_pos(v: i64) -> i32 {
    if v < i64::from(i32::MIN) {
        i32::MIN
    } else if v > i64::from(i32::MAX) {
        i32::MAX
    } else {
        v as i32
    }
}

fn saturate_i32(v: f64) -> i32 {
    if v <= f64::from(i32::MIN) {
        i32::MIN
    } else if v >= f64::from(i32::MAX) {
        i32::MAX
    } else {
        v as i32
    }
}

fn saturate_u32(v: f64) -> u32 {
    if v <= 0.0 {
        0
    } else if v >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        v as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard for roadmap §12.2 (per-launch window drift): ONE
    /// launch cannot detect drift, so this helper simulates TWO full launch
    /// cycles — persist, restore (clamp into the work area), persist — and
    /// hands back the rect persisted after each launch. If a future change
    /// makes restore non-idempotent, failures of launch_twice name the
    /// regression.
    fn launch_twice(initial: Rect, work_area: Rect, min_visible: u32) -> (Rect, Rect) {
        let persisted_after_first = initial.clamped_to(work_area, min_visible);
        let persisted_after_second = persisted_after_first.clamped_to(work_area, min_visible);
        (persisted_after_first, persisted_after_second)
    }

    #[test]
    fn two_launches_are_stable_after_a_scale_change() {
        // The product promise covers "scaling has changed": launch 1 scales
        // the saved rect to the new DPI and clamps it into the work area;
        // every later launch must be a fixed point at that scale — zero
        // drift, exactly like the same-scale case.
        let area = Rect::new(0, 0, 1920, 1040);
        let saved = Rect::new(100, 100, 800, 600);
        let Some(scaled) = saved.scaled(1.5) else {
            panic!("1.5 is finite and positive; scaled cannot fail");
        };
        let (first, second) = launch_twice(scaled, area, 100);
        assert_eq!(
            first, second,
            "zero drift across launches after the 1.5x scale change"
        );
        // 1200x900 at 1.5x hangs 10 px off the bottom edge — still visible
        // enough (890 px >= min_visible 100), so the position is kept, and
        // kept again on the next launch.
        assert_eq!(first, Rect::new(150, 150, 1200, 900));
        assert!(first.intersects(area));
    }

    #[test]
    fn two_launches_are_stable_for_a_visible_window() {
        let area = Rect::new(0, 0, 1920, 1040);
        let (first, second) = launch_twice(Rect::new(100, 100, 800, 600), area, 100);
        assert_eq!(first, second, "zero drift across launches");
        assert_eq!(
            first,
            Rect::new(100, 100, 800, 600),
            "visible rect untouched"
        );
    }

    #[test]
    fn two_launches_are_stable_after_an_offscreen_restore() {
        // The saved rect sits on a monitor that no longer exists: launch 1
        // pulls it back on-screen, launch 2 must not move it further.
        let area = Rect::new(0, 0, 1920, 1040);
        let (first, second) = launch_twice(Rect::new(5000, -3000, 800, 600), area, 100);
        assert_eq!(first, second, "restore is a fixed point after launch 1");
        assert!(first.intersects(area));
        assert!(!first.is_empty());
    }

    #[test]
    fn clamped_fully_offscreen_rects_come_back_same_size() {
        let area = Rect::new(0, 0, 1920, 1040);
        let offscreen = [
            Rect::new(5000, 100, 800, 600),  // right of the monitor
            Rect::new(-3000, 100, 800, 600), // left
            Rect::new(100, 5000, 800, 600),  // below
            Rect::new(100, -3000, 800, 600), // above
            Rect::new(5000, 5000, 800, 600), // diagonal
        ];
        for off in offscreen {
            let out = off.clamped_to(area, 100);
            assert_eq!((out.w, out.h), (800, 600), "size preserved for {off:?}");
            assert!(out.intersects(area), "on-screen for {off:?}");
            assert!(!out.is_empty(), "never empty for {off:?}");
            assert_eq!(out, out.clamped_to(area, 100), "idempotent for {off:?}");
        }
    }

    #[test]
    fn clamped_already_visible_rects_are_unchanged() {
        let area = Rect::new(0, 0, 1920, 1040);
        let visible = [
            Rect::new(0, 0, 800, 600),
            Rect::new(1120, 440, 800, 600), // flush with the bottom-right corner
            Rect::new(500, 200, 800, 600),
            Rect::new(1820, 100, 800, 600), // hangs off right, exactly min_visible visible
        ];
        for r in visible {
            assert_eq!(r.clamped_to(area, 100), r, "visible rect {r:?} untouched");
        }
    }

    #[test]
    fn clamped_oversized_rect_pins_to_origin_keeps_size() {
        let area = Rect::new(0, 0, 1920, 1040);
        // Off the right edge: nothing visible horizontally, and the rect is
        // wider than the work area, so it is pinned to the work-area origin.
        let r = Rect::new(5000, 0, 3000, 2000);
        let out = r.clamped_to(area, 100);
        assert_eq!(out, Rect::new(0, 0, 3000, 2000));
        assert!(out.intersects(area));
        assert_eq!(out, out.clamped_to(area, 100), "idempotent");
        // A larger-than-area rect that already covers the whole work area is
        // visible enough on both axes and is left exactly where it is.
        let covering = Rect::new(-500, -500, 3000, 2000);
        assert_eq!(covering.clamped_to(area, 100), covering);
    }

    #[test]
    fn clamping_is_idempotent_across_a_grid_of_inputs() {
        let xs = [-2500, -800, -100, 0, 100, 1800, 1900, 2500];
        let ys = [-2500, -800, -100, 0, 100, 900, 1000, 2500];
        let sizes = [(800, 600), (3000, 2000)];
        let areas = [Rect::new(0, 0, 1920, 1040), Rect::new(2560, 48, 1440, 900)];
        let min_visibles = [1, 100, 10_000];
        for x in xs {
            for y in ys {
                for (w, h) in sizes {
                    for area in areas {
                        for min_visible in min_visibles {
                            let r = Rect::new(x, y, w, h);
                            let out = r.clamped_to(area, min_visible);
                            assert_eq!(
                                out,
                                out.clamped_to(area, min_visible),
                                "not a fixed point for {r:?} in {area:?} at {min_visible}"
                            );
                            assert_eq!((out.w, out.h), (w, h), "size preserved for {r:?}");
                            assert!(!out.is_empty(), "never empty for {r:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn intersects_and_intersection_follow_exclusive_edges() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(10, 0, 10, 10); // touches the right edge only
        assert!(!a.intersects(b));
        assert_eq!(a.intersection(b), None);

        let c = Rect::new(5, 5, 10, 10); // overlaps the bottom-right quadrant
        assert!(a.intersects(c));
        assert_eq!(a.intersection(c), Some(Rect::new(5, 5, 5, 5)));

        let d = Rect::new(-5, -5, 20, 20); // contains a
        assert_eq!(a.intersection(d), Some(a));

        let empty = Rect::new(0, 0, 0, 600);
        assert!(!a.intersects(empty));
        assert_eq!(a.intersection(empty), None);
    }

    #[test]
    fn area_and_is_empty_are_exact() {
        assert_eq!(Rect::new(120, 90, 800, 600).area(), 480_000);
        assert_eq!(
            Rect::new(0, 0, u32::MAX, u32::MAX).area(),
            u64::from(u32::MAX) * u64::from(u32::MAX)
        );
        assert!(Rect::new(-5, -5, 0, 10).is_empty());
        assert!(Rect::new(-5, -5, 10, 0).is_empty());
        assert!(!Rect::new(-5, -5, 1, 1).is_empty());
    }

    #[test]
    fn scaled_identity_at_one() {
        let r = Rect::new(120, 90, 800, 600);
        assert_eq!(r.scaled(1.0), Some(r));
        let neg = Rect::new(-1920, -1080, 3840, 2160);
        assert_eq!(neg.scaled(1.0), Some(neg));
    }

    #[test]
    fn scaled_values_at_1_5_and_2_are_exact_for_this_rect() {
        let r = Rect::new(120, 90, 800, 600);
        // 120x1.5=180, 90x1.5=135, 800x1.5=1200, 600x1.5=900 — no rounding
        // needed, so the documented rule reproduces them exactly.
        assert_eq!(r.scaled(1.5), Some(Rect::new(180, 135, 1200, 900)));
        assert_eq!(r.scaled(2.0), Some(Rect::new(240, 180, 1600, 1200)));
    }

    #[test]
    fn scaled_rounding_is_half_away_from_zero() {
        // -3x0.5=-1.5 -> -2, 5x0.5=2.5 -> 3 (away from zero), w/h likewise.
        assert_eq!(
            Rect::new(-3, 5, 3, 5).scaled(0.5),
            Some(Rect::new(-2, 3, 2, 3))
        );
    }

    #[test]
    fn scaled_then_unscaled_loses_no_pixels_for_integer_factors() {
        let r = Rect::new(-123, 456, 800, 600);
        assert_eq!(r.scaled(2.0).and_then(|s| s.scaled(0.5)), Some(r));
        let r3 = Rect::new(-999, 1002, 900, 603);
        assert_eq!(r3.scaled(3.0).and_then(|s| s.scaled(1.0 / 3.0)), Some(r3));
    }

    #[test]
    fn scaled_round_trip_limit_is_pinned_not_folklore() {
        // Power-of-two factors stay exact at ANY magnitude: x2 then x0.5 are
        // binary exponent shifts with nothing to round.
        let wide = Rect::new(-134_217_728, 134_217_727, 800, 600);
        assert_eq!(wide.scaled(2.0).and_then(|s| s.scaled(0.5)), Some(wide));
        // Small magnitudes: the f32 reciprocal of 3 (~6e-8 relative error)
        // still lands back on the exact pixel.
        let small = Rect::new(-4_000_000, 4_000_000, 800, 600);
        assert_eq!(
            small.scaled(3.0).and_then(|s| s.scaled(1.0 / 3.0)),
            Some(small)
        );
        // Crossover (measured): at ~1.3e8 px the reciprocal's error exceeds
        // half a pixel — x3 then x1/3 drifts by 4 px. This pins the limit the
        // doc claims: exact only for powers of two or magnitudes far below
        // ~8 million px. If this ever round-trips exactly again, widen the
        // doc claim in the same change.
        assert_ne!(
            wide.scaled(3.0).and_then(|s| s.scaled(1.0 / 3.0)),
            Some(wide),
            "x3 then x1/3 at ~1.3e8 px is documented as approximately correct"
        );
    }

    #[test]
    fn scaled_rejects_zero_negative_and_non_finite_factors() {
        let r = Rect::new(0, 0, 10, 10);
        assert_eq!(r.scaled(0.0), None);
        assert_eq!(r.scaled(-0.0), None);
        assert_eq!(r.scaled(-1.0), None);
        assert_eq!(r.scaled(f32::NAN), None);
        assert_eq!(r.scaled(f32::INFINITY), None);
    }

    #[test]
    fn rect_serialises_round_trip() -> Result<(), serde_json::Error> {
        let r = Rect::new(-40, 300, 1280, 720);
        let json = serde_json::to_string(&r)?;
        assert_eq!(json, r#"{"x":-40,"y":300,"w":1280,"h":720}"#);
        assert_eq!(serde_json::from_str::<Rect>(&json)?, r);
        Ok(())
    }

    /// AGENTS.md: core never panics on paths reachable from I/O. These
    /// functions are pure, so extreme inputs must merely produce defined
    /// (saturating) results — never a panic.
    #[test]
    fn geometry_extremes_do_not_panic() {
        let huge = Rect::new(i32::MIN, i32::MIN, u32::MAX, u32::MAX);
        let _ = huge.area();
        let _ = huge.intersects(huge);
        let _ = huge.intersection(huge);
        let _ = huge.clamped_to(huge, u32::MAX);
        let _ = huge.scaled(f32::MAX);
        let tiny = Rect::new(i32::MAX, i32::MAX, 0, 0);
        let _ = tiny.clamped_to(huge, u32::MAX);
        let _ = tiny.scaled(1.0e6);
        let _ = huge.scaled(1.0e-9);
    }
}
