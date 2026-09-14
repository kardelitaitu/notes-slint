//! THE PIN'S ROUND TRIP, PORT-SIDE: asked in one session, answered by the platform,
//! persisted to disk, and come back as the NEXT session's own first fact - with the
//! window put back where it was left, not shoved by the pin.
//!
//! WHY THIS FILE EXISTS AND WHERE ITS HALF STOPS. ADR-0006 item 6 owes "the pin check
//! mark's round trip" on the Slint side. The claim decomposes into four legs, and this
//! file walks the three a machine without input can walk AT THE PORT:
//!   (1) an ask is an APPLY, not an echo - `Command::SetPinned(true)` reaches the host
//!       seam as `Topmost` and comes back as `Event::Pinned(true)` only BECAUSE the apply
//!       confirmed it (`engine.rs:1715 apply_topmost` is the sole emitter of both `Pinned`
//!       and `PinFailed`);
//!   (2) the confirmed bit PERSISTS - it lands in session.json through core's own writer on
//!       the shutdown path, not in a memory the process keeps;
//!   (3) a RELAUNCH re-applies it without being asked again, delivers the answer to the
//!       fresh session as an event, and moves the window ONCE, at the rect on disk - so the
//!       pin never disturbs the placement (whitepaper 5.5: restore the frame, then apply
//!       topmost; and the port's own order is pin-then-geometry, which geometry.rs's
//!       `a_refused_move_is_reported_not_swallowed` already owns);
//!   (4) the CHECK MARK paints - `Event::Pinned` -> `ui.set_pinned` -> chrome's pin cell.
//!       The two windowless legs of (4) are welded in `crates/bridge-slint/src/surface.rs`
//!       (`the_answer_leg_of_the_pins_round_trip_flips_the_real_model_the_check_mark_reads`
//!       and `the_caption_pin_cell_binds_the_very_bit_the_answer_writes`); that file's
//!       header names the one pixel no machine without a window can see.
//!
//! WHAT WAS ALREADY PAID FOR, so this file is a WELD and not a repeat. Each row cites the
//! owner of the half it does NOT re-prove:
//!   session.rs:geometry_and_the_pin_bit_survive_a_restart_and_leave_no_temp_litter proves
//!     (2) - a dirty pin reaches session.json and a second engine's `startup_state()` reads
//!     it back - but it never registers a window, so it sees no platform call and no event,
//!     and therefore cannot say the next launch actually topmosts anything.
//!   geometry.rs:a_maximized_session_is_never_moved_but_still_pinned proves the topmost call
//!     made from a session restored off disk, for a MAXIMISED session, and deliberately
//!     drops its receiver (`let (gateway, _rx, host)`): the event a restored pin sends to a
//!     fresh session is unasserted there, and a normal (moved) session is the other arm of
//!     the registration.
//!   corners_refusal.rs asserts an `Event::Pinned(true)` arrives, but only as the answer to a
//!     live ask the engine had never confirmed - not as a restore.
//!   geometry.rs's rows at :1469, :1510 and :1596 assert the pin bit's persistence BESIDE
//!     their geometry claims, never what a relaunch then DOES with it.
//! Nothing in this repo walked ask -> confirm -> disk -> relaunch -> re-apply -> event in one
//! run, and nothing asserted that a RESTORED pin arrives as an event. That weld is this file,
//! and every leg in it is asserted on its own too.
//!
//! The arrangement is geometry.rs's and corners_refusal.rs's: commands in through
//! [`Gateway::send`], events out through the handed-back receiver, every platform call made
//! by the shared fake (nothing here touches Win32), and one restart that is the SAME
//! DIRECTORY rather than a copy of it, because that is what a restart is.

/// The shared fake carries helpers this file does not use (`restore_reads`, `drop_takes`,
/// ...); silencing them HERE is the precedent `corners_refusal.rs`, `hang.rs` and
/// `roundtrip_port.rs` state, so the support file itself stays untouched.
#[allow(dead_code)]
#[path = "support/host_mock.rs"]
mod host_mock;

use std::path::Path;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use host_mock::{Answers, Host};
use notes_api::{Command, Event, Gateway, Rect, Session, Settings, StateDir, WindowHandle};

use notes_core::session::{read_session, write_session};
use notes_platform::FrameRect;

// thiserror is a dependency of notes-api, not of this test target; naming it keeps the
// per-target unused-crate-dependencies lint honest.
use thiserror as _;

