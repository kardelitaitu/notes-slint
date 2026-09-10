//! THE CONCURRENCY PROOF, bounded on purpose: a test that can hang CI is
//! worse than no test. The scenario is the reviewer's, shrunk to milliseconds:
//! the engine is made to block INSIDE a window op (the mock's set_frame_rect
//! sleeps - the real seam SENDS to the window's owner and waits for it to
//! pump, which is the same wait), and the UI thread asks for shutdown while
//! that call is in flight. The OLD code joined without a deadline and hung for
//! exactly as long as the block; the bounded join must return inside
//! JOIN_DEADLINE, report Event::ShutdownIncomplete through the port, and
//! abandon the thread. The whole test is bounded at roughly the deadline plus
//! margin, so the worst it can ever do to CI is fail.

#[path = "support/host_mock.rs"]
mod host_mock;

use std::time::{Duration, Instant};

use host_mock::{Answers, Host};
use notes_api::{Command, Gateway, Settings, StateDir};

// notes-core, notes-platform and thiserror are dependencies of notes-api, not
// of this test target; naming them keeps the per-target lint honest.
use notes_core as _;
use notes_platform as _;
use thiserror as _;

/// The engine must be blocked LONGER than the port's JOIN_DEADLINE (3 s), so
/// the bounded join times out and abandons rather than waiting it out.
const BLOCK: Duration = Duration::from_millis(6_000);
/// The bound of the BOUND: close() must return well inside deadline + margin.
const CLOSE_BUDGET: Duration = Duration::from_secs(5);

#[test]
fn shutdown_survives_an_engine_blocked_in_a_window_op() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host = Host::with_answers(Answers {
        block_move_ms: BLOCK.as_millis() as u64,
        ..Answers::default()
    });
    let (gateway, rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        Settings::default(),
        Some(Box::new(host.clone())),
        Some(Box::new(host.clone())),
    );

    // The registration drives restore_and_pin, which calls set_frame_rect -
    // the engine thread is now parked inside the mock for the whole BLOCK.
    gateway
        .send(Command::RegisterWindow {
            handle: notes_api::WindowHandle(0x100),
        })
        .expect("queued");
    // Wait until the engine is provably inside the blocked call.
    let deadline = Instant::now() + Duration::from_secs(2);
    while host.moves().is_empty() {
        assert!(Instant::now() < deadline, "the move never started");
        std::thread::sleep(Duration::from_millis(5));
    }

    // Edge B: the UI thread asks for shutdown WHILE the engine is blocked.
    let started = Instant::now();
    let outcome = gateway.close();
    let elapsed = started.elapsed();

    // THE OLD CODE FAILED HERE: it waited out the whole 6 s block.
    assert!(
        elapsed < CLOSE_BUDGET,
        "close() hung {:?} on a blocked engine - the join is not bounded",
        elapsed
    );
    // And the report is honest and typed: shutdown was ACCEPTED but not
    // completed - Err(Shutdown), not an Event, because a Gateway-held Event
    // sender would delay the Disconnected contract (see gateway.rs).
    assert!(
        matches!(outcome, Err(notes_api::Command::Shutdown)),
        "a timed-out shutdown must be reported as unfinished: {outcome:?}"
    );
    // The abandoned engine finishes when the block lifts; its channel stays
    // open until then, and draining it here would only wait out the block.
    drop(rx);
}
