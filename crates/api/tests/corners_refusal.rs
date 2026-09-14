//! THE WINDOWS-10 CORNER REFUSAL, WALKED THROUGH A REAL `Gateway`.
//!
//! The knob has been wired since the corner slice landed - the shared fake records
//! [`host_mock::Call::CornerRounding`] and refuses on
//! [`host_mock::Answers::fail_corner_rounding`], and `Engine::apply_corner_rounding`
//! turns a refusal into [`Event::CornerRoundingFailed`] - but no integration test
//! ever TURNED it. `engine.rs`'s own corner tests drive a local `CornerBackend` by
//! calling `handle()` directly: no Gateway, no engine thread, no channel. So "an OS
//! that has never heard of attr 33 gets that answer back to a bridge" rested on unit
//! tests inside the crate that owns the code under test.
//!
//! This file closes that gap the way `geometry.rs` closed the geometry gap: commands
//! in through [`Gateway::send`], events out through the receiver the caller was
//! handed, every platform call made by the shared fake (nothing here touches Win32),
//! and every assertion a recorded call or an event.
//!
//! THE FRAGILE SEAM, NAMED WHERE IT LIVES. The bridge's corner policy does not ask
//! "was I refused", it asks "was I refused BY THE SET CALL", and it decides that from
//! a STRING: `crates/bridge-slint/src/surface.rs:1915-1951` holds
//! `const SET_CORNER_API: &str = "DwmSetWindowAttribute";` and
//! `corner_refusal_is_permanent` retires the asking on exactly
//! `reason.contains(SET_CORNER_API)`. What it matches is `PlatformError`'s Display
//! text, flattened by the port - `engine.rs:1784-1788` does `reason:
//! error.to_string()` - so a whole permanence decision, the difference between one
//! log line on Windows 10 and a pointless corner ask on every maximise forever, hangs
//! on a prose sentence surviving `to_string()`. That is the seam FIX-A named, and the
//! reason this file exists: the assertions below are on the substring THE BRIDGE
//! MATCHES, not on a paraphrase of it. Reword `PlatformError::Win32`'s Display, stop
//! naming the call that refused, or drop the field, and this test goes red here
//! instead of that quietly becoming product behaviour.

/// The shared fake carries helpers only `geometry.rs` needs (`moves`, `topmost`,
/// `restore_reads`); this file needs the answers and the corner call list, so the
/// unused ones are silenced HERE - the precedent hang.rs and roundtrip_port.rs state,
/// so the shared support file stays untouched.
#[allow(dead_code)]
#[path = "support/host_mock.rs"]
mod host_mock;

use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use host_mock::{Answers, Call, Host};
use notes_api::{Command, Event, Gateway, Session, Settings, StateDir, WindowHandle};

use notes_core::session::write_session;

// notes-platform and thiserror are dependencies of notes-api, not of this test
// target; naming them keeps the per-target unused-crate-dependencies lint honest.
use notes_platform as _;
use thiserror as _;

/// Long enough that a loaded machine cannot flake; short enough that a hung engine
/// fails the run instead of hanging it forever.
const ANSWER: Duration = Duration::from_secs(5);

/// THE STRING. `crates/bridge-slint/src/surface.rs:1917` holds this exact const and
/// `corner_refusal_is_permanent` (line 1949) matches the event's `reason` against
/// it. Copied rather than imported on purpose: `api` may not import a bridge and a
/// bridge may import `api` plus its own toolkit and nothing else (AGENTS.md), so the
/// whole contract between the two IS a string. The honest way to test a string
/// contract is to name the string on both sides; change one and this is the red light.
const SET_CORNER_API: &str = "DwmSetWindowAttribute";

/// What Windows 10 answers `DWMWA_WINDOW_CORNER_PREFERENCE` (attr 33) with: not
/// silence, a refusal. `crates/api/src/engine.rs:2893-2898` words the same case for
/// the in-crate fixture, and the OS text travels untranslated - which is why this
/// asserts the sentence ARRIVES carrying the call's name, not the sentence itself.
const W10_REFUSAL: &str = "The parameter is incorrect. (os error 87)";

/// The handle the window is registered under, and the number the recorded corner ask
/// must carry back: an ask aimed at a handle nothing registered is the D10 violation
/// this suite exists to catch. `i64` because that is the port's handle type
/// (`command.rs:35`); the recorded call narrows it to the `isize` the seam takes.
const HANDLE: i64 = 0x100;

/// Starts an engine on `dir` with the fake host answering `answers`, and hands the
/// SAME host back for assertions: the engine owns one clone, the test the other, and
/// the call log is shared through the `Arc` inside.
///
/// [`Gateway::start_with_host`] and not [`Gateway::start`], and only because the
/// mock is NOT the default host here: on Windows `start` builds the real
/// `notes_platform::windows::Backend` (D46, `gateway.rs:230-268`), which answers a
/// made-up handle with `InvalidHandle` - a TRANSIENT refusal, the other half of
/// FIX-A - and never the Windows-10 case at all. The thread, both channels and every
/// routing decision are the real ones; only the host is swapped, which is what D47's
/// seam is for.
fn start_with(dir: &Path, answers: Answers) -> (Gateway, Receiver<Event>, Host) {
    let host = Host::with_answers(answers);
    let (gateway, rx) = Gateway::start_with_host(
        StateDir(dir.to_path_buf()),
        Settings::default(),
        Some(Box::new(host.clone())),
        Some(Box::new(host.clone())),
    );
    (gateway, rx, host)
}

