//! Where the window is, and where it can be put. Reading and writing a rect, and
//! reporting the work area a window should stay inside - reported, never enforced.

use ::windows::Win32::Foundation::{POINT, RECT};
use ::windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
    MONITORINFOEXW, MonitorFromPoint, MonitorFromRect, MonitorFromWindow,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    GetWindowPlacement, GetWindowRect, SET_WINDOW_POS_FLAGS, SWP_NOACTIVATE, SWP_NOZORDER,
    SetWindowPos, WINDOWPLACEMENT,
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
    // SAFETY: GetWindowRect is given the HWND that `to_hwnd` accepted from IsWindow
    // at check time (the call re-validates it and fails closed if the window died
    // since), plus a pointer to `rect`, a local repr(C) RECT that is aligned,
    // writable and alive for the whole call. Win32 writes at most that struct;
    // nothing on this side dereferences the handle.
    unsafe { GetWindowRect(hwnd, &mut rect) }
        .map_err(|error| win32_error("GetWindowRect", error))?;
    Ok(from_win32(rect))
}

/// The restore (normal) frame rect of `handle` - see
/// [`crate::WindowBackend::restore_frame_rect`].
pub fn restore_frame_rect(handle: isize) -> PlatformResult<FrameRect> {
    let hwnd = to_hwnd(handle)?;
    let mut placement = WINDOWPLACEMENT {
        length: core::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..WINDOWPLACEMENT::default()
    };
    // SAFETY: GetWindowPlacement reads only `length`, set above to exactly
    // `size_of::<WINDOWPLACEMENT>()` as the API demands, and writes its fields
    // into `&mut placement`, a live repr(C) local that outlives the call. The
    // HWND is the one `to_hwnd` accepted from IsWindow at check time; a window
    // destroyed since makes the call return FALSE rather than fault, and the
    // returned Result maps that FALSE into an error carrying this call's OS code.
    // `rcNormalPosition` is reported whatever the current show state; the
    // show-state fields are copied but never interpreted here.
    unsafe { GetWindowPlacement(hwnd, &mut placement) }
        .map_err(|error| win32_error("GetWindowPlacement", error))?;
    Ok(from_win32(placement.rcNormalPosition))
}

/// The numeric suffix of Win32's display-device name (\\.\\DISPLAY<n>) - the
/// monitor id the caller persists; see
/// [`crate::HostFacts::work_area_for_rect`] for what that id does and does
/// not guarantee. 0 is the honest "unparseable" answer; Win32's display
/// devices always carry the number, so it is never produced in practice.
fn display_ordinal(device: &[u16; 32]) -> u32 {
    // szDevice is a fixed [u16; 32] buffer: the device name is NUL-terminated
    // and NUL-padded, so the lossy string carries trailing U+0000s. The device
    // name is BACKSLASH BACKSLASH DOT BACKSLASH DISPLAY<n> and contains no
    // other digits, so collecting every ASCII digit from the whole name yields
    // exactly the ordinal; an unparsable name answers 0, honestly.
    let name = String::from_utf16_lossy(device);
    let digits: String = name.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse().unwrap_or(0)
}
/// The work area of the monitor `rect` belongs to - see
/// [`crate::HostFacts::work_area_for_rect`].
pub fn work_area_for_rect(rect: FrameRect) -> PlatformResult<(FrameRect, u32)> {
    let win_rect = RECT {
        left: rect.x,
        top: rect.y,
        right: rect.right(),
        bottom: rect.bottom(),
    };
    // SAFETY: MonitorFromRect takes a pointer to `win_rect`, a live repr(C)
    // local that outlives the call, and a by-value flag enum; it dereferences
    // the rect only during the call and answers with a monitor handle.
    // MONITOR_DEFAULTTONEAREST is documented never to return null (only
    // MONITOR_DEFAULTTONULL can), and the handle is re-checked before use.
    let monitor = unsafe { MonitorFromRect(&win_rect, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return Err(PlatformError::NoMonitor);
    }
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = core::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: GetMonitorInfoW reads only `cbSize`, set above to exactly
    // `size_of::<MONITORINFOEXW>()`, which declares the buffer as the EX form:
    // it writes the MONITORINFO fields through the `&mut info.monitorInfo`
    // pointer and the szDevice tail into the bytes that immediately follow,
    // inside the enclosing repr(C) MONITORINFOEXW local, which is live and
    // outlives the call - the C layout contract (MONITORINFOEXW = MONITORINFO
    // followed by szDevice) is what makes the enclosing struct the right
    // buffer. A monitor unplugged since the lookup makes the call answer
    // FALSE, which `.ok()` maps into an error carrying this call's OS code.
    unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) }
        .ok()
        .map_err(|error| win32_error("GetMonitorInfoW", error))?;
    Ok((
        from_win32(info.monitorInfo.rcWork),
        display_ordinal(&info.szDevice),
    ))
}

