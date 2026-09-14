//! STRIP-4 debt reckoning, row 3: THE PANIC HOOK FINALLY EATS ITS OWN GUN.
//!
//! The hook in src/product.rs has existed since 2fc924f3 and the record said so in as many
//! words: only panic_note was tested, "no live panic was provoked". That is code-paid and
//! proof-owed - a voice nobody had ever heard use. This file is the missing child: a REAL
//! notes-slint.exe, a REAL panic, and the REAL line on its REAL stderr.
//!
//! WHAT IS ASSERTED, and why each half is owed:
//!   (a) the child exits NONZERO *and* the hook line is on its stderr. The PAIR is the gate: an
//!       exit code alone does not say who ended the process, and a printed line alone does not
//!       say the run then died of it. The number 101 is printed as detail only, never asserted -
//!       THIS box builds the default panic = "unwind", which is why 101 is what shows up, but
//!       panic = "abort" is one profile key away and an aborting panic still voices both lines
//!       and still exits nonzero. Asserting the exact code would make the test cargo's profile
//!       rather than the hook its subject.
//!   (b) a line STARTING with the bridge's one voice - "notes-gpui: panic:", plumbing::report's
//!       prefix plus the word panic_note puts first - that also carries the probe's token, so the
//!       line is provably THIS panic and not some other panic passing by.
//!   (c) std's own "panicked at" report is ALSO on that stderr. This is the half no unit test can
//!       reach: the hook chains the default hook (default_hook(info)), and OUR line beside STD'S
//!       is what proves the chain ran. A hook that printed and then swallowed the default would
//!       pass (a) and (b) and still lose the location and the backtrace a person needs.
//!
//! THE CONTROL, and exactly what it proves: the same binary, the same pipe, NO order given, must
//! not voice the probe line. That is a statement about THE GATE and is written as one. It is NOT
//! "the product does nothing" - this is a GUI binary, on a station that may not be able to host a
//! window at all (this box currently has pump trouble), and a bad run is entitled to print what it
//! honestly prints. If the control goes red on something OTHER than the token, that is a fact about
//! the machine and belongs in the record, not in a weakened assert. The assert stays.
//!
//! ONE ORDER, PRE-WINDOW, BY DESIGN - and the refusal beside it. product.rs panics before
//! state_dir() and before Gateway::start, so this child touches no session file and never asks for
//! a window. A mid-loop variant would need a timer, the timer would race the event pump, and what
//! ships is a flaky test with a proof's name on it. So: no second order, and the comment at the
//! gate itself says why.
//!
//! Shape precedent: crates/core/tests/crash_safety.rs:27-31 and :45-103 - a real child, its own
//! captured pipe, a deadline that KILLS rather than waits, and the per-target dependency shims,
//! because a test target is its own crate and this one uses std alone.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// Per-target unused-crate shims: the lint fires per test binary, and this binary needs nothing
// from this package's four dependencies.
use notes_api as _;
use raw_window_handle as _;
use rfd as _;
use slint as _;

/// The variable product.rs reads, and the token its panic message carries. One name each, so the
/// two asserts cannot disagree about what was ordered.
const PROBE_VAR: &str = "NOTES_PANIC_PROBE";
const PROBE_TOKEN: &str = "NOTES_PANIC_PROBE=startup";
/// The voice, prefix and all: every line this bridge speaks goes through plumbing::report and
/// nothing else writes. Asserted as a PREFIX because "which voice" is half of the claim.
const HOOK_VOICE: &str = "notes-gpui: panic:";

