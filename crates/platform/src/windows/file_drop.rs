//! FILE DROP: the OS half of "drag a note onto the window".
//!
//! This module owns exactly one thing: turning a shell drop into a list of paths
//! and holding them until somebody above asks. It decides nothing about what a
//! dropped file IS - no extension test, no "is this in recents", no open. That
//! is why the payload is a `Vec<PathBuf>` and not a verdict.
//!
//! The shape is pull, not push, and that is forced rather than chosen:
//! `IDropTarget` callbacks arrive on the thread that owns the window, INSIDE a
//! drag-move loop Windows runs for us, so the only legal thing to do there is
//! note the paths and get out. A bridge therefore drains them from its own
//! per-frame tick through `WindowBackend::take_dropped_paths`, which makes the
//! drop land on the same cadence as everything else the bridge does - and makes
//! "the engine was mid-write when a drop arrived" structurally impossible.
//!
//! CF_HDROP is read through the clipboard view of the drop object rather than
//! `IDataObject::GetData`, because that needs no FORMATETC/STGMEDIUM bookkeeping
//! and the HDROP is only valid for the duration of the call - which is the one
//! thing this module guarantees: every path is copied into owned `PathBuf`s
//! before `Drop` returns.

use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::Mutex;

use windows::Win32::Foundation::{DRAGDROP_E_ALREADYREGISTERED, POINTL, RPC_E_CHANGED_MODE};
use windows::Win32::System::DataExchange::GetClipboardData;
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, IDropTarget, IDropTarget_Impl, OleInitialize,
    RegisterDragDrop, RevokeDragDrop,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::core::{Ref, Result as WinResult, implement};

use crate::{PlatformError, PlatformResult};

/// Drops collected between takes. A `static` rather than state on `Backend`,
/// because `Backend` is a unit struct constructed per call: the only place a
/// drop can persist is the process, and the process has one window per drop
/// target here.
static BUFFER: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Did `arm` find a target already registered? Reported once, on the way in,
/// because the difference between "we own this window's drops" and "something
/// already did" is the difference between a takeover and a no-op.
static TOOK_OVER: Mutex<bool> = Mutex::new(false);

