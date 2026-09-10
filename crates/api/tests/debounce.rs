//! M4/D11: the autosave debounce actually DEBOUNCES. A burst of Flushes that
//! all land inside one idle window produces exactly ONE save, at the LAST
//! revision; after that quiet period, a new Flush still saves - the deadline
//! re-arms after firing. Nothing in the suite had ever waited out the 750 ms
//! tick, so the cadence was only ever checked structurally.
//!
//! SIGNAL-DRIVEN, deliberately: every positive wait is a recv with a generous
//! bound - waiting for the event, never for the clock. The two negative
//! assertions (no second save; one failure only in the session tests) have no
//! event to wait for, because absence has no event; those hold a bounded
//! silence window of two tick periods, which is a bound, not a poll.
//!
//! CI margin, stated: the positive bounds are 5 s, more than six idle periods,
//! so ordinary jitter only delays an assertion. The one red-risk is the engine
//! thread being preempted for a full 750 ms BETWEEN the three sends of the
//! burst - three channel sends take microseconds, so that needs a stall of the
//! entire idle period inside a three-instruction loop, and if a machine is
//! that loaded, red is the honest answer. The silence windows can only end
//! early under a stall; they cannot fabricate an event.
//!
//! FINDING (this test is #[ignore]d, not green): the debounce this file was
//! written to prove DOES NOT EXIST. Engine::flush saves IMMEDIATELY on every
//! accepted Flush - the deadline is touched only by on_tick, so AUTOSAVE_IDLE
//! gates the session/settings write alone. This test ran red with Saved
//! revision 1 arriving in 0.04 s: three Flushes, three saves. Deferring the
//! document save to the tick is a real design change (the save path, the
//! shutdown drain and the reentrancy event-count contracts all move), so it
//! is routed, not smuggled in. The day the debounce lands, delete the
//! #[ignore] and this file is the proof.

use std::fs;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use notes_api::{Command, Event, Gateway, Settings, StateDir};

// notes-core, notes-platform and thiserror are dependencies of notes-api, not
// of this test target; naming them keeps the per-target lint honest.
use notes_core as _;
use notes_platform as _;
use thiserror as _;

/// Generous bound for an event that WILL come: more than six idle periods of
/// the engine's 750 ms cadence (engine::AUTOSAVE_IDLE), which this file proves.
const ANSWER: Duration = Duration::from_secs(5);

/// The silence window of a negative assertion: two idle periods plus change.
const QUIET: Duration = Duration::from_millis(750 * 2 + 100);

/// Waits for the FIRST event satisfying `f`, and returns it. A timeout is a
/// failure with the name of what never came, not a silent skip.
fn wait_for(
    rx: &Receiver<Event>,
    bound: Duration,
    what: &str,
    f: impl Fn(&Event) -> bool,
) -> Event {
    let deadline = Instant::now() + bound;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(event) if f(&event) => return event,
            Ok(_) => continue,
            Err(_) => panic!("timed out waiting for {what}"),
        }
    }
}

#[ignore = "the engine has no document debounce yet: flush() saves per Flush, AUTOSAVE_IDLE gates only session/settings - see the FINDING note above"]
#[test]
fn a_burst_of_flushes_saves_once_at_the_last_revision_and_the_deadline_re_arms() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A .notes file: armed for autosave the moment it opens (ADR-0001), so a
    // Flush is a save-in-waiting rather than an immediate NoTarget answer.
    let path = dir.path().join("debounce.notes");
    fs::write(&path, b"zero\n").expect("write the fixture");

    let (gateway, rx) = Gateway::start(StateDir(dir.path().to_path_buf()), Settings::default());
    gateway
        .send(Command::Open { path: path.clone() })
        .expect("queued");
    wait_for(
        &rx,
        ANSWER,
        "the Open to load",
        |ev| matches!(ev, Event::Loaded { path: p, .. } if p == &path),
    );

    // THE BURST: three revisions inside one idle window. Three channel sends
    // take microseconds; the 750 ms deadline cannot split them.
    for revision in 1..=3u64 {
        gateway
            .send(Command::Flush {
                text: format!("revision {revision}\n"),
                revision,
            })
            .expect("queued");
    }

    // SIGNAL, not clock: the save itself is what we wait for.
    let saved = wait_for(&rx, ANSWER, "the coalesced save", |ev| {
        matches!(
            ev,
            Event::Saved {
                revision: 1..=3,
                ..
            }
        )
    });
    match saved {
        Event::Saved { revision, .. } => {
            assert_eq!(
                revision, 3,
                "one save, at the LAST revision of the burst (D11), not the first"
            );
        }
        other => panic!("expected Saved, got {other:?}"),
    }

    // EXACTLY ONE: wait out two idle periods. A second Saved here is a broken
    // debounce. (A Clean AutosaveSkipped arriving in this window is the engine
    // being honest about the ticks that find nothing to do - filtered.)
    let quiet_until = Instant::now() + QUIET;
    while let Ok(event) = rx.recv_timeout(quiet_until.saturating_duration_since(Instant::now())) {
        assert!(
            !matches!(event, Event::Saved { .. }),
            "the burst produced more than one save: {event:?}"
        );
    }
    // And the bytes on disk are the last revision, written exactly once.
    assert_eq!(
        fs::read(&path).expect("read the file back"),
        b"revision 3\n",
        "the coalesced save wrote the last buffer"
    );

    // THE RE-ARM: after the quiet period, a new Flush still saves - the
    // deadline did not die with the tick that fired it.
    gateway
        .send(Command::Flush {
            text: "revision 4\n".to_string(),
            revision: 4,
        })
        .expect("queued");
    wait_for(&rx, ANSWER, "the re-armed save", |ev| {
        matches!(ev, Event::Saved { revision: 4, .. })
    });
    assert_eq!(
        fs::read(&path).expect("read the file back"),
        b"revision 4\n",
        "the second save is the second buffer"
    );
    gateway.close().ok();
}
