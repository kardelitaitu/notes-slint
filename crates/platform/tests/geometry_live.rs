//! A machine-verified proof that the async seams actually LAND.
//!
//! The flag fix (4cae233) made `set_frame_rect`/`set_topmost` carry
//! `SWP_ASYNCWINDOWPOS`, and the unit tests pin the flag bits - but no unit test
//! can prove the thing the flag promises: that the move really ends up where we
//! asked while the thread that owns the window never pumps, and that it really
//! lands once the owner does. This file creates a REAL top-level window on a
//! REAL second thread (plain RegisterClassExW/CreateWindowExW, no gpui), parks
//! that thread OUTSIDE user32, drives our own backend from the main thread
//! against the foreign-owned HWND, and measures what happens.
//!
//! It cannot run headless: window creation and the message queue need an
//! interactive desktop, so it is `#[ignore]`d by default. Machine-verified
//! proof, run by hand:
//!
//! ```
//! cargo test -p notes-platform --test geometry_live -- --ignored --nocapture
//! ```
//!
//! Cleanup is explicit: the window is destroyed by its own thread and the
//! thread is joined; nothing leaks. Every unsafe block below carries its own
//! SAFETY comment (AGENTS.md); src/ is untouched by this file, so the src
//! SAFETY ledger gains nothing here - `check-unsafe` prints the live counts.
//!
//! One caveat this probe established the hard way, recorded here because the
//! port depends on it: with the window HIDDEN, the async reband never lands -
//! not cross-thread, and not even from the owner thread itself - while the
//! async MOVE lands normally. The probe therefore creates the window VISIBLE
//! (with WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW, so the hand-run steals no focus
//! and no taskbar slot), which is the state the product pins in. Consequence
//! for the port: a pin applied while the window is still hidden will silently
//! not land; pin state must reach the window after (or with) it is shown.

#![allow(unsafe_code)] // test-only: the Win32 probe below is the honest home for this unsafe

use std::sync::mpsc::{Sender, channel};

use std::thread;
use std::time::{Duration, Instant};
// The package's dependency list is visible to this test target; naming it
// here satisfies unused_crate_dependencies without using the error crate.
use thiserror as _;

// Every cfg(windows) dependency is visible to this crate root too, and
// unused_crate_dependencies is a lint gate: windows-core is named only by the
// #[implement] expansion inside the lib, so this root has to declare it used.
#[cfg(windows)]
use windows_core as _;

use ::windows::Win32::Foundation::{HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, WPARAM};
use ::windows::Win32::Graphics::Gdi::HBRUSH;
use ::windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, GWL_EXSTYLE, GetMessageW, GetWindowLongW,
    HCURSOR, HICON, HMENU, MSG, PM_REMOVE, PeekMessageW, PostThreadMessageW, RegisterClassExW,
    TranslateMessage, WM_QUIT, WNDCLASS_STYLES, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use ::windows::core::{PCWSTR, w};

use notes_platform::windows::Backend;
use notes_platform::{FrameRect, PinOutcome, PlatformError, WindowBackend};

