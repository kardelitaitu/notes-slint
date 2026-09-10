//! Geometry, in exactly one unit.
//!
//! This module is pure Rust - no `windows` types, no `notes-core` types, no serde -
//! so the conversions can be tested with no window and no display server.

/// A window rectangle in **physical frame pixels, Win32 `GetWindowRect` space**: the
/// device-pixel bounds including title bar and borders, measured from the origin of
/// the virtual screen.
///
/// That is the only unit legal anywhere in this crate: no logical points, no DIPs, no
/// client-area pixels and no normalised coordinates cross a platform seam. A caller
/// holding a rect measured in another space converts it before calling, or passes a
/// `scale` and lets [`FrameRect::scaled`] do that one step.
///
/// `x`/`y` are signed because a monitor left of, or above, the primary monitor has a
/// negative origin. `w`/`h` are unsigned because a window cannot have a negative
/// extent, and the type then has nowhere to put one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRect {
    /// Left edge, in physical frame pixels from the virtual-screen origin.
    pub x: i32,
    /// Top edge, in physical frame pixels from the virtual-screen origin.
    pub y: i32,
    /// Width in physical frame pixels; never negative, by construction.
    pub w: u32,
    /// Height in physical frame pixels; never negative, by construction.
    pub h: u32,
}

impl FrameRect {
    /// Builds a rect from origin and extent, in the documented unit.
    #[must_use]
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// `w` as a signed extent, for the Win32 calls that take `i32`. Saturates at
    /// `i32::MAX`: nothing here can place a 2-gigapixel window, and it must not wrap
    /// into a negative one.
    #[must_use]
    pub const fn width(&self) -> i32 {
        if self.w > i32::MAX as u32 {
            i32::MAX
        } else {
            self.w as i32
        }
    }

    /// `h` as a signed extent. See [`FrameRect::width`].
    #[must_use]
    pub const fn height(&self) -> i32 {
        if self.h > i32::MAX as u32 {
            i32::MAX
        } else {
            self.h as i32
        }
    }

    /// The right edge, saturating rather than wrapping, so a rect whose far edge
    /// leaves the coordinate range stays comparable.
    #[must_use]
    pub const fn right(&self) -> i32 {
        self.x.saturating_add(self.width())
    }

    /// The bottom edge, saturating like [`FrameRect::right`].
    #[must_use]
    pub const fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height())
    }

    /// True when either extent is zero: a rect that could not be shown.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// True when `other` lies wholly inside `self`, edges inclusive.
    #[must_use]
    pub const fn contains(&self, other: Self) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    /// Multiplies every origin and extent by `scale`: the one piece of arithmetic in
    /// this crate, and the reason [`crate::WindowBackend::set_frame_rect`] takes a
    /// scale at all.
    ///
    /// Rounding is half-away-from-zero on every component, so a rect grows
    /// symmetrically about the origin instead of drifting by a pixel. `scale` is a
    /// caller-supplied ratio between the pixel space `self` was measured in and the
    /// space the destination expects; this function does not know, guess or query
    /// what it is. A scale that is not finite or not positive leaves the rect
    /// unchanged: a nonsense ratio is the caller's arithmetic slip, and the last thing
    /// a slip should cause is a window thrown at a saturated coordinate.
    #[must_use]
    pub fn scaled(self, scale: f32) -> Self {
        if !scale.is_finite() || scale <= 0.0 {
            return self;
        }
        let s = f64::from(scale);
        // Float-to-int casts saturate in Rust, so an absurd scale lands on the range
        // limits instead of panicking or wrapping.
        let axis = |v: i32| (f64::from(v) * s).round() as i32;
        let extent = |v: u32| (f64::from(v) * s).round() as i64;
        Self {
            x: axis(self.x),
            y: axis(self.y),
            w: extent(self.w).clamp(0, i64::from(u32::MAX)) as u32,
            h: extent(self.h).clamp(0, i64::from(u32::MAX)) as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FrameRect;

    #[test]
    fn extent_is_unsigned_so_a_negative_size_is_unrepresentable() {
        // The invariant the crate leans on: no rect can carry a negative extent into
        // a Win32 call, because the type has nowhere to put one.
        let r = FrameRect::new(-40, -80, 0, 0);
        assert_eq!((r.width(), r.height()), (0, 0));
        assert!(r.is_empty());
        let huge = FrameRect::new(0, 0, u32::MAX, u32::MAX);
        assert_eq!((huge.width(), huge.height()), (i32::MAX, i32::MAX));
        assert_eq!((huge.right(), huge.bottom()), (i32::MAX, i32::MAX));
    }

    #[test]
    fn origins_stay_signed_for_monitors_left_of_the_primary() {
        let r = FrameRect::new(-1920, -300, 800, 600);
        assert_eq!((r.right(), r.bottom()), (-1120, 300));
        assert!(!r.is_empty());
    }

    #[test]
    fn scaling_by_one_is_the_identity() {
        let r = FrameRect::new(-100, 200, 1024, 768);
        assert_eq!(r.scaled(1.0), r);
    }

    #[test]
    fn scaling_is_symmetric_about_the_origin() {
        let right = FrameRect::new(100, 100, 800, 600).scaled(1.5);
        let left = FrameRect::new(-100, -100, 800, 600).scaled(1.5);
        assert_eq!(right, FrameRect::new(150, 150, 1200, 900));
        assert_eq!(left, FrameRect::new(-150, -150, 1200, 900));
    }

    #[test]
    fn scaling_rounds_half_away_from_zero_on_every_component() {
        assert_eq!(FrameRect::new(101, 0, 0, 0).scaled(1.5).x, 152);
        assert_eq!(FrameRect::new(-101, 0, 0, 0).scaled(1.5).x, -152);
        // 125% of odd extents must not lose or invent a pixel asymmetrically.
        assert_eq!(
            FrameRect::new(0, 0, 3, 5).scaled(1.25),
            FrameRect::new(0, 0, 4, 6)
        );
    }

    #[test]
    fn a_useless_scale_changes_nothing_instead_of_saturating_the_rect() {
        let r = FrameRect::new(10, 20, 300, 200);
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(r.scaled(scale), r, "scale {scale} must be ignored");
        }
    }

    #[test]
    fn an_absurd_but_legal_scale_saturates_and_never_panics() {
        let r = FrameRect::new(i32::MIN, i32::MAX, u32::MAX, 1).scaled(f32::MAX);
        assert_eq!(r.x, i32::MIN);
        assert_eq!(r.y, i32::MAX);
        // Every extent really does overflow at 3.4e38x, so each one lands on the top
        // of its own range rather than wrapping, panicking, or going negative.
        assert_eq!((r.w, r.h), (u32::MAX, u32::MAX));
        // Shrinking to nothing is legal and lands on zero, not on a negative.
        assert!(FrameRect::new(0, 0, 800, 600).scaled(0.0001).is_empty());
    }

    #[test]
    fn containment_uses_the_saturated_edges() {
        let work = FrameRect::new(0, 0, 1920, 1040);
        assert!(work.contains(FrameRect::new(0, 0, 800, 600)));
        assert!(work.contains(FrameRect::new(1120, 440, 800, 600)));
        assert!(!work.contains(FrameRect::new(1120, 441, 800, 600)));
        assert!(!work.contains(FrameRect::new(-1, 0, 800, 600)));
        // A rect whose edges leave the coordinate range is inside nothing.
        assert!(!work.contains(FrameRect::new(0, 0, u32::MAX, u32::MAX)));
    }
}
