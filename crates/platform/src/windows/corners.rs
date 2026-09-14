//! The shape of a window's corners, asked of the component that actually draws them.
//!
//! On Windows 11 that component is the desktop window manager, and it already owns the radius,
//! the antialiasing and the shadow. This module does not draw a corner and does not decide when a
//! window should be rounded: it makes one attribute call per request and reports what the OS
//! answered. Whether THIS window should be round right now - normal versus maximised, supported
//! versus not - is decided above this crate, by the bridge, through `api`.
//!
//! Why an attribute and not the transparent-window trick: alpha from a window that asks for
//! `background: transparent` does not survive this workspace's renderer (measured 2026-09-14: a
//! `no-frame` Slint window on `renderer-software` composites its transparent band as `#000000`,
//! while an opaque control in the same pixels reads its own colour). The measurement, and the
//! alternatives, are recorded in
//! `.agents/notes/proposed/2026-09-14-rounded-corners-dwm.md`.

use ::windows::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND,
    DwmGetWindowAttribute, DwmSetWindowAttribute,
};

use super::{to_hwnd, win32_error};
use crate::{PlatformError, PlatformResult};

/// `cbAttribute` for this attribute: `DWM_WINDOW_CORNER_PREFERENCE` is a `#[repr(transparent)]`
/// newtype over `i32`, so the four bytes the API is told to read are exactly the size of the
/// value being passed. Computed, not written as a literal, so a windows-rs change that re-sizes
/// the type cannot leave this call reading past it.
const PREFERENCE_BYTES: u32 = size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32;

/// Asks DWM to round `handle`'s corners (`round`) or to square them off.
///
/// `Ok(())` means the window now REPORTS the requested preference, which is not the same sentence
/// as "the corners are round on the screen" - see the read-back below for what this can and cannot
/// see. `Err` is the refusal, and the two cases a caller should expect from it are a bad handle
/// and an OS that does not know attribute 33: `DWMWA_WINDOW_CORNER_PREFERENCE` is documented as
/// Windows 11+, so on Windows 10 - R11's support floor - this returns
/// [`PlatformError::Win32`] and the window stays exactly as it was. Nothing here papers over
/// that, and nothing here decides it is fine.
pub fn set_corner_rounding(handle: isize, round: bool) -> PlatformResult<()> {
    let hwnd = to_hwnd(handle)?;
    let asked = if round {
        DWMWCP_ROUND
    } else {
        DWMWCP_DONOTROUND
    };
    // SAFETY: `hwnd` is the handle `to_hwnd` accepted from `IsWindow` at check time; the window
    // may have been destroyed since, and DwmSetWindowAttribute re-validates it internally and
    // fails closed into [`PlatformError::Win32`], never undefined behaviour. The pointer argument
    // addresses a live, correctly-sized local (`asked`) for the duration of the call, and 33 is
    // the attribute whose documented payload IS a `DWM_WINDOW_CORNER_PREFERENCE` - the API reads
    // four bytes and writes nothing through it. `Result` is mapped, never unwrapped.
    if let Err(error) = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            core::ptr::from_ref(&asked).cast(),
            PREFERENCE_BYTES,
        )
    } {
        return Err(win32_error("DwmSetWindowAttribute", error));
    }
    // The read-back, and the honest half-sentence about it. This mirrors `set_topmost`'s rule
    // ("the FFI answer is not the verdict") but NOT its wait: topmost needed a bounded poll
    // because `SWP_ASYNCWINDOWPOS` posts the reband and the documented hidden-window case is a
    // change that never lands (measured, topmost.rs:16-35). This call is neither async nor
    // deferred - it stores a per-window attribute - so a second read is enough, and a spin here
    // would be a loop copied for the look of diligence.
    //
    // What the read-back DOES catch: an OS that stored something other than what was asked,
    // including a preference some other party owns (a policy, a theme) reading back as
    // `DWMWCP_DEFAULT`. What it cannot catch, and the reason `Ok(())` is not "the corners are
    // round": DWM reports the preference it holds even in the cases where it then declines to
    // draw it - maximised with `ROUND`, or a session without composition. Those are the ones the
    // bridge answers by asking for `DONOTROUND` at the right moments, and the ones an eye-pass
    // has to confirm.
    let mut actual = DWM_WINDOW_CORNER_PREFERENCE::default();
    // SAFETY: same handle and same lifetime argument as the call above, with a writable local of
    // the identical type and size; the API writes at most four bytes into it and only on success.
    if let Err(error) = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            core::ptr::from_mut(&mut actual).cast(),
            PREFERENCE_BYTES,
        )
    } {
        return Err(win32_error("DwmGetWindowAttribute", error));
    }
    if actual == asked {
        Ok(())
    } else {
        // The typed refusal this crate already has, with the call that produced the
        // contradiction named. The message is THIS crate's sentence rather than the OS's, which
        // [`PlatformError::Win32`]'s field doc now says out loud: there is no OS text to pass
        // through when a call succeeds and the window disagrees, and inventing an HRESULT to fit
        // a prose field would be the worse lie.
        Err(PlatformError::Win32 {
            api: "DwmGetWindowAttribute",
            message: format!(
                "the window reports {actual:?}, not the requested {asked:?} (attribute {})",
                DWMWA_WINDOW_CORNER_PREFERENCE.0
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{PREFERENCE_BYTES, set_corner_rounding};
    use crate::PlatformError;

    #[test]
    fn a_bad_handle_is_refused_before_dwm_is_asked() {
        // Same shape as topmost's test and for the same reason: a USER handle value is 4-byte
        // aligned, so every value below names no window on this station and no other process can
        // flake this by creating one. The refusal must come from `IsWindow`, not from DWM.
        for round in [true, false] {
            for handle in [0, 0x1234_5679, 0x0000_000f, isize::MIN + 1] {
                let verdict = set_corner_rounding(handle, round);
                assert!(
                    matches!(verdict, Err(PlatformError::InvalidHandle)),
                    "handle {handle:#x} round={round}: {verdict:?}",
                );
            }
        }
    }

    #[test]
    fn the_attribute_is_handed_the_bytes_it_documents() {
        // `cbAttribute` too large reads past the local, too small is refused or zero-padded into
        // a wrong preference. Both are invisible in review, so the size is pinned: DWM documents
        // this attribute as a 4-byte enum, and a windows-rs that changed the type would change
        // this number and fail here rather than at the call site.
        assert_eq!(PREFERENCE_BYTES, 4);
        assert_eq!(PREFERENCE_BYTES, size_of::<i32>() as u32);
    }
}
