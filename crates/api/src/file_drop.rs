//! FILE DROP at the port: turn a window's shell-drop receiving on, and hand back
//! the thing that turns it off again.
//!
//! This module is a type bridge and nothing more. `WindowHandle` crosses the port
//! as an `i64` because no platform type may name itself here (rule 2 of the crate
//! docs), and every `notes-platform` seam takes an `isize`; the two cannot see
//! each other, so somebody has to carry the value across, and it is not the
//! bridge, which may import neither crate's neighbour. That translation is the
//! whole of what lives here.
//!
//! What does NOT live here: any notion of what a dropped file means.
//! `notes-platform` buffers the paths an `IDropTarget` receives and hands them out
//! on a pull (`WindowBackend::take_dropped_paths`), and deciding anything about a
//! path that arrives — whether it is a note, whether the buffer may be thrown
//! away, whether the user should be asked first — belongs above the port, in the
//! bridge that pulled. This module has no `Event`, no `Command` and no `PathBuf`
//! in its signature, so it cannot make that decision even by accident, which is
//! why rule 1 is safe here rather than merely respected.

use crate::command::WindowHandle;

/// Start receiving shell drops for `handle`.
///
/// **EXECUTES ON THE CALLER THREAD.** Nothing is queued, no channel is touched,
/// and the `Result` answers THIS call rather than arriving later as an
/// [`Event`](crate::Event). That makes it the port's one synchronous
/// `Result`-returning call, and rule 4 of the crate docs says so by name; it is
/// synchronous because it has to be, not because it is convenient.
///
/// # The thread warning, which is the contract
///
/// **`arm_file_drop` must be called from the thread that pumps this window**, and
/// the returned guard must be dropped on that same thread — which is bridge
/// discipline rather than something the types enforce: a [`DropGuard`] carries
/// an `isize`, and Rust will let it be moved anywhere.
///
/// `OleInitialize` builds the COM apartment of the thread that *calls it*, and
/// `RegisterDragDrop` binds the drop target to the window's owner. Move this call
/// into the engine's [`RegisterWindow`](crate::Command::RegisterWindow) arm — where
/// the handle already arrives, so it looks like its home — and it registers from
/// the engine thread, which pumps nothing. The callbacks then wait for a pump
/// that will never run, and the wait is inside the *drag loop Windows drives for
/// the sender*: Explorer freezes mid-drag, across the whole desktop, with no
/// error returned to anybody in this process. A wrong thread here is not a
/// degraded window; it is somebody else's shell hanging.
///
/// # Errors
///
/// [`DropArmError::InvalidHandle`] for a handle that names no window, without a
/// platform call (`to_isize` decides that alone). Anything `notes-platform`
/// refuses becomes [`DropArmError::Ole`] carrying that crate's own sentence
/// untranslated — the same rule `Event::GeometryNotRestored` follows, because OS
/// text is the one clue a user has and the port may not invent a friendlier one.
/// A drop that never armed degrades the window rather than breaking it, so the
/// caller decides what to say and this returns instead of panicking.
#[must_use = "the guard is what stops this window receiving drops; dropped at once it arms and disarms in one statement"]
pub fn arm_file_drop(handle: WindowHandle) -> Result<DropGuard, DropArmError> {
    // The pure predicate FIRST, on every host: `0` is the port's no-window value,
    // so it is a caller with no window yet, not an OS failure, and the two answers
    // mean different things to whoever reads them.
    let hwnd = to_isize(handle)?;
    platform_arm(hwnd)?;
    Ok(DropGuard { hwnd, _priv: () })
}

/// The window this port's drop registration is live for.
///
/// Dropping it calls `notes-platform`'s `disarm`, which revokes the target, so the
/// guard's lifetime IS the registration's lifetime. It is deliberately not
/// `Clone` and cannot be built outside this module (the `_priv` marker): two copies
/// of one guard would revoke one registration twice, and a hand-built guard would
/// revoke a registration nobody made.
///
/// **Drop it on the thread that pumps the window** — see [`arm_file_drop`] for why
/// the pair is thread-affine. In practice that means the guard is fielded by the
/// bridge that armed it, next to the window it names, and released before that
/// window is destroyed.
///
/// A refusal on the way out is swallowed, and not from carelessness: a destructor
/// has nowhere to return an `Err`, the port has no channel to report one on (this
/// call does not own an event sender, by design), and the only two refusals are
/// "was not armed" and "the window is already gone", neither of which a bridge
/// could act on. The paths `notes-platform` already buffered are NOT discarded by
/// `disarm` — that crate's own call, and its own reason: a drop in flight toward a
/// closing window is data, not litter.
pub struct DropGuard {
    /// The `HWND`-as-`isize` this guard registered, and revokes on drop.
    ///
    /// A number, not a lease: it names no window by itself, and `disarm` re-checks
    /// it the way every other `notes-platform` seam does.
    hwnd: isize,
    /// Private marker: the only way to hold one of these is to have armed one.
    _priv: (),
}

