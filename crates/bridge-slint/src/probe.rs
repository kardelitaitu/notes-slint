//! SLINT SPIKE, slice 1 - THE WINDOW CONTRACT, and nothing else.
//!
//! One question, with a printed number as its answer: can a Slint window be (a)
//! placed at the rect the PORT REPORTS (its `InitialState`, never the file itself)
//! BEFORE it is ever visible, (b) shown maximised when that snapshot says so, and (c)
//! yield an HWND to the port without a
//! visible correction jump afterwards? That is deal-breaker 1 of the spike. No
//! editor, no menu, no text widget: anything that is not the question is not in
//! this file.
//!
//! Startup order mirrors `bridge-gpui` (AGENTS.md, whitepaper §5.5): query state ->
//! create window AT the stored rect -> show -> read the handle -> `RegisterWindow` ->
//! let the port apply the pin.
//!
//! THE NEEDLES are deliberately in the shape `xtask smoke` already reads - the
//! `notes-gpui: ` prefix, `startup: `, `event: ` and `status line: `. The prefix
//! keeps gpui's name on purpose: smoke greps that literal, and a spike that renames it
//! proves nothing about the harness.
//!
//! WHY THE FRAME RECT COMES FROM A FILE: a bridge may not import `windows` (AGENTS.md,
//! and check-arch's `windows*` family ban), so this crate cannot call GetWindowRect.
//! The PORT can and does - `RegisterWindow` measures through `restore_frame_rect` and
//! writes session.json with the answer - so for the duration of a spike the port IS
//! this bridge's GetWindowRect. A shipped bridge would never read a state file behind
//! the port, and nothing here should be copied into one.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use notes_api::{
    Command, DropGuard, Exit, Gateway, Settings, StateDir, WindowHandle, resolve_state_dir,
};
// STRIP-2b: hwnd_of moved to plumbing, but the probe's own RISK-3 print still asks the
// toolkit for the INNER handle directly, so the trait is in scope here too.
use raw_window_handle::HasWindowHandle;
use slint::{ComponentHandle, LogicalPosition, LogicalSize, Timer, TimerMode};
mod plumbing;
pub(crate) use plumbing::{
    Fingerprint, arm_drop_target, do_no_harm, fingerprint_of, fnv1a, hwnd_of, lf, note_dot,
    port_said, publish_title, report, send,
};
mod surface;
mod title_contract;
#[cfg(test)]
use surface::undo_quarantined;
// STRIP-1: the probe reaches the surface through this ONE list. `unused_imports` is allowed here
// and nowhere else, because five of these names are consumed only by this crate's tests, which
// still live in probe.rs; when the tests that read them move to surface.rs the allow moves with
// them rather than becoming a permanent fixture.
#[allow(unused_imports)]
pub(crate) use surface::{
    DialogKind, DialogReply, EDITED_IS_DIRTY_WITNESS, Pump, Route, SHORTCUTS, answer_dialog,
    ask_dialog, caption_glyph, dialog_starting_dir, dialog_words, drain, legend, lock_verdict,
    next_generation, note_adoption, route_of, suggested_name, text_pump, text_pump_by_compare,
};

mod ui_gen;
// The generated component, re-exported at the root so the crate's shape does not change with it.
use ui_gen::Spike;

