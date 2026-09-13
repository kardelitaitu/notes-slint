// The strip's transitional state, stated rather than silenced: the modules below were written
// for a root that ALSO measured and probed, so a dozen items (the dialog mailbox, the
// fingerprint witness, the pin hold, the legend's own print) are used by the probe and unused
// HERE while STRIP-4 items 1-4 are still owed. Deleting them would break the probe; calling
// them from here would mean writing the acts into the product, which is the opposite of this
// slice. So the allow lives at this root, where the lie is smallest, and the comment names
// what will remove it.
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

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use notes_api::{Command, DropGuard, Gateway, Settings, StateDir, WindowHandle, resolve_state_dir};
use slint::{
    CloseRequestResponse, ComponentHandle, LogicalPosition, LogicalSize, Timer, TimerMode,
};

mod plumbing;
mod surface;
mod title_contract;
mod ui_gen;

use plumbing::{arm_drop_target, hwnd_of, note_dot, publish_title, report, send};
use surface::{Pump, drain, legend, text_pump};
use ui_gen::Spike;

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

fn main() {
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
        if tick_drops.borrow().is_none() {
            if let Some(hwnd) = hwnd_of(ui.window()) {
                report(&format!(
                    "hwnd = {hwnd:#x} on the first wake that could read it"
                ));
                send(
                    &tick_gw,
                    Command::RegisterWindow {
                        handle: WindowHandle(hwnd),
                    },
                );
                arm_drop_target(hwnd, &tick_drops);
            }
        }
        drain(&tick_events, &tick_pump, &ui.as_weak());
        text_pump(&ui, &tick_gw, &tick_pump);
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
    let held = drop_guard.borrow_mut().take().is_some();
    report(&format!(
        "drop: disarmed on exit (a guard was held: {held})"
    ));
    tick.stop();
    // HONEST ABOUT WHAT THIS DOES NOT DO: the tick closure holds a clone of the gateway, so
    // dropping this Rc does NOT shut the engine down - the process exiting does. The probe prints a
    // whole shutdown around Exit::{QueueClosed, Abandoned, Panicked} and flushes the buffer on the
    // way out; that sequence is STRIP-4 item 2, and it is the one place where this root is quieter
    // than the probe for a real reason rather than by omission. Naming it here so the next reader
    // does not mistake a dropped Rc for a joined engine.
}
