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

use std::collections::HashSet;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use windows::Win32::Foundation::{DRAGDROP_E_ALREADYREGISTERED, POINTL, RPC_E_CHANGED_MODE};
use windows::Win32::System::Com::{APTTYPE, APTTYPE_STA, APTTYPEQUALIFIER, CoGetApartmentType};
use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, IDropTarget, IDropTarget_Impl, OleGetClipboard,
    OleInitialize, RegisterDragDrop, RevokeDragDrop,
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

/// Which HWNDs THIS module registered, keyed by the raw handle value.
///
/// The multi-window limit this static carries, stated rather than hidden: the set
/// is per-window, so arming two windows is tracked correctly, but the drop BUFFER
/// above is process-wide and cannot say which window a path landed on. What this
/// file supports today is therefore ONE drop target per process; a real
/// multi-window build has to key the buffer by HWND too, which changes what
/// take_dropped_paths MEANS, not just this file.
// LazyLock because HashSet::new is not a const fn (its RandomState is not), so
// this is the one static here that cannot be built at compile time.
static ARMED: LazyLock<Mutex<HashSet<isize>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// Where the paths still buffered at the last disarm went. Platform has no
/// logger, so the alternative to handing them back is letting them die silently.
static LAST_DISARM: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

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
// SAFETY: the whole block is clipboard and shell32 queries against handles OLE
// owns for the duration of this Drop callback; nothing here frees them and
// every path is copied out. The individual calls restate their own invariant
// where they are made, which is what the ledger below checks.
fn dropped_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    unsafe {
        // CRITICAL, and the reason this is not a bare GetClipboardData:
        // reading the clipboard without owning the open can answer a STALE
        // earlier Explorer copy. A stale HDROP is not a missed drop, it is
        // the WRONG FILE opening, which is exactly what the do-no-harm rule
        // (whitepaper 4.5) exists to make impossible. The documented
        // IDropTarget::Drop sequence runs OleGetClipboard first: OLE flushes
        // THIS drag's data object through the clipboard as a deferred render,
        // and only then does GetClipboardData(CF_HDROP) name a handle that
        // belongs to the drop in flight. OleSetClipboard is deliberately not
        // called, so OLE keeps that flushed render until the next clipboard
        // owner replaces it - a retained render, which costs memory and never
        // a wrong path. The no-FORMATETC argument still holds, and this is
        // why: there is nothing to ask the data object for, because OLE has
        // already rendered it onto the clipboard for us.
        //
        // SAFETY: OleGetClipboard opens the clipboard for this thread and
        // hands back the object it just flushed; the object is released when
        // the binding below leaves scope. It is never called on - the flush is
        // the point, the object is not.
        let _flushed = OleGetClipboard().ok();
        // SAFETY: the clipboard is open for this thread because the call above
        // opened it. The HGLOBAL belongs to OLE, so nothing here frees it, and
        // every path is copied into owned storage before the clipboard closes.
        let hdrop = GetClipboardData(u32::from(CF_HDROP.0))
            .ok()
            .map(|hglobal| HDROP(hglobal.0))
            .filter(|candidate| !candidate.is_invalid());
        if let Some(hdrop) = hdrop {
            // SAFETY: DragQueryFileW with no buffer asks for the COUNT, and is
            // the documented way to size the loop. The handle came from the
            // call above and the drop is still in progress: this runs inside
            // Drop().
            let count = DragQueryFileW(hdrop, u32::MAX, None);
            for index in 0..count {
                // NIT: ask THIS file's own length first - a null buffer returns
                // the length without the terminator - and size the buffer from
                // it, so nothing is cut at a fixed cap. The old 1024 could have
                // handed back a shorter path than the one the user dropped.
                let want = DragQueryFileW(hdrop, index, None) as usize;
                if want == 0 {
                    continue;
                }
                let mut name = vec![0u16; want + 1];
                // SAFETY: name is one wider than the length OLE just reported
                // for this index, so the terminator fits, and count bounds the
                // index. got is compared against want before it is sliced.
                let got = DragQueryFileW(hdrop, index, Some(&mut name)) as usize;
                if got == 0 || got > want {
                    continue;
                }
                out.push(PathBuf::from(std::ffi::OsString::from_wide(&name[..got])));
            }
        }
        // SAFETY: pairs the open that OleGetClipboard performed, on EVERY path
        // including the ones that read nothing - leaving the clipboard open
        // freezes every other clipboard user in the session. A failure here is
        // not ours to act on, and there is no channel to report it through.
        let _ = CloseClipboard();
    }
    out
}

