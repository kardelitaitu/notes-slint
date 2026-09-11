//! Above every other window, or not. One call, and no opinion about which.

use ::windows::Win32::UI::WindowsAndMessaging::{
    HWND_NOTOPMOST, HWND_TOPMOST, SET_WINDOW_POS_FLAGS, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, WS_EX_TOPMOST,
};

use std::thread;
use std::time::{Duration, Instant};

use ::windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GetWindowLongPtrW};

use super::{to_hwnd, win32_error};
use crate::PinOutcome;

/// How long the style read-back waits for an async reband to land before it
/// declares NotApplied. A pumping owner lands the band in ~1-2 ms (measured
/// by the geometry_live probe); a parked owner never lands it, which is
/// exactly the failure the verdict exists to report. The wait must be long
/// enough for the first truth and short enough not to stall the caller.
const BAND_LAND_WINDOW: Duration = Duration::from_millis(10);

/// `SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS`, spelled as one
/// const so a reviewer can see the bits that confine this call to a z-order change
/// and nothing else. `SWP_ASYNCWINDOWPOS` is the anti-deadlock bit: without it, a
/// `SetWindowPos` whose caller and window are attached to different input queues
/// blocks the caller until the thread that owns the window pumps - which can be
/// forever. It also means this returns before the reband has landed; the caller
/// obligation that follows is documented on
/// [`crate::WindowBackend::set_frame_rect`].
const TOPMOST_FLAGS: SET_WINDOW_POS_FLAGS =
    SET_WINDOW_POS_FLAGS(SWP_NOMOVE.0 | SWP_NOSIZE.0 | SWP_NOACTIVATE.0 | SWP_ASYNCWINDOWPOS.0);

/// Puts `handle` in the topmost band (`on`) or takes it out of it.
///
/// Whether this window *should* be topmost - user intent, the persisted flag, one
/// window pinned at a time, the shortcut, the menu tick - is decided above this
/// crate: `api` routes the command, `core` holds the state, this performs the move.
///
/// Issued with `SWP_ASYNCWINDOWPOS` (see [`TOPMOST_FLAGS`]): this returns before
/// the band has actually switched, so a placement read straight back may still
/// show the old band.
pub fn set_topmost(handle: isize, on: bool) -> PinOutcome {
    let hwnd = match to_hwnd(handle) {
        Ok(hwnd) => hwnd,
        Err(err) => return PinOutcome::Failed(err),
    };
    let insert_after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
    // SAFETY: SetWindowPos receives the HWND that `to_hwnd` accepted from IsWindow
    // at check time. The window may have been destroyed since; SetWindowPos
    // re-validates the handle internally and fails closed, which maps to
    // [`PlatformError::Win32`] - not to `InvalidHandle`, and not to undefined
    // behaviour. `insert_after` is one of the two documented
    // well-known window handles (-1 topmost, -2 not-topmost) and never something this
    // side dereferences; no pointer crosses at all, and TOPMOST_FLAGS carries
    // SWP_NOMOVE and SWP_NOSIZE, which make the four zero coordinates ignored.
    // SWP_ASYNCWINDOWPOS changes only liveness - the call posts the reband instead
    // of blocking on the owner's pump - never what is written. The returned
    // Result is mapped into the verdict, never unwrapped.
    let call = unsafe { SetWindowPos(hwnd, Some(insert_after), 0, 0, 0, 0, TOPMOST_FLAGS) };
    if let Err(error) = call {
        return PinOutcome::Failed(win32_error("SetWindowPos", error));
    }
    // The FFI answer is not the verdict: a call can succeed and change nothing -
    // the documented hidden-window case, where an async reband never lands - so
    // the window's own style is read back, and because the reband is ASYNC it is
    // read back until it lands or until BAND_LAND_WINDOW expires. A single
    // immediate read would misreport every cross-queue reband (measured: the bit
    // becomes visible ~1.4 ms after the call returns, on the owner's pump).
    let deadline = Instant::now() + BAND_LAND_WINDOW;
    loop {
        // SAFETY: GetWindowLongPtrW is a pure style query on the HWND `to_hwnd`
        // accepted at check time and that SetWindowPos just answered for; it
        // reads one integer out of the window's USER handle record, writes
        // nothing on this side, and the index is the documented GWL_EXSTYLE.
        let actual = style_is_topmost(unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) });
        if actual == on {
            return PinOutcome::Applied;
        }
        if Instant::now() >= deadline {
            return PinOutcome::NotApplied {
                expected: on,
                actual,
            };
        }
        // Yield-spin rather than sleep: the landing is a ~1 ms event and the
        // wait is bounded, so a spin gives microsecond resolution without a
        // timer-granularity overshoot.
        thread::yield_now();
    }
}

