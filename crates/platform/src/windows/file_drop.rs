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
use windows::Win32::System::Com::{
    APTTYPE, APTTYPE_MAINSTA, APTTYPE_STA, APTTYPEQUALIFIER, CoGetApartmentType,
};
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
/// function PROVES the thread qualifies instead of trusting the caller:
/// `OleInitialize`, then `CoGetApartmentType`, then the
/// `admits_a_drop_target` predicate - which admits BOTH flavours of STA. See
/// the MAJOR-1 note in the body for the measurement that reversed the first
/// version of that predicate.
///
/// The OLE initialisation count is deliberately NOT paid back, and that choice
/// is now LOAD-BEARING rather than merely convenient: the toolkit has already
/// run its own `OleInitialize` on the event-loop thread - the measured
/// `apartment 3` below IS that primary STA - so our call answers S_FALSE with
/// near-certainty, the count we would release is WINIT'S, and an
/// `OleUninitialize` on the way out would tear down the apartment the toolkit
/// is still running in. `windows` also types `OleInitialize` as `Result<()>`,
/// so S_OK and S_FALSE are indistinguishable once `ok()` has dropped
/// informational severity: recording a bool could only pay back a case this
/// file cannot see. One initialisation per process, chosen, and the reason the
/// takeover below is safe to attempt at all.
pub fn arm(hwnd: isize) -> PlatformResult<DropArm> {
    let hwnd = crate::windows::to_hwnd(hwnd)?;
    let key = hwnd.0 as isize;
    // SAFETY: OleInitialize initialises the COM apartment for the CALLING
    // thread, is documented as re-entrant-counted and safe to call again, and
    // None is the reserved argument. RPC_E_CHANGED_MODE is not a failure HERE
    // only in the sense that the apartment gate below judges the thread on its
    // merits instead of accepting this call's word for it.
    if let Err(e) = unsafe { OleInitialize(None) } {
        if e.code() != RPC_E_CHANGED_MODE {
            return Err(PlatformError::Win32 {
                api: "OleInitialize",
                message: e.message().to_string(),
            });
        }
    }
    // MAJOR-1, HALF OF IT SURVIVED. The gate survives: probing the apartment
    // is the only way to know whether OLE will dispatch a target on THIS
    // thread, and swallowing RPC_E_CHANGED_MODE blind was a real bug. What the
    // dossier overturned on measured ground is the PREDICATE. The live run's
    // apartment value 3 is APTTYPE_MAINSTA - the process's primary STA, created
    // by winit's own OleInitialize on the event-loop thread, which is the very
    // thread we arm from and the best apartment a drop target can have. The
    // first version of this gate refused 3, i.e. it refused the good case and
    // did more damage than the bug it fixed. Admit STA (0) and MAINSTA (3);
    // keep refusing MTA (1), where OLE marshals onto RPC workers, and NA (2),
    // where the thread is neutral and cannot be dispatched at all.
    let mut apartment = APTTYPE(0);
    let mut qualifier = APTTYPEQUALIFIER(0);
    // SAFETY: both out-parameters are writable storage owned by this frame,
    // and CoGetApartmentType either fills them or returns without writing.
    unsafe { CoGetApartmentType(&mut apartment, &mut qualifier) }
        .map_err(|e| crate::windows::win32_error("CoGetApartmentType", e))?;
    if !admits_a_drop_target(apartment) {
        // The QUALIFIER is named in the message because the apartment number
        // alone misleads the next reader. IMPLICIT_MTA (1) is the interesting
        // one: a thread the runtime made MTA without anyone asking, which is
        // precisely what a plain "just initialise it" fix cannot see. And
        // NA_ON_STA (3) does NOT mean "this thread is an STA" - every NA_ON_*
        // qualifier says the CALL came from an object-neutral context, so the
        // apartment is still unusable for dispatch and is still refused. Read
        // the pair, never the headline.
        return Err(PlatformError::Win32 {
            api: "CoGetApartmentType",
            message: format!(
                "apartment is {} with qualifier {}: not STA (0) or MAINSTA (3), so a target registered here would be marshalled onto RPC workers or never dispatched at all",
                apartment.0, qualifier.0
            ),
        });
    }
    // MAJOR-3, HALF OF IT SURVIVED. The EXIT half stands and this file keeps
    // it: ARMED is the disarm gate, so a window this module never registered is
    // never unregistered on the way out - that half is about not destroying
    // somebody else's state. The ARM half does not survive: the pre-emptive
    // revoke is now UNCONDITIONAL, because arming means TAKING the window from
    // the toolkit's own handler, and a revoke that only fired for windows we
    // already owned could never have revoked winit - it guarded exactly the case
    // the feature exists to handle. An Ok here says a target was there and is
    // now gone, which IS the takeover, so it is recorded as one and this is the
    // only place the flag is set. The refusal (nothing was registered) is the
    // honest "nothing to take" answer and is read for that alone.
    //
    // COVENANT: this takes over from winit's own FileDropHandler. Its object
    // stays allocated - we unregister it, we never free it - and simply stops
    // being called, so every DroppedFile event winit would have emitted through
    // its event loop is now ours to deliver. Until slint forwards DroppedFile
    // for a custom window (upstream issue 1967) there is no other way to get the
    // payload, so this is stealing a delivery we owe, not one we may claim: the
    // reopen condition is 1967 landing, at which point the takeover is a bug and
    // the arm must be withdrawn. Re-read this paragraph before keeping it.
    // SAFETY: `hwnd` is validated by to_hwnd. RevokeDragDrop on a window with
    // no target is a documented plain refusal; nothing is released twice,
    // because a refusal releases nothing and a success releases the one
    // registration OLE held. The answer is read only to distinguish took-over
    // from nothing-was-there; the registration below is still what is judged.
    // The ARMED read is a FACT-CHECK, not a guard, and the difference matters:
    // it is taken BEFORE the revoke so that re-arming a window we already hold
    // reports took_over = false (we displaced our own target, which is stealing
    // from nobody) while displacing the toolkit's handler reports true. The
    // revoke itself stays UNCONDITIONAL - this read changes only what is said
    // afterwards, never what is unregistered.
    let ours = lock_or_recover(&ARMED).contains(&key);
    let preexisting = unsafe { RevokeDragDrop(hwnd) }.is_ok();
    let mut took_over = took_over_from(preexisting, ours);
    let target: IDropTarget = FileDropTarget.into();
    // SAFETY: `hwnd` names a live window (to_hwnd checked it) on a thread the
    // apartment gate above admitted, and `target` is a valid IDropTarget whose
    // vtable outlives this call because the box is forgotten on the very next
    // line - OLE keeps the pointer past the end of this statement, which is
    // precisely why a normal drop here would be a use-after-free.
    let mut registered = unsafe { RegisterDragDrop(hwnd, &target) };
    // MAJOR-2: UNTOUCHED, and still unconditional ahead of every branch below.
    // Both the retry and the final error path leave this function, so a placed
    // forget would run on one path only, and running the Release is not safe on
    // any of them: on success OLE holds the pointer, and on failure our box's
    // post-failure refcount is documented too thinly to guess from. One leaked
    // small object per attempt is the safe side of that asymmetry.
    std::mem::forget(target);
    if registered
        .as_ref()
        .err()
        .is_some_and(|e| e.code() == DRAGDROP_E_ALREADYREGISTERED)
    {
        // AFTER an unconditional revoke, ALREADYREGISTERED can only mean a
        // RACE: between our revoke and our register, somebody else registered a
        // target - a concurrent second arm, or the toolkit reinstalling its own
        // handler. Revoke and register ONE more time. A second ALREADYREGISTERED
        // is a genuinely contested window and is returned as an error: a retry
        // loop here would hide a livelock behind a success.
        // SAFETY: the same validated HWND, and this pair of revokes is the
        // documented call; the refusal of a revoke is not acted on because the
        // register immediately below is what is judged.
        unsafe {
            RevokeDragDrop(hwnd).ok();
        }
        // A fresh target: the first box was forgotten above, and whether OLE
        // looked at it before refusing is not documented, so it is not reused.
        let retry: IDropTarget = FileDropTarget.into();
        // SAFETY: as the first registration, and the box is forgotten on the
        // next line for the same reason (MAJOR-2).
        registered = unsafe { RegisterDragDrop(hwnd, &retry) };
        std::mem::forget(retry);
        // The retry removed a target that appeared AFTER our revoke, which by
        // the definition above IS a takeover, whoever the other party was.
        took_over = true;
    }
    if let Err(e) = registered {
        return Err(crate::windows::win32_error("RegisterDragDrop", e));
    }
    // Only a registration WE made enters ARMED - the disarm gate below.
    lock_or_recover(&ARMED).insert(key);
    Ok(DropArm { took_over })
}