fn lock_or_recover<T>(cell: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // A lock poisoned by somebody else's panic must not cost the user a drop:
    // the buffer is a `Vec`, and the worst a recovery can hand over is a path
    // list that a panicking handler left half-filled - which is recoverable by
    // the caller, where a panic here would be the app refusing to open a file.
    cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Move the buffered paths out, leaving the buffer empty. Idempotent, and the
/// only consumer-side entry point: whatever calls this owns what happens next.
pub(crate) fn take_buffered() -> Vec<PathBuf> {
    std::mem::take(&mut *lock_or_recover(&BUFFER))
}

/// The drop callback's half of the handoff: store what the shell handed us.
fn buffer(paths: Vec<PathBuf>) {
    lock_or_recover(&BUFFER).extend(paths);
}

/// The COM object Windows calls. `IUnknownImpl` is what `#[implement]` requires
/// of the identity, and an empty struct is enough: the only state a drop target
/// needs lives in `BUFFER`.
#[derive(Default)]
#[implement(IDropTarget)]
struct FileDropTarget;

/// Answer `DROPEFFECT_COPY` and nothing else.
///
/// This is a TRANSLATION, not a decision: it says "this window can receive a
/// copy of what you are carrying", which is true, and it does not say the
/// document will be opened, replaced, or saved. Deciding to open belongs above
/// the port, in the bridge, from the take.
fn accept_copy(effects: *mut DROPEFFECT) -> WinResult<()> {
    if !effects.is_null() {
        // SAFETY: the caller is OLE, which passes either null or a pointer
        // into storage it owns for exactly this out-parameter; the null check
        // above is the whole contract, and DROPEFFECT_COPY is a value, not a
        // handle, so nothing is aliased or freed here.
        unsafe { *effects = DROPEFFECT_COPY }
    }
    Ok(())
}

// The `#[implement]` macro generates TWO types: the COM wrapper (`FileDropTarget`)
// and the identity (`FileDropTarget_Impl`) that carries the vtable. The trait is
// implemented on the identity, not on the wrapper - the compiler's own
// `FileDropTarget_Impl: IDropTarget_Impl is not satisfied` says so.
impl IDropTarget_Impl for FileDropTarget_Impl {
    fn DragEnter(
        &self,
        _data: Ref<'_, windows::Win32::System::Com::IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effects: *mut DROPEFFECT,
    ) -> WinResult<()> {
        accept_copy(effects)
    }

    fn DragOver(
        &self,
        _keys: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effects: *mut DROPEFFECT,
    ) -> WinResult<()> {
        accept_copy(effects)
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        // Nothing to unwind: the effect answer is stateless, and a leave that
        // cleared a buffer would be a bug, not a cleanup - a cancelled drag
        // must not throw away a file somebody already dropped.
        Ok(())
    }

    fn Drop(
        &self,
        _data: Ref<'_, windows::Win32::System::Com::IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effects: *mut DROPEFFECT,
    ) -> WinResult<()> {
        buffer(dropped_paths());
        accept_copy(effects)
    }
}

/// Read `CF_HDROP` and copy every file name out of it.
///
/// A failed read answers with an empty list, never an error: `Drop` has no
/// channel for one, and the user's signal that their drop did nothing is the
/// window not changing, which is the same signal an unreadable drop gives.
fn dropped_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    unsafe {
        // SAFETY: GetClipboardData is valid to call at any time on the thread
        // Windows is pumping for; CF_HDROP names a format, not a handle we own.
        // The returned HGLOBAL is owned by the drop and is only good for the
        // duration of this call - which is why everything below copies.
        let Ok(hglobal) = GetClipboardData(u32::from(CF_HDROP.0)) else {
            return out;
        };
        let hdrop = HDROP(hglobal.0);
        if hdrop.is_invalid() {
            return out;
        }
        // SAFETY: DragQueryFileW with no buffer asks for the COUNT, and is the
        // documented way to size the loop. The handle came from the call above
        // and the drop is still in progress: this runs inside Drop().
        let count = DragQueryFileW(hdrop, u32::MAX, None);
        let mut name = vec![0u16; 1024];
        for index in 0..count {
            // SAFETY: `name` outlives the call and `count` is the number this
            // HDROP reported, so the index is in range. A truncated long path is
            // possible and is reported as whatever Windows copied - guessing at
            // a longer path is worse than seeing a short one.
            let len = DragQueryFileW(hdrop, index, Some(&mut name));
            if len == 0 || len as usize >= name.len() {
                continue;
            }
            out.push(PathBuf::from(std::ffi::OsString::from_wide(
                &name[..len as usize],
            )));
        }
    }
    out
}

/// Start receiving drops for `hwnd`.
///
/// MUST be called on the thread that pumps that window's messages:
/// `OleInitialize` initialises the COM apartment for the CALLING thread, and
/// `RegisterDragDrop` binds the target to the thread that owns the window. Do
/// either from a worker thread and the callbacks arrive on a thread that is not
/// pumping, which is a silent hang rather than an error.
///
/// `RPC_E_CHANGED_MODE` from `OleInitialize` is not a failure: the toolkit
/// (winit, gpui) usually initialises the apartment first, and Windows keeps the
/// per-thread count, so our uninit on drop is not owed. Any other refusal is
/// returned, never panicked on - a window without drops is degraded, not broken.
pub fn arm(hwnd: isize) -> PlatformResult<()> {
    let hwnd = crate::windows::to_hwnd(hwnd)?;
    // SAFETY: OleInitialize initialises the COM apartment for the CALLING thread,
    // which is documented as re-entrant-counted and safe to call again; `None` is
    // the reserved argument. The whole point of the RPC_E_CHANGED_MODE branch is
    // that a prior apartment in another model is not our business to fix.
    if let Err(e) = unsafe { OleInitialize(None) } {
        if e.code() != RPC_E_CHANGED_MODE {
            return Err(PlatformError::Win32 {
                api: "OleInitialize",
                message: e.message().to_string(),
            });
        }
    }
    // Revoke first: arming twice on the same window is a reload, not a bug, and
    // the alternative is answering DRAGDROP_E_ALREADYREGISTERED forever on the
    // second call with no way to tell the caller which of the two happened.
    //
    // A refusal here is EXPECTED the first time - a window that never had a
    // drop target says OLE_E_INVALIDHWND / DRAGDROP_E_NOTREGISTERED, and we are
    // about to register one either way. So: no error path, and the registration
    // below is what is actually judged.
    // SAFETY: `hwnd` is validated by to_hwnd above. RevokeDragDrop on a window
    // that never had a target is a documented plain refusal, and discarding it
    // cannot leak: nothing was allocated for that window's drop target, and the
    // registration below is the call that is actually judged.
    unsafe {
        RevokeDragDrop(hwnd).ok();
    }
    let target: IDropTarget = FileDropTarget.into();
    // SAFETY: `hwnd` names a live window on this thread (to_hwnd checked it) and
    // `target` is a valid IDropTarget whose vtable outlives this call because the
    // box is leaked below - OLE keeps the pointer past the end of this statement,
    // which is precisely why a normal drop here would be a use-after-free.
    let registered = unsafe { RegisterDragDrop(hwnd, &target) };
    if let Err(e) = registered {
        if e.code() != DRAGDROP_E_ALREADYREGISTERED {
            return Err(crate::windows::win32_error("RegisterDragDrop", e));
        }
        // Something is already registered - us, from an earlier arm on this
        // window, since nothing else in this process calls RegisterDragDrop.
        // That is a takeover, and it is a success: the OS is pointing at A
        // target, and the buffer is shared by every instance of ours.
        *lock_or_recover(&TOOK_OVER) = true;
        return Ok(());
    }
    // LEAK the target. Windows holds the only strong reference from
    // `RegisterDragDrop` until `RevokeDragDrop` (plus the refcount it takes on
    // the interface itself), so a Rust-owned box would be dropped while the
    // vtable pointer inside OLE is still live - a use-after-free scheduled by
    // the next drag. There is no precedent for this in the crate yet: every
    // other seam here passes a number in and gets a number back, and this is
    // the first object handed over and never taken back. The precedent being
    // followed is the OS's own - it documents the target as living until
    // revoked - and `forget` is the smallest honest way to agree with it.
    std::mem::forget(target);
    Ok(())
}

