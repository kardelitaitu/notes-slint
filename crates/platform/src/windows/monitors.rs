//! Where the window is, and where it can be put. Reading and writing a rect, and
//! reporting the work area a window should stay inside - reported, never enforced.

use ::windows::Win32::Foundation::{POINT, RECT};
use ::windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
    MonitorFromPoint, MonitorFromWindow,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, SET_WINDOW_POS_FLAGS, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
};

use super::{to_hwnd, win32_error};
use crate::{FrameRect, PlatformError, PlatformResult};

/// `SWP_NOZORDER | SWP_NOACTIVATE`: placing a window must not change the z-order
/// (that is the topmost seam) and must not steal focus. Origin and extent are both
/// applied, so no NOMOVE/NOSIZE bit belongs here.
const PLACEMENT_FLAGS: SET_WINDOW_POS_FLAGS =
    SET_WINDOW_POS_FLAGS(SWP_NOZORDER.0 | SWP_NOACTIVATE.0);

/// The Win32 corner-based rect, as the origin-and-extent rect of this crate.
///
/// Each extent is a saturated subtraction of two `i32` values floored at zero before
/// it lands in a `u32` field, so a rect Win32 reports with reversed edges reads as
/// empty instead of wrapping into a giant window.
fn from_win32(rect: RECT) -> FrameRect {
    FrameRect {
        x: rect.left,
        y: rect.top,
        w: rect.right.saturating_sub(rect.left).max(0) as u32,
        h: rect.bottom.saturating_sub(rect.top).max(0) as u32,
    }
}

/// The frame rectangle of `handle` - see [`crate::WindowBackend::frame_rect`].
pub fn frame_rect(handle: isize) -> PlatformResult<FrameRect> {
    let hwnd = to_hwnd(handle)?;
    let mut rect = RECT::default();
    // SAFETY: GetWindowRect is given the HWND that `to_hwnd` accepted from IsWindow,
    // plus a pointer to `rect`, a local repr(C) RECT that is aligned, writable and
    // alive for the whole call. Win32 writes at most that struct; nothing on this
    // side dereferences the handle.
    unsafe { GetWindowRect(hwnd, &mut rect) }
        .map_err(|error| win32_error("GetWindowRect", error))?;
    Ok(from_win32(rect))
}

/// Moves and resizes `handle` to `r`, after `r.scaled(scale)` - see
/// [`crate::WindowBackend::set_frame_rect`].
pub fn set_frame_rect(handle: isize, r: FrameRect, scale: f32) -> PlatformResult<()> {
    let hwnd = to_hwnd(handle)?;
    let placed = r.scaled(scale);
    // SAFETY: the HWND is the one `to_hwnd` accepted from IsWindow; the four
    // coordinates are i32 values rather than pointers; and `None` is the documented
    // null insert-after handle, only meaningful because PLACEMENT_FLAGS carries
    // SWP_NOZORDER. No memory is shared with Win32 in this call.
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            placed.x,
            placed.y,
            placed.width(),
            placed.height(),
            PLACEMENT_FLAGS,
        )
    }
    .map_err(|error| win32_error("SetWindowPos", error))
}

/// The work area of the monitor `handle` sits on: that monitor minus the taskbar and
/// any reserved edge, so a window placed inside it stays usable.
///
/// Which monitor that is stays a Win32 answer (`MONITOR_DEFAULTTONEAREST`, the one
/// already holding most of the window). Nothing here chooses a monitor, and nothing
/// here moves the window into the area it reports.
pub fn monitor_work_area(handle: isize) -> PlatformResult<FrameRect> {
    let hwnd = to_hwnd(handle)?;
    // SAFETY: MonitorFromWindow takes the validated HWND and a by-value flag enum,
    // dereferences nothing, and may return a null monitor handle - which
    // `work_area_of` checks before use.
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    work_area_of(monitor)
}

/// The work area of the primary monitor - see
/// [`crate::WindowBackend::primary_work_area`].
pub fn primary_work_area() -> PlatformResult<FrameRect> {
    // SAFETY: MonitorFromPoint takes a by-value POINT (0, 0, which by definition
    // belongs to the primary monitor) and a flag enum, and dereferences nothing. The
    // returned handle is validated before use.
    let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    work_area_of(monitor)
}