/// Which apartments OLE will actually dispatch an `IDropTarget` on.
///
/// MEASURED, and the measurement is what reversed the first version of this
/// predicate: APTTYPE_STA = 0, APTTYPE_MTA = 1, APTTYPE_NA = 2,
/// APTTYPE_MAINSTA = 3 (windows-0.61.3, Win32/System/Com/mod.rs:933-937).
/// MAINSTA is in the list because it is the primary STA - winit's own, on the
/// event-loop thread - and an STA is an STA for dispatch purposes. Nothing else
/// is admitted, including APTTYPE_CURRENT (-1) and any value a future windows
/// release adds: the safe default is to refuse and be told, not to guess.
const ADMISSIBLE_APARTMENTS: [APTTYPE; 2] = [APTTYPE_STA, APTTYPE_MAINSTA];

fn admits_a_drop_target(apartment: APTTYPE) -> bool {
    ADMISSIBLE_APARTMENTS.contains(&apartment)
}

/// What an arm is allowed to REPORT: a takeover is a takeover only when the
/// target we displaced was not our own.
///
/// `preexisting` is the revoke's answer (something WAS registered here); `ours`
/// is the ARMED record taken before it. A re-arm of a window this module already
/// holds is therefore `false` - we replaced ourselves, which is not stealing from
/// anybody - and taking the window from winit's handler is `true`. Kept a free
/// function rather than an inline `&&` so the four cases are unit-testable without
/// a window, an apartment, or the revoke actually happening.
fn took_over_from(preexisting: bool, ours: bool) -> bool {
    preexisting && !ours
}

