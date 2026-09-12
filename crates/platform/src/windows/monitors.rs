//! Where the window is, and where it can be put. Reading and writing a rect, and
//! reporting the work area a window should stay inside and the DPI scale of the
//! monitor that owns one - reported, never enforced.
//!
//! Every read here (GetWindowRect, GetWindowPlacement, the MonitorFrom* and
//! GetMonitorInfoW queries) takes no SWP_ flags: reads do not marshal across input
//! queues, so SWP_ASYNCWINDOWPOS - which the two writing seams carry - has no read
//! path to appear on.

use ::windows::Win32::Foundation::{POINT, RECT};
use ::windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
    MONITORINFOEXW, MonitorFromPoint, MonitorFromRect, MonitorFromWindow,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    GetWindowPlacement, GetWindowRect, SET_WINDOW_POS_FLAGS, SW_SHOW, SW_SHOWMAXIMIZED,
    SW_SHOWNORMAL, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPlacement,
    SetWindowPos, WINDOWPLACEMENT,
};

use super::{to_hwnd, win32_error};
use crate::{FrameRect, Placement, PlatformError, PlatformResult, ShowState};

/// `SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS`: placing a window must not
/// change the z-order (that is the topmost seam) and must not steal focus. Origin
/// and extent are both applied, so no NOMOVE/NOSIZE bit belongs here.
///
/// `SWP_ASYNCWINDOWPOS` is the load-bearing bit: without it, a `SetWindowPos`
/// whose caller and window are attached to different input queues SENDS
/// `WM_WINDOWPOSCHANGING`/`WM_WINDOWPOSCHANGED` and the caller BLOCKS until the
/// thread that owns the window pumps - which can be forever. With it, the call
/// returns before the move has landed; the caller obligation that follows is
/// documented on [`crate::WindowBackend::set_frame_rect`].
const PLACEMENT_FLAGS: SET_WINDOW_POS_FLAGS =
    SET_WINDOW_POS_FLAGS(SWP_NOZORDER.0 | SWP_NOACTIVATE.0 | SWP_ASYNCWINDOWPOS.0);

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

/// Maps `WINDOWPLACEMENT.showCmd` to a [`ShowState`]: the one place this crate
/// reads that field, and a pure function, so the mapping is testable without a
/// window.
///
/// The test is EQUALITY against `SW_SHOWMAXIMIZED` - the same test gpui itself
/// uses - and not the `WPF_RESTORETOMAXIMIZED` bit of `flags`: that bit names
/// where the window would go WHEN restored, so it survives an un-maximise and
/// reading it here would latch a stale "maximised". `showCmd` is the current
/// state, and nothing here acts on it.
fn show_state(cmd: u32) -> ShowState {
    if cmd == SW_SHOWMAXIMIZED.0 as u32 {
        ShowState::Maximized
    } else if cmd == SW_SHOWNORMAL.0 as u32 || cmd == SW_SHOW.0 as u32 {
        ShowState::Normal
    } else {
        ShowState::Unknown
    }
}

/// The restore rect and current show state of `handle` - see
/// [`crate::WindowBackend::restore_frame_rect`].
pub fn restore_frame_rect(handle: isize) -> PlatformResult<Placement> {
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
    // `rcNormalPosition` is reported whatever the current show state; both it and
    // `showCmd` are copied out and mapped, never acted on.
    unsafe { GetWindowPlacement(hwnd, &mut placement) }
        .map_err(|error| win32_error("GetWindowPlacement", error))?;
    Ok(Placement {
        restore_rect: from_win32(placement.rcNormalPosition),
        show: show_state(placement.showCmd),
    })
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

/// The scale of the monitor `rect` belongs to, as physical pixels per logical
/// pixel - see [`crate::HostFacts::scale_for_rect`].
pub fn scale_for_rect(rect: FrameRect) -> PlatformResult<f32> {
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
    scale_of_monitor(monitor)
}

/// The `GetDpiForMonitor` half of [`scale_for_rect`], so the two refusals and
/// the out-pointer handshake have one testable seam - the same shape
/// `work_area_of` gives [`work_area_for_rect`].
fn scale_of_monitor(monitor: HMONITOR) -> PlatformResult<f32> {
    if monitor.is_invalid() {
        return Err(PlatformError::NoMonitor);
    }
    let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
    // SAFETY: GetDpiForMonitor is given `monitor`, an HMONITOR the lookup above
    // answered (or a test's deliberately garbage value, which the API refuses
    // with E_INVALIDARG rather than faulting), the by-value MDT_EFFECTIVE_DPI
    // request, and two live u32 locals that are the only storage the call
    // writes - no pointer is retained past the call, and the Result is mapped,
    // never unwrapped.
    unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }
        .ok()
        .map_err(|error| win32_error("GetDpiForMonitor", error))?;
    // Per-monitor DPI is uniform across the axes (gpui asserts x == y; this
    // crate panics on nothing), so the x axis is used and the invariant is
    // stated here instead of asserted.
    Ok(dpi_x as f32 / USER_DEFAULT_SCREEN_DPI)
}