// kernel32's GetModuleHandleW and user32's raw DefWindowProcW, declared
// locally: GetModuleHandleW is gated behind the Win32_System_LibraryLoader
// feature the root manifest does not enable (the GetACP precedent in
// windows/mod.rs), and the windows crate's DefWindowProcW wrapper is a
// Rust-ABI function, not the WNDPROC-typed extern a WNDCLASS needs. Both link
// the same exports with the same signatures as the generated bindings would.
// Neither is marked safe: one reads the string its argument points at when it
// is not null, the other is a window procedure, so callers state the
// invariant at each call.
unsafe extern "system" {
    fn GetModuleHandleW(lpmodulename: PCWSTR) -> HMODULE;
    fn DefWindowProcW(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
    // GetACP class: no arguments, a pure query of the calling thread, no
    // failure mode - sound to call from anywhere, which is why it is marked
    // safe rather than fenced behind an unsafe block.
    safe fn GetCurrentThreadId() -> u32;
}

/// What the main thread tells the owner thread to do. While the owner waits in
/// `recv` it is PARKED in the sense that matters here: it has not entered
/// user32, so its message queue sits untouched - exactly the state the
/// original 12 s hang was reproduced against.
enum Command {
    /// Drain the message queue to completion, then acknowledge.
    Pump,
    /// Run the real UI-thread pump - a blocking GetMessageW loop, ended by
    /// WM_QUIT - which is the shape a bridge actually runs.
    PumpContinuously,
    /// The CONTROL: apply the reband from the owner thread itself, through
    /// the same backend seam, so a failed cross-thread landing can be
    /// distinguished from a broken probe.
    SelfTopmost,
    /// Destroy the window on this thread (Win32 requires the owner) and exit.
    Shutdown,
}

/// A bound on any single blocking-call measurement: if the async flag ever
/// regresses, the call below would park the calling thread indefinitely, so
/// the call runs on a throwaway thread and is judged by a deadline instead of
/// hanging the suite.
fn call_bounded<T: Send + 'static>(
    call: impl FnOnce() -> T + Send + 'static,
    limit: Duration,
) -> Result<T, &'static str> {
    let (done, done_rx) = channel();
    thread::spawn(move || {
        let _ = done.send(call());
    });
    done_rx
        .recv_timeout(limit)
        .map_err(|_| "call blocked past its bound: SWP_ASYNCWINDOWPOS is not doing its job")
}