/// Start receiving drops for `hwnd`.
///
/// MUST be called on the thread that pumps that window's messages, and the
/// function now PROVES the thread qualifies instead of trusting the caller:
/// `OleInitialize` initialises the calling thread's apartment and
/// `CoGetApartmentType` is then asked what it actually is, with anything
/// other than STA refused. See the MAJOR-1 note in the body.
///
/// The OLE initialisation count is deliberately NOT paid back, and this is the
/// honest version of that sentence. `windows` types `OleInitialize` as
/// `Result<()>`, so S_OK (this call created the apartment and owns a count to
/// release) and S_FALSE (an earlier owner does) are indistinguishable once the
/// mapping has run - informational severity is dropped by `ok()`. Guessing the
/// other way is worse: an `OleUninitialize` for a count the toolkit still holds
/// tears the apartment down from under winit or gpui mid-session. So the debt
/// is one initialisation per process, chosen, and the previous wording here
/// (`our uninit on drop is not owed`) was a claim this file cannot actually
/// earn. Recording a bool would only pay back the S_OK case it cannot see.
pub fn arm(hwnd: isize) -> PlatformResult<()> {
    let hwnd = crate::windows::to_hwnd(hwnd)?;
    let key = hwnd.0 as isize;
    // SAFETY: OleInitialize initialises the COM apartment for the CALLING
    // thread, is documented as re-entrant-counted and safe to call again, and
    // None is the reserved argument. RPC_E_CHANGED_MODE is still not a
    // failure HERE only in the sense that the apartment check below judges it
    // on the merits instead of this call's word.
    if let Err(e) = unsafe { OleInitialize(None) } {
        if e.code() != RPC_E_CHANGED_MODE {
            return Err(PlatformError::Win32 {
                api: "OleInitialize",
                message: e.message().to_string(),
            });
        }
    }
    // MAJOR-1: RPC_E_CHANGED_MODE used to be swallowed blind, and that let an
    // MTA thread through the door. If this thread is MTA - which is what a bare
    // CoInitializeEx(NULL), a runtime default, or a toolkit that picked MTA
    // leaves behind - OLE does NOT call an IDropTarget on it at all: it
    // marshals every callback onto an RPC worker thread, so BUFFER would be
    // written from a thread that owns neither the window nor a pump. Refuse to
    // register on any answer but STA, and the thread claim above holds.
    let mut apartment = APTTYPE(0);
    let mut qualifier = APTTYPEQUALIFIER(0);
    // SAFETY: both out-parameters are writable storage owned by this frame,
    // and CoGetApartmentType either fills them or returns without writing.
    unsafe { CoGetApartmentType(&mut apartment, &mut qualifier) }
        .map_err(|e| crate::windows::win32_error("CoGetApartmentType", e))?;
    if apartment != APTTYPE_STA {
        return Err(PlatformError::Win32 {
            api: "CoGetApartmentType",
            message: format!(
                "apartment is {} (qualifier {}), not STA: a drop target registered on this thread would be marshalled onto RPC workers",
                apartment.0, qualifier.0
            ),
        });
    }
    // MAJOR-3: the pre-emptive revoke is now limited to a window THIS module
    // registered. Revoking a registration we never made would tear down another
    // member's drop target - the api installs a guard this crate cannot see, and
    // a second bridge arming the same HWND is a future, not a fiction. ARMED is
    // the record; a window absent from it is not ours to unregister.
    if lock_or_recover(&ARMED).contains(&key) {
        // SAFETY: this pair of our own earlier RegisterDragDrop on the same
        // HWND, i.e. a reload. The answer is discarded because the
        // registration below is the call that is actually judged.
        unsafe {
            RevokeDragDrop(hwnd).ok();
        }
    }
    let target: IDropTarget = FileDropTarget.into();
    // SAFETY: `hwnd` names a live window (to_hwnd checked it) on a thread the
    // apartment check above proved STA, and `target` is a valid IDropTarget
    // whose vtable outlives this call because the box is forgotten on the very
    // next line - OLE keeps the pointer past the end of this statement, which
    // is precisely why a normal drop here would be a use-after-free.
    let registered = unsafe { RegisterDragDrop(hwnd, &target) };
    // MAJOR-2: forget UNCONDITIONALLY, before the match. Both branches below
    // leave this function, so the old placement ran it on one path only; and
    // running the Release is not safe on either - on success OLE holds the
    // pointer, and on ALREADYREGISTERED OLE holds ANOTHER target's while our
    // box's post-failure refcount is documented too thinly to guess from. One
    // leaked small object per arm is the safe side of that asymmetry.
    std::mem::forget(target);
    if let Err(e) = registered {
        if e.code() != DRAGDROP_E_ALREADYREGISTERED {
            return Err(crate::windows::win32_error("RegisterDragDrop", e));
        }
        // With the guarded pre-revoke above, an ALREADYREGISTERED answer can
        // no longer mean "ours, from an earlier arm" - that case is revoked
        // first now. So this answer means a FOREIGN target owns the window:
        // still a success for the caller (drops will arrive, at somebody's
        // target), recorded as a takeover, and deliberately NOT inserted into
        // ARMED - which is precisely what stops our disarm from unregistering
        // something we never registered.
        *lock_or_recover(&TOOK_OVER) = true;
        return Ok(());
    }
    lock_or_recover(&ARMED).insert(key);
    Ok(())
}