/// Moves and resizes `handle` to `r`, after `r.scaled(scale)` - see
/// [`crate::WindowBackend::set_frame_rect`].
pub fn set_frame_rect(handle: isize, r: FrameRect, scale: f32) -> PlatformResult<()> {
    let hwnd = to_hwnd(handle)?;
    let placed = r.scaled(scale);
    // SAFETY: the HWND is the one `to_hwnd` accepted from IsWindow at check time -
    // SetWindowPos re-validates it and fails closed, so a window destroyed in the
    // meantime is an error rather than a write into someone else's window; the four
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
    // SAFETY: MonitorFromWindow takes the check-time-validated HWND and a by-value
    // flag enum,
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
///
/// The two refusals are kept apart on purpose: no monitor at all (a null handle from
/// the lookup) is [`PlatformError::NoMonitor`], while a monitor that stopped
/// existing - unplugged, or stale since the lookup - arrives as
/// [`PlatformError::Win32`] with the OS code, because that is a different
/// diagnosis for the caller and losing the code would hide it.
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
    // rather than fault. `BOOL::ok` converts that FALSE into an error from the
    // calling thread immediately, so the code it carries is this call's.
    unsafe { GetMonitorInfoW(monitor, &mut info) }
        .ok()
        .map_err(|error| win32_error("GetMonitorInfoW", error))?;
    Ok(from_win32(info.rcWork))
}