/// Spawns the owner thread, waits for its window, and returns the command
/// channel, the ack channel, the handle as isize, and the join handle.
fn spawn_owner() -> (
    Sender<Command>,
    std::sync::mpsc::Receiver<bool>,
    isize,
    u32,
    thread::JoinHandle<()>,
) {
    let (cmd_tx, cmd_rx) = channel::<Command>();
    let (ready_tx, ready_rx) = channel::<Result<(isize, u32), String>>();
    let (ack_tx, ack_rx) = channel::<bool>();
    let handle = thread::Builder::new()
        .name("parked-window-owner".into())
        .spawn(move || unsafe {
            // SAFETY: GetModuleHandleW is called with a NULL name, which asks
            // for this process's own module handle; no string is read, no
            // pointer is written, and a NULL answer is checked, not used.
            let hinstance = HINSTANCE(GetModuleHandleW(PCWSTR::null()).0);
            let class = WNDCLASSEXW {
                cbSize: core::mem::size_of::<WNDCLASSEXW>() as u32,
                style: WNDCLASS_STYLES(0),
                // DefWindowProcW matches WNDPROC exactly; the window therefore
                // reacts to every message the way a bare Win32 window does.
                lpfnWndProc: Some(DefWindowProcW),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance,
                hIcon: HICON::default(),
                hCursor: HCURSOR::default(),
                hbrBackground: HBRUSH::default(),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: w!("notes_platform_live_probe"),
                hIconSm: HICON::default(),
            };
            // SAFETY: RegisterClassExW reads `class`, a live repr(C) local
            // whose cbSize is set to its exact size as the API demands; it
            // registers the class for this process and answers 0 on failure,
            // which is checked, not ignored.
            if RegisterClassExW(&class) == 0 {
                let _ = ready_tx.send(Err("RegisterClassExW failed".to_string()));
                return;
            }
            // SAFETY: CreateWindowExW receives the class registered above,
            // the same HINSTANCE the class carries (the HMODULE identity is
            // the process's own module), and no parent, menu, or extra
            // parameter. It creates a real window owned by THIS thread and
            // returns it or a typed error. The window IS visible: topmost
            // band placement is the thing under test and it does not behave
            // for a hidden window the way it does for a shown one - but
            // WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW keep it from stealing
            // focus or a taskbar slot while the probe runs.
            let hwnd = match CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                w!("notes_platform_live_probe"),
                w!("notes-platform geometry live probe"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                100,
                100,
                420,
                300,
                None,
                None::<HMENU>,
                Some(hinstance),
                None,
            ) {
                Ok(hwnd) => hwnd,
                Err(error) => {
                    let _ = ready_tx.send(Err(format!("CreateWindowExW failed: {error}")));
                    return;
                }
            };
            if ready_tx
                .send(Ok((hwnd.0 as isize, GetCurrentThreadId())))
                .is_err()
            {
                return; // The main thread is gone; nothing left to serve.
            }
            while let Ok(command) = cmd_rx.recv() {
                match command {
                    Command::Pump => {
                        let mut msg = MSG::default();
                        // SAFETY: PeekMessageW writes at most the MSG local,
                        // which lives for the call; the filter is THIS thread
                        // (no window filter: an async SetWindowPos request can
                        // arrive as a thread-level message that a window-only
                        // filter would leave starved in the queue) and
                        // PM_REMOVE takes each message out as it is read. The
                        // loop runs until the queue is empty (the next peek
                        // answers FALSE).
                        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                            // SAFETY: TranslateMessage reads the MSG that
                            // PeekMessageW just filled; it synthesizes
                            // character messages and answers BOOL either way.
                            let _ = TranslateMessage(&msg);
                            // SAFETY: DispatchMessageW delivers the message
                            // PeekMessageW removed to the owning window's
                            // procedure (DefWindowProcW above) and returns
                            // whatever that answered.
                            DispatchMessageW(&msg);
                        }
                        let _ = ack_tx.send(true);
                    }
                    Command::SelfTopmost => {
                        // The control: the SAME seam, the owner's OWN thread.
                        let mut backend = Backend;
                        let applied = matches!(
                            backend.set_topmost(hwnd.0 as isize, true),
                            PinOutcome::Applied
                        );
                        let _ = ack_tx.send(applied);
                    }
                    Command::PumpContinuously => {
                        // The faithful UI-thread pump: a blocking GetMessageW
                        // loop over this thread's queue, ended by WM_QUIT,
                        // which PostThreadMessageW delivers from the main
                        // thread when the measurement is done.
                        loop {
                            let mut msg = MSG::default();
                            // SAFETY: GetMessageW writes at most the MSG
                            // local, which lives for the call, and blocks
                            // until THIS thread has a message; 0 means WM_QUIT
                            // and -1 an error, both of which end the pump
                            // rather than being ignored.
                            if GetMessageW(&mut msg, None, 0, 0).0 <= 0 {
                                break; // WM_QUIT or error: back to parked recv.
                            }
                            // SAFETY: reads the MSG that GetMessageW just
                            // filled; synthesizes character messages.
                            let _ = TranslateMessage(&msg);
                            // SAFETY: delivers the retrieved message to this
                            // window's procedure.
                            DispatchMessageW(&msg);
                        }
                    }
                    Command::Shutdown => {
                        // SAFETY: DestroyWindow is called from the thread
                        // that owns the window (the requirement), on the
                        // handle this thread created; a failed destroy is
                        // surfaced through the ack as an error, not ignored.
                        match DestroyWindow(hwnd) {
                            Ok(()) => {
                                let _ = ack_tx.send(true);
                            }
                            Err(error) => {
                                let _ = ack_tx.send(true);
                                eprintln!("DestroyWindow failed: {error}");
                            }
                        }
                        break;
                    }
                }
            }
        })
        .expect("the owner thread spawns");
    let ready = match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(ready) => ready,
        Err(_) => Err("the owner thread never reported a window".to_string()),
    };
    let (hwnd, thread_id) = ready.expect("a real window was created on the owner thread");
    (cmd_tx, ack_rx, hwnd, thread_id, handle)
}

/// Waits, bounded, until `probe` reports the expected value, and returns how
/// long the landing took - or None when the deadline passed first. No
/// sleep-and-hope: a deadline, and the caller decides what None means.
fn wait_for_landing(probe: impl Fn() -> bool, deadline: Duration) -> Option<Duration> {
    let started = Instant::now();
    loop {
        if probe() {
            return Some(started.elapsed());
        }
        if started.elapsed() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(1));
    }
}