/// What a successful `arm` returns: the fact about how it succeeded.
///
/// One field, and it is the only question the caller can act on: did this arm
/// displace somebody else's drop target (the toolkit's own handler, per the
/// covenant in [`arm`]), or did it take a window that had nothing registered?
/// Nothing else is reported because nothing else is platform's to decide - the
/// port wraps this in its guard and the bridge decides whether to say anything.
/// It is deliberately NOT `#[non_exhaustive]`: that attribute would forbid the
/// struct being built by name outside this crate, which is harmless today and
/// would only make a test that wants to pin the field pay for a real arm.
pub struct DropArm {
    /// True when this arm took the window over from a target that was already
    /// there and was not ours. False for a clean window AND for a re-arm of our
    /// own earlier registration - see [`took_over_from`].
    pub took_over: bool,
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
    // MAJOR-3's surviving half, from the exit side: a window absent from ARMED
    // was never ours to unregister, so no OS call is made and the answer is a
    // clean Ok. The DRAGDROP_E_NOTREGISTERED mapping below stays reserved for a
    // revoke we actually attempted.
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

#[cfg(test)]
mod tests {
    use super::{
        ARMED, admits_a_drop_target, buffer, disarm, last_disarm_dropped, lock_or_recover,
        take_buffered, took_over_from,
    };
    // Test-only: these three exist solely to pin the refusals, and naming them at
    // module level would trip unused_imports on a non-test build.
    use std::path::PathBuf;
    use windows::Win32::System::Com::{
        APTTYPE_CURRENT, APTTYPE_MAINSTA, APTTYPE_MTA, APTTYPE_NA, APTTYPE_STA,
    };
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
    /// What `arm` may REPORT, decided by pure logic and testable with no window,
    /// no apartment and no drag. A re-arm of a window we already hold is NOT a
    /// takeover - reporting one would tell the operator we stole something from
    /// the toolkit when we only replaced ourselves. Displacing the toolkit's own
    /// handler IS. A window with nothing registered is neither.
    #[test]
    fn a_re_arm_of_our_own_window_is_not_reported_as_a_takeover() {
        assert!(
            took_over_from(true, false),
            "the toolkit had a target and the revoke removed it: that is the takeover"
        );
        assert!(
            !took_over_from(true, true),
            "we displaced our OWN earlier target: a re-arm, not a takeover"
        );
        assert!(!took_over_from(false, false), "nothing was registered");
        assert!(
            !took_over_from(false, true),
            "nothing was registered to lose"
        );
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
        assert!(last_disarm_dropped().is_empty());
        assert!(take_buffered().is_empty(), "the buffer was not drained");
    }

    /// The predicate is where the measured reversal lives, so it gets its own
    /// test and needs no window, apartment or drag: 0 (STA) and 3 (MAINSTA -
    /// the primary STA winit creates, which is what the live run reported) are
    /// admitted; 1 (MTA) and 2 (NA) are refused, and so is -1 (CURRENT),
    /// because an unlisted value must refuse rather than guess.
    #[test]
    fn the_apartment_gate_admits_both_stas_and_refuses_the_neutrals() {
        for admitted in [APTTYPE_STA, APTTYPE_MAINSTA] {
            assert!(
                admits_a_drop_target(admitted),
                "{} must be admitted",
                admitted.0
            );
        }
        for refused in [APTTYPE_MTA, APTTYPE_NA, APTTYPE_CURRENT] {
            assert!(
                !admits_a_drop_target(refused),
                "{} must be refused",
                refused.0
            );
        }
        assert_eq!(APTTYPE_MAINSTA.0, 3, "the measurement this gate rests on");
        assert_eq!(APTTYPE_STA.0, 0);
    }
}