/// The `GetMonitorInfoW` half of both entry points above, so the `cbSize`
/// handshake exists once.
fn work_area_of(monitor: HMONITOR) -> PlatformResult<FrameRect> {
    if monitor.is_invalid() {
        return Err(PlatformError::NoMonitor);
    }
    let mut info = MONITORINFO {
        cbSize: core::mem::size_of::<MONITORINFO>() as u32,
        ..MONITORINFO::default()
    };
    // SAFETY: GetMonitorInfoW reads only `cbSize`, set above to exactly
    // `size_of::<MONITORINFO>()` as the API demands, and writes its two RECTs into
    // `&mut info`, a live repr(C) local that outlives the call. `monitor` was just
    // checked non-null; a monitor unplugged since then makes the call return FALSE
    // rather than fault.
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        Ok(from_win32(info.rcWork))
    } else {
        Err(PlatformError::NoMonitor)
    }
}

#[cfg(test)]
mod tests {
    use super::{PLACEMENT_FLAGS, from_win32, monitor_work_area, set_frame_rect};
    use crate::{FrameRect, PlatformError, PlatformResult, WindowBackend};
    use ::windows::Win32::Foundation::RECT;
    use ::windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SET_WINDOW_POS_FLAGS, SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE,
        SWP_NOZORDER,
    };

    #[test]
    fn placement_touches_neither_z_order_nor_activation() {
        // A dropped SWP_NOZORDER un-pins the window as a side effect of a move; a
        // dropped SWP_NOACTIVATE steals focus. Both are invisible in review.
        let expected = SET_WINDOW_POS_FLAGS(SWP_NOZORDER.0 | SWP_NOACTIVATE.0);
        assert_eq!(PLACEMENT_FLAGS, expected);
        assert!(PLACEMENT_FLAGS.contains(SWP_NOZORDER));
        assert!(PLACEMENT_FLAGS.contains(SWP_NOACTIVATE));
    }

    #[test]
    fn a_win32_rect_is_converted_without_wrapping_or_losing_sign() {
        let reversed = RECT {
            left: 100,
            top: 50,
            right: 20,
            bottom: 10,
        };
        assert!(from_win32(reversed).is_empty());
        // Negative origins are the normal case for a monitor left of the primary.
        let second = RECT {
            left: -1920,
            top: -200,
            right: -1120,
            bottom: 400,
        };
        assert_eq!(from_win32(second), FrameRect::new(-1920, -200, 800, 600));
    }

    #[test]
    fn a_bad_handle_never_reaches_a_monitor_or_placement_call() {
        for handle in [0, 2, 3, isize::MIN] {
            let result: PlatformResult<FrameRect> = monitor_work_area(handle);
            assert!(
                matches!(result, Err(PlatformError::InvalidHandle)),
                "handle {handle:#x}: {result:?}",
            );
            let result = set_frame_rect(handle, FrameRect::new(0, 0, 100, 100), 1.5);
            assert!(
                matches!(result, Err(PlatformError::InvalidHandle)),
                "{result:?}"
            );
        }
    }

    /// Tolerant by design: a build agent may have no interactive desktop, a session-0
    /// window station, or no monitor at all, and each of those is a legitimate `Err`.
    /// So the test describes a real answer and otherwise passes, and it never creates
    /// a window, takes focus, or sleeps.
    #[test]
    fn a_work_area_is_inside_the_primary_monitor_or_the_call_errored() {
        // SAFETY: GetSystemMetrics takes a by-value index enum and dereferences
        // nothing; it returns 0 when there is no desktop, which is what makes this
        // test safe to run headless.
        let (screen_w, screen_h) =
            unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        if screen_w <= 0 || screen_h <= 0 {
            return; // No desktop: nothing to describe, nothing to fail.
        }
        let Ok(work) = super::super::Backend.primary_work_area() else {
            return; // A station with no monitor: the error is the honest answer.
        };
        let primary = FrameRect::new(0, 0, screen_w as u32, screen_h as u32);
        assert!(
            primary.contains(work),
            "work area {work:?} escaped the primary monitor {primary:?}",
        );
        assert!(
            work.w <= primary.w && work.h <= primary.h,
            "work area {work:?} is larger than the screen {primary:?}",
        );
    }
}