// shcore's GetDpiForMonitor, declared locally: the windows crate gates this
// symbol behind the `Win32_UI_HiDpi` feature, which the root manifest does not
// enable (the feature list lives in the manager-owned root manifest). The
// generated binding would link the very same api-set DLL with the very same
// signature - windows-0.61's own emission for GetDpiForMonitor is exactly this
// raw-dylib link - so here it is spelled by hand. Deliberately NOT marked
// `safe` (unlike GetACP in `super`): it writes through its two out-pointers,
// so every call goes through an unsafe block stating that invariant.
#[link(
    name = "api-ms-win-shcore-scaling-l1-1-1.dll",
    kind = "raw-dylib",
    modifiers = "+verbatim"
)]
unsafe extern "system" {
    fn GetDpiForMonitor(
        hmonitor: HMONITOR,
        dpitype: i32,
        dpix: *mut u32,
        dpiy: *mut u32,
    ) -> ::windows::core::HRESULT;
}

/// `MDT_EFFECTIVE_DPI` - windows's `MONITOR_DPI_TYPE`, spelled as its documented
/// discriminant so no HiDpi feature is needed for a type: the DPI the system
/// uses to scale UI on that monitor for this process's awareness mode, the
/// variant the toolkit itself queries.
const MDT_EFFECTIVE_DPI: i32 = 0;

/// Win32's `USER_DEFAULT_SCREEN_DPI`: what "100%" means. The scale is reported
/// against this base, in the unit GPUI's `scale_factor` uses.
const USER_DEFAULT_SCREEN_DPI: f32 = 96.0;