impl Drop for DropGuard {
    fn drop(&mut self) {
        // Errors are unactionable from a destructor — see [`DropGuard`].
        platform_disarm(self.hwnd);
    }
}

/// Why the registration did not happen.
///
/// Three arms because three different things went wrong and the copy differs for
/// each, and because a bridge cannot branch on a string: an `InvalidHandle` means
/// the caller acted too early (no window yet), an `Ole` means the OS refused a real
/// window (a degraded notepad, still usable), and `Unsupported` means this host
/// has no such thing to refuse (no message to show at all). Its `Display` text is
/// the user-visible half of the last two, so those strings are pinned in a test.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DropArmError {
    /// The value names no window: `0`, the port's no-window value, or a number
    /// that cannot even be an `HWND` on this host. Decided here, without a single
    /// platform call: `to_isize` decides that arm alone, from the dto.
    #[error("no window to receive drops")]
    InvalidHandle,
    /// A real window, and OLE refused it. The payload is `notes-platform`'s own
    /// sentence — its `PlatformError` `Display`, API name included — passed
    /// through untranslated.
    #[error("the OS refused the drop target: {0}")]
    Ole(String),
    /// This host has no file-drop registration to attempt: `notes-platform`'s Win32
    /// module is not built here. A build fact, never an OS answer, so a bridge
    /// reading this has nothing to render and nothing to retry.
    #[error("this platform cannot receive file drops")]
    Unsupported,
}

/// The pure half, and the only judgement this crate makes: can this dto honestly
/// be handed to a `notes-platform` seam as the `isize` it asks for?
///
/// Two refusals, both answerable with no OS in sight. `0` is
/// [`WindowHandle`]'s own documented no-window value. A value that does not fit an
/// `isize` cannot name an `HWND` on this host either — that branch is dead on a
/// 64-bit host, where the two types are the same width, and live on a 32-bit one.
///
/// Anything else is handed down and judged by `IsWindow` inside
/// `notes-platform`, which is where that check belongs: a check is a check, not a
/// lease, and this crate keeps no window alive.
fn to_isize(handle: WindowHandle) -> Result<isize, DropArmError> {
    if handle.0 == 0 {
        return Err(DropArmError::InvalidHandle);
    }
    isize::try_from(handle.0).map_err(|_| DropArmError::InvalidHandle)
}

/// Arm the target on this host. Off Windows there is no such call to make: see
/// [`platform_disarm`] for the matching pair, and [`arm_file_drop`] for the contract
/// both serve.
///
/// The cfg is a build fact, not a decision, which is the
/// same reason `gateway::platform_host` is split in two.
#[cfg(windows)]
fn platform_arm(hwnd: isize) -> Result<(), DropArmError> {
    notes_platform::windows::file_drop::arm(hwnd).map_err(|error| match error {
        notes_platform::PlatformError::InvalidHandle => DropArmError::InvalidHandle,
        other => DropArmError::Ole(other.to_string()),
    })
}

/// See [`platform_arm`].
#[cfg(not(windows))]
fn platform_arm(_hwnd: isize) -> Result<(), DropArmError> {
    Err(DropArmError::Unsupported)
}

/// See [`platform_arm`]. On Windows this is the revoke half of the pair; the
/// refusal it can return is unactionable from a destructor — see [`DropGuard`].
#[cfg(windows)]
fn platform_disarm(hwnd: isize) {
    let _ = notes_platform::windows::file_drop::disarm(hwnd);
}

/// See [`platform_disarm`]. Unreachable in practice: a guard cannot exist on a host
/// where nothing arms. The body keeps the field read, so the type check is the
/// same on every host.
#[cfg(not(windows))]
fn platform_disarm(_hwnd: isize) {}

#[cfg(test)]
mod tests {
    use super::{DropArmError, DropGuard, arm_file_drop, to_isize};
    use crate::command::WindowHandle;

    /// The refusal a caller with no window gets is decided INSIDE this crate, on a
    /// pure predicate — before `notes-platform` is named, and therefore before any
    /// COM call, any apartment and any thread question. `to_isize` is the whole
    /// of that path and touches nothing outside itself; [`arm_file_drop`] runs it
    /// first on every host, which is what makes the second assertion below a claim
    /// about ORDER rather than about two functions that happen to agree.
    #[test]
    fn a_handle_that_names_no_window_is_refused_without_reaching_the_platform() {
        assert_eq!(to_isize(WindowHandle(0)), Err(DropArmError::InvalidHandle));
        assert_eq!(
            arm_file_drop(WindowHandle(0)).err(),
            Some(DropArmError::InvalidHandle),
            "and the public call asks the predicate before it asks the OS"
        );
    }

