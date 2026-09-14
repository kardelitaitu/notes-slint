// STILL NEEDED AFTER STEP B, and now it names exactly what: the callback wiring landed, so the
// dialog mailbox, ask_dialog/answer_dialog and the whole of wire_callbacks are LIVE here — they are
// no longer part of the reason this allow exists. What is left is items that live in surface.rs and
// belong to the probe only. STRIP-5 (2026-09-15) shrank that list by two entries: `wire_callbacks`
// and its dialog mailbox are live (the C2 hooks), `caption_glyph` now has a REAL product caller — the
// toggle-max hook in surface.rs names the asset in its report line — and the two surface fns that
// are dead in the PROBE root (the probe wires its own handlers) carry their own per-item allow THERE,
// which is the honest direction for an allow. What is left, and the only reason this line still
// exists, is the measurement half of `Pump`: `asked`, `hold_reported`, `ticks`, `last_bucket`,
// `strokes`, `quarantine_reported` and the act-machine counters after surface.rs:700. Deleting those
// fields would break the other root; reading them from here would be writing acts into the product,
// which is STRIP-4's opposite. A per-field allow on ~25 lines is the remaining step, and it is OWED,
// not done — this box spent itself on the two features.
#![allow(dead_code)]
#![windows_subsystem = "windows"]

//! STRIP-2b half 2, dated 2026-09-15: THE DATED LIE ENDS HERE. Until this file existed, the
//! product-owned name "notes-slint" built probe.rs, and the manifest said so out loud. It now
//! builds THIS file, and the probe keeps the name it deserves: notes-slint-probe -> src/probe.rs.
//! Cargo's "file found to be present in multiple build targets" warning died with the sharing,
//! which was its only purpose.
//!
//! THE SHAPE, and the law that holds it: this root declares the modules and owns no policy. It
//! contains NO Event arm and NO send() of its own - every event is read by surface::drain and
//! every command goes through plumbing::send. If a product need turns out to be unexposed, the
//! fix is to move that decision out of the probe INTO surface in the same commit (the guard law
//! that has already caught this crate twice), never to write a second copy here. What stays in a
//! root is exactly what a root must be: where the state dir comes from, that a window exists,
//! where it is placed, whose handle the port is told about, when the loop wakes, and what a close
//! request means.
//!
//! NO CONSOLE ATTACH, on purpose, and not by omission: "grep -rn AttachConsole crates/" finds
//! nothing anywhere in this repo, and a console attach is a WinAPI call - the "windows*" family
//! that check-arch forbids to a bridge. So this root matches bridge-gpui exactly:
//! windows_subsystem = "windows" (its main.rs:1) plus stderr as the only voice
//! (bridge-gpui/src/editor.rs:45; main.rs:752, "a windows_subsystem binary has no other voice").
//! That survives smoke, because smoke reads a CHILD'S PIPE and a pipe is inherited whether or not a
//! console exists; it survives a person launching from a terminal for the same reason. What it
//! cannot do is show a trace to someone who double-clicked the exe - and that is BACKLOG FOR
//! notes-platform, not a line for this crate: an api call ("may I have a console, and did I get
//! one") answered by the crate that owns the unsafe-and-FFI budget. Same seam, same reason, as the
//! unparented file dialog.
//!
//! STRIP-4 OWES THIS ROOT (named so the absence is a decision, not a surprise):
//! (1) the geometry settle EPISODE in bridge-gpui's shape - GEOMETRY_QUIET 250 ms / GEOMETRY_FORCE
//!     1 s, unbounded (bridge-gpui/src/main.rs:380, :385, :525-533) - the pair of consts the probe
//!     caps at two sends for its own measurement reasons;
//! (2) the honest shutdown (final flush, then join, then name what the buffer held), which in the
//!     probe is a hundred lines of witness around Exit::{QueueClosed, Abandoned, Panicked};
//! (3) the panic hook, which a windows_subsystem binary NEEDS because a panic is otherwise
//!     invisible (bridge-gpui/src/main.rs:1396);
//! (4) the recents exists-mark, the legend keys in a real bar, and the pin check mark's round trip.
//! STATUS (the ADR-0006 ledger): (1) PAID at 2fc924f3 live, (2) PAID on real stderr by
//! smoke.rs's PRODUCT_CLOSE_NEEDLES, (3) PAID by tests/panic_hook.rs at ebbb755c; (4) is owed ONLY
//! its pin check-mark round trip - see 2026-09-14-adr0006-ledger.md.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notes_api::{
    Command, DropGuard, Exit, Gateway, Rect, Settings, StateDir, WindowHandle, resolve_state_dir,
};
use slint::{
    CloseRequestResponse, ComponentHandle, LogicalPosition, LogicalSize, Timer, TimerMode,
};
// FOCUS-AT-STARTUP (the fn below is the reason): 1.17's PUBLIC slint::Window has no focus door at
// all - api.rs lists show / hide / place / resize / dispatch_event and stops. These two names are the
// SAME path the macro's own generated .focus() compiles to (i-slint-compiler generator/rust.rs:3635
// emits WindowInner::from_pub(..).set_focus_item(.., FocusReason::Programmatic)), so the root borrows
// the toolkit's door rather than inventing one - the only alternative is an imperative focus() in the
// markup, which main.slint:178 forbids for routing; claim_focus is the caret handoff it allows.
use slint::private_unstable_api::re_exports::{FocusReason, WindowInner};

mod plumbing;
mod surface;
mod title_contract;
mod ui_gen;

use plumbing::{arm_drop_target, fingerprint_of, hwnd_of, note_dot, publish_title, report, send};
use surface::{
    DialogReply, Pump, Retake, answer_dialog, ask_corners, drain, legend, restore_from_session,
    retry_take, text_pump, wire_callbacks,
};
use ui_gen::Spike;

/// A6 / THE DOOR: the one place in this bridge allowed to send on a TIMER, and it is a step in the
/// product's own wake - after the drain that fills the owed record, beside the pump that already
/// holds a gateway legitimately. Two rules it exists to keep.
///
/// THE CHANNEL: a retry is a Save. The record carries the exact text, revision and epoch that were
/// refused and this sends THAT pair, not a fresh reading of the buffer. Not a Flush at all, because
/// the default case is a foreign file whose FIRST save failed: armed is set only on write success,
/// so should_flush refuses a retry of that file on ForeignFileNotArmed, and with the toggle off on
/// AutosaveDisabled. A retry routed past those gates is a retry that can never fire - which is what
/// made the old witness-clearing lane structurally dead rather than merely inelegant.
///
/// THE END: six attempts, 750 ms doubling to a 10 s cap, then terminal. At the cap the record is
/// gone, the witness stays dirty, the reason stays on the status line, and the next user act carries
/// the text. A cap costs nothing here precisely because the terminal state hands the write over;
/// an uncapped loop against a permanent refusal is the resource leak.
/// A6, step four: the door's DECISION, split out so it can be tested with no window and no queue.
/// This is where the law lives in code: an owed retry becomes a `Command::Save` carrying the refused
/// pair, and nothing else the record can say - not yet due, nothing owed, the ladder spent - becomes a
/// command at all. Exactly one line in the door below is left untested, and it is the line that hands
/// the command to the port, because that line needs a live gateway.
fn retry_sends(take: &Retake) -> Option<Command> {
    match take {
        Retake::Send {
            text,
            revision,
            epoch,
            ..
        } => Some(Command::Save {
            text: text.clone(),
            revision: *revision,
            epoch: *epoch,
        }),
        Retake::Idle | Retake::Waiting | Retake::Terminal => None,
    }
}

fn retry_door(gw: &Rc<RefCell<Option<Gateway>>>, pump: &RefCell<Pump>) {
    let take = retry_take(pump);
    if let Retake::Send {
        text,
        revision,
        epoch,
        next,
        attempt,
    } = &take
    {
        report(&format!(
            "retry: attempt {attempt} - re-sending Command::Save rev={revision} epoch={epoch} ({bytes} bytes) - a retry is a Save, never a Flush, next back-off {next:?}",
            bytes = text.len()
        ));
    } else if matches!(take, Retake::Terminal) {
        report(
            "retry: terminal - the cap is spent, the text stays unsent, the reason stays on the line, and the next act carries it",
        );
    }
    // The one untested line, and it is untested because it needs a live port: the decision above is
    // the part that can be wrong in a way a test could not see.
    if let Some(command) = retry_sends(&take) {
        send(gw, command);
    }
}

