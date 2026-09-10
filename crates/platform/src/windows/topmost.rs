//! Above every other window, or not. One call, and no opinion about which.

use ::windows::Win32::UI::WindowsAndMessaging::{
    HWND_NOTOPMOST, HWND_TOPMOST, SET_WINDOW_POS_FLAGS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SetWindowPos,
};

use super::{to_hwnd, win32_error};
use crate::PlatformResult;

/// `SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE`, spelled as one const so a reviewer
/// can see the three bits that confine this call to a z-order change and nothing
/// else.
const TOPMOST_FLAGS: SET_WINDOW_POS_FLAGS =
    SET_WINDOW_POS_FLAGS(SWP_NOMOVE.0 | SWP_NOSIZE.0 | SWP_NOACTIVATE.0);

/// Puts `handle` in the topmost band (`on`) or takes it out of it.
///
/// Whether this window *should* be topmost - user intent, the persisted flag, one
/// window pinned at a time, the shortcut, the menu tick - is decided above this
/// crate: `api` routes the command, `core` holds the state, this performs the move.
pub fn set_topmost(handle: isize, on: bool) -> PlatformResult<()> {
    let hwnd = to_hwnd(handle)?;
    let insert_after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
    // SAFETY: SetWindowPos receives the HWND that `to_hwnd` accepted from IsWindow
    // at check time. The window may have been destroyed since; SetWindowPos
    // re-validates the handle internally and fails closed, which maps to
    // [`PlatformError::Win32`] - not to `InvalidHandle`, and not to undefined
    // behaviour. `insert_after` is one of the two documented
    // well-known window handles (-1 topmost, -2 not-topmost) and never something this
    // side dereferences; no pointer crosses at all, and TOPMOST_FLAGS carries
    // SWP_NOMOVE and SWP_NOSIZE, which make the four zero coordinates ignored.
    // The returned Result is mapped, never unwrapped.
    unsafe { SetWindowPos(hwnd, Some(insert_after), 0, 0, 0, 0, TOPMOST_FLAGS) }
        .map_err(|error| win32_error("SetWindowPos", error))
}

#[cfg(test)]
mod tests {
    use super::{TOPMOST_FLAGS, set_topmost};
    use crate::{PlatformError, PlatformResult};
    use ::windows::Win32::UI::WindowsAndMessaging::{
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    };

    #[test]
    fn a_bad_handle_is_refused_and_never_reaches_the_z_order() {
        // A USER handle value is 4-byte aligned, and the meaningful bits of an HWND
        // are 32-bit; every value below is misaligned or has the 64-bit sign bit set,
        // so none can name a live window and no other process can flake this. No
        // window is created, focused or waited on here.
        for on in [true, false] {
            for handle in [0, 0x1234_5679, 0x0000_000f, isize::MIN + 1] {
                let result: PlatformResult<()> = set_topmost(handle, on);
                assert!(
                    matches!(result, Err(PlatformError::InvalidHandle)),
                    "handle {handle:#x} on={on}: {result:?}",
                );
            }
        }
    }

    #[test]
    fn the_flags_move_z_order_and_nothing_else() {
        // A dropped SWP_ bit is invisible in review and silently moves, resizes or
        // repaints the window, so the exact combination is pinned here.
        assert_eq!(TOPMOST_FLAGS, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        for flag in [SWP_NOMOVE, SWP_NOSIZE, SWP_NOACTIVATE] {
            assert!(TOPMOST_FLAGS.contains(flag), "missing {flag:?}");
        }
        for flag in [SWP_SHOWWINDOW, SWP_FRAMECHANGED, SWP_NOCOPYBITS] {
            assert!(!TOPMOST_FLAGS.contains(flag), "carries {flag:?}");
        }
    }
}
