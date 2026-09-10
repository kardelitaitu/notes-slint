//! The Win32 side of the seams: functions that take an `isize` HWND, make one API
//! call, and translate the failure. Nothing else.
//!
//! This is the only module in the crate that allows `unsafe`. The crate manifest
//! spells the lint as `deny` (widening the workspace `forbid`, which would leave no
//! way to call Win32 at all), and the allow is scoped to this module tree, so
//! `unsafe` cannot drift into `geometry` or the trait: there it is still a hard
//! error rather than something to catch in review. Every unsafe block below carries
//! its own `// SAFETY:` comment naming the call and the invariant that makes it
//! valid (AGENTS.md).

#![allow(unsafe_code)]

pub mod monitors;
pub mod topmost;

use ::windows::Win32::Foundation::HWND;
use ::windows::Win32::UI::WindowsAndMessaging::IsWindow;

use crate::{FrameRect, PlatformError, PlatformResult, WindowBackend};

/// The Win32 implementation of [`WindowBackend`].
///
/// A unit struct: it holds no handle and owns no window, so there is nothing to
/// leak, no thread affinity to record, and no ordering constraint on when it is
/// used. Construct it, hand it to `api`, call it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Backend;

impl WindowBackend for Backend {
    fn set_topmost(&mut self, handle: isize, on: bool) -> PlatformResult<()> {
        topmost::set_topmost(handle, on)
    }

    fn frame_rect(&self, handle: isize) -> PlatformResult<FrameRect> {
        monitors::frame_rect(handle)
    }

    fn set_frame_rect(&mut self, handle: isize, r: FrameRect, scale: f32) -> PlatformResult<()> {
        monitors::set_frame_rect(handle, r, scale)
    }

    fn primary_work_area(&self) -> PlatformResult<FrameRect> {
        monitors::primary_work_area()
    }
}

/// Turns an `isize` into an `HWND`, or refuses it.
///
/// This guard is the contract: a null, stale, forged or already-destroyed handle
/// becomes [`PlatformError::InvalidHandle`] *before* any call that would use it to
/// look up window state, which is what keeps a bad handle an error instead of an
/// access violation.
pub(crate) fn to_hwnd(handle: isize) -> PlatformResult<HWND> {
    if handle == 0 {
        return Err(PlatformError::InvalidHandle);
    }
    let hwnd = HWND(handle as *mut core::ffi::c_void);
    // SAFETY: IsWindow is a lookup in the window table, not a dereference - Win32
    // accepts any value-shaped HWND, including 0x1 and isize::MIN, and returns FALSE
    // for anything that is not a live window. The pointer above is built from an
    // isize already known to be non-zero and is never read through on this side.
    if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        Ok(hwnd)
    } else {
        Err(PlatformError::InvalidHandle)
    }
}

/// The one error translation, so no call site invents its own message shape.
pub(crate) fn win32_error(api: &'static str, error: ::windows::core::Error) -> PlatformError {
    PlatformError::Win32 {
        api,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::Backend;
    use crate::{FrameRect, PlatformError, PlatformResult, WindowBackend};

    /// Runs the three handle-taking seams and returns their verdicts. None of the
    /// handles used here can name a live window: kernel handles are 4-byte aligned,
    /// and every value the tests pass is misaligned or out of range. So the
    /// assertions are exact and cannot be flaked by another process on the desktop.
    fn all_handle_seams(backend: &mut Backend, handle: isize) -> Vec<PlatformResult<()>> {
        vec![
            backend.set_topmost(handle, true),
            backend.frame_rect(handle).map(|_| ()),
            backend.set_frame_rect(handle, FrameRect::new(0, 0, 10, 10), 1.0),
        ]
    }

    #[test]
    fn a_null_handle_is_invalid_handle_not_a_panic() {
        let mut backend = Backend;
        for result in all_handle_seams(&mut backend, 0) {
            assert!(
                matches!(result, Err(PlatformError::InvalidHandle)),
                "expected InvalidHandle, got {result:?}"
            );
        }
    }

    #[test]
    fn a_garbage_handle_is_refused_before_it_is_used() {
        let mut backend = Backend;
        for handle in [1, 2, 3, -1, 0x4000_0002, isize::MIN] {
            for result in all_handle_seams(&mut backend, handle) {
                assert!(
                    matches!(result, Err(PlatformError::InvalidHandle)),
                    "handle {handle:#x}: expected InvalidHandle, got {result:?}"
                );
            }
        }
    }

    #[test]
    fn the_backend_is_send_and_the_trait_is_object_safe() {
        // api holds a `Box<dyn WindowBackend>`. If either property regresses, api has
        // to depend on the windows crate instead - which is the seam disappearing.
        fn assert_object_safe<T: WindowBackend + Send + ?Sized>() {}
        assert_object_safe::<Backend>();
        assert_object_safe::<dyn WindowBackend>();
    }
}