/// Drains until `want` events match `wanted` and hands back EVERY event seen.
/// Signal-driven, never a sleep: the engine is one thread consuming one queue in
/// order, so an event that has not arrived by the deadline has not been emitted.
fn until_wanted(rx: &Receiver<Event>, want: usize, wanted: &dyn Fn(&Event) -> bool) -> Vec<Event> {
    let deadline = Instant::now() + ANSWER;
    let mut seen = Vec::new();
    while seen.iter().filter(|event| wanted(event)).count() < want {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            !left.is_zero(),
            "timed out waiting for {want} matching events; saw {seen:?}"
        );
        match rx.recv_timeout(left) {
            Ok(event) => seen.push(event),
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                panic!("the engine closed the event channel before the verdict; saw {seen:?}")
            }
        }
    }
    seen
}

/// Every corner ask the host was actually given, in order. The event stream is silent
/// on success, so this list is the only receipt that the seam was driven at all - and
/// with what value, against what handle.
fn corner_asks(host: &Host) -> Vec<(isize, bool)> {
    host.calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::CornerRounding { handle, round } => Some((handle, round)),
            _ => None,
        })
        .collect()
}

/// THE WINDOWS-10 PATH, end to end: the bridge asks for round corners, the fake OS
/// has never heard of the attribute, and the refusal comes back over a real channel
/// carrying the one word the bridge's permanence decision reads.
#[test]
fn a_refused_corner_ask_comes_back_naming_the_call_the_bridge_matches_on() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_session(dir.path(), &Session::default()).expect("write the session fixture");
    let (gateway, rx, host) = start_with(
        dir.path(),
        Answers {
            fail_corner_rounding: Some(W10_REFUSAL.to_string()),
            ..Answers::default()
        },
    );

    gateway
        .send(Command::RegisterWindow {
            handle: WindowHandle(HANDLE),
        })
        .expect("queued");
    // TWO asks of one shape, because the second half of the claim is that the PORT
    // retires nothing: `apply_corner_rounding` has no `confirmed` latch and no
    // dedupe (`engine.rs:1764-1778` says so), so on an OS that has never heard of
    // attr 33 every maximise earns the same refusal. That repetition is exactly why
    // the bridge needs `corners_refused` - and therefore exactly why the string it
    // matches on has to survive the flattening.
    gateway
        .send(Command::SetCornerRounding(true))
        .expect("queued");
    gateway
        .send(Command::SetCornerRounding(true))
        .expect("queued");

    let seen = until_wanted(&rx, 2, &|event| {
        matches!(event, Event::CornerRoundingFailed { .. })
    });
    let refusals: Vec<&String> = seen
        .iter()
        .filter_map(|event| match event {
            Event::CornerRoundingFailed { reason } => Some(reason),
            _ => None,
        })
        .collect();
    assert_eq!(
        refusals.len(),
        2,
        "one refusal per refused ask, nothing latched in the port: {seen:?}"
    );
    for reason in refusals {
        // THE STRING CONTRACT, asserted the way the bridge asserts it.
        assert!(
            reason.contains(SET_CORNER_API),
            "the refusal has to name {SET_CORNER_API} - the substring \
             crates/bridge-slint/src/surface.rs:1949 matches on - or the bridge can \
             never tell a permanent refusal from a transient one: {reason}"
        );
        assert!(
            reason.contains(W10_REFUSAL),
            "and it has to carry the OS's own words untranslated: {reason}"
        );
    }
    // What the port actually handed the OS: two asks, one shape, the handle this test
    // registered. The event is a verdict about a call, not a guess about one.
    assert_eq!(
        corner_asks(&host),
        vec![(HANDLE as isize, true), (HANDLE as isize, true)],
        "both asks reached the seam, on the registered handle, with the value asked \
         for"
    );
    gateway.close().expect("shutdown joins the engine");
}

/// THE OTHER HALF, which a test on the refusal alone cannot say: an OS that KNOWS
/// attr 33 answers `Ok`, and `Ok` is SILENCE. Silence is the cheap thing to fake - a
/// command that was never routed emits nothing either - so the claim rides on a LATER
/// command's event as a barrier: one thread, one queue, in order, so once the pin
/// verdict queued behind the corner ask has arrived, the ask has been handled and the
/// port has said nothing about it.
#[test]
fn an_accepted_corner_ask_says_nothing_and_the_engine_keeps_answering() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_session(dir.path(), &Session::default()).expect("write the session fixture");
    // `Answers::default()` IS the Windows 11 answer - `fail_corner_rounding: None`,
    // because the OS this product is developed on knows the attribute.
    let (gateway, rx, host) = start_with(dir.path(), Answers::default());

    gateway
        .send(Command::RegisterWindow {
            handle: WindowHandle(HANDLE),
        })
        .expect("queued");
    gateway
        .send(Command::SetCornerRounding(true))
        .expect("queued");
    // The barrier, queued AFTER the corner ask. Unlike the corner arm, this one
    // always answers: nothing has confirmed `pinned = true` yet, so `apply_topmost`
    // emits `Event::Pinned(true)`.
    gateway.send(Command::SetPinned(true)).expect("queued");
    let seen = until_wanted(&rx, 1, &|event| matches!(event, Event::Pinned(true)));

    assert!(
        !seen
            .iter()
            .any(|event| matches!(event, Event::CornerRoundingFailed { .. })),
        "an accepted ask has NO event at all - no success twin, and no refusal: \
         {seen:?}"
    );
    assert_eq!(
        corner_asks(&host),
        vec![(HANDLE as isize, true)],
        "the silence is an ACCEPTED ask, not an unrouted one: the seam was called \
         exactly once, on the registered handle"
    );
    gateway.close().expect("shutdown joins the engine");
}