/// Moves and resizes `handle` to `r`, after `r.scaled(scale)` - see
/// [`crate::WindowBackend::set_frame_rect`].
///
/// Issued with `SWP_ASYNCWINDOWPOS` (see [`PLACEMENT_FLAGS`]): this returns
/// before the move has landed, so a rect read straight back may still be the
/// pre-move rect.
pub fn set_frame_rect(handle: isize, r: FrameRect, scale: f32) -> PlatformResult<()> {
    let hwnd = to_hwnd(handle)?;
    let placed = r.scaled(scale);
    // SAFETY: the HWND is the one `to_hwnd` accepted from IsWindow at check time -
    // SetWindowPos re-validates it and fails closed, so a window destroyed in the
    // meantime is an error rather than a write into someone else's window; the four
    // coordinates are i32 values rather than pointers; and `None` is the documented
    // null insert-after handle, only meaningful because PLACEMENT_FLAGS carries
    // SWP_NOZORDER. SWP_ASYNCWINDOWPOS changes only liveness - the call posts the
    // move instead of blocking on the owner's pump - never what is written. No
    // memory is shared with Win32 in this call.
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

/// Writes `rcNormalPosition` and NOTHING ELSE - see
/// [`crate::WindowBackend::set_restore_frame_rect`].
///
/// This is deliberately a read-modify-write of one field rather than a
/// hand-built `WINDOWPLACEMENT`: `showCmd`, `flags` and the min/max tracking sizes
/// come back out of `GetWindowPlacement` and go straight back in untouched, so the
/// window keeps the show state it has (a maximised window stays maximised) and the
/// `WPF_RESTORETOMAXIMIZED` bit a toolkit set is not quietly rewritten by a port
/// that only wanted to correct a number. A zeroed struct passed to
/// `SetWindowPlacement` would have un-maximised the window it meant to annotate,
/// which is the opposite of the seam.
///
/// No `SWP_` flags exist on this call - `SetWindowPlacement` takes none - and it is
/// NOT documented as async: like `SetWindowPos` without `SWP_ASYNCWINDOWPOS` it may
/// send to the window owner, so a caller on another thread can wait for a pump.
/// That is why nothing in `api` stamps a move-in-flight guard from here, and why
/// the un-maximise TARGET (which is what this writes) is allowed to land late.
pub fn set_restore_frame_rect(handle: isize, rect: FrameRect) -> PlatformResult<()> {
    let hwnd = to_hwnd(handle)?;
    let mut placement = WINDOWPLACEMENT {
        length: core::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..WINDOWPLACEMENT::default()
    };
    // SAFETY (read half): GetWindowPlacement reads only `length`, set above to
    // exactly `size_of::<WINDOWPLACEMENT>()` as the API demands, and writes its
    // fields into a live repr(C) local that outlives the call. The HWND is the
    // one `to_hwnd` accepted from IsWindow; a window destroyed since makes the
    // call fail rather than fault.
    unsafe { GetWindowPlacement(hwnd, &mut placement) }
        .map_err(|error| win32_error("GetWindowPlacement", error))?;
    // Corner-based in, corner-based out: width and height are saturated
    // additions so an extreme stored rect cannot wrap into negative extents.
    placement.rcNormalPosition = RECT {
        left: rect.x,
        top: rect.y,
        right: rect.x.saturating_add(rect.width()),
        bottom: rect.y.saturating_add(rect.height()),
    };
    // SAFETY (write half): SetWindowPlacement reads the same local - every
    // field of WINDOWPLACEMENT is by value (RECT and POINT), so there is no
    // nested pointer for Win32 to follow - and applies it to the HWND Win32
    // re-validates, failing closed on a window that died since the check. The
    // struct is passed by const pointer and is not used again after the call.
    unsafe { SetWindowPlacement(hwnd, &placement) }
        .map_err(|error| win32_error("SetWindowPlacement", error))
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
        restore_frame_rect, scale_for_rect, scale_of_monitor, set_frame_rect, show_state,
        work_area_for_rect, work_area_of,
    };
    use crate::{FrameRect, PlatformError, PlatformResult, ShowState, WindowBackend};
    use ::windows::Win32::Foundation::RECT;
    use ::windows::Win32::Graphics::Gdi::HMONITOR;
    use ::windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SET_WINDOW_POS_FLAGS, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN,
        SM_XVIRTUALSCREEN, SW_HIDE, SW_SHOW, SW_SHOWDEFAULT, SW_SHOWMAXIMIZED, SW_SHOWMINIMIZED,
        SW_SHOWMINNOACTIVE, SW_SHOWNORMAL, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOZORDER,
        WPF_RESTORETOMAXIMIZED,
    };

    #[test]
    fn placement_touches_nothing_else_and_never_blocks_the_caller() {
        // A dropped SWP_NOZORDER un-pins the window as a side effect of a move; a
        // dropped SWP_NOACTIVATE steals focus; a missing SWP_ASYNCWINDOWPOS parks
        // the calling thread inside user32 until the window's owner pumps - which
        // is a reproduced hang, not a hypothetical. All three are invisible in
        // review, so the exact combination is pinned: an assertion about the code,
        // not about timing. The reads above take no flags at all, so there is no
        // read path for the async bit to leak onto.
        let expected =
            SET_WINDOW_POS_FLAGS(SWP_NOZORDER.0 | SWP_NOACTIVATE.0 | SWP_ASYNCWINDOWPOS.0);
        assert_eq!(PLACEMENT_FLAGS, expected);
        for flag in [SWP_NOZORDER, SWP_NOACTIVATE, SWP_ASYNCWINDOWPOS] {
            assert!(PLACEMENT_FLAGS.contains(flag), "missing {flag:?}");
        }
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

    /// The mapper is the whole of this crate's reading of `showCmd`, so its three
    /// answers are pinned here - no window, no desktop, no `GetWindowPlacement`.
    ///
    /// The maximised case is the one that matters: the test is EQUALITY against
    /// `SW_SHOWMAXIMIZED`, the same test gpui itself uses. `WPF_RESTORETOMAXIMIZED`
    /// is a bit of a DIFFERENT field (`flags`) and says where the window would go on
    /// restore, so a window that was un-maximised long ago still carries it - reading
    /// it would latch a stale "maximised" into the saved session. Its value (2) IS
    /// the iconic `showCmd`, so it is asserted here as not reaching Maximized.
    #[test]
    fn the_maximised_show_cmd_is_the_only_one_that_maps_to_maximized() {
        assert_eq!(
            show_state(SW_SHOWMAXIMIZED.0 as u32),
            ShowState::Maximized,
            "SW_SHOWMAXIMIZED ({}) is the maximised state",
            SW_SHOWMAXIMIZED.0,
        );
        assert_eq!(
            show_state(WPF_RESTORETOMAXIMIZED.0),
            ShowState::Unknown,
            "the restore target (flags bit {}) is not a show state",
            WPF_RESTORETOMAXIMIZED.0,
        );
    }

    /// The two codes that describe an ordinary on-screen window both map to Normal:
    /// `SW_SHOWNORMAL` (also `SW_RESTORE`'s value) and `SW_SHOW`.
    #[test]
    fn an_ordinary_window_maps_to_normal() {
        for cmd in [SW_SHOWNORMAL, SW_SHOW] {
            assert_eq!(
                show_state(cmd.0 as u32),
                ShowState::Normal,
                "{cmd:?} is an ordinary window",
            );
        }
    }

    /// Minimised has no case, by contract: `SW_SHOWMINIMIZED` (the iconic state) and
    /// every other code this mapper does not name answer `Unknown`. Calling an icon
    /// "Normal" would claim a case this enum does not have, and calling it
    /// "Maximized" would be worse - `Unknown` decides nothing, which is the point.
    #[test]
    fn an_iconic_and_an_unnamed_code_map_to_unknown() {
        assert_eq!(
            show_state(SW_SHOWMINIMIZED.0 as u32),
            ShowState::Unknown,
            "minimised is not a case of its own",
        );
        for cmd in [SW_HIDE, SW_SHOWMINNOACTIVE, SW_SHOWDEFAULT] {
            assert_eq!(
                show_state(cmd.0 as u32),
                ShowState::Unknown,
                "{cmd:?} names nothing this mapper claims",
            );
        }
        assert_eq!(show_state(u32::MAX), ShowState::Unknown);
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

    /// The scale seam's refusals are the same two diagnoses the work-area seam
    /// makes: no monitor at all is `NoMonitor`, a monitor that stopped existing
    /// is a `Win32` mapping carrying the OS code - never a fabricated 0.0 or
    /// 1.0 scale, because a default scale is exactly the derived lie this read
    /// exists to avoid.
    #[test]
    fn a_null_and_a_dead_monitor_are_typed_failures_not_scales() {
        let null = scale_of_monitor(HMONITOR(core::ptr::null_mut()));
        assert!(matches!(null, Err(PlatformError::NoMonitor)), "{null:?}");
        // 4-byte aligned and non-null, so it clears the guard and reaches the
        // API, but the meaningful bits of an HMONITOR are 32-bit: with the
        // 64-bit sign bit set this cannot name a monitor on any station.
        let dead = HMONITOR(isize::MIN as *mut core::ffi::c_void);
        let result = scale_of_monitor(dead);
        assert!(
            matches!(&result, Err(PlatformError::Win32 { api, .. }) if *api == "GetDpiForMonitor"),
            "{result:?}"
        );
    }

    /// The scale of the monitor that owns a known rect is a measurement, not
    /// a constant: finite and positive on any live desktop, 1.0 on a 100% box.
    /// The measured number is printed so it is on the record.
    #[test]
    fn the_scale_of_a_known_rect_is_a_finite_positive_measurement() {
        let primary = primary_work_area().expect("a primary monitor exists");
        let scale = scale_for_rect(primary).expect("the rect has a monitor");
        eprintln!("scale measured for the primary monitor: {scale}");
        assert!(scale.is_finite(), "{scale} is not a scale");
        assert!(scale > 0.0, "{scale} is not a scale");
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