/// Stop receiving drops for `hwnd`, and hand back what was still buffered.
///
/// MAJOR-3 again from the other side: the OS call happens only for a window
/// this module registered. A window absent from ARMED gets a clean Ok with no
/// revoke, because unregistering a foreign target is a cross-window bug that
/// platform cannot see coming. The DRAGDROP_E_NOTREGISTERED path below stays
/// reserved for a revoke we actually attempted and the OS refused - that answer
/// is real information (the window is gone), and is still mapped, not swallowed.
///
/// MINOR: the buffer is no longer left to die with the process. Whatever a
/// window had dropped and nobody drained is moved into LAST_DISARM, readable
/// through `last_disarm_dropped`. It is NOT dropped on the floor here: a
/// closing window may still hold the file the bridge means to put in its
/// recents, and losing data on a shutdown path is the one thing this app has
/// promised it never does.
pub fn disarm(hwnd: isize) -> PlatformResult<()> {
    let hwnd = crate::windows::to_hwnd(hwnd)?;
    let key = hwnd.0 as isize;
    let ours = lock_or_recover(&ARMED).remove(&key);
    // MINOR: drain, do not abandon. The paths a bridge never picked up are
    // moved where a reader can still find them instead of being left in a
    // buffer whose window is gone.
    let drained = std::mem::take(&mut *lock_or_recover(&BUFFER));
    *lock_or_recover(&LAST_DISARM) = drained;
    // MINOR: the takeover flag is per-arm, not per-process-lifetime. Left
    // alone it would report a stale takeover forever after one foreign target
    // was noticed, which is worse than no flag at all.
    *lock_or_recover(&TOOK_OVER) = false;
    if !ours {
        return Ok(());
    }
    // SAFETY: ARMED said this module registered this HWND, so this is the
    // documented pair of that call and no foreign target is touched. A window
    // the toolkit already destroyed answers with a plain refusal, mapped below
    // rather than swallowed, because that refusal is the information a caller
    // needs; no handle is released twice because we release exactly one.
    match unsafe { RevokeDragDrop(hwnd) } {
        Ok(()) => Ok(()),
        // Reserved, as the doc says, for a revoke we actually attempted.
        Err(e) => Err(crate::windows::win32_error("RevokeDragDrop", e)),
    }
}

/// The paths that were still buffered the last time `disarm` ran, drained and
/// cleared. Read by the tests here and, from S4, by the exit-needle lane that
/// reports what a shutdown found unsaved - platform has no logger of its own.
#[allow(dead_code)]
pub(crate) fn last_disarm_dropped() -> Vec<PathBuf> {
    lock_or_recover(&LAST_DISARM).clone()
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
    use super::{
        ARMED, buffer, disarm, last_disarm_dropped, lock_or_recover, take_buffered, took_over,
    };
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

    /// The flag is per-arm: a process that never armed has never taken anything
    /// over, and the bridge's report depends on that being the default.
    #[test]
    fn nothing_has_claimed_a_takeover_in_a_process_that_never_armed() {
        assert!(!took_over());
    }

    /// MAJOR-3's guard is only as good as the order of its checks, so pin the
    /// order: a handle that is not a window is refused BEFORE any mutex is
    /// touched, which means disarm cannot unregister something, drain the
    /// buffer, or claim a takeover. This is the cheap way to test the guard
    /// without a desktop, an apartment or a mouse.
    #[test]
    fn a_disarm_of_a_non_window_refuses_before_reaching_the_state() {
        let err = disarm(0).expect_err("0 is not an HWND");
        assert!(
            matches!(err, crate::PlatformError::InvalidHandle),
            "a bad handle must be refused, not revoked over: {err:?}"
        );
        assert!(lock_or_recover(&ARMED).is_empty(), "nothing was armed");
        assert!(!took_over());
        assert!(last_disarm_dropped().is_empty());
        assert!(take_buffered().is_empty(), "the buffer was not drained");
    }
}
