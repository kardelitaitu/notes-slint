//! SLINT SPIKE, slice 1 - THE WINDOW CONTRACT, and nothing else.
//!
//! One question, with a printed number as its answer: can a Slint window be (a)
//! placed at the rect `session.json` holds BEFORE it is ever visible, (b) shown
//! maximised when the file says so, and (c) yield an HWND to the port without a
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
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use notes_api::{
    Command, Event, Gateway, Rect, Settings, StateDir, WindowHandle, resolve_state_dir,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, Timer, TimerMode};

slint::slint! {
    export component Spike inherits Window {
        title: "notes-slint spike (window contract)";
        in property <string> status: "starting";
        Text {
            text: root.status;
            x: 10px;
            y: 10px;
        }
    }
}

/// The same voice as the gpui bridge - see the header for why the name stays.
fn report(why: &str) {
    eprintln!("notes-gpui: {why}");
}

/// Portable-first, so both binaries read the SAME session.json: a `data` directory
/// beside the exe wins, which is `target/debug/data` for a dev build.
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

fn json_flag(src: &str, key: &str) -> Option<bool> {
    let at = src.find(&format!("\"{key}\""))?;
    Some(src[at..at + 40].contains("true"))
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
            let zoom = json_flag(&src, "maximized").unwrap_or(false);
            format!("frame rect (port-measured) t+{at:?}: {rect} maximized={zoom}")
        }
        Err(err) => format!("frame rect t+{at:?}: no session.json yet ({err})"),
    }
}

/// The HWND. Slint's own `window_handle()` is infallible and returns ITS handle
/// object; the raw-window-handle question is the INNER call, which is a `Result`.
/// That inner answer is what risk 3 is about, so it is reported separately.
fn hwnd_of(window: &slint::Window) -> Option<i64> {
    // Two steps, and the first must be a binding: the outer handle owns what the inner
    // one borrows.
    let outer = window.window_handle();
    let handle = outer.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(win32) => Some(win32.hwnd.get() as i64),
        _ => None,
    }
}

/// Rect plus show state - the same two halves the gpui bridge diffs, because a
/// maximise can leave the rect alone. Both reads are already PHYSICAL here
/// (`position() -> PhysicalPosition`, `size() -> PhysicalSize`, i-slint-core
/// api.rs:562 / :576), so unlike the restore path there is no scale to apply.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Fingerprint {
    rect: Rect,
    maximized: bool,
}

fn fingerprint_of(window: &slint::Window) -> Fingerprint {
    let position = window.position();
    let size = window.size();
    Fingerprint {
        rect: Rect::new(position.x, position.y, size.width, size.height),
        maximized: window.is_maximized(),
    }
}

/// The quiet window before a change is reported, in the spirit of the gpui bridge's
/// debounce. Shortened so the probe fits inside its own lifetime.
const QUIET: Duration = Duration::from_millis(1200);

/// When the probe prints its second reading, and when it ends.
const SAMPLE: Duration = Duration::from_millis(300);
const END: Duration = Duration::from_millis(3000);

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
    report(&format!(
        "startup: session.json says {}x{} at {},{} scale {} maximized={} pinned={}",
        session.rect.w,
        session.rect.h,
        session.rect.x,
        session.rect.y,
        session.scale_factor,
        session.maximized,
        session.pinned
    ));

    let Ok(ui) = Spike::new() else {
        report("startup: no component");
        return;
    };
    let window = ui.window();

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
    let started = Instant::now();
    let hwnd = hwnd_of(window);
    match hwnd {
        Some(hwnd) => {
            report(&format!(
                "hwnd = {hwnd:#x} FIRST VISIBLE {}",
                measured(&dir, started.elapsed())
            ));
            send(
                &gateway,
                Command::RegisterWindow {
                    handle: WindowHandle(hwnd),
                },
            );
            if initial.pinned {
                send(&gateway, Command::SetPinned(true));
            }
        }
        None => report("NO HWND after show: risk 3 CONFIRMED, the contract fails here"),
    }
    drain(&events);

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
    let third_gw = Rc::clone(&gateway);
    let third_events = Rc::clone(&events);
    tick.start(TimerMode::Repeated, Duration::from_millis(8), {
        let state = Rc::clone(&state);
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let now = started.elapsed();
            let seen = fingerprint_of(ui.window());
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
                    drain(&third_events);
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
            if now >= END {
                report(&format!("probe over {}", measured(&third_dir, now)));
                ui.window().hide().ok();
            }
        }
    });
    ui.run().ok();
    drain(&events);
    report(&format!("exit {}", measured(&dir, started.elapsed())));
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

fn send(gateway: &Rc<RefCell<Option<Gateway>>>, command: Command) {
    let borrowed = gateway.borrow();
    let Some(gateway) = borrowed.as_ref() else {
        return;
    };
    if gateway.send(command).is_err() {
        report("the engine had already exited: a command came back undelivered");
    }
}

/// Name every drained event, and keep the last-wins status line: the two needles.
fn drain(events: &Receiver<Event>) {
    let mut batch = Vec::new();
    while let Ok(event) = events.try_recv() {
        batch.push(event);
    }
    for event in &batch {
        report(&format!("event: {}", describe(event)));
    }
    if let Some(event) = batch.last() {
        report(&format!("status line: {}", describe(event)));
    }
}

/// The event's name, plus its rect when it has one.
fn describe(event: &Event) -> String {
    match event {
        Event::GeometryNotRestored { rect, .. } => {
            format!(
                "GeometryNotRestored {}x{} at {},{}",
                rect.w, rect.h, rect.x, rect.y
            )
        }
        other => format!("{other:?}")
            .split(['(', ' ', ':'])
            .next()
            .unwrap_or("unknown")
            .to_string(),
    }
}