/// THE PORT RULE AND NOTHING ELSE. The probe overrides its state dir with a directory beside its
/// own exe ("slint-probe data") so that a thousand probe runs cannot dirty a person's real session;
/// a product does not have that option, and this function is the whole difference between the two
/// roots at startup. The portable form is kept: a "data" directory beside the exe means a portable
/// install and wins over %APPDATA%, which is the packaging rule from the plan, not a probe habit.
fn state_dir() -> StateDir {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let portable = exe_dir.join("data").is_dir();
    let appdata = (!portable)
        .then(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .flatten();
    resolve_state_dir(&exe_dir, appdata.as_deref())
}

/// STRIP-4a row 2: the settle watch, in bridge-gpui's SHAPE - an episode, not a count. Its
/// constants are gpui's (main.rs:380 GEOMETRY_QUIET 250 ms, :385 GEOMETRY_FORCE 1 s), and what is
/// deliberately NOT inherited is the probe's cap of two sends: that cap was a measurement budget
/// for one act walk, and a user who keeps dragging has to end up with the port holding the rect
/// the window is ACTUALLY at. Unbounded here, on purpose.
const GEOMETRY_QUIET: Duration = Duration::from_millis(250);
const GEOMETRY_FORCE: Duration = Duration::from_millis(1000);
/// How long this root is willing to wait for the two things that must not hang it: the last
/// flush, and (after an abandoned join) the port's terminal signal that the engine is gone. The
/// second number is gpui's 10 s (bridge-gpui/src/main.rs:2478-2481), the first deliberately far
/// shorter: AUTOSAVE_IDLE is 750 ms, so 2 s is already two and a half idle periods and a disk that has
/// not answered by then is not going to answer before a person gives up.
const SAVE_WAIT: Duration = Duration::from_millis(2000);
const EXIT_REWAIT: Duration = Duration::from_secs(10);
/// FOCUS-AT-STARTUP: how many 8 ms wakes this root keeps asking for a focus item before it prints the
/// failure instead. 250 is ~2 s, and the window is mapped and laid out inside the first few pumps, so
/// a window still unfocused after that is not a race to win but a fact to print. The ceiling is the
/// point: the commit before this one retired a REGISTRATION retry that ran ~125 times a second forever
/// (see `Register`), and an uncapped focus retry is the same storm in a different name.
const FOCUS_TRIES: u16 = 250;

/// B-S4: THE FLOOR, in logical CLIENT pixels, and the first value the markup's two knobs have ever
/// had. Both numbers are read off chrome.slint's own About panel rather than picked.
///
/// 340 is the panel's width with NO padding: popup-left's floor is zero (chrome.slint:230-234), so
/// the 8px side margins are POLICY while the panel's own width is OBLIGATION - under 340 no x places
/// the licence at all, which is precisely what about-overflow asserts (host-width < about.width).
///
/// 262 is the panel's 260 plus exactly ONE Theme.menu-gap of 2, the gap popup-top keeps under the
/// box (chrome.slint:198-202). NOT 263 and NOT 264: a Slint Rectangle draws its border CENTERED on
/// the geometry edge (i-slint-core graphics/border_radius.rs:195-199, the inner/outer pair at
/// half_border_width; item_rendering.rs:523,544 clip children by that same border_width INSIDE the
/// item box), so the 260px literal is already border-inclusive and counting border-width a second
/// time would ask the floor for half a pixel of antialiasing.
///
/// These two lines are the RUST side of a contract whose numbers belong to the markup, and
/// tests::the_floor_contract_is_ordering_and_this_slice_sets_no_floor_value is what keeps them
/// married: it parses the About block, computes the demand from what it finds, and compares these
/// consts against it as ORDERING (a floor below the demand clips the licence and fails; a floor
/// above it is never an error). That is why the consts carry these exact names.
const FLOOR_WIDTH: f32 = 340.0;
const FLOOR_HEIGHT: f32 = 262.0;

/// HYGIENE (P1b fix 1): the LATE-REGISTRATION LATCH. The guard this replaced was
/// "is the drop lease still empty", and plumbing.rs:255-262 deliberately leaves that lease empty
/// when `arm_file_drop` refuses - so one refusing handle re-sent `RegisterWindow`, re-ran the arm
/// and reprinted both of its lines every 8 ms tick: about 125 attempts a second, forever, in a
/// product that had already been told no. The latch is a bool plus the two facts that make
/// "the same Err twice" decidable at all: WHICH handle was refused, and how many times.
#[derive(Clone, Copy, Default)]
struct Register {
    /// The handle the last attempt named. None until there has been an attempt.
    last: Option<i64>,
    /// Attempts made against `last`, so a NEW handle is not punished for the old one's refusal.
    tries: u8,
    /// THE LATCH: the arm landed, or this same handle was refused twice. Either way the wake
    /// stops asking and stops printing.
    done: bool,
}

/// One attempt's outcome, as a pure decision so it can be tested without a window: armed ends it
/// (the ordinary case, first wake that can read an HWND); the same handle refused twice ends it;
/// a different handle restarts the count, because that is a different window and the old verdict
/// says nothing about it.
fn register_says(previous: Register, hwnd: i64, armed: bool) -> Register {
    if armed {
        return Register {
            last: Some(hwnd),
            tries: previous.tries.saturating_add(1),
            done: true,
        };
    }
    let tries = if previous.last == Some(hwnd) {
        previous.tries.saturating_add(1)
    } else {
        1
    };
    Register {
        last: Some(hwnd),
        tries,
        done: tries >= 2,
    }
}

/// HYGIENE (P1b fix 3): the close wait's verdict, as words this file owns. A save that landed is
/// still the best answer, but it is no longer the ONLY answer the wait accepts, and a reader of
/// the line must be able to tell "autosave is off, the port said so" from "the disk never
/// answered" - the second is the one that costs a person bytes.
fn flush_verdict(saves_moved: bool, settled_moved: bool, answer: &str, dirty: bool) -> String {
    if saves_moved {
        "landed".to_string()
    } else if settled_moved {
        format!(
            "was ANSWERED with {answer} - the port is done with this flush, nothing more was coming"
        )
    } else if !dirty {
        "was not needed - the buffer matched what was sent".to_string()
    } else {
        "DID NOT land in the wait, closing anyway".to_string()
    }
}

/// What the watch remembers: the rect last seen, when it last differed from what the port was
/// told, and when the port was last told.
#[derive(Default)]
struct Settle {
    seen: Option<Rect>,
    changed_at: Option<Instant>,
    sent_at: Option<Instant>,
}

/// THE DECISION, separated from the measuring so it can be tested without a window.
/// Quiet long enough: the drag ended, tell the port. Still moving but the last tell was a FORCE
/// interval ago: tell anyway, because a drag that never stops must not mean a stale rect forever
/// (gpui's comment on its own force: a resize episode is bounded by FORCE, not by a count).
fn settle_says(moved_for: Duration, since_send: Option<Duration>) -> bool {
    moved_for >= GEOMETRY_QUIET || since_send.is_some_and(|ago| ago >= GEOMETRY_FORCE)
}

/// WHAT THIS WAKE'S MEASUREMENT MEANS to the watch, as pure as the decision beside it and for
/// the same stated reason: no window, no channel, no clock, so a test can ask it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Wake {
    /// The first rect this root has ever read. There is nothing for it to differ FROM, so it is a
    /// BASELINE: the watch learns the number and opens no episode.
    Baseline,
    /// All four numbers identical to the last wake.
    Same,
    /// A rect the watch has not seen. THIS is the user moving or resizing the window.
    Moved,
}

/// THE COMPARISON, lifted out of the wake unchanged in every particular except one: a watch that
/// remembers nothing CANNOT have seen a change. It used to answer "changed" anyway, because
/// `Settle::default()` seeds `seen: None` and the old test was a plain four-number inequality
/// against an Option that had no value in it - so wake 1 of every launch looked exactly like a
/// hand on an edge. That is not cosmetic: `Command::GeometryChanged` is a bare trigger, the
/// engine answers it by MEASURING the live window (api engine.rs:783-795 via
/// platform::frame_rect) and persisting what it measured, so every launch rewrote session.json
/// with whatever the window happened to be. Under a window floor that becomes destructive rather
/// than merely noisy: a saved 120x120 comes back 340x264 and the file is corrected to lie.
fn wake_says(previous: Option<Rect>, measured: Rect) -> Wake {
    let Some(then) = previous else {
        return Wake::Baseline;
    };
    if then.x == measured.x && then.y == measured.y && then.w == measured.w && then.h == measured.h
    {
        Wake::Same
    } else {
        Wake::Moved
    }
}

/// THE GATE, in one place so the wake below and the test agree on it: only a MOVE opens an
/// episode, and an episode (`changed_at` set) is the ONLY thing the send is willing to wait on.
/// A baseline therefore queues nothing, however long the run sits idle - which is this function's
/// whole claim, and the reason `settle_says` below did not have to change by one character.
fn arms_episode(wake: Wake) -> bool {
    matches!(wake, Wake::Moved)
}

/// The restore asked for a size and the window came back at a DIFFERENT one, in a direction a
/// clamp owns. Both directions count now that B-S4 raised a real floor.
///
/// SMALLER than asked is the old case, and still the one with no other explanation.
///
/// BIGGER than asked counts only when what was asked for sat UNDER the floor - that IS the floor
/// speaking, a seeded 120x120 coming back as 340x262, exactly the run where the file and the window
/// disagree, and what this line was written to voice. Bigger-than-asked with a want already above
/// the floor stays silent, because that is a maximised session returning from the port's own
/// snapshot and "floored" about it would be a trace line about nothing.
///
/// HALF a pixel of slack on both, because `want` is a divided-by-scale float while the read-back is
/// an integer.
fn floored_says(want: LogicalSize, measured: Rect) -> bool {
    let slack = 0.5;
    let (w, h) = (measured.w as f32, measured.h as f32);
    let smaller = want.width - w > slack || want.height - h > slack;
    let raised = (w - want.width > slack && want.width < FLOOR_WIDTH)
        || (h - want.height > slack && want.height < FLOOR_HEIGHT);
    smaller || raised
}

/// STRIP-4a row 3, the part that CAN be unit-tested: the one line a panic gets, before the
/// default hook unwinds. A windows_subsystem binary makes a panic otherwise INVISIBLE
/// (bridge-gpui/src/main.rs:1396 says exactly that), so the line has to name itself as a panic,
/// carry the payload, and carry the location when the runtime had one - because the exit code
/// alone does not tell a person WHICH invariant broke.
fn panic_note(what: &str, where_: Option<&str>) -> String {
    match where_ {
        Some(at) => format!("panic: {what} at {at}"),
        None => format!("panic: {what}"),
    }
}