/// Launch the product, optionally ordering the startup panic, read its stderr to a deadline, and
/// hand back the lines plus the exit status if it had one. The pipe is drained on its own thread
/// so the deadline bounds the CHILD rather than the speed of its talking: a GUI that will not
/// answer is killed, exactly as crash_safety kills a wedged writer.
fn launch(order: bool, patience: Duration) -> (Vec<String>, Option<std::process::ExitStatus>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_notes-slint"));
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    if order {
        cmd.env(PROBE_VAR, "startup");
    } else {
        cmd.env_remove(PROBE_VAR);
    }
    let mut child = cmd
        .spawn()
        .expect("the product binary spawns - cargo sets CARGO_BIN_EXE for");
    let stderr = child
        .stderr
        .take()
        .expect("stderr was piped above, so it is still here");
    let reader = std::thread::spawn(move || {
        BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
            .collect::<Vec<String>>()
    });
    let started = Instant::now();
    let mut ended = None;
    while started.elapsed() < patience {
        ended = child.try_wait().expect("try_wait on the product child");
        if ended.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    // The deadline is the interesting half of the control, and it is the reason the wait below is
    // unconditional: a GUI binary that will not answer in time is KILLED, never waited out - which
    // is exactly what crash_safety does to a wedged writer. Then ONE wait on every path, so the
    // reaped status is always read and no zombie is left on the box for the session.
    if ended.is_none() {
        let _ = child.kill();
    }
    let _reaped = child
        .wait()
        .expect("the child is reaped on every path out of this helper");
    // `ended` deliberately stays None on the killed path: the gate is "did this child die OF the
    // panic", and a death this test handed it is not that answer. The status above is read only so
    // the wait is real, and the nonzero code is asserted from the voluntary one.
    let lines = reader
        .join()
        .expect("the stderr reader outlives the pipe it holds");
    (lines, ended)
}

#[test]
fn a_real_child_that_really_panics_is_voiced_by_the_hook_and_still_chains_std() {
    let (lines, ended) = launch(true, Duration::from_secs(30));
    let Some(status) = ended else {
        panic!(
            "a child ordered to panic at startup was still running after 30 s and was killed - \
             the hook did not own the run. stderr: {lines:#?}"
        );
    };
    // (a) GATE: nonzero. The 101 is named in the message as the unwind-profile detail and is
    // never itself the condition - see the panic=abort caveat in the header.
    assert!(
        !status.success(),
        "a provoked panic that still exits {status} is not a panic this hook owns: {lines:#?}"
    );

    // (b) the hook line, in the bridge's voice, naming THIS panic.
    let Some(voiced) = lines
        .iter()
        .find(|line| line.starts_with(HOOK_VOICE) && line.contains(PROBE_TOKEN))
    else {
        panic!(
            "the hook never spoke: no line starting {HOOKVOICE:?} and carrying {TOKEN:?} on the \
             stderr of a child that WAS ordered to panic. stderr: {lines:#?}",
            HOOKVOICE = HOOK_VOICE,
            TOKEN = PROBE_TOKEN
        );
    };

    // (c) std's report beside ours - the chained default hook, and nothing else that proves it.
    let std_reported = lines.iter().any(|line| line.contains("panicked at"));
    assert!(
        std_reported,
        "our line ({voiced}) arrived with no std \"panicked at\" behind it: the hook printed and \
             did not chain the default hook, so the location and the backtrace are lost - the one half \
             a unit test can never see. stderr: {lines:#?}"
    );

    println!("panic_hook: exit {status}; the child said: {voiced}");
}

#[test]
fn the_probe_is_inert_when_the_variable_is_absent() {
    // THE CONTROL, and its stated limit (header): 1.5 s is longer than this binary needs to print
    // its startup lines, and what is asserted is ONLY that the gate did not fire. The child is
    // killed by the deadline either way - a clean no-op is not on offer from a GUI binary.
    let (lines, _) = launch(false, Duration::from_millis(1500));
    let spoke = lines
        .iter()
        .any(|line| line.starts_with(HOOK_VOICE) && line.contains(PROBE_TOKEN));
    assert!(
        !spoke,
        "the gate fired with NOTHING ordered: a control run voiced {PROBE_TOKEN:?}, so the \
             startup panic is no longer behind its variable. stderr: {lines:#?}"
    );
}