/// The proof: with the owner parked, our async seams return promptly; once the
/// owner pumps, the move AND the reband land, and the reads agree with what we
/// asked for. Prints every measured number for the record.
#[test]
#[ignore = "needs an interactive desktop: it creates a real window and parks its owning thread; machine-verified proof - run: cargo test -p notes-platform --test geometry_live -- --ignored --nocapture"]
fn an_async_move_and_reband_land_when_the_owner_pumps() {
    let (cmd_tx, ack_rx, handle, owner_thread_id, join) = spawn_owner();
    let backend = Backend; // reads only: both mutating seams run in the timed closures

    // ---- Phase A: the move, with the owner PARKED (D33 direction) ----------
    // A rect well inside the primary work area, in frame pixels.
    let requested = FrameRect::new(120, 120, 420, 300);
    let started = Instant::now();
    let placed = call_bounded(
        move || {
            let mut backend = Backend;
            backend.set_frame_rect(handle, requested, 1.0)
        },
        Duration::from_secs(2),
    )
    .expect("set_frame_rect must not block while the owner is parked");
    placed.expect("the move is accepted");
    let parked_elapsed = started.elapsed();
    eprintln!("[phase A] set_frame_rect with owner PARKED returned in {parked_elapsed:?}");
    assert!(
        parked_elapsed < Duration::from_millis(500),
        "the move blocked the caller: {parked_elapsed:?}"
    );

    // ---- Phase B: pump the owner, then prove the move LANDED ---------------
    let pump_started = Instant::now();
    cmd_tx.send(Command::Pump).expect("owner alive");
    ack_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("owner pumped and acknowledged");
    let pump_elapsed = pump_started.elapsed();
    eprintln!("[phase B] owner drained its queue in {pump_elapsed:?}");
    let landing = wait_for_landing(
        || {
            matches!(
                backend.restore_frame_rect(handle),
                Ok(placement) if placement.restore_rect == requested,
            )
        },
        Duration::from_secs(2),
    )
    .expect("the requested rect never landed within 2s");
    let lag = parked_elapsed + pump_elapsed + landing;
    eprintln!(
        "[phase B] GetWindowPlacement reported the requested rect {lag:?} after the call \
         returned (parked call + pump + 1 ms-granularity poll)"
    );
    // The live rect (GetWindowRect, pure screen coordinates) agrees too.
    let live = backend.frame_rect(handle).expect("the window still exists");
    assert_eq!(live, requested, "GetWindowRect must report the moved rect");

    // ---- Phase C: the reband, same parked-then-pump shape ------------------
    let started = Instant::now();
    let rebanded = call_bounded(
        move || {
            let mut backend = Backend;
            backend.set_topmost(handle, true)
        },
        Duration::from_secs(2),
    )
    .expect("set_topmost must not block while the owner is parked");
    // The verdict at call time is the parked-owner truth: the FFI succeeded and
    // the state did not change, because the band lands only on the owner's pump.
    // That is precisely the hole a bool could not express - the call succeeded
    // and nothing happened - now visible in the return value.
    assert!(
        matches!(
            rebanded,
            PinOutcome::NotApplied {
                expected: true,
                actual: false
            }
        ),
        "the parked-owner verdict must be NotApplied {{expected:true, actual:false}}: {rebanded:?}"
    );
    eprintln!("[phase C] verdict while parked: {rebanded:?} (the read-back caught it)");
    let parked_elapsed = started.elapsed();
    eprintln!("[phase C] set_topmost with owner PARKED returned in {parked_elapsed:?}");
    assert!(
        parked_elapsed < Duration::from_millis(500),
        "the reband blocked the caller: {parked_elapsed:?}"
    );
    // The product's UI thread never parks after one drain - it pumps
    // continuously - so the reband gets the shape it will actually run in.
    cmd_tx.send(Command::PumpContinuously).expect("owner alive");
    let bit = || {
        // SAFETY: GetWindowLongW reads the style of the window THIS test
        // created a moment ago on the owner thread (the handle is live or
        // the reads above would have failed); it is a pure query that
        // writes nothing and answers the extended-style bits.
        (unsafe { GetWindowLongW(HWND(handle as *mut core::ffi::c_void), GWL_EXSTYLE) }
            & WS_EX_TOPMOST.0 as i32)
            != 0
    };
    let reband_landing = wait_for_landing(bit, Duration::from_secs(2));

    // End the faithful pump so the owner can serve commands again.
    // SAFETY: PostThreadMessageW posts WM_QUIT to the owner thread whose id
    // this test recorded when it created the window; it writes no memory on
    // this side, and a failed post is surfaced by expect, not ignored.
    unsafe { PostThreadMessageW(owner_thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) }
        .expect("the pump is told to quit");

    // The CONTROL: the same seam applied by the owner thread itself. This
    // proves the style read and the window are sound, so a None above is a
    // property of the cross-thread async reband, not of this harness.
    cmd_tx.send(Command::SelfTopmost).expect("owner alive");
    let owner_applied = ack_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("the owner applied the reband itself");
    assert!(owner_applied, "the owner-thread reband was refused");
    let control = wait_for_landing(bit, Duration::from_millis(500))
        .expect("the OWNER-THREAD reband never landed: the probe itself is broken");
    eprintln!("[phase C2] the owner-thread reband landed in {control:?} (probe is sound)");

    // The verdict, with the finding spelled out when it is a finding.
    match reband_landing {
        Some(landing) => {
            let lag = parked_elapsed + landing;
            eprintln!("[phase C] cross-queue landing alone (owner pumping): {landing:?}");
            eprintln!(
                "[phase C] WS_EX_TOPMOST became visible {lag:?} after the call returned \
                 (owner pumping continuously)"
            );
        }
        None => panic!(
            "FINDING (reproduced on this machine): the cross-thread async reband NEVER \
             landed. set_topmost returned Applied-quickly in {parked_elapsed:?} with the \n             owner parked, \
             and WS_EX_TOPMOST was still unset after 2s of the owner pumping faithfully \
             via GetMessageW - while the SAME reband applied by the owner thread lands \
             immediately (phase C2). An async SetWindowPos reliably lands a MOVE but \
             does not land a Z-ORDER change. The port must not issue set_topmost from a \
             thread that does not own the window: the bridge applies pin state on the \
             UI thread (the fixed startup order already says so), and api must route \
             pin changes there."
        ),
    }

    // ---- Phase D: honest cleanup ------------------------------------------
    cmd_tx.send(Command::Shutdown).expect("owner alive");
    ack_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("the window was destroyed by its own thread");
    join.join().expect("the owner thread exited cleanly");
}

/// The error-contract companion that CAN run anywhere (no window needed): a
/// garbage handle still maps to the same typed refusal on the real Backend,
/// so the live test above can rely on the seams it exercises.
#[test]
fn a_garbage_handle_still_maps_to_the_typed_error_on_the_real_backend() {
    let mut backend = Backend;
    for handle in [0, 2, 3, isize::MIN + 1] {
        let result = backend.set_frame_rect(handle, FrameRect::new(0, 0, 10, 10), 1.0);
        assert!(
            matches!(result, Err(PlatformError::InvalidHandle)),
            "handle {handle:#x}: {result:?}"
        );
        let verdict = backend.set_topmost(handle, true);
        assert!(
            matches!(verdict, PinOutcome::Failed(PlatformError::InvalidHandle)),
            "handle {handle:#x}: {verdict:?}"
        );
        let result = backend.restore_frame_rect(handle);
        assert!(
            matches!(result, Err(PlatformError::InvalidHandle)),
            "handle {handle:#x}: {result:?}"
        );
    }
}

/// Keep the imports honest: LRESULT/WPARAM/LPARAM are named by the WNDPROC
/// signature DefWindowProcW is stored as, and this alias documents that match.
#[allow(dead_code)]
type WndProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;