/// FOCUS-AT-STARTUP, and the measured fact that makes it necessary: launched by the OS rather than by
/// a click, this product's chords are DEAD until a pointer press lands inside the window. Not a
/// theory about the harness - the mechanism is readable in 1.17 and both halves of it are true:
///   * every chord this app answers lives in `capture-key-pressed` on the markup's OUTERMOST
///     FocusScope (ui/main.slint:188), and Slint runs that capture pass only along the path from the
///     window to its CURRENT focus item (i-slint-core window.rs:1074, then the `capture_key_event`
///     loop at :1098). No focus item, no path, no chord - the keystroke is swallowed and the status
///     line never moves;
///   * the thing that installs a focus item is a pointer press (i-slint-backend-winit accesskit.rs:810
///     calls set_focus_item with FocusReason::PointerClick), which is exactly why the one click the
///     E2E run made unlocked every key for the rest of the session. `forward-focus: editor` names WHO
///     to focus once the scope is asked; nothing in the markup did the asking. The door used is a
///     version TRIPWIRE: `slint::private_unstable_api` is #[doc(hidden)] - a slint version bump can
///     break this door; check i-slint-core generated .focus() when raising the pin.
///
/// So the root does the asking, from the head of the chain - the same call the generated .focus()
/// makes, with FocusReason::Programmatic, which is the reason the walk is allowed to start at the
/// WindowItem and land on the first item that accepts.
///
/// VISIBLE is the condition, not a nicety, and it is why this returns a verdict instead of a shrug:
/// the key dispatcher itself drops a focus item whose item is not visible (window.rs:1076-1081, "Reset
/// the focus... not great, but better than keeping it"), and the tree has no geometry until winit has
/// pumped once - so focus landed on an item that is not visible is WORSE than none at all, because the
/// user's first keystroke eats it and still finds no chord. A `false` here therefore means "ask again
/// on the next wake" (the caller's budget, `FOCUS_TRIES`), and `true` means STOP ASKING FOR GOOD:
/// set_focus_item redirects to the open popup's window when one exists (window.rs:1232-1239), so a
/// root that kept re-asserting focus could park it in somebody's menu. It lands once, then it keeps
/// its hands off - which is also why this does not contradict main.slint:178: that rule is about
/// stealing focus to ROUTE a key, and the markup now names this fn as its one exception. This takes
/// the focus exactly once, before any key was routed or any caret existed to lose.
fn claim_focus(window: &slint::Window) -> bool {
    let inner = WindowInner::from_pub(window);
    let held = || {
        inner
            .focus_item
            .borrow()
            .upgrade()
            .is_some_and(|item| item.is_visible())
    };
    if held() {
        return true;
    }
    let Some(root) = inner.window_item_rc() else {
        return false;
    };
    inner.set_focus_item(&root, true, FocusReason::Programmatic);
    held()
}