/// The style-word half of the verdict, so the bit comparison is testable
/// without a window: `WS_EX_TOPMOST` (0x8) set in the extended style word.
fn style_is_topmost(style: isize) -> bool {
    style & WS_EX_TOPMOST.0 as isize != 0
}

#[cfg(test)]
mod tests {
    use super::{TOPMOST_FLAGS, set_topmost, style_is_topmost};
    use crate::{PinOutcome, PlatformError};
    use ::windows::Win32::UI::WindowsAndMessaging::{
        SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOMOVE,
        SWP_NOSIZE, SWP_SHOWWINDOW,
    };

    #[test]
    fn a_bad_handle_is_refused_and_never_reaches_the_z_order() {
        // A USER handle value is 4-byte aligned, and the meaningful bits of an HWND
        // are 32-bit; every value below is misaligned or has the 64-bit sign bit set,
        // so none can name a live window and no other process can flake this. No
        // window is created, focused or waited on here. The refusal arrives as the
        // verdict's Failed arm carrying the same typed error as before.
        for on in [true, false] {
            for handle in [0, 0x1234_5679, 0x0000_000f, isize::MIN + 1] {
                let verdict = set_topmost(handle, on);
                assert!(
                    matches!(&verdict, PinOutcome::Failed(PlatformError::InvalidHandle)),
                    "handle {handle:#x} on={on}: {verdict:?}",
                );
            }
        }
    }

    /// The style-word comparison is the whole of the read-back, so it is
    /// pinned without a window: the bit, its absence, and words that carry
    /// other bits beside it.
    #[test]
    fn the_style_word_answers_only_its_own_bit() {
        assert!(!style_is_topmost(0), "no style, no pin");
        assert!(style_is_topmost(0x8), "WS_EX_TOPMOST alone");
        assert!(
            style_is_topmost(0x8 | 0x10 | 0x80),
            "topmost among other bits"
        );
        assert!(!style_is_topmost(0x10 | 0x80), "other bits, no pin");
        assert!(!style_is_topmost(isize::MIN), "sign bit is not the pin");
        assert!(style_is_topmost(-1), "all-ones carries every bit");
    }

    #[test]
    fn the_flags_change_only_z_order_and_never_block() {
        // A dropped SWP_ bit is invisible in review and silently moves, resizes or
        // repaints the window; a missing SWP_ASYNCWINDOWPOS parks the calling
        // thread inside user32 until the window's owner pumps - a reproduced hang,
        // not a hypothetical. So the exact combination is pinned here: an
        // assertion about the code, not about timing.
        assert_eq!(
            TOPMOST_FLAGS,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS
        );
        for flag in [SWP_NOMOVE, SWP_NOSIZE, SWP_NOACTIVATE, SWP_ASYNCWINDOWPOS] {
            assert!(TOPMOST_FLAGS.contains(flag), "missing {flag:?}");
        }
        for flag in [SWP_SHOWWINDOW, SWP_FRAMECHANGED, SWP_NOCOPYBITS] {
            assert!(!TOPMOST_FLAGS.contains(flag), "carries {flag:?}");
        }
    }
}