#[cfg(test)]
mod tests {
    use super::{
        PLACEMENT_FLAGS, frame_rect, from_win32, monitor_work_area, primary_work_area,
        restore_frame_rect, set_frame_rect, work_area_for_rect, work_area_of,
    };
    use crate::{FrameRect, PlatformError, PlatformResult, WindowBackend};
    use ::windows::Win32::Foundation::RECT;
    use ::windows::Win32::Graphics::Gdi::HMONITOR;
    use ::windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SET_WINDOW_POS_FLAGS, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN,
        SM_XVIRTUALSCREEN, SWP_NOACTIVATE, SWP_NOZORDER,
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
        // 2 and 3 are misaligned for a USER handle, 0 is null, and isize::MIN + 1
        // has the 64-bit sign bit set while the meaningful bits of an HWND are
        // 32-bit: none of them can name a live window.
        for handle in [0, 2, 3, isize::MIN + 1] {
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

    /// The restore rect goes through the same handle guard as the live rect, so a
    /// garbage handle maps to the SAME refusal for both - and never panics.
    #[test]
    fn restore_frame_rect_maps_a_garbage_handle_like_frame_rect() {
        for handle in [0, 2, 3, isize::MIN + 1] {
            let live = frame_rect(handle).map(|_| ());
            let restore = restore_frame_rect(handle).map(|_| ());
            assert!(
                matches!(restore, Err(PlatformError::InvalidHandle)),
                "handle {handle:#x}: expected InvalidHandle, got {restore:?}",
            );
            assert_eq!(
                core::mem::discriminant(&live),
                core::mem::discriminant(&restore),
                "handle {handle:#x}: the two rect seams must refuse alike",
            );
        }
    }

    /// A rect on the primary resolves to the primary: its work area comes back
    /// whole and the display ordinal is parsed, not invented.
    #[test]
    fn a_rect_on_the_primary_resolves_to_the_primary() {
        let primary = primary_work_area().expect("a primary monitor exists");
        let inside = FrameRect::new(primary.x + 10, primary.y + 10, 100, 100);
        let (work, id) = work_area_for_rect(inside).expect("the rect has a monitor");
        assert_eq!(work, primary, "max overlap is the primary itself");
        assert_ne!(id, 0, "the ordinal is parsed from the device name");
    }

    /// On a station whose desktop extends past the primary's right edge, a rect
    /// ten pixels past that edge belongs to the OTHER monitor: the returned work
    /// area overlaps it and the id differs from the primary's. The guard keeps
    /// the test honest on stations without a right-hand secondary.
    #[test]
    fn a_rect_past_the_primary_edge_resolves_to_the_other_monitor() {
        let primary = primary_work_area().expect("a primary monitor exists");
        let (virtual_x, virtual_w) = unsafe {
            // SAFETY: GetSystemMetrics takes a by-value index enum and
            // dereferences nothing; it is a pure query of the desktop metrics.
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
            )
        };
        let past = FrameRect::new(primary.right() + 10, primary.y + 10, 60, 60);
        let (work, id) = work_area_for_rect(past).expect("nearest monitor is returned");
        let overlap = i64::from(work.right().min(past.right())) - i64::from(work.x.max(past.x));
        let (_, primary_id) =
            work_area_for_rect(FrameRect::new(primary.x + 10, primary.y + 10, 100, 100))
                .expect("primary lookup");
        if primary.right() < virtual_x + virtual_w {
            assert!(
                overlap > 0,
                "the rect must overlap the monitor it belongs to"
            );
            assert_ne!(
                id, primary_id,
                "a rect past the primary edge belongs to another monitor"
            );
        } else {
            eprintln!(
                "station has no desktop past the primary's right edge; only the 
                 nearest-monitor path was exercised (overlap {overlap}, id {id})"
            );
        }
    }

    /// (99999, 99999) overlaps nothing: MONITOR_DEFAULTTONEAREST answers with a
    /// real monitor's work area - the caller clamps, this crate does not - and
    /// an empty rect degenerates to its left/top corner, which Win32 resolves
    /// like a point at the origin. Both are defined answers, never a panic.
    #[test]
    fn a_far_off_and_an_empty_rect_degrade_without_panicking() {
        let far = work_area_for_rect(FrameRect::new(99_999, 99_999, 100, 100));
        assert!(matches!(&far, Ok((work, _)) if !work.is_empty()), "{far:?}");
        let zero = work_area_for_rect(FrameRect::new(0, 0, 0, 0));
        assert!(
            matches!(&zero, Ok((work, _)) if !work.is_empty()),
            "{zero:?}"
        );
    }

    /// The two refusals are different diagnoses, so they must not share a variant: a
    /// null monitor handle is `NoMonitor`, a monitor that stopped existing is a
    /// `Win32` mapping that still carries the OS code.
    #[test]
    fn a_null_monitor_and_a_dead_monitor_are_not_the_same_refusal() {
        let null = work_area_of(HMONITOR(core::ptr::null_mut()));
        assert!(matches!(null, Err(PlatformError::NoMonitor)), "{null:?}");
        // 4-byte aligned and non-null, so it clears the guard and reaches the API,
        // but an HWND/HMONITOR's meaningful bits are 32-bit: with the 64-bit sign bit
        // set this cannot name a monitor on any station. GetMonitorInfoW answers
        // FALSE, and the point of the test is that the code survives the mapping.
        let dead = HMONITOR(isize::MIN as *mut core::ffi::c_void);
        let result = work_area_of(dead);
        assert!(
            matches!(&result, Err(PlatformError::Win32 { api, .. }) if *api == "GetMonitorInfoW"),
            "{result:?}"
        );
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