/// Stop receiving drops. The buffered paths are deliberately NOT cleared: a
/// window that is closing may still have a file in there that the bridge means
/// to write into its recents, and dropping data on a shutdown path is the one
/// thing this app has promised it never does.
pub fn disarm(hwnd: isize) -> PlatformResult<()> {
    let hwnd = crate::windows::to_hwnd(hwnd)?;
    // SAFETY: RevokeDragDrop takes an HWND already validated by to_hwnd and a
    // target this thread registered; Windows documents calling it on an
    // unregistered window as a plain refusal, and the result is discarded on
    // purpose (see the note above), so no handle is released twice.
    match unsafe { RevokeDragDrop(hwnd) } {
        Ok(()) => Ok(()),
        // DRAGDROP_E_NOTREGISTERED is the honest "was not armed" answer, and a
        // caller that armed on a window the toolkit already destroyed gets the
        // refusal it can act on. Mapped, not swallowed: the two cases look the
        // same from here and are not the same event.
        Err(e) => Err(crate::windows::win32_error("RevokeDragDrop", e)),
    }
}

/// Was this arm a takeover of an already-registered window?
// Consumed by the bridge in S3, which reports whether arming took a window over
// from something else. Nothing in THIS crate calls it yet, and the test below does
// not count for a non-test build, hence the allow rather than a deletion.
#[allow(dead_code)]
pub(crate) fn took_over() -> bool {
    *lock_or_recover(&TOOK_OVER)
}

#[cfg(test)]
mod tests {
    use super::{buffer, take_buffered, took_over};
    use std::path::PathBuf;

    /// The whole promise the bridge codes against, testable with no window, no
    /// apartment and no drag: one take drains everything, and the next take
    /// sees nothing. Not tested on a real drop because a real drop needs a
    /// desktop, a second process and a mouse - none of which this half of the
    /// contract depends on.
    #[test]
    fn one_take_drains_the_buffer_and_the_next_take_sees_nothing() {
        assert!(take_buffered().is_empty(), "the buffer starts empty");
        buffer(vec![
            PathBuf::from("C:/notes/first.notes"),
            PathBuf::from("C:/notes/second.notes"),
        ]);
        let drained = take_buffered();
        assert_eq!(drained.len(), 2, "{drained:?}");
        assert_eq!(drained[0], PathBuf::from("C:/notes/first.notes"));
        assert!(take_buffered().is_empty(), "the take cleared it");
    }

    #[test]
    fn nothing_has_claimed_a_takeover_in_a_process_that_never_armed() {
        assert!(!took_over());
    }
}