fn main() {
    // STRIP-4a row 3: install FIRST, so a panic in the port's own startup is caught by it too.
    // The default hook still runs afterwards (that is what keeps std's report and a NONZERO exit
    // code), and nothing here calls process::exit itself: aborting on a panic would take the
    // trace line with it.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // The payload, not Display: reading it as a str-or-String is what works on every
        // toolchain this crate may be built with (PanicHookInfo::message is the newer, nicer
        // door, and rust-version here does not promise it). A payload that is neither is
        // still named, because the point of the line is THAT it happened, not the prose.
        let what = info
            .payload()
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| info.payload().downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "(unprintable panic payload)".to_string());
        let where_ = info
            .location()
            .map(|at| format!("{}:{}", at.file(), at.line()));
        report(&panic_note(&what, where_.as_deref()));
        default_hook(info);
    }));

    // STRIP-4 debt reckoning, row 3: the hook was CODE-PAID AND PROOF-OWED, and this is the door
    // the proof reads. tests/panic_hook.rs spawns THIS binary with the variable set and asserts the
    // line the hook above really voices. ONE order only, and BEFORE the port exists, so the provoked
    // run touches no state dir and never asks for a window. There is deliberately NO mid-loop
    // variant: an act timed inside the pump races the event loop, which is a flaky timer wearing a
    // proof's name - and a proof that sometimes fails proves nothing at all.
    if std::env::var_os("NOTES_PANIC_PROBE").is_some() {
        panic!("NOTES_PANIC_PROBE=startup: provoked before the port was asked for a snapshot");
    }

    // ---- whitepaper 5.5, in order: query, place, show, handle, register, pin ----
    let dir = state_dir();
    report(&format!("startup: state dir {}", dir.0.display()));
    let (gateway, events) = Gateway::start(dir.clone(), Settings::default());
    // Receiver is not Clone and both this frame and the timer need it; an Rc is the cheap answer
    // and &Rc<Receiver<_>> still coerces to the &Receiver<_> that drain takes.
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

    let Ok(ui) = Spike::new() else {
        report("startup: no component");
        return;
    };
    let window = ui.window();

    // THE LEGEND, RENDERED. menu.rs draws it on the first frame for the same reason, and a
    // windows_subsystem binary cannot print a user interface: with no menu bar drawn on Windows,
    // the status line is the only place a person learns that Ctrl+O and Alt+1 exist. The property
    // is set BEFORE show, so no frame can render without it; the trace still carries it, because a
    // needle is evidence for smoke as well as for a person reading a log.
    let words = legend();
    ui.set_status(words.clone().into());
    report(&format!("chord: legend {words}"));
    // The untitled arm is an act, not an absence: name it before anything is visible, so the
    // markup default is never seen by a user or a test.
    publish_title(&ui, None, false, "startup");

    let scale = if session.scale_factor.is_finite() && session.scale_factor > 0.0 {
        session.scale_factor
    } else {
        1.0
    };
    // FRAME pixels in, LOGICAL CLIENT pixels out. The division is the bridge's own; the
    // frame-to-client step is the thing neither bridge can do before a window exists (AGENTS.md
    // puts geometry RESTORE in the bridge and geometry STORAGE in core). A constant Y offset that
    // repeats on every relaunch is the first bridge's bar-height bug, so the port's own numbers
    // are printed rather than trusted.
    report(&format!(
        "startup: {}",
        plumbing::port_said(
            &session.rect,
            session.scale_factor,
            session.maximized,
            session.pinned
        )
    ));
    let want = LogicalSize::new(session.rect.w as f32 / scale, session.rect.h as f32 / scale);
    let at = LogicalPosition::new(session.rect.x as f32 / scale, session.rect.y as f32 / scale);
    // B-S4, and THE ORDER IS THE REQUIREMENT, not a style: the floor goes up BEFORE the restored
    // size is asked for, never after. Raised first, winit clamps inner_size against min_inner_size
    // while the window is being created (winit window.rs:1267-1272), so wake 1 measures the
    // ALREADY-FLOORED rect, books it as the baseline (see wake_says), and an idle launch sends
    // nothing at all. Raised after, the snap-up is a genuine rect change that reaches the
    // GEOMETRY_QUIET window, emits Command::GeometryChanged, and lets the engine persist the
    // FLOORED rect - a saved 120x120 turned into 340x262 after one run, which is the exact
    // destruction 4b92cef2 exists to prevent. The floor test asserts both calls stay above this
    // line rather than trusting the comment that says so.
    ui.set_floor_width(FLOOR_WIDTH);
    ui.set_floor_height(FLOOR_HEIGHT);
    window.set_size(want);
    window.set_position(at);
    if session.maximized {
        window.set_maximized(true);
        report(&format!(
            "startup: the port said maximised, is_maximized() = {}",
            window.is_maximized()
        ));
    }
    // show() answers with a Result, not a bool: the OS can refuse to map a window, and the
    // reason is the news - so it is printed, not folded into "refused".
    if let Err(e) = ui.show() {
        report(&format!(
            "startup: show() failed: {e}: nothing further to do"
        ));
        return;
    }

    // The drop target's lease, declared BEFORE the registration that arms it so it outlives that
    // arm, and held by the ROOT: it is a resource, not a measurement, so it does not live inside
    // Pump - the unit tests build Pump::default() on any thread and a DropGuard would make that
    // struct window-owned.
    let drop_guard: Rc<RefCell<Option<DropGuard>>> = Rc::new(RefCell::new(None));
    match hwnd_of(window) {
        Some(hwnd) => {
            send(
                &gateway,
                Command::RegisterWindow {
                    handle: WindowHandle(hwnd),
                },
            );
            // send is a post, not a call: RegisterWindow answers with no Event, and its one failure
            // is already printed by send itself. So the moment the handle is named is the moment the
            // arm is right. arm_drop_target reports WHOSE target it took - the toolkit's, or
            // nobody's - because "it returned Ok" is not an answer a user can act on.
            arm_drop_target(hwnd, &drop_guard);
            if initial.pinned {
                send(&gateway, Command::SetPinned(true));
            }
        }
        None => report("startup: NO HWND after show: the window contract fails here"),
    }

    // FOCUS-AT-STARTUP, the first ask (what it is for: see `claim_focus`). It usually MISSES here -
    // there is no geometry before winit's first pump, so there is no visible item to land on - and
    // lands on a wake a few milliseconds later. What must not happen is the window coming up holding
    // no focus item at all, because that is the user's first keystroke going into a void. The Cell is
    // the budget, and zero means it already landed: the wake stops asking on that same zero, so this
    // root never re-asserts a focus the person (or a menu) has since moved somewhere else.
    let focus_budget = Rc::new(Cell::new(FOCUS_TRIES));
    if claim_focus(window) {
        focus_budget.set(0);
        report("focus: taken before the loop - a chord is live from the first keystroke");
    } else {
        report(&format!(
            "focus: no focus item yet, the wakes will keep asking ({FOCUS_TRIES} tries)"
        ));
    }

    // C1, STRIP-5 (2026-09-15): ASK FOR THE DOCUMENT THE SESSION WAS HOLDING. The session was read
    // for its rect, its scale, its maximised bit and its pin - and never for its `path`, which is why
    // a relaunch showed an empty editor while the draft's bytes sat in <state>/notes/untitled.notes.
    // One command, from the surface fn that owns the send (the root-law rule in this file's header:
    // no `send()` here); the answer is the `Loaded` arm in drain that already exists. An ask before
    // RegisterWindow is fine - the engine queues, and gpui sends its own STEP 5 pre-loop.
    if let Some(path) = session.path.clone() {
        restore_from_session(&gateway, &path);
    } else {
        report("startup: the session named no document, so nothing is asked for");
    }

    let pump = Rc::new(RefCell::new(Pump::default()));
    // AUTOSAVE FROM THE STORED STATE, in both directions. The probe OVERRIDES this bit to true
    // because autosave is what that spike was watching; a product reads it, draws the menu check
    // from it, and lets only the user's Ctrl+T (routed by surface::route_of) change it. Nothing is
    // sent here, which is the point.
    pump.borrow_mut().autosave = initial.autosave_enabled;
    report(&format!(
        "autosave: from stored state = {}",
        initial.autosave_enabled
    ));
    note_dot(&pump, &ui.as_weak(), false, "start");

    // STEP B, dated 2026-09-15: the toolkit's asks get their ONE hook call, here, after the window
    // exists and after Pump holds the stored autosave bit - both of which the handlers read. Before
    // this line the product root had no handler for open-asked / save-as-asked / autosave-asked /
    // open-at-index / quit-asked at all, so every chord and every menu row fell on the floor:
    // main.slint fired the callback into a Rust side that had never registered one.
    // The mailbox is NOT decoration - ask_dialog answers THROUGH it even when SLINT_NO_DIALOG makes
    // it skip the modal, so an Open/SaveAs ask with no reader would be a silent dead end. One
    // try_recv per wake in the tick below, never an await, exactly the probe's rule.
    let (dialog_tx, dialog_rx) = mpsc::channel::<DialogReply>();
    let dialog_rx = Rc::new(RefCell::new(dialog_rx));
    wire_callbacks(&ui, &gateway, &pump, &dialog_tx);
    report(
        "wiring: menu rows, chords and the title-band drag hooked (open, save-as, autosave, recent, quit, drag)",
    );

    // ---- the loop: a WAKE, not a clock ----
    // 8 ms is a poll interval, and the probe's 8 ms tick is a SCHEDULE: twenty-four *_AT consts, an
    // END that hides the window at 28 s, and acts that open files nobody asked for. This has none
    // of those. Nothing here fires ON TIME: drain fires ON DATA (an empty drain is a try_recv that
    // misses) and text_pump acts ON A DIFFERENCE (it compares the buffer and does nothing when
    // nothing changed). That is the whole difference between a probe and a product, in 12 lines.
    let tick_gw = Rc::clone(&gateway);
    let tick_events = Rc::clone(&events);
    let tick_pump = Rc::clone(&pump);
    let tick_drops = Rc::clone(&drop_guard);
    let tick_register = Rc::new(RefCell::new(Register::default()));
    let tick_dialog_rx = Rc::clone(&dialog_rx);
    let tick_focus = Rc::clone(&focus_budget);
    let tick_settle = Rc::new(RefCell::new(Settle::default()));
    // B-S5: one refusal sentence per unfit episode, not one per wake. The reader that uses it is in
    // the geometry block below, beside the mirrors it reads.
    let tick_about_said = Rc::new(Cell::new(false));
    // B-S3a: the size THIS root asked the window for, kept only so the first baseline can be
    // compared against the request it is supposed to answer. Copy, and read-only from here on.
    let tick_want = want;
    let weak = ui.as_weak();
    let tick = Timer::default();
    tick.start(TimerMode::Repeated, Duration::from_millis(8), move || {
        let Some(ui) = weak.upgrade() else { return };
        // THE HANDLE ARRIVES LATE, and this is the line that proves it: the attempt above printed
        // "NO HWND after show", which is ALSO what the probe prints at startup - the probe carries
        // the answer into a later tick and registers there. Duplicating that retry here would be
        // the second copy the law forbids, so what the retry needs is said plainly instead:
        // winit materialises the platform window the first time the toolkit actually pumps events,
        // so the root asks once per wake until it has a name for it, and then never again (the
        // guard being held is the flag). If the handle never appears, no drag lands and no pin
        // applies, and the startup print above is the evidence - which is the contract failing
        // loudly rather than quietly, the only acceptable form of a risk.
        // HYGIENE (P1b fix 1): THE STORM, latched. The guard used to be
        // `tick_drops.borrow().is_none()` alone, and plumbing's arm leaves that Option EMPTY WHEN
        // IT REFUSES (plumbing.rs:255-262) - so a handle the platform would not take was
        // re-registered, re-armed and re-printed on every 8 ms wake: ~125 tries a second, forever.
        // The latch says NO MORE after a success (the ordinary ending) or after the same handle
        // has been refused twice, and the second refusal is where the one warn line goes.
        let latched = tick_register.borrow().done;
        if !latched && tick_drops.borrow().is_none() {
            if let Some(hwnd) = hwnd_of(ui.window()) {
                if tick_register.borrow().last.is_none() {
                    report(&format!(
                        "hwnd = {hwnd:#x} on the first wake that could read it"
                    ));
                }
                send(
                    &tick_gw,
                    Command::RegisterWindow {
                        handle: WindowHandle(hwnd),
                    },
                );
                arm_drop_target(hwnd, &tick_drops);
                // The lease is what plumbing can leave behind; still-empty is its "no".
                let armed = tick_drops.borrow().is_some();
                let next = register_says(*tick_register.borrow(), hwnd, armed);
                *tick_register.borrow_mut() = next;
                if next.done && !armed {
                    report(&format!(
                        "hwnd: registration refused for {hwnd:#x} {} times - NOT asking again this run; no drag lands and no pin applies until a handle does take the arm",
                        next.tries
                    ));
                }
            }
        }
        // FOCUS-AT-STARTUP, the retry, on the same wakes that carry the late HWND: while the budget
        // is unspent, ask once more; the moment the claim lands, spend it for good. The ceiling is the
        // difference between this and the registration storm the last commit retired - a wake that
        // cannot claim costs ONE comparison, and after ~2 s it costs that not even a print.
        if tick_focus.get() > 0 {
            if claim_focus(ui.window()) {
                tick_focus.set(0);
                report("focus: the chord scope holds a VISIBLE focus item - keys are live without a click");
            } else {
                let left = tick_focus.get().saturating_sub(1);
                tick_focus.set(left);
                if left == 0 {
                    report(&format!(
                        "focus: STILL no visible focus item after {FOCUS_TRIES} wakes - every chord is dead until something focuses the window"
                    ));
                }
            }
        }
        // STRIP-4a row 2: measure, then ask the pure decision. Two things are deliberately NOT
        // done here: no rect is sent on the FIRST frame (nothing changed, and a send the port
        // treats as a move would let a restore be re-stored with the toolkit's rounding drift),
        // and no cap on how many sends one session may make.
        {
            let now = Instant::now();
            let print = fingerprint_of(ui.window());
            let measured = print.rect;
            // HYGIENE (P1b fix 2): MINIMISED IS NOT A PLACE. Windows parks the window at
            // -32000,-32000 at 160x28 (plumbing.rs:186-194 measures it), so a rect read while
            // minimised is a fact about the OS's parking lot, not about where a person put the
            // note - and the port measures the LIVE window when GeometryChanged lands, so telling
            // it about a park is how a park gets persisted. plumbing's Fingerprint already
            // carries the bit; this root only ever read `.rect`. Skipping while minimised means
            // the park is never seen, never compared and never told, and the pending episode
            // fires on the wake after a restore, on the real rect.
            let parked = print.minimized;
            // C4: THE CORNER POLICY, on the same read and deliberately NOT on the debounce
            // below. A shape change is not a rect the port must persist quietly 250 ms later;
            // it is the thing the user just did, and Win11's own decorations answer in the
            // same frame. One comparison per wake, and `ask_corners` swallows every repeat,
            // so this is the safety net for every way a maximisation happens that this root
            // did not initiate: Win+Up, a snap layout, dragging the note to a screen edge,
            // the caption button, or a restored maximised session.
            //
            // FIX-A (D1): THE SAME TRUTH THE REGISTRATION ABOVE IS GATED ON, THREADED - the
            // startup law's hazard applied one line further down. `Register::last` is set only
            // inside the `if let Some(hwnd)` two statements after the `Command::RegisterWindow`
            // send, so `last.is_some()` IS the sentence "the port holds a handle for this
            // window", and the gateway is FIFO: on the wake that registers, that send is
            // already in the channel ahead of the corner ask made here. Before that wake the
            // engine holds no handle, and its corner arm is `if let Some(handle) = self.window`
            // with NO else and NO refusal event (crates/api/src/engine.rs:838-852) - the ask
            // vanished, while this wake latched the shape and printed "corners: round". A
            // square note for the session behind a log line claiming otherwise, unrecoverable,
            // because the dedupe then refused every later ask for that shape. Silence is the
            // fix, and it costs nothing: the wake after the handle lands asks for real.
            let wired = tick_register.borrow().last.is_some();
            ask_corners(&tick_gw, &tick_pump, !print.maximized, parked, wired);
            let mut st = tick_settle.borrow_mut();
            // B-S3a: the comparison is the pure `wake_says` now, and the difference it makes is
            // one branch. A wake that the watch has never seen a rect for is a BASELINE - it seeds
            // `seen` and leaves `changed_at` alone, which is the state the send below cannot fire
            // from. Everything else about this arm is what it was: same four numbers compared, same
            // `changed_at = Some(now)` on a real move, and the park still never seen, never
            // compared, never told.
            let wake = wake_says(st.seen, measured);
            if !parked {
                // The divergence is SPOKEN. A baseline that does not match what the restore asked
                // for is a clamp of some kind talking - today the toolkit's own minimum and the
                // frame-to-client arithmetic, tomorrow a floor this root set - and a run log that
                // shows only the surviving rect cannot tell "restored faithfully" from "restored
                // then overruled". One line, on the wake that seeds, never again this run.
                if wake == Wake::Baseline && floored_says(tick_want, measured) {
                    report(&format!(
                        "geometry: floored {:.0}x{:.0} -> {}x{} (Slint's own window.size(), NOT an OS frame measurement - the window on screen can be smaller; not the size the restore asked for)",
                        tick_want.width, tick_want.height, measured.w, measured.h
                    ));
                }
                st.seen = Some(measured);
                if arms_episode(wake) {
                    st.changed_at = Some(now);
                }
            }
            if let Some(changed) = st.changed_at.filter(|_| !parked) {
                let moved_for = now.saturating_duration_since(changed);
                let since_send = st.sent_at.map(|at| now.saturating_duration_since(at));
                if settle_says(moved_for, since_send) {
                    send(&tick_gw, Command::GeometryChanged);
                    st.sent_at = Some(now);
                    st.changed_at = None;
                    report(&format!(
                        "geometry: told the port after {moved_for:?} quiet (forced: {})",
                        since_send.is_some_and(|ago| ago >= GEOMETRY_FORCE)
                    ));
                }
            }
            // B-S5: THE MIRRORS' FIRST READER. chrome.slint now gates the ACT - row 5's press and
            // the about-asks door both ask about-fits before they raise the panel - and a gate that
            // changes nothing on screen leaves the person holding a row that did nothing. Somebody
            // has to say why, and it cannot be the write that did not happen. So this reads BOTH
            // sides: Chrome's own width flag, through the out mirror B-S2 opened for exactly this
            // (the first reader that mirror has ever had), and the rect this wake measured; then it
            // says the reason once per unfit episode, on the status line the pin refusal already
            // uses (surface.rs "pin refused: {reason}") and in the log where the numbers live.
            //
            // THE ASYMMETRY, STATED NOT HIDDEN: a refused POINTER press leaves no trace Rust can
            // see - the bit simply never changes, and a row that declined emits nothing - so the
            // sentence is keyed to the state a person is looking at (popup open, row on screen,
            // host too small for what the row offers) rather than to the press itself. The Rust-side
            // door gets the same words for the same reason. Nothing here reads a 340/260 literal out
            // of markup: the two need_* numbers are the floor's own consts, which
            // the_floor_contract... proves sit at or above the panel's parsed demand.
            let (need_w, need_h) = (FLOOR_WIDTH as u32, FLOOR_HEIGHT as u32);
            let unfit = measured.w < need_w || measured.h < need_h || ui.get_about_overflow();
            if unfit && ui.get_menu_shown() && !tick_about_said.get() {
                tick_about_said.set(true);
                report(&format!(
                    "about: declined - the licence box needs {}x{} of host, this window measured {}x{}, and Chrome's own width flag says overflow={}",
                    need_w, need_h, measured.w, measured.h, ui.get_about_overflow()
                ));
                ui.set_status("about: this window is too small to hold the licence".into());
            }
            if !unfit {
                tick_about_said.set(false);
            }
        }
        drain(&tick_events, &tick_pump, &ui.as_weak());
        text_pump(&ui, &tick_gw, &tick_pump);
        // A6: THE DOOR, the only timed send in the bridge. Nothing is owed on most wakes, and
        // retry_take says so without touching the gateway.
        retry_door(&tick_gw, &tick_pump);
        // The picker's answer, if a person finished choosing. Read here and nowhere else, so no
        // callback on the loop ever waits for a modal (probe.rs:722-728 in shape, minus its act
        // prints): while the dialog thread is still blocked, this try_recv simply misses.
        {
            let rx = tick_dialog_rx.borrow();
            while let Ok(reply) = rx.try_recv() {
                answer_dialog(reply, &tick_gw, &tick_pump, &ui);
            }
        }
    });

    // CLOSE IS QUIT. The probe holds the window on the first request on purpose, because it is
    // measuring what a second request costs; that rule must not follow the UI into a product. A
    // person pressing Alt+F4 once means close, so the first request is granted. (The honest
    // shutdown - final flush, join, name the buffer - is STRIP-4 item 2 above; what this closure
    // guarantees is that ONE request ends the run rather than two.)
    let closing = Rc::clone(&pump);
    window.on_close_requested(move || {
        closing.borrow_mut().closes += 1;
        report("close: requested #1, granted (a product closes the first time)");
        CloseRequestResponse::HideWindow
    });

    report("startup: entering the loop; nothing in it fires on a schedule");
    if let Err(e) = ui.run() {
        report(&format!("event loop: {e}"));
    }
    // The guard drops here, and dropping it is what disarms the target. Printed, because a lease
    // that ends quietly is how dragging onto the window stops working between two runs and nobody
    // notices until a user files it.
    // STRIP-4a row 1: THE HONEST SHUTDOWN. WHERE it runs is the finding, not a detail. The brief
    // asked for the sequence on granted close, inside the callback; the port documents why that
    // exact place is a measured deadlock - Gateway::close warns "never call it from the engine
    // thread", and the note above its bounded join records the hang it replaced: "the thread
    // calling close() from the window's own close callback is the one thread that owner needs in
    // order to pump" (api/src/gateway.rs:444-458). So the close callback only COUNTS and GRANTS,
    // and everything below runs after ui.run() returned: same sequence, minus the hang.
    //
    // ORDER, because the order is what makes the join mean something: (1) kill the wake timer and
    // with it every clone its closure held, (2) one last compare of buffer against what was sent,
    // (3) wait BOUNDED for the Saved that answers it, (4) UnregisterWindow so the engine stops
    // measuring a window that is gone, (5) disarm the drop lease, (6) close() - Shutdown plus the
    // port's own bounded join - and answer with a trace line and an exit code, NEVER with a panic.
    drop(tick);
    report(&format!(
        "shutdown: gateway clones still alive after the timer died: {} (1 = the wake closure is really gone)",
        Rc::strong_count(&gateway)
    ));
    // HYGIENE (P1b fix 3): THE AUTOSAVE-OFF TAX. Both counters are read BEFORE the last flush
    // goes out, and the wait now exits on EITHER of them moving: `saves` is a save that landed,
    // `saves_settled` is ANY terminal answer (Saved / AutosaveSkipped / SaveFailed). Watching
    // only the first guaranteed the whole 2.0 s to a session with autosave OFF, because that
    // session's answer is AutosaveSkipped and a Saved is never coming. The line says which.
    let saves_at_entry = pump.borrow().saves;
    let settled_at_entry = pump.borrow().saves_settled;
    text_pump(&ui, &gateway, &pump);
    drain(&events, &pump, &ui.as_weak());
    let dirty_at_exit = pump.borrow().dirty;
    let asked = Instant::now();
    while dirty_at_exit
        && pump.borrow().saves == saves_at_entry
        && pump.borrow().saves_settled == settled_at_entry
        && asked.elapsed() < SAVE_WAIT
    {
        std::thread::sleep(Duration::from_millis(10));
        text_pump(&ui, &gateway, &pump);
        drain(&events, &pump, &ui.as_weak());
    }
    let verdict = {
        let p = pump.borrow();
        flush_verdict(
            p.saves != saves_at_entry,
            p.saves_settled != settled_at_entry,
            p.saves_answer,
            dirty_at_exit,
        )
    };
    report(&format!(
        "shutdown: the last flush {verdict} (dirty at exit: {}, waited {:?})",
        dirty_at_exit,
        asked.elapsed()
    ));
    send(&gateway, Command::UnregisterWindow);
    let held = drop_guard.borrow_mut().take().is_some();
    report(&format!(
        "drop: disarmed on exit (a guard was held: {held})"
    ));
    match gateway.borrow_mut().take() {
        None => report("shutdown: no gateway left to close - something already took it"),
        Some(handle) => match handle.close() {
            // The ordinary ending, and the only one that may say nothing alarming: every accepted
            // command, the final session write included, was carried out before this returned.
            Ok(()) => report("shutdown: joined cleanly, the session write ran"),
            // No reader left and the thread joined: an engine that had already exited on its own
            // terms. Silent about ONE thing only - whether this call stopped a save (it did not,
            // there was nothing to stop) - and NOT a claim that a save happened.
            Err(Exit::QueueClosed) => {
                report("shutdown: the queue had no reader; the thread joined")
            }
            Err(Exit::Abandoned(waited)) => {
                // A verdict about the DEADLINE, not about the engine: a Flush queued behind
                // Shutdown does real work (temp write, fsync, rename) and on slow storage that
                // outruns the port's own join budget while the engine is perfectly healthy.
                // So wait again, bounded, for the port's terminal signal - the channel closing -
                // exactly as the first bridge does, and if THAT expires, leave nonzero: a silent
                // exit in the middle of a possible save is the one outcome this app must not have.
                report(&format!(
                    "shutdown: the engine was still working after {waited:?} - waiting again, bounded, for the port to close its channel",
                ));
                let since = Instant::now();
                let mut gone = false;
                while since.elapsed() < EXIT_REWAIT {
                    if let Err(mpsc::RecvTimeoutError::Disconnected) =
                        events.recv_timeout(Duration::from_millis(50))
                    {
                        gone = true;
                        break;
                    }
                }
                report(&format!(
                    "shutdown: {} after {:?}",
                    if gone {
                        "the engine went away"
                    } else {
                        "THE ENGINE NEVER WENT AWAY - the newest edit may not be on disk"
                    },
                    since.elapsed()
                ));
                if !gone {
                    std::process::exit(1);
                }
            }
            Err(Exit::Panicked) => {
                // The case this whole path was rewritten for: an engine that unwound drops the
                // queue and joins INSTANTLY, which is observably identical to a clean exit unless
                // the thread's own ending is consulted. Nonzero, and worded so no reader can
                // mistake it for "your note was saved".
                report(
                    "shutdown: THE ENGINE PANICKED on the way out - nothing after this line can promise the bytes landed",
                );
                std::process::exit(1);
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panic_note_names_a_panic_the_payload_and_the_place() {
        // Row 3's testable half, and the ONLY half this ships: a trigger would mean writing a
        // deliberate panic into a product root, which is not a thing to leave in a binary that
        // people run. The hook itself is three lines and is checked by reading it.
        assert_eq!(
            panic_note("index out of bounds", Some("product.rs:7")),
            "panic: index out of bounds at product.rs:7"
        );
        assert_eq!(panic_note("gone", None), "panic: gone");
        // The word that makes it findable in a merged log, and the prefix the smoke contract
        // owns - the note goes out through report(), never through its own writer.
        assert!(panic_note("x", None).starts_with("panic:"));
    }

    // STRIP-4 debt reckoning, row 3's OTHER half. The hook is a closure a unit test cannot call,
    // so what a unit test CAN do is forbid the order from drifting. The live proof - a child that
    // really panicked and really was heard - is tests/panic_hook.rs, which exists because this file
    // used to say out loud that no live panic was ever provoked.
    #[test]
    fn the_panic_hook_is_installed_before_the_port_and_chains_the_default() {
        let whole = include_str!("product.rs");
        let installed = whole
            .find("std::panic::take_hook()")
            .expect("the hook takes ownership of the default before replacing it");
        let set = whole
            .find("std::panic::set_hook(")
            .expect("and only then installs its own");
        let port = whole
            .find("Gateway::start(")
            .expect("the port is started somewhere in this root");
        assert!(
            installed < set && set < port,
            "order drifted: a hook installed after Gateway::start at {port} leaves a panic inside the port startup unvoiced"
        );
        // The window the hook owns: its install up to the port. Three things live inside it.
        let between = &whole[installed..port];
        assert!(
            between.contains("default_hook(info)"),
            "the hook stopped chaining std's default hook, and that call is what keeps the panicked-at report, the location and the nonzero exit alive beside our line"
        );
        assert!(
            !between.contains("process::exit"),
            "an exit between taking the hook and starting the port ends the run and takes the trace line with it - main owns no exit here, only the shutdown arms do"
        );
        // The gate the child test drives sits in that window, behind its variable, so a shipped
        // binary can never panic at startup for a person who did not order it.
        assert!(
            between.contains("var_os(\"NOTES_PANIC_PROBE\")")
                && between.contains("panic!(\"NOTES_PANIC_PROBE"),
            "the startup probe left the hook's window or lost its gate - tests/panic_hook.rs reads this exact pair"
        );
    }

    #[test]
    fn a_payload_that_prints_as_nothing_is_still_named_as_a_panic() {
        // The third arm of the hook's downcast ladder: a payload that is neither a String nor a
        // &str - a u32, a struct, anything that panics with a VALUE - carries no prose, and the line
        // says so in words of this file's own. Before this test that fallback had NO owner: rename
        // it inside the closure and nothing anywhere went red, which is how a panic hook ends up
        // printing "panic: " and nothing else.
        const UNPRINTABLE: &str = "(unprintable panic payload)";
        assert_eq!(
            panic_note(UNPRINTABLE, None),
            "panic: (unprintable panic payload)",
            "the word panic is the only thing this case carries, so it may not be optional"
        );
        assert!(
            include_str!("product.rs").contains(UNPRINTABLE),
            "the hook's fallback no longer matches the string this test pins - the wording is drifting in one place only, and the smoke contract reads the other"
        );
    }

    #[test]
    fn a_drag_that_never_stops_still_tells_the_port() {
        // Row 2's pure decision: quiet ends an episode, force bounds a drag, and NOTHING caps the
        // count - which is the probe's rule explicitly not inherited. If these numbers drift from
        // gpui's, the two bridges stop agreeing about when a rect is official.
        assert!(!settle_says(Duration::from_millis(200), None));
        assert!(settle_says(GEOMETRY_QUIET, None));
        assert!(!settle_says(
            Duration::from_millis(50),
            Some(Duration::from_millis(500))
        ));
        assert!(
            settle_says(Duration::from_millis(50), Some(GEOMETRY_FORCE)),
            "a continuous drag must send once per FORCE interval forever"
        );
        assert_eq!(
            (GEOMETRY_QUIET, GEOMETRY_FORCE),
            (Duration::from_millis(250), Duration::from_millis(1000))
        );
    }

    #[test]
    fn an_idle_first_wake_seeds_the_watch_and_queues_nothing() {
        // B-S3a, and the only reason the comparison had to leave the wake: the first rect this
        // root ever reads is the one the RESTORE put on the window, not one a hand moved. Seeded
        // with Settle::default() (seen: None), the old four-number inequality answered "different"
        // against NOTHING, changed_at opened on wake 1, and since settle_says answers true as soon
        // as the quiet reaches GEOMETRY_QUIET, every single launch sent Command::GeometryChanged
        // with no user input whatsoever. GeometryChanged is a bare trigger - the engine measures
        // the LIVE window and persists what it sees - so session.json was rewritten on every
        // start, and under a window floor that stops being noise and becomes a lie with a number
        // in it: 120x120 saved, 340x264 stored, forever, one launch at a time.
        let placed = Rect::new(40, 60, 800, 600);
        assert_eq!(
            wake_says(None, placed),
            Wake::Baseline,
            "a first reading has nothing to differ from, so it cannot be a change"
        );
        // The gate the send actually sits behind, reachable without a window: no episode opens on
        // a baseline, so there is no changed_at for settle_says to be asked about and NOTHING can
        // be queued - not after 250 ms, not after an hour idle.
        assert!(
            !arms_episode(Wake::Baseline),
            "an idle first wake must queue nothing"
        );
        assert!(
            arms_episode(Wake::Moved),
            "a real move still arms the episode"
        );
        assert!(!arms_episode(Wake::Same), "a stable window never re-times");
        // Same-vs-Moved, on all four numbers, exactly as the inline comparison had it.
        assert_eq!(wake_says(Some(placed), placed), Wake::Same);
        for other in [
            Rect::new(41, 60, 800, 600),
            Rect::new(40, 61, 800, 600),
            Rect::new(40, 60, 801, 600),
            Rect::new(40, 60, 800, 601),
        ] {
            assert_eq!(
                wake_says(Some(placed), other),
                Wake::Moved,
                "a {other:?} after {placed:?} is a change and must still arm"
            );
        }
        // And the clamp's voice, which is what the baseline buys the right to report: smaller
        // speaks, equal is silent, and LARGER is silent too - a maximised session comes back far
        // bigger than the stored rect and that is a restore working, not a floor.
        let want = LogicalSize::new(800.0, 600.0);
        assert!(
            floored_says(want, Rect::new(40, 60, 340, 264)),
            "came back below what the restore asked for: say it"
        );
        assert!(!floored_says(want, placed));
        assert!(!floored_says(want, Rect::new(40, 60, 1920, 1040)));
        // And the direction B-S4 creates for real: a 120x120 seed that comes back AT the floor
        // must speak, because that is the run where session.json and the window disagree. Want
        // already above the floor, given a work area, stays silent - a maximised restore is not a
        // clamp, and the second assertion is the one that keeps it that way.
        assert!(
            floored_says(LogicalSize::new(120.0, 120.0), Rect::new(40, 60, 340, 262)),
            "a rect raised to the floor is the divergence this line exists to voice"
        );
        assert!(
            !floored_says(LogicalSize::new(800.0, 600.0), Rect::new(0, 0, 1920, 1080)),
            "a maximised restore is not a floor"
        );
        // Half a pixel is the scale division rounding, not a divergence.
        assert!(!floored_says(LogicalSize::new(799.6, 600.0), placed));
    }

    #[test]
    fn the_register_latch_closes_on_success_and_on_the_second_same_refusal() {
        // P1b fix 1's latch, pure: a success ends it on the first wake; the SAME handle refused
        // twice ends it too (the storm was one refusal retried ~125 times a second); a NEW handle
        // is a different window and owes its own budget, so the give-up line can fire once only.
        let armed = register_says(Register::default(), 0x10, true);
        assert!(armed.done && armed.tries == 1, "a success is not retried");
        let first = register_says(Register::default(), 0x20, false);
        assert!(
            !first.done,
            "one refusal is the late-HWND race, so one retry is owed"
        );
        let second = register_says(first, 0x20, false);
        assert!(
            second.done && second.tries == 2,
            "the same refusal twice closes the latch"
        );
        let fresh = register_says(second, 0x30, false);
        assert!(
            !fresh.done && fresh.tries == 1,
            "a new handle is a new window, not a repeat"
        );
    }

    #[test]
    fn an_answered_flush_ends_the_close_wait_even_when_nothing_saved() {
        // P1b fix 3's verdict, pure: the states a reader must not confuse. Autosave OFF answers
        // AutosaveSkipped and no Saved ever comes - that used to be a guaranteed 2.0 s per close.
        assert!(flush_verdict(true, true, "Saved", true).starts_with("landed"));
        assert!(
            flush_verdict(false, true, "AutosaveSkipped", true)
                .contains("ANSWERED with AutosaveSkipped"),
            "skipped must read as an answer, not as a loss"
        );
        assert!(
            flush_verdict(false, true, "SaveFailed", true).contains("ANSWERED with SaveFailed")
        );
        assert!(flush_verdict(false, false, "", false).contains("was not needed"));
        assert!(
            flush_verdict(false, false, "", true).contains("DID NOT land"),
            "the silent case keeps its alarming words - that is the one that can cost bytes"
        );
    }

    // ---- WS-B B-S2: the About mirrors and the inert floor knobs --------------------------
    //
    // WHY THESE SIT HERE AND NOT IN plumbing.rs, in the right order after review. PRIMARY, and it
    // is a placement rule rather than a counting rule: what a floor and a mirror assert is a
    // PRODUCT claim, and ADR-0006 §4 donates exactly such claims to Leg::Product to be re-earned
    // there (0006:70-72) - so the product's own mod tests is their home whoever compiles them.
    // SECONDARY belt, not the reason: four modules (plumbing, surface, title_contract, ui_gen)
    // compile into BOTH bins, so a test written there would also run inside notes-slint-probe.exe.
    // What that clause actually protects is narrower than this comment used to claim - it forbids
    // the instrument gaining new things it REPORTS after its verdict, not a module gaining tests -
    // and it is the reporting that stays frozen here: nothing below prints into a run, and the
    // probe's needles are untouched.
    //
    // Both needles read MARKUP TEXT, never a property at runtime: unit tests here get no window,
    // and the whole crate's guard style is "say it in text, count it in text". The two
    // include_str! lines below are the only way to reach ../ui from a test, and the squeeze makes
    // each needle survive a re-indent, because a comment that reflows must not read as drift.

    /// Whitespace out of a line of markup, so a binding is matched on what it SAYS and not on how
    /// far the formatter happened to indent it.
    fn squeezed(text: &str) -> String {
        text.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// The number in the first `<prefix><digits>px` at or after `from`, read OUT of the markup.
    /// These literals are frozen evidence (chrome.slint:1001-1002) and copying them into a Rust
    /// literal is how a number ends up owned twice - so no panel size below is typed by hand.
    /// A missing or unparseable literal panics: that is the drift this is here to catch.
    fn px_after(text: &str, from: usize, prefix: &str) -> f64 {
        let hit = text[from..].find(prefix).unwrap_or_else(|| {
            panic!("the markup no longer contains \"{prefix}\" after offset {from}")
        });
        let start = from + hit + prefix.len();
        let digits: String = text[start..]
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        digits
            .parse::<f64>()
            .unwrap_or_else(|err| panic!("\"{prefix}\" no longer parses as a length: {err}"))
    }

    /// The value a const DECLARATION gives `name`, read from the RIGHT of its equals sign. Two
    /// review findings are the shape of this function. (1) Scanning forward from the NAME reads a
    /// digit run out of the TYPE token: `const FLOOR_WIDTH: f32 = 340.0;` yielded 32, so the exact
    /// spelling this guard exists to check went red the day a floor was named correctly. (2) A bare
    /// substring match arms the comparison against any number in prose that merely mentions the
    /// name, so the line must really be a `const <name>` declaration and not a comment about one.
    /// None means "no such declaration yet", which is the pending branch. A declaration whose value
    /// is not a numeric literal PANICS rather than passing: a floor this guard cannot read is a
    /// floor nobody read.
    fn const_number(head: &str, name: &str) -> Option<f64> {
        let line = head.lines().find(|l| {
            let t = l.trim();
            if t.starts_with("//") || t.starts_with("///") {
                return false;
            }
            match t.find("const ") {
                Some(at) => t[at + "const ".len()..].trim_start().starts_with(name),
                None => false,
            }
        })?;
        let (_, right) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("{name} is declared with no value to read"));
        let run: String = right
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        assert!(
            !run.is_empty(),
            "{name} is declared as something other than a numeric literal, and this guard can only              enforce what it can read - name the number or move the contract to the value's owner"
        );
        match run.parse::<f64>() {
            Ok(value) => Some(value),
            Err(err) => panic!("{name}'s value {run:?} does not parse: {err}"),
        }
    }

    /// Every Rust file the crate ships, read at run time. A census that hand-lists its inputs stops
    /// covering anything new the day somebody adds a file, and the setter census below has to see a
    /// call from ANYWHERE in the crate - product, probe, plumbing, or a test written next month.
    fn rust_files(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(path);
            }
        }
    }

    /// THE CONTRACT, as a direction rather than a number: a floor clips the licence when - and
    /// only when - it sits BELOW what the panel demands. Cut as ordering on purpose. Equality
    /// (`floor == demanded`) would go red when a later slice trimmed a harmless pixel off the
    /// floor while the panel stayed whole, and a guard that punishes a safe change trains people
    /// to edit the guard.
    fn clips_the_panel(floor: f64, demanded: f64) -> bool {
        floor < demanded
    }

    #[test]
    fn abouts_three_placement_flags_are_out_mirrors_and_chrome_still_derives_them() {
        let main = squeezed(include_str!("../ui/main.slint"));
        let chrome = include_str!("../ui/chrome.slint");

        // Needle 1a: the mirrors exist, and Rust cannot reach an element inside a component by id,
        // so a binding is the ONLY door that has ever worked here. about-shown's door is pre-existing
        // and had better still be there; the three flags below are the ones this slice opened, and
        // about-raised is the third because it is the VERTICAL axis - the one floor-height acts on.
        for mirror in [
            "inproperty<bool>about-shown:chrome.about-open;",
            "outproperty<bool>about-floored:chrome.about-floored;",
            "outproperty<bool>about-overflow:chrome.about-overflow;",
            "outproperty<bool>about-raised:chrome.about-raised;",
        ] {
            assert!(
                main.contains(mirror),
                "the mount lost a one-way About mirror: {mirror}"
            );
        }
        // And the visibility is the half that makes "one-way" mean something, so it gets its own
        // assertion rather than hiding inside the one above. 1.17 emits get_x for every public
        // property and set_x for every property that is NOT read_only, and read_only comes only from
        // declaring a property out (i-slint-compiler generator/rust.rs:1062-1090). As `in`, these
        // three compiled ui.set_about_floored(..) and friends - a mirror Rust can overwrite is not a
        // mirror, it is a second writer with a getter on it. `out` deletes the setter from the
        // generated API, which is the difference between a claim and a hope.
        for was_in in [
            "inproperty<bool>about-floored",
            "inproperty<bool>about-overflow",
            "inproperty<bool>about-raised",
        ] {
            assert!(
                !squeezed(include_str!("../ui/main.slint")).contains(was_in),
                "{was_in} is back to IN, which re-opens ui.set_... on a read-only mirror"
            );
        }
        // A mirror must not become a lever from the markup side either: the mount binds Chrome's
        // outputs and writes none of them. probe.rs's census covers Chrome's own writes and the
        // about-open assign form; this covers the three NEW names, which had no other guard.
        for assigned in ["about-floored =", "about-overflow =", "about-raised ="] {
            assert!(
                !include_str!("../ui/main.slint").contains(assigned),
                "main.slint now ASSIGNS {assigned}, which would make the mount a second writer"
            );
        }

        // Needle 1b: the About bindings inside Chrome are untouched, read as squeezed text so a
        // rewrap of Chrome's prose cannot look like a changed predicate.
        let chrome_squeezed = squeezed(chrome);
        assert!(
            chrome_squeezed.contains(
                "inproperty<bool>about-overflow:root.host-width>0px&&root.host-width<about.width;"
            ),
            "about-overflow no longer reads root.host-width < about.width"
        );
        assert!(
            chrome_squeezed.contains("inproperty<bool>about-floored:root.host-width>0px&&root.popup-left(about.width)<8px;"),
            "about-floored no longer reads popup-left(about.width) < 8px"
        );
        assert!(
            chrome_squeezed.contains("inproperty<bool>about-raised:root.host-height>0px&&root.popup-top(about.height)<Theme.bar-height;"),
            "about-raised, the vertical twin, also had better still read the same thing"
        );
    }

    /// B-S5: the licence act is GATED on the panel's own two demand terms, the gate WRAPS the two
    /// existing writes instead of adding a third surface, and the refusal SPEAKS through a reader of
    /// the mirrors B-S2 opened. Four separate claims, because four separate things could rot.
    #[test]
    fn the_about_act_is_gated_on_the_panels_own_terms_and_the_decline_speaks() {
        let chrome = include_str!("../ui/chrome.slint");
        let chrome_squeezed = squeezed(chrome);
        let whole = include_str!("product.rs");
        let head = &whole[..whole
            .find("mod tests")
            .expect("product.rs lost the mod tests block this guard reads")];

        // 1. THE GATE ITSELF, read as ORDERING on the panel's own numbers - never as equality, and
        // never as a copy of them. The width term is about.width with no padding; the height term is
        // about.height plus the ONE menu-gap popup-top keeps below it; the border is not a term
        // because a Slint Rectangle draws it centered on the edge, so the box already includes it.
        // The leading host-width <= 0px clause is the unknown-host case: it FITS, because a gate
        // that refuses on its own missing measurement is worse than no gate.
        assert!(
            chrome_squeezed.contains("property<bool>about-fits:root.host-width<=0px||(root.host-width>=about.width&&root.host-height>=about.height+Theme.menu-gap);"),
            "about-fits no longer compares the host against about.width and about.height plus one Theme.menu-gap with >=: the gate was replaced by a literal, or turned into an equality"
        );
        // squeezed() eats newlines too, so the binding has to be cut out by its own delimiters
        // rather than read as a line: from the declaration's colon to the semicolon that ends it.
        let predicate = chrome_squeezed
            .split_once("property<bool>about-fits:")
            .map(|(_, rest)| rest.split(';').next().unwrap_or(""))
            .expect("about-fits is gone from chrome.slint; nothing gates the About act");
        for restated in ["340", "260", "262"] {
            assert!(
                !predicate.contains(restated),
                "the gate now carries the literal {restated}: a copied demand drifts the day the panel is resized, which is the whole reason the flags were mirrored"
            );
        }

        // 2. WRAPPED, NOT ADDED. probe.rs counts LINES CONTAINING the open-write substring and pins
        // the total, so a third open - or an else branch that writes it, or a comment that spells it
        // - goes red over there. This is the local half of the same claim: exactly two sites raise
        // the panel, both are inside the gate on the SAME line, and neither has an else.
        let opens: Vec<&str> = chrome
            .lines()
            .filter(|line| line.contains("about-open = true"))
            .collect();
        assert_eq!(
            opens.len(),
            2,
            "the licence is now opened from {} places, and the census in probe.rs expects the two              it counts (the about-asks door and row 5)",
            opens.len()
        );
        for line in &opens {
            assert!(
                line.contains("if root.about-fits {") && !line.contains("else"),
                "an open site is not wrapped in the gate on its own line: {line}"
            );
        }

        // 3. THE READ IS REAL, not a wish. about-overflow is the flag that says the panel is wider
        // than its host, and until this slice NOTHING in the workspace read the mirrored trio -
        // which is why B-S2's out-mirroring needed a customer to be honest about. The head, not the
        // whole file, so this test's own prose cannot arm it.
        for door in ["ui.get_about_overflow(", "ui.get_menu_shown("] {
            assert!(
                head.contains(door),
                "the About mirror {door} has no reader again, and the gate's reason cannot be said"
            );
        }
        assert!(
            head.contains("\"about: declined"),
            "the refusal stopped saying why: the log sentence is gone from the shipping code"
        );
        assert!(
            head.contains("let (need_w, need_h) = (FLOOR_WIDTH as u32, FLOOR_HEIGHT as u32);"),
            "the decline message no longer quotes the floor's own consts - it has started copying              the demand a third time"
        );
        // 4. AND IT IS STILL ORDERING on the Rust side too: a host is refused for being SMALLER than
        // the demand, not for differing from it by a pixel of antialiasing.
        assert!(
            head.contains("measured.w < need_w || measured.h < need_h"),
            "the unfit test stopped being an ordering comparison, so it would refuse a host that              fits or accept one that does not"
        );

        // 5. THE OTHER HALF OF THIS COMMIT: the floored trace line must name where its number came
        // from. Measured four times on this machine: it printed "-> 340x262" from Slint's own
        // window.size() while GetWindowRect said the window was 120x120 for the whole run. A line
        // that reports one source in the grammar of the other is a lie on the screen, so the source
        // is now IN the string and this needle keeps it there.
        let floored = head
            .split_once("geometry: floored ")
            .map(|(_, rest)| rest.split('\n').next().unwrap_or(""))
            .expect("the floored trace line is gone from the shipping code entirely");
        assert!(
            floored.contains("window.size()") && floored.contains("NOT an OS"),
            "the geometry: floored line stopped naming its source ({floored}); it must say the              number is Slint's bookkeeping, because the OS frame can be smaller"
        );
    }

    #[test]
    fn the_floor_contract_is_ordering_and_this_slice_sets_no_floor_value() {
        let main = squeezed(include_str!("../ui/main.slint"));

        // The knobs, and the 0px that makes them inert. slint-core builds the window's min
        // constraint as Some ONLY when a min exceeds zero, so 0px on both axes never reaches
        // set_min_inner_size - which is why notes-slint-probe.exe, which mounts this same Spike
        // and never sets a knob, still measures a genuine 180px host.
        for knob in [
            "inproperty<length>floor-width:0px;",
            "inproperty<length>floor-height:0px;",
            "min-width:root.floor-width;",
            "min-height:root.floor-height;",
        ] {
            assert!(
                main.contains(knob),
                "the floor door changed shape: {knob} is gone from the markup"
            );
        }

        // The demand, read out of the frozen markup rather than written down twice - and the
        // DESIGNER'S RULING on what it is, since that ruling is the only reason a guard like this
        // earns its keep. Read the two clauses separately, because they are not the same question.
        //
        //   * VERTICAL = about.height + exactly ONE Theme.menu-gap. popup-top subtracts one gap
        //     below the panel (chrome.slint:198-202) and nothing else. NO border term, and that is
        //     the correction: a Slint Rectangle draws its border CENTERED ON the geometry edge
        //     (i-slint-core graphics/border_radius.rs:195-199, the inner/outer(half_border_width)
        //     pair; item_rendering.rs:523,544 clip children by that same border_width INSIDE the
        //     item box), so the box's `height: 260px` is already border-inclusive. Counting the
        //     hair again was a demand made of half a pixel of antialiasing.
        //   * HORIZONTAL = about.width with no padding at all, and the asymmetry between the two
        //     clauses is the ruling, not an oversight: popup-left's floor is ZERO
        //     (chrome.slint:230-234), so the 8px side margins are POLICY while the panel's own
        //     width is OBLIGATION - below it no x places this panel at all, which is exactly what
        //     about-overflow says (host-width < about.width, margins spent and all).
        let chrome = include_str!("../ui/chrome.slint");
        let about = chrome
            .find("about := Rectangle {")
            .expect("chrome.slint has an About panel block");
        let panel_w = px_after(chrome, about, "width: ");
        let panel_h = px_after(chrome, about, "height: ");
        let border = px_after(chrome, about, "border-width: ");
        let gap = px_after(include_str!("../ui/theme.slint"), 0, "menu-gap: ");
        let demand_w = panel_w;
        let demand_h = panel_h + gap;

        // The contract, tested as a DIRECTION at the boundary rather than against a value: equal
        // clears, a hair under clips, more than enough never clips. The third case is what makes
        // this ordering and not equality - an == predicate fails it.
        assert!(
            !clips_the_panel(demand_w, demand_w),
            "a floor AT the demand clears it"
        );
        assert!(
            clips_the_panel(demand_w - 0.5, demand_w),
            "a hair below the demand must read as clip-capable"
        );
        assert!(
            !clips_the_panel(demand_h + 100.0, demand_h),
            "a generous floor is never an error - equality would say otherwise"
        );
        assert!(
            demand_w > 0.0 && demand_h > panel_h,
            "the padding has to actually pad, or the vertical clause is the width clause in a costume"
        );
        // The ONE place this test wants equality, because it is about the ARITHMETIC and not about
        // a clip: the demand is the box plus one gap, and the border it reads here (a real number,
        // deliberately not used) is drawn on the edge rather than beyond it. Re-add it as a term
        // and this line is the one that says so.
        assert_eq!(
            demand_h,
            panel_h + gap,
            "the vertical demand grew a term - the border ({border}px) is centered on the box's own edge and is not extra"
        );
        assert_eq!(
            demand_w, panel_w,
            "the horizontal demand grew a term - popup-left's floor is zero, so its margins are policy while only the width is obligation"
        );

        // B-S4: the door is now OPEN on purpose, so the census flipped from "no caller anywhere" to
        // "exactly ONE caller, in ONE place, in the RIGHT order". Three things it still refuses, each
        // for a reason that outlives this slice: a second caller (a floor set from two places is a
        // floor nobody owns), a caller in another file (the probe has its own main and must never
        // raise a floor, or its frozen 180px arm stops measuring a 180px host), and a caller BELOW
        // the size ask - which is the destruction 4b92cef2 exists to prevent, so the ordering is
        // asserted rather than commented. The needles are BUILT at run time because a census that
        // greps its own grep line is the trap this crate already named (plumbing.rs:579).
        let whole = include_str!("product.rs");
        let head = &whole[..whole
            .find("mod tests")
            .expect("this file has a tests module")];
        let ask = head
            .find("window.set_size(want)")
            .expect("the startup still asks for the restored size");
        let mut sources = Vec::new();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        rust_files(&root.join("src"), &mut sources);
        rust_files(&root.join("tests"), &mut sources);
        assert!(
            sources.len() >= 8,
            "the setter census found only {} files under {} - the layout moved and this guard now reads nothing",
            sources.len(),
            root.display()
        );
        for axis in ["width", "height"] {
            let door = format!("set_floor_{}(", axis);
            for file in &sources {
                let text = std::fs::read_to_string(file)
                    .unwrap_or_else(|err| panic!("{} unreadable: {err}", file.display()));
                let sanctioned =
                    file.file_name().and_then(|name| name.to_str()) == Some("product.rs");
                let scope = if sanctioned { head } else { &text[..] };
                assert_eq!(
                    scope.matches(&door).count(),
                    usize::from(sanctioned),
                    "{} raises the floor {} time(s); exactly one sanctioned call lives in this
                     file's startup and nowhere else - the probe root must never gain one",
                    file.display(),
                    scope.matches(&door).count()
                );
            }
            // THE ORDERING, the requirement rather than the style. Below the size ask, the snap-up
            // is a real rect change: it reaches GEOMETRY_QUIET, emits GeometryChanged, and the
            // engine persists the FLOORED rect - a saved 120x120 becomes 340x262 after one run.
            // Above it, winit clamps as the window is created, wake 1 measures the floored rect,
            // books it as the baseline, and an idle launch sends nothing.
            assert!(
                head[..ask].contains(&door),
                "{door} is no longer ABOVE the startup's size ask at offset {ask}: raised after the
                     ask, the clamp reads as a user move and gets persisted",
            );
            // ... and the same door from the markup side, where a handler could bind the knob
            // without a single Rust call existing anywhere.
            let assign = format!("floor-{} =", axis);
            assert!(
                !include_str!("../ui/main.slint").contains(&assign),
                "the markup now ASSIGNS {assign} from inside a handler: a floor bound in markup walks around every guard on this side of the seam",
            );
        }

        // The contract, armed. The day these two consts were named, the pending branch below
        // stopped running and this one started: it reads each value from after its equals sign
        // (never from the type token, which is how the first draft mis-parsed 340.0 as 32) and
        // compares it against the demand parsed out of chrome.slint above. ORDERING, so a floor
        // bigger than the panel is never an error and only a clip-capable one is.
        for (name, demanded) in [("FLOOR_WIDTH", demand_w), ("FLOOR_HEIGHT", demand_h)] {
            match const_number(head, name) {
                Some(value) => assert!(
                    !clips_the_panel(value, demanded),
                    "{name} = {value} sits below the {demanded} the About panel demands - the licence would clip, which is the only thing this contract forbids"
                ),
                None => {
                    // Reachable only by renaming the consts away while the startup still calls both
                    // setters - which the census above has already caught. Kept, because a pending
                    // branch nobody can reach is cheaper than a guard that quietly stops checking.
                    assert!(
                        main.contains("floor-width:0px") && main.contains("floor-height:0px"),
                        "{name} is no longer declared here, and the markup's 0px default is the only floor left - which means the knob is set from somewhere this guard does not read"
                    );
                }
            }
        }
    }

    #[test]
    fn an_owed_retry_decides_a_save_and_the_door_is_called_after_the_drain() {
        // A6, step four: the weak point of the whole slice was a door no test could reach, because a
        // send needs a live port. Splitting the decision off the sending turns "no test at all" into
        // "the decision is tested and exactly one line performs it" - that line is the send(gw, ...)
        // in retry_door, and it is still unverified. Say it plainly: this test does NOT prove a byte
        // reached the channel.
        let owed = Retake::Send {
            text: "typed while the disk said no".to_string(),
            revision: 7,
            epoch: 3,
            next: Duration::from_millis(1500),
            attempt: 4,
        };
        match retry_sends(&owed) {
            Some(Command::Save {
                text,
                revision,
                epoch,
            }) => assert_eq!(
                (text.as_str(), revision, epoch),
                ("typed while the disk said no", 7, 3),
                "a retry re-sends the REFUSED pair - not a fresh reading of the buffer, which is a Flush                  wearing a Save's coat, and not the latest revision either, which the epoch guard would                  discard"
            ),
            Some(Command::Flush { .. }) => panic!("a retry rode the Flush lane, which is the bug"),
            Some(other) => panic!("a retry rode the wrong command: {other:?}"),
            None => panic!("an owed retry decided to send nothing"),
        }
        // And the three answers that must send NOTHING. The last one is the cap: at the end of the
        // ladder a silent door is the correct door, because the next user act carries the text.
        for quiet in [Retake::Idle, Retake::Waiting, Retake::Terminal] {
            assert!(
                retry_sends(&quiet).is_none(),
                "this answer from the record must not send a command"
            );
        }
        // ORDERING, and nothing more: named as such because this crate already has enough source-slice
        // asserts passing for behaviour, and one mistaken for proof is worse than the gap. What these
        // three facts guard is the door's place in the wake - after the drain that FILLS the record,
        // and beside the pump that already sends through this gateway, which is the only reason the
        // door may live in a root that owns a gateway at all.
        // Sliced at this file's own test module, because the strings below appear IN that slice too:
        // an un-sliced grep of this file matched its own needle and reported the door as running
        // after a drain that is literally further down the page. The self-grep trap, caught by the
        // test that exists to catch ordering, which is the one lesson worth paying for.
        let whole = include_str!("product.rs");
        let src = &whole[..whole.find("mod tests").expect("the tests module")];
        let door = src
            .find("retry_door(&tick_gw, &tick_pump);")
            .expect("the door is called in the tick");
        let drain = src
            .find("drain(&tick_events, &tick_pump")
            .expect("the tick drain");
        let pump = src
            .find("text_pump(&ui, &tick_gw, &tick_pump);")
            .expect("the flush pump call");
        let start = src.find("Gateway::start(").expect("the port start");
        assert!(
            door > drain,
            "a door before the drain reads yesterday's record"
        );
        assert!(
            door > pump,
            "and the door belongs beside the pump that holds this gateway"
        );
        assert!(
            door > start,
            "and it is nowhere in the startup band the panic-hook guard slices"
        );
    }
}