/// Long enough that a loaded machine cannot flake; short enough that a hung engine fails the
/// run instead of hanging it forever.
const ANSWER: Duration = Duration::from_secs(5);

/// The work area the shared fake reports (`Answers::default`), named so the claim "this
/// restore was not a clamp" is a measurement and not a hope.
const WORK_AREA: (i32, i32, u32, u32) = (0, 0, 1920, 1032);

/// The rect this trip is left at: fully on-screen, and NOT the monitor default, so "came
/// back in the place you left it" is a claim about the persisted number.
const LEFT_AT: (i32, i32, u32, u32) = (120, 80, 900, 620);

fn tuple_of(rect: FrameRect) -> (i32, i32, u32, u32) {
    (rect.x, rect.y, rect.w, rect.h)
}

/// The fake's answers for this trip: it REPORTS the rect the session was seeded with, so
/// "the port persisted what the host measured" and "the port restored the rect it was told"
/// are the SAME number - which keeps every assertion below about the pin and off the measure
/// (`a_fresh_install_writes_the_session_with_the_rect_the_host_reports` owns that row).
fn answers() -> Answers {
    Answers {
        restore: Some(FrameRect::new(LEFT_AT.0, LEFT_AT.1, LEFT_AT.2, LEFT_AT.3)),
        ..Answers::default()
    }
}

/// Launch a session on `dir`, with the shared fake as its whole platform.
fn start_with(dir: &Path, host: &Host) -> (Gateway, Receiver<Event>) {
    Gateway::start_with_host(
        StateDir(dir.to_path_buf()),
        Settings::default(),
        Some(Box::new(host.clone())),
        Some(Box::new(host.clone())),
    )
}

/// Registration is where 5.5's restore-and-apply happens.
fn register(gateway: &Gateway, handle: i64) {
    gateway
        .send(Command::RegisterWindow {
            handle: WindowHandle(handle),
        })
        .expect("queued");
}