/// PER-INSTANCE ISOLATION IS A BRIDGE CHOICE, NOT A PORT GAP - and this is the
/// correction to what this file claimed last slice. `Gateway::start` takes the
/// directory as an argument (`pub fn start(state_dir: StateDir, settings: Settings)`
/// at crates/api/src/gateway.rs:258) and resolves nothing itself; `resolve_state_dir`
/// is a pure function the CALLER runs, which is how `xtask`'s tests point a Gateway at
/// a throwaway dir. So api needs no new surface: the bridge simply picks.
///
/// The pick below still runs the production rule (portable `data` beside the exe wins,
/// else %APPDATA%\notes-gpui) and reports it, then OVERRIDES the answer with a
/// directory beside this spike's own probe files. Without that both debug bridges read
/// and write `target/debug/data`, and a pin or geometry needle printed here can be a
/// fact about the OTHER process - which is what happened twice in earlier slices.
fn state_dir() -> StateDir {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let portable = exe_dir.join("data").is_dir();
    let appdata = (!portable)
        .then(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .flatten();
    let resolved = resolve_state_dir(&exe_dir, appdata.as_deref());
    let mine = exe_dir.join("slint-probe").join("data");
    if std::fs::create_dir_all(&mine).is_ok() {
        report(&format!(
            "state-dir: isolated {} (port rule alone would give {})",
            mine.display(),
            resolved.0.display()
        ));
        return StateDir(mine);
    }
    report("state-dir: could not create the isolated dir, sharing the resolved one");
    resolved
}

/// An integer out of session.json, by scan. Crude on purpose: a probe may not add a
/// serde dependency to the graph it is measuring.
fn json_int(src: &str, key: &str) -> Option<i64> {
    let at = src.find(&format!("\"{key}\""))?;
    let rest = &src[at..];
    let colon = rest.find(':')? + 1;
    let digits: String = rest[colon..]
        .chars()
        .skip_while(|c| c.is_whitespace())
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    digits.parse().ok()
}

/// A JSON bool, read from its OWN value and nothing else.
///
/// RC2 - the one that mattered. This used to slice 40 bytes from the key and ask whether
/// that window contained "true". On a pretty-printed session the 40 bytes after the
/// `"maximized"` key are `: false,` + newline + `"pinned"` - so a file holding
/// maximized:false, pinned:true answered TRUE. Every `maximized=` needle this spike
/// printed through `measured()` was therefore a PIN INDICATOR wearing a maximise label,
/// including the ones that fed the phantom maximise story. Two bugs in three lines: the
/// window is key-order dependent, and `at + 40` panics outright on a short file. Reading
/// the value up to its own terminator has neither problem.
fn json_flag(src: &str, key: &str) -> Option<bool> {
    let at = src.find(&format!("\"{key}\""))?;
    let value = src[at..].split_once(':')?.1;
    // Stop at the end of THIS value - comma, newline, closing brace - not at an arbitrary
    // number of bytes after the key name.
    let value = value
        .trim_start()
        .split([',', '\n', '}', ' '])
        .next()?
        .trim();
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// The frame rect as the PORT last measured and wrote it - this spike's GetWindowRect.
fn measured(dir: &StateDir, at: Duration) -> String {
    match std::fs::read_to_string(dir.0.join("session.json")) {
        Ok(src) => {
            let rect = match (
                json_int(&src, "x"),
                json_int(&src, "y"),
                json_int(&src, "w"),
                json_int(&src, "h"),
            ) {
                (Some(x), Some(y), Some(w), Some(h)) => format!("{}x{} at {},{}", w, h, x, y),
                _ => "UNPARSEABLE".to_string(),
            };
            // Both bits, each from its own value: this is the line that has to AGREE with
            // the bytes on disk, and it did not until RC2 (see json_flag). Printing the
            // pin beside the maximise is what makes a swap visible instead of plausible.
            let zoom = json_flag(&src, "maximized").unwrap_or(false);
            let pin = json_flag(&src, "pinned").unwrap_or(false);
            format!("frame rect (port-measured) t+{at:?}: {rect} maximized={zoom} pinned={pin}")
        }
        Err(err) => format!("frame rect t+{at:?}: no session.json yet ({err})"),
    }
}

/// S4d: the LOCKED fixture - small, and read-only on disk - and the story of why the lever
/// changed twice. The 9 MiB version this replaces could never have worked: D9 is a REFUSAL, not a
/// verdict attached to a load. `Engine::open` stats the bytes and answers
/// `Event::LoadFailed { reason: TooLarge }` (event.rs:241-248), so no `Loaded` and no FileMeta
/// ever arrive for an oversized file - which is exactly why the old act printed silence where it
/// expected a lock, and why it was waiting on an adoption the port had already decided not to
/// send. The `oversize` arm of `lock_verdict` is kept (if the port ever starts carrying it, the
/// bridge is one call from honouring it) and is UNREACHABLE today: all four FileMeta construction
/// sites in engine.rs (995, 1073, 1156, 1318) hard-code `oversize: false`.
///
/// The attribute is therefore the only lever that reaches `FileMeta::read_only`, and std cannot
/// set it on a stable toolchain (`PermissionsExt::set_readonly` is the unstable
/// `windows_permissions_ext`, rust#152956) while the `windows` crate is precisely what the
/// layering rule forbids a bridge to import. So the bridge asks the OS's own utility through a
/// process: `attrib +R`. No dependency, no FFI, no rule bent - and the child's stderr is
/// reported, because a silent failure here would be indistinguishable from a port that never
/// answers, which is the exact mistake this act is replacing.
fn write_lock_fixture(path: &Path) -> std::io::Result<u64> {
    let body = "locked by the disk, not by size\n";
    std::fs::write(path, body.as_bytes())?;
    Ok(body.len() as u64)
}

/// S4f: the 9 MiB fixture, whose ONLY job is to be refused. D9 answers from the stat alone, so
/// this file never becomes a document - which is exactly what makes it the cheapest live witness
/// for the `Event::LoadFailed` arm the read-only fixture cannot exercise (that one is readable,
/// so it answers `Loaded`). Kept deliberately separate from write_lock_fixture: two different
/// verdicts, two different proofs.
fn write_big_fixture(path: &Path) -> std::io::Result<u64> {
    use std::io::Write;
    let chunk = "x".repeat(1024);
    let mut file = std::fs::File::create(path)?;
    for _ in 0..9216 {
        file.write_all(chunk.as_bytes())?;
    }
    file.flush()?;
    Ok(9216 * chunk.len() as u64)
}

fn run_attrib(path: &Path, flag: &str) -> std::io::Result<()> {
    let made = std::process::Command::new("attrib")
        .arg(flag)
        .arg(path)
        .output()?;
    if made.status.success() {
        return Ok(());
    }
    Err(std::io::Error::other(format!(
        "attrib {flag} {} exited {}: {}",
        path.display(),
        made.status,
        String::from_utf8_lossy(&made.stderr).trim()
    )))
}

/// The quiet window before a change is reported, in the spirit of the gpui bridge's
/// debounce. Shortened so the probe fits inside its own lifetime.
const QUIET: Duration = Duration::from_millis(1200);

/// When the probe prints its second reading, and when it ends.
const SAMPLE: Duration = Duration::from_millis(300);

// S3 (pin + pump): when the strip asks on its own, how long the confirmed style is
// watched before the experiment reports, and how long an answer to the re-ask is
// allowed to take to arrive. The run has to outlive all three, so END grew.
const PIN_AT: Duration = Duration::from_millis(600);
const HOLD: Duration = Duration::from_millis(1000);
const SETTLE: Duration = Duration::from_millis(300);

// The clincher: the run has to outlive PIN_AT + HOLD + SETTLE by enough margin to
// show WHERE the answer lands, and LATE_ASK exists because the previous run proved
// the registration answer can miss the whole loop - so the ask waits for that answer
// but never waits forever.
const LATE_ASK: Duration = Duration::from_millis(4000);

/// S4c: the save-failure cycle. Three acts inside the 25 s lifetime: take the path away
/// (READ_ONLY_AT), type (KEY2_AT) so the debounce has something to send and the port must
/// fail, then put it back (WRITE_BACK_AT) so the next save lands and the latch clears.
/// The middle act is deliberately NOT a read-only attribute: an autosave writes by
/// renaming a temp file OVER its target, and Windows lets a rename replace a read-only
/// file - the attribute would prove nothing. What reliably fails is a DIRECTORY sitting
/// on the target path, a lesson this repo already paid for (crates/xtask/src/smoke.rs:
/// "Access is denied (os error 5) ... exactly what renaming a file onto a directory
/// does"). Same error, staged on purpose, in the one place it can be undone.
const READ_ONLY_AT: Duration = Duration::from_millis(12000);
const KEY2_AT: Duration = Duration::from_millis(13500);
const WRITE_BACK_AT: Duration = Duration::from_millis(17000);

/// S6: THE CHORDS, WITH NO HANDS EITHER. A key press cannot be injected - there is no
/// public Slint API to feed a FocusScope a KeyEvent (checked against the vendored
/// 1.17.1 sources: no `dispatch_key_input`, no `KeyInputEvent` anywhere in `slint` or
/// `i-slint-core`), so the tick drives the SAME root callbacks the capture handler calls,
/// walking the SHORTCUTS table to decide which one and printing the needle from the table's
/// own display string. That makes the table load-bearing at runtime instead of a comment.
/// What it still does NOT prove is the near half - a physical key reaching
/// capture-key-pressed and matching there. That half is on the manual list, and the
/// compile-time witness for it is `slint-viewer --check` on the capture tree.
/// S7: the overlay acts - open, clamp at three host widths, click-away, Escape - and then
/// ABOUTSLINT's three: open the licence screen through the row's own door, read it back, and
/// dismiss it through the same dismissal door the popup uses. Spaced, not
/// instantaneous: a resize only reaches Chrome's clamp after the next layout pass, so reading
/// popup-x in the same statement that asked for the new size would print the old one. The
/// window is put back to its starting size before the act ends, because the NEXT launch
/// restores whatever size this one persisted.
/// ELEVEN steps now, the last of them an observation with no act behind it: the eight that were
/// here before are untouched in both order and output, so every old tally still means what it
/// meant, and the two About needles sit after them.
const OVERLAY_AT: Duration = Duration::from_millis(15000);
const OVERLAY_EVERY: Duration = Duration::from_millis(260);

const CHORD_AT: Duration = Duration::from_millis(18500);

/// Where steps 1..n of the walk start, and why the first step is alone: the audit needs a
/// seed-current window - Open makes the seed current, SEED_KEY_AT types into it, and the
/// 750 ms debounce decides when the save can land. Every other chord breaks that window.
/// Alt+2 opens a DIFFERENT recent (this run's was s4-loop, which is how the audit silently
/// saved the wrong file twice, printing a perfectly confident hash line about it), Save As
/// re-points the engine, and Auto-save disarms it. So the tail starts after the flush would
/// have landed, and the seed keeps its moment.
const CHORD_TAIL_AT: Duration = Duration::from_millis(21600);
/// 1100 ms, not 800: the seed's own save (SEED_KEY_AT plus the 750 ms debounce) has to
/// land INSIDE the walk, between the two steps that make the seed current and the step that
/// turns autosave off. At 800 ms Auto-save fired 150 ms before the flush, and the audit
/// silently disappeared again - which is how a needle that never prints is worth more than a
/// comment that says it should.
const CHORD_EVERY: Duration = Duration::from_millis(500);

/// Which table rows to drive, by index into SHORTCUTS: Open, Alt+2, Auto-save, Save As,
/// Clear recents. The order is a constraint, not a preference, and it cost a run to learn:
///  * Alt+2 before Clear - clearing first empties the list the recents act reads, so the Alt
///    needle would prove nothing.
///  * Save As AFTER the seed keystroke (SEED_KEY_AT, 19.2 s). Ctrl+S re-points the engine at
///    s4-loop.notes, and the seed's own flush lands ~750 ms after that keystroke; driving
///    Save As first meant no Event::Saved ever carried the seed path, so the do-no-harm[saved]
///    audit silently vanished from the run - the §4.5 line-ending question went UNTESTED,
///    twice, and nothing complained because the needle was simply absent. This order keeps
///    the seed current across its own save: Open makes it current, Alt+2 re-selects the same
///    file, and only then does Save As move on.
///
/// Quit closes the sequence and has no chord, which is the table's own absence, not an
/// oversight.
const CHORD_DRIVE: &[usize] = &[0, 5, 2, 1, 3];
/// S8: the caption acts, and they close the run. Nine steps from here: minimize, observe,
/// un-minimize, maximize THROUGH THE BUTTON'S OWN DOOR, a drag attempt on a maximised window,
/// observe, restore, observe, close. The last replaces the Quit row's act - same callback,
/// different door - because that is the point of the slice: the app gains a pointer way to
/// reach what the chords and the rows already reached.
const CAPTION_AT: Duration = Duration::from_millis(23700);
const CAPTION_EVERY: Duration = Duration::from_millis(350);

/// S9 follow-up: one keystroke AFTER the Open row made the seed the current document, so
/// an autosave lands ON THE SEED and the do-no-harm rule is tested where it can actually
/// break - on the write, not just on the read. The engine saves to whatever path is
/// current, so this sits after Open AND after Alt+2 (an Open of the same file - a second
/// Open re-adopts last_sent and CANCELS a pending flush, which is the collision the previous
/// ordering hit); the 750 ms debounce then puts the Saved at ~20.55 s, ahead of the
/// Auto-save step at 20.7 s.
/// ADR-0001 is why this exists. The seed is a FOREIGN file, and a foreign document is not
/// armed for autosave until an explicit SaveAs names it - so every earlier attempt to audit
/// a save OF THE SEED produced no Event::Saved at all: the bridge typed, the debounce
/// elapsed, the Flush went out (or was skipped) and the engine simply did not write, which
/// reads as a missing needle rather than a disarmed document. Arming = an explicit SaveAs of
/// the seed onto itself, BEFORE the keystroke; it is also a save-path test on its own, since
/// the text is unchanged and only the line endings could move.
const SEED_ARM_AT: Duration = Duration::from_millis(19400);
const SEED_KEY_AT: Duration = Duration::from_millis(20200);

/// S4b: when the one in-run keystroke lands. After the pin experiment has finished
/// (PIN_AT + HOLD + SETTLE) and after the seed Open has adopted a buffer, so the tick
/// count, the dot and the flush are all measured against a settled baseline.
const LATE_KEY: Duration = Duration::from_millis(9000);
/// Print one tick needle per this-many milliseconds, from inside the Timer callback.
const TICK_MS: u64 = 2000;

/// When the probe drives a recent row by itself: after the seed's open, so the list
/// holds more than one file and the click has something to aim at.
const CLICK_AT: Duration = Duration::from_millis(3000);

// R1/R2 act times. The frame act goes first so the maximise is in the state the
// close act then has to carry out of the window, and the two closes are far enough
// apart to see whether the declined one really left the window on screen.
/// S5: the frame act stands down for the DRAG CHAIN runs, for the same documented reason
/// the close pair did (see SYNTHETIC_CLOSE_ACTS): a window that ends maximised comes BACK
/// maximised, and from then on every relaunch restores a maximised frame - which masks the
/// one thing the chain is measuring, a moved rect surviving a round trip. Its own evidence
/// ("frame: synthetic toggled to ...") is on record from every earlier run. Flip to true to
/// restore it; S5's finding about what a drag does TO a maximised window is in the report,
/// and it needed this act ON to be seen.
const SYNTHETIC_FRAME_ACTS: bool = false;

const MAX_AT: Duration = Duration::from_millis(14000);
const CLOSE_AT: Duration = Duration::from_millis(18000);
const CLOSE2_AT: Duration = Duration::from_millis(21000);

/// S9: THE SYNTHETIC CLOSE PAIR STANDS DOWN FOR THIS RUN, on purpose.
///
/// The pair proves the decline holds and that a typed-then-closed race is carried by the
/// shutdown path - both real, both already witnessed in earlier runs. But it also made
/// the menu Quit row untestable: by the time Quit fired, closes was already 1, so the
/// pre-existing "grant anything that is not the first request" branch answered it and the
/// run proved nothing about the QUIT PRECEDENCE. With the pair off, the FIRST close
/// request this app ever sees is the Quit row's, and only quit_requested can grant it.
/// Flip to true to bring both acts back; their evidence and this one cannot share a run,
/// because a granted close ends it.
const SYNTHETIC_CLOSE_ACTS: bool = false;
/// S5: when the synthetic drag runs. BEFORE MAX_AT, because a maximised window reports a
/// normalised position and this arithmetic is about chrome, not show state; and well before
/// the save cycle, so the port has time to store the moved rect before the run ends.
const DRAG_AT: Duration = Duration::from_millis(7000);

/// S5: the SECOND drag, deliberately placed AFTER MAX_AT so it lands on a maximised window
/// when the frame act is on. That is the regression test for the finding this guard
/// answers: the store must still hold the rect from DRAG_AT, not a target computed from the
/// maximised frame origin. When the frame act stands down, this is just another move.
const DRAG2_AT: Duration = Duration::from_millis(16000);

/// S4e: THE LOCK, MOVED - after the chord walk (CHORD_TAIL_AT 21.6 s plus its 500 ms steps) and
/// before the caption machine (CAPTION_AT 23.7 s), which is the one seam nothing else owns. The 4d
/// run showed what the startup placement cost: an early Open puts the fixture IN the recents list,
/// the Alt+1..0 walk then re-opens it, and every stretch with a locked document current is a
/// stretch the pump correctly refuses to save - so the flush chain dropped from 4 to 1 and this one
/// act silently rewrote another slice's evidence. Movement, not mechanics: the act now runs last,
/// restores the seed as the open document, and the run closes unlocked with the cleanup at exit.
const LOCK_AT: Duration = Duration::from_millis(23000);
/// The answer measured 10.7 ms, and the old 2 s bound was pre-loop sleep that ate the run's tick
/// budget (invocations 3056 -> 1105). Tight now, and the report still says plainly when it expires.
const LOCK_WAIT: Duration = Duration::from_millis(250);

const END: Duration = Duration::from_millis(28000);

fn main() {
    let dir = state_dir();
    report(&format!("startup: state dir {}", dir.0.display()));

    let (gateway, events) = Gateway::start(dir.clone(), Settings::default());
    // `Receiver` is not `Clone`, but the repeated `Timer` closure below has to drain it
    // as well as the caller. An `Rc` is the cheap answer, and `&Rc<Receiver<_>>` still
    // coerces to the `&Receiver<_>` that `drain` takes - no other line changes shape.
    let events = Rc::new(events);
    let gateway = Rc::new(RefCell::new(Some(gateway)));
    let Some(initial) = gateway
        .borrow_mut()
        .as_mut()
        .and_then(|gateway| gateway.startup_state())
    else {
        report("startup: startup_state() gave no snapshot, nothing to place");
        return;
    };
    let session = initial.session.clone();
    // S9: the file read is GONE from the startup path. The numbers below were already
    // InitialState's - only the wording admitted the habit. The snapshot carries rect,
    // monitor, scale, the maximise bit, the pin bit and the open path, so nothing about
    // PLACING this window needs a second opinion from a file the bridge does not own, and
    // nothing had to be added to the port to drop it. The gate is the startup_state()
    // match above: the snapshot is handed out exactly once, synchronously, before the
    // window exists - it does not arrive on the channel, so there is no arrival to wait
    // for. And no fact needed here is missing from it, so this is not an add-the-event
    // case; the one thing the file still answers is "what did the port persist", which
    // is what measured() is now left for.
    report(&port_said(
        &session.rect,
        session.scale_factor,
        session.maximized,
        session.pinned,
    ));

    let Ok(ui) = Spike::new() else {
        report("startup: no component");
        return;
    };
    let window = ui.window();
    // S6: the legend FIRST, before any event can overwrite the status line - menu.rs prints
    // it on the first frame for the same reason: with no menu bar, this is the only place a
    // user learns the keys exist. Printed rather than only set, because a needle is evidence
    // and a status line at t=0 is overwritten by the first Event.
    report(&format!("chord: legend {}", legend()));
    // The untitled arm is an ACT, not an absence: name it before anything is
    // visible, so the markup default cannot be seen by a user or a test.
    publish_title(&ui, None, false, "startup");

    // ---- (a) placed at the stored rect, BEFORE anything is visible ------------
    // The stored rect is FRAME pixels, physical; Slint takes LOGICAL CLIENT pixels.
    // The scale division is the bridge's own; the frame-to-client step is the one
    // NEITHER bridge can do (the chrome is not knowable before a window exists) and
    // is the reason the numbers below can differ from the seed by a border.
    let scale = if session.scale_factor.is_finite() && session.scale_factor > 0.0 {
        session.scale_factor
    } else {
        1.0
    };
    let want = LogicalSize::new(session.rect.w as f32 / scale, session.rect.h as f32 / scale);
    let at = LogicalPosition::new(session.rect.x as f32 / scale, session.rect.y as f32 / scale);
    window.set_size(want);
    window.set_position(at);
    report(&format!(
        "before-show: asked for {want:?} at {at:?} -> toolkit reads {:?}",
        fingerprint_of(window)
    ));
    // RISK 3, measured before the loop has spun even once.
    report(&format!(
        "before-show: HWND available = {} (inner = {:?})",
        hwnd_of(window).is_some(),
        window.window_handle().window_handle().err().map(|_| "err")
    ));

    // ---- (b) the show state the file asked for -------------------------------
    if session.maximized {
        window.set_maximized(true);
        report(&format!(
            "before-show: set_maximized(true) -> is_maximized() = {}",
            window.is_maximized()
        ));
    }

    if window.show().is_err() {
        report("show() refused: no window, nothing further to measure");
        return;
    }
    let shown = fingerprint_of(window);
    report(&format!("after-show: toolkit reads {shown:?}"));
    // S5, THE RESTORE HALF of the question the drag asks. The port stores FRAME pixels and
    // this toolkit reports frame-inclusive physical px, so a correct restore is delta <0,0>.
    // The first bridge's bug shows up here as a constant offset in Y (the bar) that repeats
    // on every relaunch - printed at BOTH ends of a run because the walk is cumulative: the
    // startup number IS the previous run's exit number.
    report(&format!(
        "drag-check[startup]: intended <{},{}> actual <{},{}> delta <{},{}>",
        session.rect.x,
        session.rect.y,
        shown.rect.x,
        shown.rect.y,
        shown.rect.x - session.rect.x,
        shown.rect.y - session.rect.y
    ));
    // S6b: the drop target's lease, declared BEFORE the first registration site so it outlives
    // both of them, and dropped explicitly on the exit path. BESIDE Pump rather than inside it,
    // on purpose: Pump is the state the unit tests build with `Pump::default()`, and a DropGuard
    // in it would make that struct thread-owned and un-constructible off the window thread. The
    // guard is a resource, not a measurement.
    let drop_guard: Rc<RefCell<Option<DropGuard>>> = Rc::new(RefCell::new(None));
    let started = Instant::now();
    let hwnd = hwnd_of(window);
    match hwnd {
        Some(hwnd) => {
            report(&format!(
                "hwnd = {hwnd:#x} FIRST VISIBLE, {}",
                port_said(
                    &session.rect,
                    session.scale_factor,
                    session.maximized,
                    session.pinned
                )
            ));
            send(
                &gateway,
                Command::RegisterWindow {
                    handle: WindowHandle(hwnd),
                },
            );
            // S6b: arm on the success path - and note what "success" can mean here. `send` is a
            // post, not a call: RegisterWindow answers with no Event, and the one failure it can
            // report ("the engine had already exited") is already printed by send itself. There
            // is no acknowledgement to wait for, so the moment the handle is named is the moment
            // the arm is right - and an arm that landed after the engine left is a guard nobody
            // drags onto, disarmed on exit like everything else.
            arm_drop_target(hwnd, &drop_guard);
            if initial.pinned {
                send(&gateway, Command::SetPinned(true));
            }
        }
        None => report("NO HWND after show: risk 3 CONFIRMED, the contract fails here"),
    }
    // The pump's memory. `pinned` in the UI is written ONLY from an Event::Pinned
    // (C3: a check mark that flips on the request is the UI believing itself), so the
    // pump is the one place the ask, the confirmation and the hold experiment live.
    // HONOUR the stored setting, then override it ONCE for the spike: autosave is what
    // this slice is trying to observe, and `Settings::default()` ships it off, which is
    // exactly why the 58-byte keystroke died as `AutosaveSkipped reason=AutosaveDisabled`.
    // A shipped bridge does the opposite - it reads the bit, draws the menu check from
    // it, and only `Command::SetAutosave` changes it.
    report(&format!(
        "autosave: InitialState says enabled={}",
        initial.autosave_enabled
    ));
    if !initial.autosave_enabled {
        send(&gateway, Command::SetAutosave(true));
        report("autosave: SetAutosave(true) sent - spike override, after registration");
    }
    let pump = Rc::new(RefCell::new(Pump::default()));
    // The autosave bit the port holds now: InitialState's answer, or ON because the
    // spike override just above sent SetAutosave(true). Stated as the sequence of
    // commands this bridge sent, because no event confirms it (see Pump::autosave).
    pump.borrow_mut().autosave = true;
    note_dot(&pump, &ui.as_weak(), false, "start");
    text_probe(&ui);

    // (a) THE SAVE PATH, so the loop can actually close. File-less, nothing can land
    // and the port's only answer is `AutosaveSkipped`; that is not a port gap, it is
    // no path. `SaveAs` is the sanctioned way to name a live buffer - it takes the
    // text with the path, arms autosave (ADR-0001) and announces a document
    // generation through `Event::Rebound`. The file goes under target\debug, created
    // here: not a core fixture (the validator owns those) and not one of smoke's
    // freshness roots (src/ and ui/), so it trips neither guard.
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
    {
        let place = exe_dir.join("slint-probe");
        if std::fs::create_dir_all(&place).is_ok() {
            let path = place.join("s4-loop.notes");
            pump.borrow_mut().loop_path = Some(path.clone());
            let text = lf(&ui.get_buffer());
            let revision = pump.borrow().edits;
            report(&format!(
                "save-as: asked {} with {} bytes (epoch={} revision={} at the ask)",
                path.display(),
                text.len(),
                pump.borrow().epoch,
                revision
            ));
            send(
                &gateway,
                Command::SaveAs {
                    path,
                    text,
                    revision,
                },
            );
            // A REAL change AFTER the save, or the loop is untested rather than
            // working: the buffer at this instant holds exactly the bytes that just
            // landed, so the next Flush is a re-send of clean text and the only answer
            // core can give is `AutosaveSkipped`. This append is the typed character.
            let grew = format!("{}X", lf(&ui.get_buffer()));
            report(&format!(
                "keystroke-after-save: buffer now {} bytes",
                grew.len()
            ));
            ui.set_buffer(grew.into());

            // (1) THE DO-NO-HARM SEAM, end to end, through the port ONLY: seed a file
            // whose bytes are CRLF on disk, open it with `Command::Open`, let the
            // adoption put LF text in the buffer, and re-hash the file on the way out.
            // If the bridge or core rewrote the ending, the hash moves and this prints
            // hash-equal=0. No notes-core, no platform - text crosses as a Command.
            let seed = place.join("s5-seed.notes");
            let raw = "line one\r\nline two\r\nline three\r\n".to_string();
            if std::fs::write(&seed, raw.as_bytes()).is_ok() {
                let mut p = pump.borrow_mut();
                p.seed = Some(seed.clone());
                p.hash_before = fnv1a(raw.as_bytes());
                p.hash_len = raw.len();
                p.flushes_at_open = p.edits;
                drop(p);
                report(&format!(
                    "seed: wrote {} bytes (CRLF x{}) fnv={:#x} to {}",
                    raw.len(),
                    raw.matches("\r\n").count(),
                    fnv1a(raw.as_bytes()),
                    seed.display()
                ));
                // S4e: the LOCKED ACT USED TO BE HERE, at startup. It moved - see LOCK_AT for why,
                // and the reason is a schedule fact, not a code one: opening the fixture early put
                // it in the recents list, and the Alt+1..0 chord walk re-opened it, so the run spent
                // parts of itself on a document the pump rightly refuses to save and someone
                // else's flush evidence went 4 -> 1.
                // The seed LAST, so the run ends up on the document every later needle assumes,
                // and `locked` falls back to false before the loop starts.
                report(&format!("open: asked {} via Command::Open", seed.display()));
                send(&gateway, Command::Open { path: seed });
                drain(&events, &pump, &ui.as_weak());
            } else {
                report("seed: could not write the CRLF file beside the exe");
            }
        } else {
            report("save-as: could not create the probe directory beside the exe");
        }
    }
    drain(&events, &pump, &ui.as_weak());

    // ---- the poll, and the second reading ------------------------------------
    let weak = ui.as_weak();
    let state = Rc::new(RefCell::new(Poll {
        handed: hwnd.is_some(),
        last: shown,
        changed: None,
        sends: 0,
        sampled: false,
    }));
    let dir = Rc::new(dir);
    let tick = Timer::default();
    let third_dir = Rc::clone(&dir);
    // S9: the dialog answer lane, created before the tick that reads it, because the tick
    // is the only thing in this binary allowed to notice that a person has finished
    // choosing. One try_recv per tick, never an await, never a timer of its own.
    let (dialog_tx, dialog_rx) = std::sync::mpsc::channel::<DialogReply>();
    let dialog_rx = Rc::new(RefCell::new(dialog_rx));
    let dialog_tx = Rc::new(dialog_tx);
    let third_gw = Rc::clone(&gateway);
    let third_events = Rc::clone(&events);
    let third_pump = Rc::clone(&pump);
    // S6b: the tick's own handle on the lease, for the registration site that actually fires in
    // a normal run (the HWND appears only after the loop spins).
    let third_drops = Rc::clone(&drop_guard);
    tick.start(TimerMode::Repeated, Duration::from_millis(8), {
        let state = Rc::clone(&state);
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let now = started.elapsed();
            // S4c: no witness is set here any more, and this note exists so a later reader does
            // not re-add one. S10c asked "has the loop ticked?" - first of `invocations`, a pump
            // counter an act can bump, then of `loop_ticked`, which no act can touch but which
            // still only ever answered WHEN the event arrived. The hazard is not about when, it is
            // about WHICH document, so note_adoption compares Pump::adopted_path and the question
            // stops being answerable by accident. `third_pump` is still borrowed below, as before.
            let seen = fingerprint_of(ui.window());
            // S8: NOTHING to feed here, and that is the finding. Window's own 'maximized'
            // builtin (builtins.slint:1524) is what Chrome's caption cell binds to, so the
            // glyph follows a state change from any source - double-click, the button, the
            // taskbar, Win+Up - without this pump remembering it. A mirror written from the
            // poll would have been a second copy of a fact the window can change behind our
            // back, which is the bug class this spike keeps finding.
            // One limit, measured: the builtin is visible to MARKUP bindings but has no generated Rust
            // getter (no Spike::get_maximized exists), so these needles read the same fact imperatively
            // via window().is_maximized(). Bar and bridge agree because they ask the same window, not
            // because either of them remembers.
            // ---- THE TICK NEEDLE: does this callback actually run during ui.run()? ----
            // `drains` is how many times the pump has polled the channel from this callback,
            // `seen` counts what the pump has taken so far. Their combination settles
            // finding 1: ticks + queue=0 + seen=0 while the port has answered means the
            // answers were never sent in-run, i.e. the pump is not the problem. If no
            // tick prints at all, it is the Timer wiring that is wrong, not the engine.
            {
                let mut pin = third_pump.borrow_mut();
                pin.ticks += 1;
                let n = pin.ticks;
                let bucket = now.as_millis() as u64 / TICK_MS;
                let show = bucket != pin.last_bucket;
                if show {
                    pin.last_bucket = bucket;
                }
                // `Receiver::len` does not exist in this std, so the depth is
                // reported indirectly: `drains` is how many times the pump has been
                // called from inside this callback, which is the quantity that actually
                // decides finding 1.
                let (queue, seen_n, answered) = (pin.drains, pin.seen, pin.answered);
                drop(pin);
                if show {
                    report(&format!(
                        "tick {n} t={}ms drains={queue} seen={seen_n} answered={answered} handed={}",
                        now.as_millis(),
                        state.borrow().handed
                    ));
                }
            }
            // S4, THE ONE-LINER: the pump now runs on EVERY tick, so event latency
            // is the 8 ms tick and not "whenever the next hand-written poll happens".
            drain(&third_events, &third_pump, &ui.as_weak());
            // S9: the picker answer, if one has come in. try_recv is a question with a
            // one-word answer; when it is not yet, the tick goes on to pump, autosave and
            // drain exactly as it did while the modal was up. That IS the rule, tested.
            {
                let rx = dialog_rx.borrow();
                while let Ok(reply) = rx.try_recv() {
                    answer_dialog(reply, &third_gw, &third_pump, &ui);
                }
            }
            // S10b: the arming of the quarantine, reported ONCE, at the moment the generation
            // first leaves zero - which, since S4c, means the FIRST REAL SWITCH by construction:
            // note_adoption steps on document IDENTITY, so the startup pair (Rebound then Loaded,
            // both naming the same path) leaves the counter at zero however late the answer
            // arrives, and undo stays native on the document the app reopened. Before S4c this
            // claim was TEMPORAL - "the adoptions arrive before the pump has ever run" - and one
            // 9 MiB open that pushed an answer past the first tick made it false in a live run.
            // The per-keystroke
            // swallow needle can only come from a real hand (1.17 has no key-injection API,
            // established in S6), so this is what a headless run can honestly show: the switch
            // happened, the capture branch is armed, and the cost is being paid from here on.
            {
                let mut p = third_pump.borrow_mut();
                if !p.quarantine_reported && ui.get_doc_generation() > 0 {
                    p.quarantine_reported = true;
                    let generation = p.generation;
                    drop(p);
                    report(&format!(
                        "undo: quarantine ARMED at generation={generation} - Ctrl+Z, Ctrl+Shift+Z and Ctrl+Y are swallowed for the rest of the session (stale stack, cross-file replay is the hazard); the per-keystroke swallow needle needs a hand on the keyboard"
                    ));
                } else {
                    drop(p);
                }
            }
            // RISK 3, second half: DOES the handle appear once the loop has actually
            // spun? If it does, this is the registration point and the delay is the
            // finding - a bridge cannot place what it cannot name.
            if !state.borrow().handed {
                if let Some(hwnd) = hwnd_of(ui.window()) {
                    state.borrow_mut().handed = true;
                    report(&format!(
                        "hwnd = {hwnd:#x} APPEARED at t+{now:?}, after the loop spun {}",
                        measured(&third_dir, now)
                    ));
                    send(
                        &third_gw,
                        Command::RegisterWindow {
                            handle: WindowHandle(hwnd),
                        },
                    );
                    // S6b: THE site that fires in practice - the handle exists only once the loop
                    // has spun, so this tick callback is the thread that pumps the window, which
                    // is the whole of the arm_file_drop contract. Same fn, same thread, same run.
                    arm_drop_target(hwnd, &third_drops);
                    drain(&third_events, &third_pump, &ui.as_weak());
                }
            }
            {
                let mut poll = state.borrow_mut();
                if seen != poll.last {
                    report(&format!("poll: fingerprint {seen:?} (was {:?})", poll.last));
                    poll.last = seen;
                    poll.changed = Some(Instant::now());
                }
                let quiet = poll.changed.is_some_and(|at| at.elapsed() >= QUIET);
                if quiet && poll.sends < 2 {
                    poll.sends += 1;
                    poll.changed = None;
                    drop(poll);
                    report("poll: settled, telling the port");
                    send(&third_gw, Command::GeometryChanged);
                    let mut poll = state.borrow_mut();
                    poll.sampled = true;
                }
            }
            if now >= SAMPLE && !state.borrow().sampled {
                state.borrow_mut().sampled = true;
                report(&format!(
                    "t+{SAMPLE:?}: toolkit reads {seen:?} {}",
                    measured(&third_dir, now)
                ));
            }
            // ---- S3, the two-writer experiment (C1) ----
            // Step 1: the strip asks, once, without being touched by hand.
            // Step 2: Event::Pinned(true) is the platform's OWN read of
            //         GWL_EXSTYLE, so it is the t0 of the hold.
            // Step 3: HOLD later, ask the SAME bit again. The port documents a
            //         repeat of an already-confirmed state as SILENT, so no answer
            //         means the style is still where the platform left it - and an
            //         answer means something cleared it and it had to be re-applied.
            //         That asymmetry is the only drift read a bridge is allowed to
            //         make: it may not import `windows`, so GetWindowLongPtrW here
            //         would be both a layering violation and the second writer.
            {
                let mut pin = third_pump.borrow_mut();
                let handed = state.borrow().handed;
                // (2): the ask waits for the port's answer to `RegisterWindow`, because
                // SetPinned with nothing registered only stores a bit and stays silent -
                // asking into that would prove nothing about topmost. LATE_ASK keeps the
                // probe from hanging on an answer that may never arrive in-run.
                if !pin.asked
                    && now >= PIN_AT
                    && handed
                    && (pin.answered || now >= LATE_ASK)
                {
                    pin.asked = true;
                    let pin_answered = pin.answered;
                    drop(pin);
                    report(&format!(
                        "pinned: auto-ask SetPinned(true) answered={} {}",
                        if pin_answered { "yes" } else { "NO - LATE_ASK forced" },
                        measured(&third_dir, now)
                    ));
                    send(&third_gw, Command::SetPinned(true));
                    drain(&third_events, &third_pump, &ui.as_weak());
                } else if let Some(at) = pin.applied_at {
                    if pin.reask_at.is_none() && at.elapsed() >= HOLD {
                        pin.reask_at = Some(Instant::now());
                        drop(pin);
                        report("pinned: re-ask SetPinned(true) after 1s hold (an answer = drift)");
                        send(&third_gw, Command::SetPinned(true));
                        drain(&third_events, &third_pump, &ui.as_weak());
                    } else if let Some(reask) = pin.reask_at {
                        if !pin.hold_reported && reask.elapsed() >= SETTLE {
                            pin.hold_reported = true;
                            let drift = pin.answered_after_reask;
                            drop(pin);
                            if drift {
                                report("pinned: DRIFT - the port RE-ANSWERED, so WS_EX_TOPMOST had been lost and had to be re-applied");
                            } else {
                                report(&format!("pinned: true WS_EX_TOPMOST=1 readback HELD across 1s (port silent on a repeat ask) {}", measured(&third_dir, now)));
                            }
                        }
                    }
                }
            }
            // ---- S4: the text loop, on the bridge's own cadence ----
            text_pump(&ui, &third_gw, &third_pump);
            // S4b: ONE keystroke from inside the loop, so 'buffer changed -> dot reddens
            // -> debounce -> flush' is proven on the tick rather than inferred from the
            // exit path. Deliberately after text_pump: the NEXT tick is the one that must
            // see it dirty, which is the ordering the user actually experiences.
            {
                let mut p = third_pump.borrow_mut();
                if !p.late_key && now >= LATE_KEY {
                    p.late_key = true;
                    let base = p.last_sent.clone();
                    drop(p);
                    report(&format!(
                        "keystroke-in-run: t+{now:?} appending 6 bytes, expecting dirty then flush"
                    ));
                    ui.set_buffer(format!("{base}+typed").into());
                }
            }
            // ---- S5: the drag act, through the one function a real drag would use ----
            {
                let mut p = third_pump.borrow_mut();
                if p.drag_step == 0 && now >= DRAG_AT {
                    p.drag_step = 1;
                    drop(p);
                    drag_by(&ui.as_weak(), &third_pump, 60, 40, "synthetic");
                    drag_release(&third_gw, "synthetic");
                    drain(&third_events, &third_pump, &ui.as_weak());
                } else if p.drag_step == 1 && now >= DRAG2_AT {
                    // THE REGRESSION PROBE. With the frame act ON this fires on a maximised
                    // window and must refuse: no move, no store ask that could carry a
                    // meaningless target, and the exit check still reads the rect the user
                    // actually placed. With the frame act off it is simply a second move, so
                    // the chain keeps working either way.
                    p.drag_step = 2;
                    drop(p);
                    drag_by(&ui.as_weak(), &third_pump, 60, 40, "synthetic-2");
                    drag_release(&third_gw, "synthetic-2");
                    drain(&third_events, &third_pump, &ui.as_weak());
                }
            }
            // ---- S4c: stage the failure, type, then clear the way ----
            {
                let mut p = third_pump.borrow_mut();
                if !p.ro_done && now >= READ_ONLY_AT {
                    p.ro_done = true;
                    let found = p.loop_path.clone();
                    drop(p);
                    match found {
                        Some(file) => {
                            let away = file.with_extension("notes.bak");
                            // Rename the real file aside, put a DIRECTORY on its path,
                            // and roll back if the second step fails - staged, checked,
                            // and reversible in one straight line each.
                            // Rename the real file aside, put a DIRECTORY on its path,
                            // and roll both back if either step fails: staged, checked,
                            // reversible - and no map_err/inspect_err to fight the lint with.
                            let attempt = std::fs::rename(&file, &away)
                                .and_then(|_| std::fs::create_dir(&file));
                            let staged = match &attempt {
                                Ok(()) => format!(
                                    "path is now a DIRECTORY, so the rename-over-target must fail: {}",
                                    file.display()
                                ),
                                Err(err) => {
                                    let _ = std::fs::remove_dir(&file);
                                    let _ = std::fs::rename(&away, &file);
                                    format!("STAGING FAILED: {err}")
                                }
                            };
                            report(&format!("save-fail: staged at t+{now:?} - {staged}"));
                        }
                        None => report("save-fail: no loop path was recorded, nothing to stage"),
                    }
                } else if !p.seed_armed && now >= SEED_ARM_AT {
                    p.seed_armed = true;
                    let found = p.seed.clone();
                    let text = lf(&ui.get_buffer());
                    let revision = {
                        p.edits += 1;
                        p.last_sent = text.clone();
                        p.edits
                    };
                    drop(p);
                    match found {
                        Some(path) => {
                            report(&format!(
                                "do-no-harm: arming the seed with an explicit SaveAs of itself (ADR-0001) -> {}",
                                path.display()
                            ));
                            send(
                                &third_gw,
                                Command::SaveAs {
                                    path,
                                    text,
                                    revision,
                                },
                            );
                            drain(&third_events, &third_pump, &ui.as_weak());
                        }
                        None => report("do-no-harm: no seed file was written, nothing to arm"),
                    }
                } else if !p.key2_done && now >= KEY2_AT {
                    p.key2_done = true;
                    let base = p.last_sent.clone();
                    drop(p);
                    report("save-fail: typed 6 bytes, expecting SaveFailed then dot=amber");
                    ui.set_buffer(format!("{base}+fail1").into());
                } else if !p.seed_key_done && now >= SEED_KEY_AT {
                    p.seed_key_done = true;
                    let base = p.last_sent.clone();
                    drop(p);
                    report("do-no-harm: typed 6 bytes INTO THE SEED document, expecting Saved then a hash print");
                    ui.set_buffer(format!("{base}+seed1").into());
                } else if !p.wb_done && now >= WRITE_BACK_AT {
                    p.wb_done = true;
                    let found = p.loop_path.clone();
                    drop(p);
                    if let Some(file) = found {
                        let away = file.with_extension("notes.bak");
                        let _ = std::fs::remove_dir(&file);
                        let _ = std::fs::rename(&away, &file);
                        let base = lf(&ui.get_buffer());
                        report("save-fail: path restored, typed 6 bytes, expecting Saved then dot clears");
                        ui.set_buffer(format!("{base}+back1").into());
                    }
                }
            }
            // ---- S7: the popup's backdrop, its clamp, and Escape ----
            // OBSERVE-THEN-ACT, one pair per step, because both halves of this lag and the
            // first draft of the act proved it by printing menu-shown=false immediately after
            // opening the popup: a resize is not applied until the window comes back from the
            // OS, and a mirrored property is not recomputed until the next traversal. Reading
            // in the same step as writing measures the world before the act, so every needle
            // here is labelled with what it is looking at: the state AFTER the previous act.
            let ostep = third_pump.borrow().overlay_step;
            if ostep < 11 {
                let at = OVERLAY_AT + OVERLAY_EVERY * (ostep as u32);
                if now >= at {
                    third_pump.borrow_mut().overlay_step = ostep + 1;
                    let w = ui.window();
                    // ABOUTSLINT: the licence screen's two needles, and they keep the rule the
                    // seven above were built on - each prints what the PREVIOUS act left, never
                    // what this one is about to do. Separate format string, so no old needle
                    // changes wording or count: "overlay[about]: shown=true" is printed once,
                    // after the row's door was bumped, and "overlay[about]: dismissed" once,
                    // after the dismissal door was.
                    if ostep == 9 || ostep == 10 {
                        report(&format!(
                            "overlay[about]: {} about-shown={} menu-shown={}",
                            if ostep == 9 {
                                "shown=true"
                            } else {
                                "dismissed"
                            },
                            ui.get_about_shown(),
                            ui.get_menu_shown()
                        ));
                    }
                    if ostep > 0 && ostep < 8 {
                        let seen = match ostep - 1 {
                            0 => "after open",
                            1 => "after 400px",
                            2 => "after 180px",
                            3 => "after restore",
                            4 => "after backdrop",
                            5 => "after reopen",
                            _ => "after escape",
                        };
                        report(&format!(
                            "overlay[{seen}]: menu-shown={} {}",
                            ui.get_menu_shown(),
                            popup_words(&ui)
                        ));
                    }
                    match ostep {
                        0 => {
                            let size = w.size();
                            third_pump.borrow_mut().start_size = Some((size.width, size.height));
                            report(&format!("overlay: host is {}x{}", size.width, size.height));
                            ui.set_toggle_asks(ui.get_toggle_asks() + 1);
                        }
                        1 => w.set_size(LogicalSize::new(400.0, 600.0)),
                        2 => w.set_size(LogicalSize::new(180.0, 600.0)),
                        3 => {
                            let (sw, sh) = third_pump
                                .borrow()
                                .start_size
                                .unwrap_or((800, 600));
                            w.set_size(LogicalSize::new(sw as f32, sh as f32));
                        }
                        4 => ui.set_close_asks(ui.get_close_asks() + 1),
                        5 => ui.set_toggle_asks(ui.get_toggle_asks() + 1),
                        6 | 7 => ui.set_close_asks(ui.get_close_asks() + 1),
                        // ABOUTSLINT: the 6th row, driven the way a pointer drives it - through
                        // the ONE door the mount has (about-asks), which is the same handler the
                        // TouchArea's clicked reaches, so the needle measures the row and not a
                        // Rust-side shortcut. Then the dismissal, through the door the backdrop
                        // and Escape reach. Step 10 observes and acts on nothing.
                        8 => ui.set_about_asks(ui.get_about_asks() + 1),
                        9 => ui.set_close_asks(ui.get_close_asks() + 1),
                        _ => {}
                    }
                }
            }
            // ---- S6: drive the chords THROUGH THE TABLE, into the row callbacks ----
            // One walk of SHORTCUTS: the display string in the needle is the table's, so a
            // row that was edited or deleted shows up in the log rather than hiding.
            let driven = third_pump.borrow().menu_act;
            let total = CHORD_DRIVE.len() as u64;
            if driven < total {
                let at = chord_at(driven);
                if now >= at {
                    let row = SHORTCUTS[CHORD_DRIVE[driven as usize]];
                    third_pump.borrow_mut().menu_act = driven + 1;
                    match route_of(row.3) {
                        Some(route) => fire(&ui, route, row.1, row.2),
                        None => report(&format!(
                            "chord: {} -> UNROUTED TABLE ROW ({}): nothing fired",
                            row.1, row.3
                        )),
                    }
                    // The same instant a real key would be followed by a redraw: the events
                    // the command answers with are drained on the next tick anyway, but this
                    // keeps the needle and its consequence adjacent in the log.
                    drain(&third_events, &third_pump, &ui.as_weak());
                }
            }
            // ---- S8: the caption buttons, driven through the doors the markup uses ----
            // S4e: THE LOCK, MET - relocated here from startup; see LOCK_AT for the schedule
            // reason. One tick, whole: prepare the fixture, ask for it, wait BOUNDED for the answer,
            // let the pump meet the lock, then put the seed back so nothing after this point runs on
            // a document the port will not save.
            {
                let due = {
                    let mut p = third_pump.borrow_mut();
                    if p.lock_acted || now < LOCK_AT {
                        false
                    } else {
                        p.lock_acted = true;
                        true
                    }
                };
                if due {
                    let fixture = third_dir.0.join("s8-locked.notes");
                    match write_lock_fixture(&fixture)
                        .and_then(|bytes| run_attrib(&fixture, "+R").map(|()| bytes))
                    {
                        Ok(bytes) => {
                            third_pump.borrow_mut().lock_fixture = Some(fixture.clone());
                            report(&format!(
                                "lock-act: wrote {bytes} bytes to {}, set +R through attrib, asking Command::Open",
                                fixture.display()
                            ));
                            // The answer comes from the engine thread, so poll `drain` - it takes
                            // what is already queued and never blocks on a channel - until EITHER
                            // kind of answer lands or the bound expires, and report which. The first
                            // version drained once, microseconds after sending, and its silence read
                            // like a dead lever.
                            let before = third_pump.borrow().load_answers;
                            let sent = Instant::now();
                            send(&third_gw, Command::Open { path: fixture });
                            while third_pump.borrow().load_answers == before
                                && sent.elapsed() < LOCK_WAIT
                            {
                                std::thread::sleep(Duration::from_millis(2));
                                drain(&third_events, &third_pump, &ui.as_weak());
                            }
                            let answers = third_pump.borrow().load_answers;
                            report(&format!(
                                "lock-act: the port {} after {:?} (load_answers {} -> {})",
                                if answers == before {
                                    "STAYED SILENT past the bound - the lever did not reach it"
                                } else {
                                    "answered"
                                },
                                sent.elapsed(),
                                before,
                                answers
                            ));
                            // The refusal, then proof it is once-per-lock: two calls to the same fn
                            // the tick calls, and only one line may appear.
                            text_pump(&ui, &third_gw, &third_pump);
                            text_pump(&ui, &third_gw, &third_pump);
                            // S4f: THE REFUSAL, WITNESSED. Same act, same door, one file that D9
                            // answers from the stat alone: this open must come back as
                            // Event::LoadFailed { reason: TooLarge }, because the read-only fixture
                            // above is READABLE and so can never exercise that arm. What makes it
                            // free is the semantics: a refusal adopts nothing, so the document never
                            // changes hands - no buffer write, no lock, no generation step - and the
                            // ladder below proves it by stepping only for the seed's own path change.
                            let big = third_dir.0.join("s8-big.notes");
                            match write_big_fixture(&big) {
                                Ok(bytes) => {
                                    report(&format!(
                                        "big-act: wrote {bytes} bytes to {}, asking Command::Open - expecting a refusal, not a document",
                                        big.display()
                                    ));
                                    let before = third_pump.borrow().load_answers;
                                    let sent = Instant::now();
                                    send(&third_gw, Command::Open { path: big });
                                    while third_pump.borrow().load_answers == before
                                        && sent.elapsed() < LOCK_WAIT
                                    {
                                        std::thread::sleep(Duration::from_millis(2));
                                        drain(&third_events, &third_pump, &ui.as_weak());
                                    }
                                    let answers = third_pump.borrow().load_answers;
                                    report(&format!(
                                        "big-act: the port {} after {:?} (load_answers {} -> {})",
                                        if answers == before {
                                            "STAYED SILENT past the bound - the refusal was never heard"
                                        } else {
                                            "answered, and the line above it is the witness"
                                        },
                                        sent.elapsed(),
                                        before,
                                        answers
                                    ));
                                }
                                Err(e) => report(&format!("big-act: could not write the fixture: {e}")),
                            }
                            // RESTORE, so the caption tail and the honest shutdown both run on the
                            // document they were written for.
                            let seed = third_pump.borrow().seed.clone();
                            if let Some(seed) = seed {
                                report(&format!(
                                    "lock-act: restoring the seed {} so the run closes unlocked",
                                    seed.display()
                                ));
                                send(&third_gw, Command::Open { path: seed });
                                drain(&third_events, &third_pump, &ui.as_weak());
                            } else {
                                report("lock-act: no seed path stored - the run stays on the fixture");
                            }
                        }
                        Err(e) => report(&format!("lock-act: fixture refused to be prepared: {e}")),
                    }
                }
            }
            let cstep = third_pump.borrow().caption_step;
            if driven == total && cstep < 9 && now >= CAPTION_AT + CAPTION_EVERY * (cstep as u32) {
                third_pump.borrow_mut().caption_step = cstep + 1;
                let w = ui.window();
                match cstep {
                    0 => {
                        report("caption[minimize]: invoking the BUTTON's door (minimize-requested)");
                        ui.invoke_minimize_requested();
                        let _ = w;
                    }
                    1 => report(&format!(
                        "caption[minimized]: fingerprint {found:?}",
                        found = fingerprint_of(w)
                    )),
                    2 => {
                        w.set_minimized(false);
                        report(&format!(
                            "caption[un-minimized]: is_minimized()={}",
                            w.is_minimized()
                        ));
                    }
                    3 => {
                        report(&format!(
                            "caption[maximize]: the SAME door as the double-click (toggle-max); glyph was {}",
                            caption_glyph(w.is_maximized())
                        ));
                        ui.invoke_toggle_max();
                    }
                    4 => {
                        report("caption[drag on a maximised window]: expect the refusal next");
                        ui.invoke_drag_delta(40.0, 30.0);
                    }
                    5 => report(&format!(
                        "caption[maximised]: glyph={} window-bit={} fingerprint {:?}",
                        caption_glyph(w.is_maximized()),
                        w.is_maximized(),
                        fingerprint_of(w)
                    )),
                    6 => {
                        report("caption[restore]: the same door again, state-aware on the Rust side");
                        ui.invoke_toggle_max();
                    }
                    7 => report(&format!(
                        "caption[restored]: glyph={} window-bit={} fingerprint {:?}",
                        caption_glyph(w.is_maximized()),
                        w.is_maximized(),
                        fingerprint_of(w)
                    )),
                    _ => {
                        report("caption[close]: the X door, quit-asked - policy note at the handler");
                        ui.invoke_quit_asked();
                    }
                }
            }
            // (2) THE SYNTHETIC CLICK. Aim at the first row that is NOT the file this
            // probe seeded, so what lands is a RECENT file the port listed.
            {
                let mut p = third_pump.borrow_mut();
                if !p.click_done && now >= CLICK_AT {
                    p.click_done = true;
                    let seed = p.seed.clone();
                    let target = p
                        .recent_paths
                        .iter()
                        .position(|path| Some(path.as_path()) != seed.as_deref());
                    drop(p);
                    match target {
                        Some(index) => {
                            open_recent(&third_gw, &third_pump, index, "synthetic");
                            drain(&third_events, &third_pump, &ui.as_weak());
                        }
                        None => report("recents: synthetic click skipped - only the seeded file is in the list"),
                    }
                }
            }
            // R1 then R2, driven from the loop so the run needs no hands.
            {
                let mut p = third_pump.borrow_mut();
                if SYNTHETIC_FRAME_ACTS && !p.max_toggled && now >= MAX_AT {
                    p.max_toggled = true;
                    drop(p);
                    toggle_max(&ui.as_weak(), &third_gw, "synthetic");
                } else if SYNTHETIC_CLOSE_ACTS && now >= CLOSE_AT && p.closes == 0 {
                    drop(p);
                    ui.set_close_arm(1);
                    let allowed = ui.get_close_allowed();
                    report(&format!(
                        "close: request_close #1 returned {allowed} (false = the decline held), visible={}",
                        ui.window().is_visible()
                    ));
                } else if SYNTHETIC_CLOSE_ACTS && now >= CLOSE2_AT && p.closes == 1 {
                    drop(p);
                    // THE QUIT RACE, made real: type, then close on the same tick. The
                    // 750 ms debounce can never fire between these two, so the only
                    // thing that can save these bytes is the shutdown path below.
                    let dirty = format!("{}Z", lf(&ui.get_buffer()));
                    ui.set_buffer(dirty.into());
                    report("exit: buffer dirtied immediately before the granted close");
                    ui.set_close_arm(2);
                    let allowed = ui.get_close_allowed();
                    report(&format!(
                        "close: request_close #2 returned {allowed}, visible-before-hide={}",
                        ui.window().is_visible()
                    ));
                    if allowed {
                        ui.window().hide().ok();
                    }
                }
            }
            if now >= END {
                do_no_harm(&third_pump, "exit");
                report(&format!("probe over {}", measured(&third_dir, now)));
                ui.window().hide().ok();
            }
        }
    });
    // The strip's click: ONE command out, nothing rendered locally. `confirmed`
    // gates the next ask, so the click follows the port's last fact rather than a
    // counter of its own.
    {
        let gw = Rc::clone(&gateway);
        let pump = Rc::clone(&pump);
        ui.on_toggled_pin(move || {
            let next = !pump.borrow().confirmed.unwrap_or(false);
            report(&format!("pinned: title-strip click -> SetPinned({next})"));
            send(&gw, Command::SetPinned(next));
        });
    }
    // A recent row is an ask, the same shape as the pin strip: index in, Command out,
    // and the text comes back through Loaded - never from the click itself.
    {
        let gw = Rc::clone(&gateway);
        let pump = Rc::clone(&pump);
        ui.on_open_at_index(move |index| {
            open_recent(&gw, &pump, index as usize, "touch");
        });
    }
    // R2: THE CLOSE CONTRACT. This is the handler an OS close would land on. It declines
    // the first request that arrives on its own - the rehearsal case that proves a decline
    // can hold - and grants any later one, UNLESS the user asked to leave: a Quit from the
    // menu is never the rehearsal case. With SYNTHETIC_CLOSE_ACTS off, this run exercises
    // exactly that clause on request #1.
    {
        let pump = Rc::clone(&pump);
        ui.window().on_close_requested(move || {
            let (n, asked_to_quit) = {
                let mut p = pump.borrow_mut();
                p.closes += 1;
                (p.closes, p.quit_requested)
            };
            report(&format!(
                "close: requested (#{n}) quit-asked={asked_to_quit}"
            ));
            if n == 1 && !asked_to_quit {
                report("close: declined, held (KeepWindowShown)");
                slint::CloseRequestResponse::KeepWindowShown
            } else {
                // NOT "second, exiting": with the pair standing down this IS request #1,
                // and the only thing that granted it is quit_requested. Print the count and
                // the reason so neither run has to be interpreted after the fact.
                report(&format!(
                    "close: GRANTED, exiting (HideWindow) on request #{n} quit-asked={asked_to_quit}"
                ));
                slint::CloseRequestResponse::HideWindow
            }
        });
    }
    // ---- S4c: THE MENU ROWS. Every handler reuses an act that already exists and
    // implements none of it again - that is the whole reason the rows forward instead of
    // answering in markup. Chrome dismisses its own popup; Rust runs the command.
    {
        let dialog_tx = Rc::clone(&dialog_tx);
        let dialog_pump = Rc::clone(&pump);
        let dialog_weak = ui.as_weak();
        ui.on_open_asked(move || {
            // S9: the row, the Ctrl+O chord and any future native menu land on this one
            // handler, and the handler now means ask a person. What comes back goes out
            // through Command::Open - the SAME door the recents rows use, which is the only
            // reason a picked file keeps the epoch, the buffer adoption and the title working.
            ask_dialog(DialogKind::Open, &dialog_weak, &dialog_pump, &dialog_tx);
        });
    }
    {
        let dialog_tx = Rc::clone(&dialog_tx);
        let dialog_pump = Rc::clone(&pump);
        let dialog_weak = ui.as_weak();
        ui.on_save_as_asked(move || {
            // The same ask with a different verb, answering through the door that already
            // exists: Command::SaveAs, with the buffer read when the ANSWER arrives rather
            // than when the user was asked - see answer_dialog.
            ask_dialog(DialogKind::SaveAs, &dialog_weak, &dialog_pump, &dialog_tx);
        });
    }
    {
        let gw = Rc::clone(&gateway);
        let pump = Rc::clone(&pump);
        let weak = ui.as_weak();
        ui.on_autosave_asked(move || {
            let (was, asks) = {
                let mut p = pump.borrow_mut();
                let was = p.autosave;
                p.autosave = !was;
                p.autosave_asks += 1;
                (was, p.autosave_asks)
            };
            report(&format!(
                "menu: autosave-row toggled {was}->{} (ask #{asks}) - MIRROR, not a report: the port echoes no autosave event",
                !was
            ));
            send(&gw, Command::SetAutosave(!was));
            let dirty = pump.borrow().dirty;
            note_dot(&pump, &weak, dirty, "autosave-row");
        });
    }
    {
        let gw = Rc::clone(&gateway);
        ui.on_clear_recents_asked(move || clear_recents(&gw, "asked"));
    }
    {
        let pump = Rc::clone(&pump);
        let weak = ui.as_weak();
        // THE POLICY, at the door: this handler is now reached by the menu's Quit row AND by
        // the S8 caption X, and both set the granted bit, which is what lets the FIRST close
        // request through instead of declining it. Decline-then-confirm was the alternative and
        // it is worse here twice over: a silent first refusal on a button is indistinguishable
        // from a broken button, and a "save changes?" dialog is the one prompt the README
        // promises this app never shows. So the spike is confirm-free on purpose, exactly like
        // Alt+F4, and the flush-before-close path is what makes that safe rather than careless.
        ui.on_quit_asked(move || {
            // THE SINGLE DOOR. Quit hides nothing and calls no gateway: it bumps
            // close-arm, which runs Window::close() in markup, which lands on the same
            // on_close_requested an OS close reaches - where the final flush and the
            // joined shutdown already live. quit_requested is the one extra fact: a user
            // asking to leave is never the rehearsal case the first request declines.
            let Some(ui) = weak.upgrade() else { return };
            let arm = {
                let mut p = pump.borrow_mut();
                p.quit_requested = true;
                p.closes + 1
            };
            report(&format!("menu: Quit row -> close-arm {arm} (single door)"));
            ui.set_close_arm(arm as i32);
            report(&format!(
                "menu: after the door: close-allowed={} visible={}",
                ui.get_close_allowed(),
                ui.window().is_visible()
            ));
        });
    }
    // R1: the strip's double-click, the promise the README makes about a frameless
    // title bar. Bound to the same fn the synthetic driver calls.
    {
        let gw = Rc::clone(&gateway);
        let weak = ui.as_weak();
        ui.on_toggle_max(move || toggle_max(&weak, &gw, "double-click"));
    }
    // S8: the caption minimize button's handler - the far end of the ask the bar makes.
    {
        let weak = ui.as_weak();
        ui.on_minimize_requested(move || minimize(&weak, "minimize button"));
    }
    // S10: the keystroke witness. One bool, one counter, no string touched - the entire point of
    // item 3. Attached in Rust because the markup forwards every editor branch to the SAME Spike
    // callback: one door, however many widgets stand behind it.
    {
        let pump = Rc::clone(&pump);
        ui.on_text_edited(move || {
            let mut p = pump.borrow_mut();
            p.strokes += 1;
            p.edited_flag = true;
            p.pending_at = Some(Instant::now());
            if p.strokes == 1 || p.strokes % 8 == 0 {
                report(&format!(
                    "edited: keystroke #{} witnessed (no buffer read happens at this rate since S10)",
                    p.strokes
                ));
            }
        });
    }
    // S10b: the swallow's voice. Raised from the capture handler when a replay key is eaten; it
    // asks nothing of the port and changes nothing here - the decision already happened in
    // markup. Its only job is that a run can show the quarantine biting rather than proving it
    // by an absence, which is the weakest kind of evidence this spike has ever leaned on.
    ui.on_undo_swallowed(move || {
        report("undo: quarantined (stale stack, cross-file replay is the hazard)");
    });
    // S5: THE DRAG WIRES, closed by the four approved markup lines. Chrome's band emits a
    // delta per frame and one release, and both land on the SAME two functions the synthetic
    // acts call - so the arithmetic, the maximised guard and the once-per-drag store ask
    // exist exactly once, whether the pointer came from a hand or from the timer.
    {
        let weak = ui.as_weak();
        let pump = Rc::clone(&pump);
        ui.on_drag_delta(move |dx, dy| {
            // Lengths in, integer physical px out. Scale 1.0 on this probe, and any
            // conversion belongs in drag_by with the rest of the geometry arithmetic - not
            // duplicated in a handler whose only job is to forward. Note what this handler
            // does NOT hold: no gateway, because moving a window asks the port for nothing.
            drag_by(&weak, &pump, dx as i32, dy as i32, "title-band");
        });
    }
    {
        let gw = Rc::clone(&gateway);
        ui.on_drag_ended(move || drag_release(&gw, "title-band"));
    }
    ui.run().ok();
    // S6b: DISARM FIRST, on the thread that armed - this one, the main thread, the only thread
    // that ever pumped this window. Dropping the guard calls the platform's `disarm`, which
    // revokes the IDropTarget; taking it here instead of letting the destructor run at process
    // exit means the revocation happens while the apartment that registered it is still alive.
    // The save worker thread never sees this Rc, so there is no other thread it could be.
    //
    // NOTED, because the brief expected a Command::UnregisterWindow here: this bridge never sends
    // it. The port has the command and the engine clears the stored handle on it, but the spike's
    // exit is a granted close (HideWindow) and a return from run(), so the port's window identity
    // dies with the process rather than with a message. This drop is therefore the FIRST teardown
    // that names the window going away - and if a later slice does add the UnregisterWindow post,
    // the drop belongs immediately before it, which is stated so nobody has to guess which side
    // of the post is safe.
    {
        let disarmed = drop_guard.borrow_mut().take();
        report(&format!(
            "drop: disarmed on exit (a guard was held: {})",
            disarmed.is_some()
        ));
        drop(disarmed);
    }
    // S4d: the run's own mess, cleared here rather than by the next run's surprise. The
    // attribute goes FIRST and that is not ceremony: a read-only file left in the probe directory
    // makes the NEXT startup write of that path fail, and a probe that makes the next probe lie is
    // worse than no probe. Both steps report, and the failure paths print the reason, because
    // "cleanup failed" without a cause is not evidence.
    {
        let fixture = pump.borrow().lock_fixture.clone();
        match fixture {
            Some(path) => {
                let cleared = run_attrib(&path, "-R");
                let removed = std::fs::remove_file(&path);
                report(&format!(
                    "lock-act: cleanup {} (attrib -R ok: {}, removed: {})",
                    path.display(),
                    cleared.is_ok(),
                    removed.is_ok()
                ));
                if let Err(e) = &cleared {
                    report(&format!("lock-act: the attribute did not clear: {e}"));
                }
                if let Err(e) = &removed {
                    report(&format!(
                        "lock-act: CLEANUP FAILED for {}: {e} - delete it by hand before trusting the next run's recents",
                        path.display()
                    ));
                }
            }
            None => report("lock-act: no fixture to clean (the act did not create one)"),
        }
        // S4f: the 9 MiB refusal fixture, named rather than remembered - it was never adopted, so
        // there is no attribute to clear and no state to consult. Leaving it behind would give the
        // next run a recent file that refuses to open, which is a lie about the disk.
        let big = dir.0.join("s8-big.notes");
        match std::fs::remove_file(&big) {
            Ok(()) => report(&format!("big-act: cleaned up {}", big.display())),
            Err(e) => report(&format!(
                "big-act: cleanup {} (nothing to clean if it says not found): {e}",
                big.display()
            )),
        }
    }
    // R2b: THE HONEST SHUTDOWN, in the order that cannot lose text. The close that
    // granted is what lands here - and by then the debounce has NOT run, so any byte
    // typed after the last tick is still only in this bridge's buffer.
    let seen_before = pump.borrow().seen;
    let text = lf(&ui.get_buffer());
    let dirty = text != pump.borrow().last_sent;
    if dirty {
        let (revision, epoch) = {
            let mut p = pump.borrow_mut();
            p.edits += 1;
            p.last_sent = text.clone();
            (p.edits, p.epoch)
        };
        report(&format!(
            "exit: final flush sent ({} bytes rev={revision} epoch={epoch})",
            text.len()
        ));
        send(
            &gateway,
            Command::Flush {
                text,
                revision,
                epoch,
            },
        );
    } else {
        report("exit: final flush skipped (buffer matches what was last sent)");
    }
    // THE SHUTDOWN, and which arm of `Exit` came back is the whole story: Ok is the
    // only one that promises the engine finished its own drain, and the api's doc is
    // blunt that QueueClosed means no such promise and Panicked means characters lost.
    // First bridge in this repo to match all three arms rather than the happy path.
    let owned = gateway.borrow_mut().take();
    match owned {
        Some(gateway) => match gateway.close() {
            Ok(()) => report("exit: Shutdown accepted, engine joined"),
            Err(Exit::QueueClosed) => report(
                "exit: ENGINE ALREADY GONE (QueueClosed) - shutdown never accepted, no promise of a final save",
            ),
            Err(Exit::Abandoned(waited)) => report(&format!(
                "exit: ABANDONED after {waited:?} - engine healthy but still running; its own exit drain owes the flush"
            )),
            Err(Exit::Panicked) => report(
                "exit: PANICKED - nothing after it ran: no drain, no final save, no session write",
            ),
        },
        None => report("exit: no gateway to shut down"),
    }
    // The answers the engine may still be sending while it unwinds its own exit.
    for _ in 0..20 {
        drain(&events, &pump, &ui.as_weak());
        std::thread::sleep(Duration::from_millis(25));
    }
    let accounted = pump.borrow().seen - seen_before;
    report(&format!("exit drain: {accounted} event(s) accounted for"));
    // S4b: the counter that settles the block-scoping question with a number instead of
    // a reading of braces. ~2500 over a 21s run at 8ms is "every tick"; 1 is "one-shot".
    report(&format!(
        "text_pump: invocations={} dirty-at-exit={} last-needle=[{}]",
        pump.borrow().invocations,
        pump.borrow().dirty,
        pump.borrow().dot_words
    ));
    report(&format!("exit {}", measured(&dir, started.elapsed())));
    // S5, THE PERSISTENCE HALF: not what the toolkit says it is right now, but what the PORT
    // WROTE - the coordinates the next launch restores from. Delta <0,0> means a dragged
    // rect survives the round trip without walking; anything else IS the bug class, in the
    // one number M3 consumes.
    match (pump.borrow().drag_target, stored_xy(&dir)) {
        (Some((tx, ty)), Some((sx, sy))) => report(&format!(
            "drag-check[exit]: intended <{tx},{ty}> actual <{sx},{sy}> delta <{},{}>",
            sx - tx as i64,
            sy - ty as i64
        )),
        (Some(_), None) => report("drag-check[exit]: the port wrote no rect to compare"),
        (None, _) => report("drag-check[exit]: no drag ran this lifecycle"),
    }
}

/// The poll's memory, as data.
struct Poll {
    /// Has the HWND been handed to the port yet? False until the registration goes
    /// out, wherever in the timeline it becomes possible - that is risk 3.
    handed: bool,
    last: Fingerprint,
    changed: Option<Instant>,
    sends: usize,
    sampled: bool,
}

/// C4/C5 measurements, taken before the loop so the numbers describe the toolkit and
/// not this probe's own scheduling. Textures of the buffer only: no file, no api call.
fn text_probe(ui: &Spike) {
    let cr_input = "alpha\r\nbeta\rgamma";
    ui.set_buffer(cr_input.to_owned().into());
    let back = ui.get_buffer().to_string();
    report(&format!(
        "cr-probe: wrote {} bytes (one CRLF + one lone CR) -> readback {} bytes CRs={} LFs={} text={:?}",
        cr_input.len(),
        back.len(),
        back.matches('\r').count(),
        back.matches('\n').count(),
        back
    ));
    let big = "x".repeat(1024 * 1024);
    let bytes = big.len();
    let at = Instant::now();
    ui.set_buffer(big.into());
    let set = at.elapsed();
    let read_at = Instant::now();
    let read_back = ui.get_buffer().len();
    let read = read_at.elapsed();
    report(&format!(
        "echo-1MiB: set_buffer({bytes} bytes) = {set:?}; get_buffer() = {read_back} bytes in {read:?}"
    ));
    // The typed character, simulated so the run needs no hands: the pump owns the rest.
    ui.set_buffer(
        "S4: one simulated keystroke, then the 750ms tick flushes\n"
            .to_owned()
            .into(),
    );
    report("keystroke: buffer written by the probe at t=0, waiting on the debounce");
}

/// R1: the double-click act, shared by the markup's `double-clicked` handler and by
/// the synthetic driver, so an unattended run exercises the same code a strip
/// double-click would. The pair this prints is the one the README promise rests on:
/// Slint's OWN `is_maximized()` bool, and - marked, not guessed - `os-showCmd`, which
/// a bridge CANNOT read: `GetWindowPlacement` lives in the `windows` family and a
/// bridge may not import it (check-arch's rule), so the field is printed as
/// UNREACHABLE unless the port itself states it. See the slice report.
fn toggle_max(weak: &slint::Weak<Spike>, gw: &Rc<RefCell<Option<Gateway>>>, via: &str) {
    let Some(ui) = weak.upgrade() else { return };
    let window = ui.window();
    let want = !window.is_maximized();
    window.set_maximized(want);
    report(&format!(
        "frame: {via} toggled to {want} -> slint-max={} os-showCmd=UNREACHABLE-bridge-side",
        window.is_maximized()
    ));
    send(gw, Command::GeometryChanged);
}

/// THE DRAG. One implementation, two triggers - the title band's `drag-delta` and the
/// synthetic act below both land here, for the same reason toggle_max does: the frame
/// arithmetic is the part that can be wrong, and it must not exist twice.
///
/// THE BUG THIS IS SHAPED AROUND: the frame/client double-count. A band that moves the
/// window by reading one kind of rect and writing the other walks it across the screen by
/// its own chrome, every drag, and the relaunch walks it again (smoke.rs: "the relaunch
/// rect walked by its own chrome"). So: read `position()`, which this toolkit reports
/// PHYSICAL and FRAME-INCLUSIVE (recorded at fingerprint_of above, i-slint-core api.rs:562)
/// add the pointer's delta to THAT, and write the sum. The bar's own height appears nowhere
/// in this function - not needing it is the test of whether the double-count is possible.
///
/// DPI, stated rather than assumed silently: `set_position` takes LOGICAL px and this probe
/// runs on the primary monitor at scale 1.0, so logical == physical and the cast below is
/// an identity. At 1.25 or 1.5 the division by `window.scale_factor()` belongs in THIS
/// function and nowhere else, which is the point of having one.
fn drag_by(weak: &slint::Weak<Spike>, pump: &RefCell<Pump>, dx: i32, dy: i32, via: &str) {
    let Some(ui) = weak.upgrade() else { return };
    let window = ui.window();
    // GUARD, decision (1): refuse a delta while the window is maximised.
    //
    // Why this is not paranoia - measured, in the run that produced the rule: a maximised
    // window reports position() <-8,-8> (the invisible 8 px border), so "read, add the
    // pointer's delta, write" is arithmetically perfect and semantically wrong. It asked
    // for <52,32>, the toolkit agreed, and the PORT STORED 52,32 as the normal position -
    // destroying a restore point the user had put at <180,130>, while every delta needle
    // read <0,0>. The check cannot see this class, because the walk it hunts for is not
    // happening: the INPUT is meaningless, not the arithmetic.
    //
    // The shipped-app candidate is (2), WINDOWS RESTORE-UNDER-POINTER: releasing a maximised
    // title bar restores the window with the cursor inside its new width, which is what a
    // user expects and what this refuses to imitate half-way. It costs two things this spike
    // does not have - the press position as an ABSOLUTE screen point (drag-delta carries a
    // delta precisely so markup cannot lie about placement, so the callback would need a
    // second argument), and a set_maximized(false) taken through toggle_max's door rather
    // than a second window call. Both are geometry decisions, and this is the one place
    // either would live. Reported once per maximised episode, because a real drag emits a
    // frame per mouse move and a flood would bury the finding.
    if window.is_maximized() {
        let mut p = pump.borrow_mut();
        if !p.drag_refused_shown {
            p.drag_refused_shown = true;
            drop(p);
            report("drag[refused]: maximised, window unmoved");
        }
        return;
    }
    // THE FRAME-INCLUSIVE READ - "dragging starts with a frame rect read", same call the
    // round-trip probe uses, taken after the show so there is no pre-show placeholder to
    // do arithmetic with.
    let here = window.position();
    let want = (here.x + dx, here.y + dy);
    window.set_position(LogicalPosition::new(want.0 as f32, want.1 as f32));
    let back = window.position();
    pump.borrow_mut().drag_target = Some(want);
    report(&format!(
        "drag[{via}]: from <{},{}> by <{dx},{dy}> slint-max={} -> intended <{},{}> toolkit reads <{},{}> delta <{},{}>",
        here.x,
        here.y,
        window.is_maximized(),
        want.0,
        want.1,
        back.x,
        back.y,
        back.x - want.0,
        back.y - want.1
    ));
    // A delta that landed means the window is not maximised: re-arm the refusal print, so a
    // later maximise is reported once again instead of never.
    pump.borrow_mut().drag_refused_shown = false;
}

/// THE RELEASE - the other half of a drag, and the reason the two are separate functions.
/// Moving the window is a per-frame act; asking the port to STORE a rect is not. Chrome
/// emits drag-ended once, the synthetic act calls this once, and the per-frame path above
/// sends nothing at all: a GeometryChanged per mouse move would have the engine measuring
/// and writing a session on every frame of a drag.
fn drag_release(gw: &Rc<RefCell<Option<Gateway>>>, via: &str) {
    report(&format!(
        "drag[{via}]: released, asking the port to store the rect"
    ));
    send(gw, Command::GeometryChanged);
}

/// The 5th row's act and Ctrl+Shift+R's, one function for both triggers. Nothing to
/// translate: the port owns the list, the cap and the dedupe, so the bridge only asks - and
/// the Event::RecentsUpdated that comes back is both the redraw AND the witness that the
/// command landed.
fn clear_recents(gw: &Rc<RefCell<Option<Gateway>>>, via: &str) {
    report(&format!("recents: {via} -> Command::ClearRecents"));
    send(gw, Command::ClearRecents);
}

/// Fire one resolved Route through the callback its ROW uses. A real key press and the
/// synthetic driver differ only in how the Route was chosen.
/// Step n of the walk: Open on its own at CHORD_AT, the rest spaced from CHORD_TAIL_AT.
/// See CHORD_TAIL_AT for why a uniform spacing silently audits the wrong file.
fn chord_at(step: u64) -> Duration {
    if step == 0 {
        CHORD_AT
    } else {
        CHORD_TAIL_AT + CHORD_EVERY * ((step - 1) as u32)
    }
}

fn fire(ui: &Spike, route: Route, display: &str, what: &str) {
    report(&format!("chord: {display} -> {what}"));
    match route {
        Route::Open => ui.invoke_open_asked(),
        Route::SaveAs => ui.invoke_save_as_asked(),
        Route::Autosave => ui.invoke_autosave_asked(),
        Route::ClearRecents => ui.invoke_clear_recents_asked(),
        Route::Recent(index) => ui.invoke_open_at_index(index as i32),
    }
}

/// The caption minimize door, shaped exactly like toggle_max: take the weak handle, ask the
/// window, report the ANSWER rather than the wish. There is no un-minimize here on purpose:
/// minimize has no two states to toggle, and restoring a minimized window is the taskbar's
/// job, not this bar's - so the button is one-way and so is the door.
fn minimize(weak: &slint::Weak<Spike>, via: &str) {
    let Some(ui) = weak.upgrade() else { return };
    let window = ui.window();
    window.set_minimized(true);
    report(&format!(
        "caption: {via} -> set_minimized(true), is_minimized()={}",
        window.is_minimized()
    ));
}

fn popup_words(ui: &Spike) -> String {
    format!(
        "popup-x={}px floored={} overflow={}",
        ui.get_popup_x().round(),
        ui.get_popup_floored(),
        ui.get_popup_overflow()
    )
}

/// The x,y the PORT last wrote - the coordinates the NEXT launch restores from, which is
/// the only place a walk can hide where the toolkit's own read cannot see it.
fn stored_xy(dir: &StateDir) -> Option<(i64, i64)> {
    let src = std::fs::read_to_string(dir.0.join("session.json")).ok()?;
    Some((json_int(&src, "x")?, json_int(&src, "y")?))
}

/// One recent row, one `Command::Open`. THE SHARED PATH: the TouchArea's generated
/// callback calls this, and so does the synthetic click in the timer, which is what
/// lets an unattended run exercise the same code a click would.
fn open_recent(gw: &Rc<RefCell<Option<Gateway>>>, pump: &RefCell<Pump>, index: usize, via: &str) {
    let found = pump.borrow().recent_paths.get(index).cloned();
    match found {
        Some(path) => {
            pump.borrow_mut().click_pending = Some(path.clone());
            report(&format!(
                "recents: {via} row {index} -> Open {}",
                path.display()
            ));
            send(gw, Command::Open { path });
        }
        None => report(&format!("recents: {via} row {index} names no path")),
    }
}

#[cfg(test)]
mod chords {
    //! The table is the contract, so the table gets tested - the way menu.rs does it in the
    //! first bridge. Nothing here can press a key: there is no public Slint API that feeds a
    //! FocusScope a KeyEvent (checked against the vendored 1.17.1 sources - no
    //! dispatch_key_input, no KeyInputEvent anywhere in slint or i-slint-core). So these
    //! guard the half that IS machine-checkable: every row routes, the markup spells the same
    //! chords, and the decisions a reader would otherwise re-litigate (Ctrl+T is auto-save,
    //! not new note) are assertions instead of comments.
    use super::*;

    const MARKUP: &str = include_str!("../ui/main.slint");
    const POPUP: &str = include_str!("../ui/chrome.slint");

    /// A double quote without writing one: the assertion below needs the QUOTED letter the
    /// markup compares against, and escaping a quote through two layers of here-string is how
    /// the first draft of this file broke the build.
    fn quoted(word: &str) -> String {
        let q = 34u8 as char;
        format!("{q}{word}{q}")
    }

    /// The capture handler's body, up to the next top-level construct.
    fn capture_body() -> &'static str {
        MARKUP
            .split("capture-key-pressed(event)")
            .nth(1)
            .expect("the FocusScope has a capture-key-pressed handler")
    }

    #[test]
    fn the_maximize_cell_shows_the_other_asset_not_a_different_shade() {
        assert_eq!(caption_glyph(false), "icons/maximize.svg");
        assert_eq!(caption_glyph(true), "icons/restore.svg");
        for glyph in [caption_glyph(false), caption_glyph(true)] {
            assert!(
                POPUP.contains(glyph),
                "chrome.slint no longer names {glyph}"
            );
        }
        assert!(POPUP.contains("root.maximized ? @image-url"));
    }

    #[test]
    fn every_glyph_the_bar_names_exists_on_disk() {
        // A typo inside @image-url is legal Slint that renders nothing: --check passes, the
        // build passes, and the button is blank until a human looks at it. This is the only
        // gate that can see it, and it names what it found before it asserts anything.
        let q = 34u8 as char;
        let mut named = Vec::new();
        for word in POPUP.split(q) {
            if word.starts_with("icons/") && word.ends_with(".svg") {
                named.push(word.to_string());
            }
        }
        assert!(
            named.len() >= 6,
            "expected the pin pair and four caption glyphs, found {named:?}"
        );
        let mut missing = Vec::new();
        for name in &named {
            let path = format!("ui/{name}");
            if !std::path::Path::new(&path).exists() {
                missing.push(path);
            }
        }
        assert!(
            missing.is_empty(),
            "referenced glyphs that do not exist on disk: {missing:?}"
        );
    }

    #[test]
    fn the_caption_buttons_reuse_the_existing_doors() {
        // Bounded at BOTH ends: the caption region is not the end of the file, and slicing to
        // EOF reaches the popup, whose six rows legitimately DO write menu-open - the first
        // draft of this test failed on exactly that, which is the assertion working as a
        // question about scope rather than about behaviour.
        let start = POPUP
            .find("---- RIGHT: the caption slot")
            .expect("the caption region");
        let end = POPUP
            .find("---- CENTRE")
            .expect("the centre region, which follows the caption");
        assert!(start < end, "the caption region must precede the centre");
        let cap = &POPUP[start..end];
        assert!(
            cap.contains("root.toggle-max-requested()"),
            "maximize must go through the double-click's door"
        );
        assert!(
            cap.contains("root.quit-asked()"),
            "close must go through the Quit row's door"
        );
        assert!(cap.contains("root.minimize-requested()"));
        assert!(
            !cap.contains("menu-open ="),
            "a caption cell must not touch popup state"
        );
        assert!(
            cap.contains("background: Theme.red"),
            "close keeps the red hover rule"
        );
        assert_eq!(
            SHORTCUTS.len(),
            14,
            "caption buttons are pointer acts, not commands: the chord table stays at fourteen"
        );
    }

    #[test]
    fn menu_open_has_exactly_one_writing_file() {
        // The single-writer proof, as two greps. Chrome owns its in-out bit; the mounter may
        // read it (the mirrors and the backdrop's visible binding do) but may not assign it,
        // and every dismissal route - hamburger, six rows, backdrop, Escape - ends inside
        // chrome.slint. ABOUTSLINT: the same proof now covers TWO bits, and it is exact about
        // the new one - Chrome owns all seven writes to about-open (two opens: the row's click
        // and the about-asks door; five closes: backdrop, Escape, the hamburger twice, and the
        // probe's dismissal bump), while the mount assigns none.
        let chrome_writes = POPUP.lines().filter(|l| l.contains("menu-open =")).count();
        let about_writes = POPUP.lines().filter(|l| l.contains("about-open =")).count();
        assert_eq!(
            about_writes, 7,
            "exactly two opens and five closes may write about-open, all inside Chrome; found {about_writes}"
        );
        assert_eq!(
            MARKUP.matches("about-open =").count(),
            0,
            "the mount reads Chrome's About bit through a binding and never assigns it"
        );
        assert!(
            chrome_writes >= 8,
            "Chrome should own every write; found {chrome_writes}"
        );
        let outside_writes = MARKUP.matches("menu-open =").count();
        assert_eq!(
            outside_writes, 0,
            "main.slint must never assign Chrome's state, only forward asks"
        );
        assert!(POPUP.contains("backdrop-asked =>"));
        assert!(POPUP.contains("escape-asked =>"));
        assert!(POPUP.contains("toggle-menu-asked =>"));
        assert!(MARKUP.contains("chrome.backdrop-asked()"));
        assert!(MARKUP.contains("chrome.escape-asked()"));
    }

    #[test]
    fn the_popup_is_mounted_last_and_the_backdrop_sits_under_it() {
        // Sibling order IS the z-order in Slint, and the measurement that found it: a popup
        // painted by a bar declared BEFORE a TextEdit was covered by that editor's pixels
        // (histogram: #2a2a2a only in the 6 px strip above the text). So the mount declares
        // backdrop, then Chrome, and the catcher can be above the text without being above
        // the menu.
        let editor = MARKUP.find("editor := TextEdit").expect("the editor");
        let backdrop = MARKUP.find("backdrop := TouchArea").expect("the backdrop");
        let chrome = MARKUP.find("chrome := Chrome").expect("Chrome");
        assert!(editor < backdrop, "the editor must be below the catcher");
        assert!(backdrop < chrome, "the catcher must be below the popup");
        assert!(
            MARKUP.contains("visible: chrome.menu-open || chrome.about-open")
                && MARKUP.contains("enabled: chrome.menu-open || chrome.about-open"),
            "the catcher exists for both dismissible surfaces - and for neither when both are shut"
        );
        // ABOUTSLINT: the panel is declared AFTER the popup inside Chrome, so sibling order
        // puts the licence screen above the menu that opens it, which is the same z-order rule
        // that decided where the backdrop is mounted.
        let menu = POPUP.find("menu := Rectangle").expect("the popup");
        let about = POPUP.find("about := Rectangle").expect("the About panel");
        assert!(
            menu < about,
            "About must be declared after the popup, not before"
        );
        assert!(POPUP.contains("visible: root.about-open"));
    }

    #[test]
    fn aboutslint_is_instantiated_exactly_once_in_markup_that_paints() {
        // THE LICENCE GUARD, grep-grade on purpose: royalty-free §2(a) is discharged by
        // DISPLAYING the widget, so the one fact that must not rot in a refactor that reads as
        // cosmetic is that the std component is instantiated - once, in markup, through the
        // style library's door. Comment lines are sliced out FIRST, because the obligation is
        // argued in prose at the top of this very file: counting prose would count nothing that
        // paints. (/// lines go with // lines - trim_start, then the same prefix test.)
        let code: String = POPUP
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            code.matches("AboutSlint {").count(),
            1,
            "the licence screen is ONE instantiation of the std widget, not a drawing of one"
        );
        assert!(
            code.contains("import { AboutSlint } from \"std-widgets.slint\";"),
            "the element must arrive through the style library's re-export - without it the\n             compiler says Unknown element and the crate stops building"
        );
        // The negative half, and it is the reason a hand-drawn badge cannot pass this test by
        // accident: the artwork belongs to the widget, not to us. Copying the logo into markup
        // would look like compliance and be none of it.
        assert!(
            !code.contains("MadeWithSlint"),
            "AboutSlint's own asset must not be re-drawn here - display THEIR widget"
        );
        // THE ROW: sixth of six, and it keeps an EMPTY chord cell, because SHORTCUTS is a
        // legend of commands and About is not one. Same shape as Quit's absence, same reason.
        assert!(
            code.contains("text: \"About Slint\""),
            "the popup lost its 6th row"
        );
        assert!(
            code.contains("row-about := TouchArea { col: 0; row: 5;"),
            "the 6th row must be clickable and Chrome's own"
        );
        assert_eq!(
            code.matches("col: 1; row: 5; text: \"\"").count(),
            1,
            "the About row's chord cell stays empty - a key beside an act with no key is drift"
        );
        assert_eq!(
            code.matches("col: 0; row: 5; colspan: 2").count(),
            2,
            "the tint and the catcher occupy the 6th row, and nothing else does"
        );
        // THE BOX: the widget reports preferred-width/height of 100%, so it fills whatever it is
        // handed and demands only its layout minimum back - a panel with no size would show
        // nothing at all, which discharges nothing.
        assert!(
            code.contains("width: 340px;"),
            "the About panel has no fixed box to fill"
        );
        assert!(
            code.contains("height: 260px;"),
            "the About panel has no fixed box to fill"
        );
    }

    #[test]
    fn the_suggested_name_is_the_title_the_user_can_already_read() {
        // The strip says "s9-seed.notes", so the dialog asks for "s9-seed.notes" - a third
        // name for the same document (Untitled.notes while the title says something else) is
        // the kind of small inconsistency that makes a dialog feel like another app.
        assert_eq!(suggested_name("s9-seed.notes"), "s9-seed.notes");
        assert_eq!(suggested_name("scratch"), "scratch.notes");
        assert_eq!(suggested_name(""), "Untitled.notes");
        assert_eq!(suggested_name("   "), "Untitled.notes");
        // A path in, a file name out, on both separators, because the title can carry the
        // file name and a user can paste a path into a name field.
        assert_eq!(suggested_name(r"C:\dev\notes\a.notes"), "a.notes");
        assert_eq!(suggested_name("/tmp/thing"), "thing.notes");
        // An extension already there is left alone - no ".notes.notes" on a second ask.
        assert_eq!(suggested_name("report.md"), "report.md");
    }

    #[test]
    fn a_cancel_is_a_word_and_never_silence() {
        assert_eq!(dialog_words(DialogKind::Open, &None), "open: cancelled");
        assert_eq!(
            dialog_words(DialogKind::SaveAs, &None),
            "save as: cancelled"
        );
        assert_eq!(
            dialog_words(
                DialogKind::Open,
                &Some(PathBuf::from(r"C:\dev\x\keep.notes"))
            ),
            "open: keep.notes"
        );
        // The label is the verb the user pressed, so the status line and the log cannot name
        // two different acts for one reply.
        assert_eq!(DialogKind::Open.label(), "open");
        assert_eq!(DialogKind::SaveAs.label(), "save as");
    }

    #[test]
    fn the_picker_starts_where_the_document_lives() {
        let here = PathBuf::from(r"C:\dev\notes\a.notes");
        let there = PathBuf::from(r"D:\other\b.notes");
        assert_eq!(
            dialog_starting_dir(Some(&here), std::slice::from_ref(&there)),
            Some(PathBuf::from(r"C:\dev\notes"))
        );
        // No current file: the first recent is the next best guess, which after a
        // Clear recents is nothing at all and rfd keeps its own default.
        assert_eq!(
            dialog_starting_dir(None, &[here.clone(), there.clone()]),
            Some(PathBuf::from(r"C:\dev\notes"))
        );
        assert_eq!(dialog_starting_dir(None, &[]), None);
        // A bare relative name has no directory to offer; an empty path must not be handed
        // to a dialog as the working directory of everything.
        assert_eq!(
            dialog_starting_dir(Some(&PathBuf::from("bare.notes")), &[]),
            None
        );
    }

    #[test]
    fn z_and_y_are_swallowed_only_after_the_first_switch() {
        // The rule as a table of facts, which is the only honest headless witness: 1.17 cannot
        // deliver a key event, so what CAN be proven is the predicate the markup implements, and
        // that it is armed by STATE rather than by the chord.
        //
        // Before any switch: native undo must reach the editor untouched.
        assert!(
            !undo_quarantined(0, "z", true, false, false),
            "first document keeps its undo"
        );
        assert!(
            !undo_quarantined(0, "y", true, false, false),
            "first document keeps its redo"
        );
        // After the first switch: every replay spelling is swallowed.
        assert!(undo_quarantined(1, "z", true, false, false));
        assert!(
            undo_quarantined(1, "Z", true, false, false),
            "shift arrives as the upper glyph"
        );
        assert!(
            undo_quarantined(1, "z", true, true, false),
            "Ctrl+Shift+Z is redo, same hazard"
        );
        assert!(undo_quarantined(1, "y", true, false, false));
        assert!(
            undo_quarantined(7, "y", true, true, false),
            "and it stays armed for the session"
        );
        // What must NOT be caught: plain typing, the menu chords, and anything with Alt.
        assert!(
            !undo_quarantined(3, "a", true, false, false),
            "select-all stays alive"
        );
        assert!(
            !undo_quarantined(3, "s", true, false, false),
            "Save As is a command, not a replay"
        );
        assert!(
            !undo_quarantined(3, "r", true, true, false),
            "Ctrl+Shift+R keeps working"
        );
        assert!(
            !undo_quarantined(3, "z", false, false, false),
            "a bare z is a letter"
        );
        assert!(
            !undo_quarantined(3, "z", true, false, true),
            "Alt is someone else's chord"
        );
        assert!(
            !undo_quarantined(3, "x", true, false, false),
            "cut stays with the editor"
        );
    }

    #[test]
    fn the_quarantine_is_a_state_rule_and_the_markup_agrees() {
        // The pair is held together here because neither side can call the other: this greps
        // every piece of the condition out of the mount, so a one-sided edit (a dropped
        // generation test, a swapped key, an accept turned into a reject) fails the build.
        assert!(
            MARKUP.contains("root.doc-generation > 0"),
            "the arming condition vanished"
        );
        assert!(MARKUP.contains("event.modifiers.control"));
        assert!(MARKUP.contains("!event.modifiers.alt"));
        assert!(MARKUP.contains("event.text.to_lowercase() == \"z\""));
        assert!(MARKUP.contains("event.text.to_lowercase() == \"y\""));
        assert!(
            !MARKUP.contains("&& !event.modifiers.shift && !event.modifiers.alt && (event.text.to_lowercase() == \"z\""),
            "shift must stay allowed: Ctrl+Shift+Z is the redo chord and carries the same stale bytes"
        );
        // ORDER is the claim: the branch raises the signal and THEN accepts. A branch that
        // accepted without signalling is invisible in a log, and one that signalled without
        // accepting has swallowed nothing - so the two statements must be adjacent, in order.
        // (The first draft asserted that the slice between them contained "EventResult", which is
        // trivially false because the accept comes AFTER the signal. It failed, and for a better
        // reason than it thought it was testing.)
        // Measured by POSITION, not by a joined literal: the mount is stored with CRLF line
        // endings in this working copy, so any assertion that spells a newline in a string is
        // testing the file's line discipline rather than the code's order. Position says the
        // same thing with no such trap - and it also reports the gap it saw.
        let signal = MARKUP
            .find("root.undo-swallowed();")
            .expect("the swallow signal");
        let gap = MARKUP[signal..]
            .find("return EventResult.accept;")
            .expect("the accept that must follow it");
        let signal_len = "root.undo-swallowed();".len();
        assert!(
            gap > signal_len,
            "the accept must come AFTER the signal, not inside it"
        );
        let between = &MARKUP[signal + signal_len..signal + gap];
        assert!(
            between.trim().is_empty(),
            "only whitespace may sit between the signal and its accept; found {between:?}"
        );
        assert!(MARKUP.contains("root.undo-swallowed();"));
        assert!(MARKUP.contains("callback undo-swallowed();"));
        // And the table stays a legend of commands: fourteen, untouched, no undo row invented.
        assert_eq!(
            SHORTCUTS.len(),
            14,
            "the quarantine must not join the chord table"
        );
        assert!(
            !SHORTCUTS
                .iter()
                .any(|row| row.2.contains("undo") || row.2.contains("redo")),
            "no port Command exists for undo; the legend may not claim one"
        );
    }

    #[test]
    fn the_lock_verdict_names_the_cause_and_only_the_cause() {
        // Four facts, because the port ships two independent bits and the UI must not blur them.
        // An unlocked file gets the EMPTY word, which is what makes `locked = !word.is_empty()`
        // safe: there is no third state where a reason exists without a lock.
        assert_eq!(lock_verdict(false, false), "");
        assert_eq!(lock_verdict(true, false), "read-only on disk");
        assert_eq!(
            lock_verdict(false, true),
            "read-only: too big to edit safely (8 MiB guard)"
        );
        let both = lock_verdict(true, true);
        assert!(both.contains("on disk") && both.contains("8 MiB"), "{both}");
        assert_ne!(lock_verdict(true, false), lock_verdict(false, true));
    }

    #[test]
    fn the_startup_adoption_is_not_a_switch() {
        // S4c: the policy is driven here, not the predicate - undo_quarantined(gen, ..) was right
        // from the start and could not fail, because the gen it was handed came from a policy that
        // stepped on the app's own startup open. A test that takes the number as an argument
        // cannot catch the code that produces it, and (the 4b lesson, one level up) a test that
        // MANUFACTURES the witness cannot audit it either: this one used to set `loop_ticked` by
        // hand and still passed while the live run armed the quarantine at startup.
        let a = PathBuf::from("C:/probe/same.notes");
        let b = PathBuf::from("C:/probe/other.notes");
        let pump = RefCell::new(Pump::default());
        // The startup pair: the engine answers the initial query with a Rebound and then a Loaded,
        // both naming the SAME path. That pair armed the quarantine under both temporal witnesses
        // as soon as an answer arrived late - i.e. whenever the disk was slow.
        assert_eq!(
            note_adoption(&pump, &a),
            0,
            "the rebind that answers startup is not a switch"
        );
        assert_eq!(
            note_adoption(&pump, &a),
            0,
            "nor is the Loaded that follows it for the same path"
        );
        // The pump counter stays decoupled on purpose: the locked act calls text_pump from
        // startup, so invocations is non-zero before the loop has ever run.
        pump.borrow_mut().invocations = 9;
        assert_eq!(
            note_adoption(&pump, &a),
            0,
            "the same path never steps, pump calls or not"
        );
        // A DIFFERENT document is a switch, and it is now the only thing that can arm.
        assert_eq!(
            note_adoption(&pump, &b),
            1,
            "a different path is the switch"
        );
        assert_eq!(
            note_adoption(&pump, &a),
            2,
            "and switching back is another one"
        );
        assert_eq!(pump.borrow().generation, 2);
        // The timing-proof claim, restated: with the counter zeroed and the same path still
        // current, nothing steps. There is no longer a moment in a run where it can.
        pump.borrow_mut().invocations = 0;
        assert_eq!(
            note_adoption(&pump, &a),
            2,
            "idempotent for the current document"
        );
        // The witness clearing lives in the same function, which is why both sites call it: an
        // adoption must never look like an edit, on either path.
        pump.borrow_mut().edited_flag = true;
        pump.borrow_mut().pending_at = Some(Instant::now());
        assert_eq!(note_adoption(&pump, &b), 3);
        assert!(
            !pump.borrow().edited_flag,
            "an adoption never counts as an edit"
        );
        assert!(
            pump.borrow().pending_at.is_none(),
            "nor starts a debounce clock"
        );
    }

    #[test]
    fn the_generation_steps_once_per_adoption_and_alternates() {
        // The undo mitigation keys on PARITY: two mutually exclusive editor branches, so every
        // flip destroys and recreates the widget. Both adoption sites call this one function,
        // which is what makes "the generation moved" mean "the widget would be rebuilt" rather
        // than "some counter somewhere changed". Six adoptions, six different documents.
        let mut g = 0;
        let mut branches = Vec::new();
        for _ in 0..6 {
            g = next_generation(g);
            branches.push(g % 2 == 0);
        }
        assert_eq!(branches, vec![false, true, false, true, false, true]);
        assert_ne!(next_generation(3), 3, "an adoption must always move");
        assert_eq!(
            next_generation(i32::MAX - 1),
            i32::MAX,
            "no wrap in a session"
        );
    }

    #[test]
    fn the_editor_is_one_widget_and_the_generation_is_only_carried() {
        // S10 said out loud, in the shape that will fail if someone half-ships it. The
        // recreation is NOT in this tree: one TextEdit, named directly by the outer focus
        // scope, and the generation present but unused as a key. The 1.17 compiler refused the
        // conditional form the mitigation needs - an id declared inside a conditional branch is
        // invisible to the enclosing scope, so forward-focus cannot name the live one, and
        // has-focus is an out-property, so focus cannot be driven back. Re-measured in the
        // S10 probes (g1, f1, x3) rather than remembered from a document.
        assert!(!MARKUP.is_empty(), "sanity: the markup is loaded");
        assert_eq!(
            MARKUP.matches("TextEdit {").count(),
            1,
            "two editors means this test is rewritten, not deleted"
        );
        assert!(
            !MARKUP.contains("if root.doc-generation"),
            "no half-shipped recreation"
        );
        assert!(MARKUP.contains("forward-focus: editor"));
        assert!(MARKUP.contains("in-out property <int> doc-generation"));
        // Item 2 and item 3, both structural.
        assert!(MARKUP.contains("wrap: TextWrap.no-wrap"));
        assert!(MARKUP.contains("callback text-edited();"));
        assert!(MARKUP.contains("edited =>"));
    }
    #[test]
    fn every_chord_routes_somewhere() {
        for row in SHORTCUTS {
            assert!(
                route_of(row.3).is_some(),
                "chord {} names act {}, which nothing implements",
                row.0,
                row.3
            );
        }
    }

    #[test]
    fn table_shape_matches_the_first_bridge() {
        assert_eq!(SHORTCUTS.len(), 14, "four commands plus ten recents");
        assert_eq!(
            SHORTCUTS.iter().filter(|r| r.0.starts_with("alt-")).count(),
            10
        );
        assert_eq!(
            SHORTCUTS
                .iter()
                .filter(|r| r.0.starts_with("ctrl-"))
                .count(),
            4
        );
        let mut displays: Vec<&str> = SHORTCUTS.iter().map(|r| r.1).collect();
        displays.sort();
        displays.dedup();
        assert_eq!(
            displays.len(),
            SHORTCUTS.len(),
            "two chords cannot share a display spelling"
        );
        let mut keys: Vec<&str> = SHORTCUTS.iter().map(|r| r.0).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(
            keys.len(),
            SHORTCUTS.len(),
            "one binding cannot route two acts"
        );
    }

    #[test]
    fn alt_zero_is_the_tenth_recent_not_the_zeroth() {
        let row = SHORTCUTS
            .iter()
            .find(|r| r.0 == "alt-0")
            .expect("alt-0 row");
        assert_eq!(row.1, "Alt+0");
        assert_eq!(row.2, "recent 10");
        assert_eq!(
            route_of(row.3),
            Some(Route::Recent(9)),
            "zero-based index, tenth entry"
        );
    }

    #[test]
    fn ctrl_t_is_toggle_autosave_and_there_is_no_new_note_chord() {
        let row = SHORTCUTS
            .iter()
            .find(|r| r.0 == "ctrl-t")
            .expect("ctrl-t row");
        assert_eq!(route_of(row.3), Some(Route::Autosave));
        assert!(
            SHORTCUTS.iter().all(|r| !r.2.contains("new")),
            "no new-note chord anywhere: it needs a port Command and an ADR conversation"
        );
    }

    #[test]
    fn legend_names_every_bound_chord_and_nothing_else() {
        let words = legend();
        for row in SHORTCUTS {
            assert!(
                words.contains(&format!("{} {}", row.1, row.2)),
                "legend lost {}",
                row.1
            );
        }
        assert_eq!(words.matches("  |  ").count(), SHORTCUTS.len() - 1);
        // ABOUTSLINT: the legend is a legend of CHORDS, and the About row has none - it is a
        // pointer act with no Command behind it, so it must not appear here even though it now
        // appears in the popup. The row's own chord cell is empty for exactly this reason.
        assert!(
            !words.to_lowercase().contains("about"),
            "the chord legend advertised the About row, which has no key and no command"
        );
    }

    #[test]
    fn markup_agrees_with_the_table_on_every_spelling() {
        // The drift guard: the popup's chord column and the capture handler are the two places
        // a chord is spelled in markup. If either drops a row, this fails instead of shipping
        // a key nothing labels or a label nothing binds.
        for row in SHORTCUTS {
            if let Some(chord) = row.0.strip_prefix("ctrl-") {
                // The LAST segment is the key: "ctrl-o" is o, and "ctrl-shift-r" is r with a
                // shift. Deriving it beats hard-coding a letter per row, and it is this
                // derivation that caught the first draft of this test failing on its own
                // assumption (it looked for a key named "shift-r", which no keyboard sends).
                let letter = chord.rsplit('-').next().unwrap_or(chord);
                assert!(
                    POPUP.contains(row.1),
                    "the popup lost its {} chord cell",
                    row.1
                );
                assert!(
                    MARKUP.contains(&quoted(letter)),
                    "the capture tree lost the {} branch",
                    row.0
                );
                if chord.contains("shift-") {
                    assert!(
                        MARKUP.contains("event.modifiers.shift"),
                        "a two-modifier chord needs the shift test, and it must come FIRST"
                    );
                    assert!(
                        MARKUP.contains("!event.modifiers.shift"),
                        "the plain-Ctrl branches must exclude shift, or Ctrl+Shift+R arrives twice"
                    );
                }
            } else {
                assert!(
                    MARKUP.contains("event.modifiers.alt"),
                    "the capture tree lost the Alt branch"
                );
            }
        }
        assert!(
            MARKUP.contains("root.clear-recents-asked()"),
            "Alt/Ctrl+Shift+R must reach the 5th row's own callback"
        );
        assert!(
            MARKUP.contains("root.open-at-index(9)"),
            "Alt+0 must reach the tenth recents row"
        );
    }

    #[test]
    fn capture_path_ends_by_rejecting_everything_it_did_not_match() {
        // The negative claim, pinned down: the handler accepts each chord it matches and its
        // LAST act is a reject, which is what leaves Ctrl+A/C/V/X and every typed character to
        // the editor. Grep-quality proof, not behavioural - behaviour needs a real key.
        // S10b moves the count by ONE statement, not two: the sixteen accepts are fourteen chords
        // plus Escape plus ONE branch that covers both replay keys (z and y) in a single return.
        // And Z is now only "left to the editor" while the generation is still zero, which
        // undo_quarantined() states as a rule and the test below greps out of the markup.
        let body = capture_body();
        assert_eq!(
            body.matches("EventResult.accept").count(),
            SHORTCUTS.len() + 2,
            "one accept per bound chord, plus Escape: fourteen commands in the table and ONE              dismissal branch, which is not a command and so is not in the legend (menu.rs has no              Escape row either - see the parity warning in main.slint). ABOUTSLINT moved the              CONDITION, not the count: that one branch now closes whatever is open, popup or licence              screen, so sixteen accepts still means fourteen + Escape + the undo quarantine."
        );
        assert_eq!(
            body.matches("EventResult.reject").count(),
            1,
            "exactly one fall-through exit"
        );
        assert!(
            body.rfind("EventResult.reject") > body.rfind("EventResult.accept"),
            "the reject must come last, or a chord silently swallows the editor's keys"
        );
    }
}