    /// A value that fits an `i64` and names a window is carried across UNCHANGED —
    /// the bridge in the name is lossless on the hosts this app ships to. Written
    /// as a table because the interesting cases are the sign and the width, not the
    /// magnitude.
    #[test]
    fn a_handle_that_names_a_window_crosses_unchanged() {
        for raw in [1i64, 0x1234, 0x7fff_ffff, -1, i64::MIN, i64::MAX] {
            if raw.is_negative() {
                // A negative i64 still fits an isize on a 64-bit host, so the
                // predicate is silent about it and `IsWindow` is the one that
                // refuses — see the Windows test below for exactly that.
                assert_eq!(to_isize(WindowHandle(raw)).ok(), isize::try_from(raw).ok());
            } else {
                assert_eq!(to_isize(WindowHandle(raw)), Ok(raw as isize));
            }
        }
    }

    /// The other half of `to_isize`: on a host where an `isize` is narrower than an
    /// `i64`, a too-wide handle is refused HERE and is never handed to the OS as a
    /// truncated number. Dead code on x86_64 by construction, which is the reason
    /// it is a cfg-expressible test rather than a comment.
    #[cfg(not(target_pointer_width = "64"))]
    #[test]
    fn a_handle_too_wide_for_this_host_is_invalid_not_truncated() {
        assert_eq!(
            to_isize(WindowHandle(i64::MAX)),
            Err(DropArmError::InvalidHandle),
            "narrowing by hand is how a wrong window gets a drop target"
        );
    }

    /// Off Windows there is no drop target to register, and the answer says THAT —
    /// not a failure with an OS sentence behind it. Ordered after the predicate, so
    /// a caller with no window still gets the honest "no window" rather than a
    /// platform claim it cannot act on.
    #[cfg(not(windows))]
    #[test]
    fn off_windows_the_answer_is_a_build_fact_and_still_ranks_below_the_handle() {
        assert_eq!(
            arm_file_drop(WindowHandle(0x1234)).err(),
            Some(DropArmError::Unsupported)
        );
        assert_eq!(
            arm_file_drop(WindowHandle(0)).err(),
            Some(DropArmError::InvalidHandle),
            "no window outranks no support"
        );
    }

    /// On Windows a handle that passes the predicate is still judged by
    /// `IsWindow`, inside `notes-platform`, and its refusal comes back as the SAME
    /// typed arm — the mapping, not the message, is what this pins. Every value
    /// used here is misaligned or has the sign bit set, so no USER handle table can
    /// hold it: the registration is refused before OLE is asked for anything, and
    /// no window in this process is touched.
    #[cfg(windows)]
    #[test]
    fn a_platform_refusal_comes_back_as_the_same_typed_arm() {
        for raw in [1isize, 3, -1, isize::MIN + 1] {
            let attempted = arm_file_drop(WindowHandle(raw as i64));
            assert!(
                matches!(attempted, Err(DropArmError::InvalidHandle)),
                "handle {raw:#x}: expected InvalidHandle, got {:?}",
                attempted.err()
            );
        }
    }

    /// Two of the three arms are copy this crate owns (the third carries
    /// `notes-platform`'s own sentence verbatim, so there is nothing here to pin
    /// beyond the prefix). `SaveError` earns the same treatment in `event.rs`: a
    /// string a bridge renders is a product surface, not a log line.
    #[test]
    fn the_copy_this_crate_owns_is_pinned() {
        assert_eq!(
            DropArmError::InvalidHandle.to_string(),
            "no window to receive drops"
        );
        assert_eq!(
            DropArmError::Unsupported.to_string(),
            "this platform cannot receive file drops"
        );
        assert_eq!(
            DropArmError::Ole("Win32 RegisterDragDrop failed: x".to_string()).to_string(),
            "the OS refused the drop target: Win32 RegisterDragDrop failed: x"
        );
    }

    /// The guard is 8 bytes plus a zero-sized marker: it holds the handle and the
    /// registration's lifetime, and nothing else — no `Sender`, no `PathBuf`, no
    /// copy of what anybody dropped.
    #[test]
    fn the_guard_carries_the_handle_and_nothing_else() {
        assert_eq!(
            core::mem::size_of::<DropGuard>(),
            core::mem::size_of::<isize>(),
            "an isize plus a private marker"
        );
    }
}