/// Wait until the engine thread has recorded at least `at_least` platform CALLS: the
/// registration is asynchronous, and `geometry.rs` learned that a poll which races it reads
/// an empty log rather than an empty run.
fn wait_for_calls(host: &Host, at_least: usize, what: &str) {
    let deadline = Instant::now() + ANSWER;
    while host.calls().len() < at_least {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The same clock on the one call this file counts, because `calls()` is satisfied by the
/// pin apply that runs BEFORE the move: a test that wants the move waits for the move.
fn wait_for_moves(host: &Host, at_least: usize, what: &str) {
    let deadline = Instant::now() + ANSWER;
    while host.moves().len() < at_least {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// THE TRIP. Session one opens UNPINNED at the rect on disk; the strip asks for the pin;
/// the platform confirms it; the answer reaches the bridge; the app quits. Session two asks
/// for nothing, and the restored bit alone must topmost the new handle, say so as
/// `Event::Pinned(true)` to the fresh session, and move the window exactly once, at the rect
/// that was left behind.
#[test]
fn a_pin_asked_in_one_session_is_the_next_sessions_own_answer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let left = Rect::new(LEFT_AT.0, LEFT_AT.1, LEFT_AT.2, LEFT_AT.3);
    write_session(
        dir.path(),
        &Session {
            rect: left,
            pinned: false,
            ..Session::default()
        },
    )
    .expect("seed an unpinned session at a known rect");

    // ---- session one -------------------------------------------------------
    let host = Host::with_answers(answers());
    let (gateway, rx) = start_with(dir.path(), &host);
    register(&gateway, 0x100);

    // Leg 0, the OFF state as an ANSWER. The check mark starts from what the port said,
    // not from a default, so a launch that restored unpinned is told it is unpinned - and
    // only the FIRST event of the registration is claimed here.
    let opened = rx.recv_timeout(ANSWER).expect("an applied restore answers");
    assert!(
        matches!(opened, Event::Pinned(false)),
        "leg 0: a launch that restored unpinned must be TOLD it is unpinned: {opened:?}"
    );
    wait_for_calls(&host, 1, "the registration's pin apply");
    assert_eq!(
        host.topmost(),
        vec![(0x100, false)],
        "leg 0: the announcement is a call the host recorded, not prose"
    );

    // THE ASK. One strip click's worth - surface.rs:1527 `on_toggled_pin` sends exactly
    // this one command and renders nothing locally.
    gateway
        .send(Command::SetPinned(true))
        .expect("the strip's ask");
    let answered = rx.recv_timeout(ANSWER).expect("the ask is answered");
    assert!(
        matches!(answered, Event::Pinned(true)),
        "leg 1: the answer must be the platform's confirmation, not an echo: {answered:?}"
    );
    assert_eq!(
        host.topmost(),
        vec![(0x100, false), (0x100, true)],
        "leg 1: one click, one apply, on the handle THIS session registered"
    );

    // Quit: the final session write happens inside the drain, which is what makes reading
    // the file below a claim about the run rather than a race with it.
    gateway.close().expect("the quit joins the engine");
    let persisted = read_session(dir.path()).expect("session.json");
    assert!(
        persisted.pinned,
        "leg 2: the confirmed pin has one home, and it is session.json"
    );
    assert_eq!(
        persisted.rect, left,
        "leg 2: and the place the window was left is still the place on disk"
    );

    // ---- session two: nothing is asked, and yet the same answer arrives -----
    let host2 = Host::with_answers(answers());
    let (gateway2, rx2) = start_with(dir.path(), &host2);
    register(&gateway2, 0x200);

    let restored = rx2
        .recv_timeout(ANSWER)
        .expect("the relaunch answers for the pin it restored");
    assert!(
        matches!(restored, Event::Pinned(true)),
        "leg 3: nobody clicked in this session, and the check mark still has its answer: {restored:?}"
    );
    wait_for_moves(&host2, 1, "the relaunch's restore move");
    assert_eq!(
        host2.topmost(),
        vec![(0x200, true)],
        "leg 3: the NEW handle is the one that got topmost, at the new number"
    );
    let moves = host2.moves();
    assert_eq!(moves.len(), 1, "leg 3: exactly one move - the restore");
    assert_eq!(
        tuple_of(moves[0].1),
        LEFT_AT,
        "leg 3: at the rect that was left behind, in frame pixels, unshoved - and inside a
         work area of {WORK_AREA:?}, so it is not a clamp wearing a restore"
    );
    gateway2.close().expect("the second quit joins the engine");
}

/// THE OTHER DIRECTION OF THE SAME WELD, because a check mark that only ever turns on is not
/// a state: an unpin persists, and a relaunch applies the FALSE bit as a call the host
/// records - never the silence a bridge could mistake for "nothing to do".
#[test]
fn an_unpin_is_the_bit_that_comes_back_and_says_so() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_session(
        dir.path(),
        &Session {
            rect: Rect::new(LEFT_AT.0, LEFT_AT.1, LEFT_AT.2, LEFT_AT.3),
            pinned: true,
            ..Session::default()
        },
    )
    .expect("seed a PINNED session");

    let host = Host::with_answers(Answers::default());
    let (gateway, rx) = start_with(dir.path(), &host);
    register(&gateway, 0x400);
    let opened = rx.recv_timeout(ANSWER).expect("the restore applies");
    assert!(
        matches!(opened, Event::Pinned(true)),
        "a pinned session comes back pinned: {opened:?}"
    );
    wait_for_calls(&host, 1, "the pinned apply");

    // The second click of the same strip: ask for false.
    gateway.send(Command::SetPinned(false)).expect("queued");
    let off = rx.recv_timeout(ANSWER).expect("the unpin is answered");
    assert!(
        matches!(off, Event::Pinned(false)),
        "leg 1 reversed: {off:?}"
    );
    gateway.close().expect("the quit joins the engine");
    assert!(
        !read_session(dir.path()).expect("session.json").pinned,
        "leg 2 reversed: an unpin is persisted too"
    );

    let host2 = Host::with_answers(Answers::default());
    let (gateway2, rx2) = start_with(dir.path(), &host2);
    register(&gateway2, 0x500);
    let restored = rx2.recv_timeout(ANSWER).expect("the relaunch applies");
    assert!(
        matches!(restored, Event::Pinned(false)),
        "leg 3 reversed: a relaunch must not resurrect a pin the user took off: {restored:?}"
    );
    wait_for_calls(&host2, 1, "the relaunch's apply");
    assert_eq!(
        host2.topmost(),
        vec![(0x500, false)],
        "leg 3 reversed: the false bit is APPLIED as false, and said as false"
    );
    gateway2.close().expect("the second quit joins the engine");
}
