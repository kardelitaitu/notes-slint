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
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use notes_api::{
    Command, Event, Exit, Gateway, Rect, Settings, StateDir, WindowHandle, resolve_state_dir,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, Timer, TimerMode};
mod title_contract;

// The UI lives in ui/main.slint, imported rather than inlined: that file is one of
// xtask smoke's freshness roots, so editing the markup behind a built binary makes the
// binary stale in the guard's eyes instead of invisible to it. 1.17 finding 4: the
// `export` line takes NO trailing semicolon, and relying on an implicit re-export of
// the last import is deprecated - so it is spelled.
slint::slint! {
    import { Spike } from "../ui/main.slint";
    export { Spike }
}

/// The same voice as the gpui bridge - see the header for why the name stays.
fn report(why: &str) {
    eprintln!("notes-gpui: {why}");
}

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
/// Print one tick needle per this-many milliseconds, from inside the Timer callback.
const TICK_MS: u64 = 2000;

/// The bridge's declared autosave cadence, the same 750 ms `bridge-gpui` states
/// (its main.rs:122) rather than reading core's - and the same rule with it: a
/// Flush only goes out when the buffer actually changed.
const AUTOSAVE_IDLE: Duration = Duration::from_millis(750);

/// When the probe drives a recent row by itself: after the seed's open, so the list
/// holds more than one file and the click has something to aim at.
const CLICK_AT: Duration = Duration::from_millis(3000);

// R1/R2 act times. The frame act goes first so the maximise is in the state the
// close act then has to carry out of the window, and the two closes are far enough
// apart to see whether the declined one really left the window on screen.
const MAX_AT: Duration = Duration::from_millis(14000);
const CLOSE_AT: Duration = Duration::from_millis(18000);
const CLOSE2_AT: Duration = Duration::from_millis(21000);
const END: Duration = Duration::from_millis(25000);

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
    let third_gw = Rc::clone(&gateway);
    let third_events = Rc::clone(&events);
    let third_pump = Rc::clone(&pump);
    tick.start(TimerMode::Repeated, Duration::from_millis(8), {
        let state = Rc::clone(&state);
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let now = started.elapsed();
            let seen = fingerprint_of(ui.window());
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
                if !p.max_toggled && now >= MAX_AT {
                    p.max_toggled = true;
                    drop(p);
                    toggle_max(&ui.as_weak(), &third_gw, "synthetic");
                } else if now >= CLOSE_AT && p.closes == 0 {
                    drop(p);
                    ui.set_close_arm(1);
                    let allowed = ui.get_close_allowed();
                    report(&format!(
                        "close: request_close #1 returned {allowed} (false = the decline held), visible={}",
                        ui.window().is_visible()
                    ));
                } else if now >= CLOSE2_AT && p.closes == 1 {
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
                do_no_harm(&third_pump);
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
    // R2: THE CLOSE CONTRACT. This is the handler an OS close would land on, and it
    // declines the FIRST request and grants the SECOND, which is the whole shape of
    // act 1 and act 2 in one run.
    {
        let pump = Rc::clone(&pump);
        ui.window().on_close_requested(move || {
            let n = {
                let mut p = pump.borrow_mut();
                p.closes += 1;
                p.closes
            };
            report(&format!("close: requested (#{n})"));
            if n == 1 {
                report("close: declined, held (KeepWindowShown)");
                slint::CloseRequestResponse::KeepWindowShown
            } else {
                report("close: second, exiting (HideWindow)");
                slint::CloseRequestResponse::HideWindow
            }
        });
    }
    // R1: the strip's double-click, the promise the README makes about a frameless
    // title bar. Bound to the same fn the synthetic driver calls.
    {
        let gw = Rc::clone(&gateway);
        let weak = ui.as_weak();
        ui.on_toggle_max(move || toggle_max(&weak, &gw, "double-click"));
    }
    ui.run().ok();
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

/// THE TITLE, from the port's fact and nowhere else. `title_words` is what the strip
/// centre shows; `window_title` is what Alt+Tab, the taskbar preview and a screen
/// reader speak. Both come from `title_contract`, the same two pure functions
/// bridge-gpui calls (its main.rs:851-861), so the wording cannot drift between
/// bridges - and both are PRINTED, because that print is the only honest measurement
/// of this mount a headless run can make: no screenshot was taken.
fn publish_title(ui: &Spike, path: Option<&Path>, loaded: bool, via: &str) {
    let words = title_contract::title_words(path, loaded);
    let title = title_contract::window_title(path, loaded);
    ui.set_title_words(words.clone().into());
    ui.set_os_title(title.clone().into());
    report(&format!("title: {via} words={words:?} os-title={title:?}"));
}

/// FNV-1a over the file's own bytes. A checksum rather than a hash crate on purpose:
/// this bridge may not add a dependency to prove a byte-for-byte claim, and the
/// question is only ever "did anything change at all".
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// LF-normalise an incoming buffer, byte-for-byte the rule `bridge-gpui` states at
/// editor.rs:2026 - CRLF collapses to LF AND a lone CR becomes LF, because core keeps
/// LF internally and the do-no-harm rule restores the file's own ending at the save
/// layer. The bridge never adds a `\r`, and now never leaks one into a Flush.
fn lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
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

/// THE TEXT LOOP. The buffer lives in the bridge and no keystroke crosses the port
/// (architecture §5.5), so what crosses is one debounced Flush: only when the content
/// actually differs from what was last sent, and only after AUTOSAVE_IDLE of quiet.
/// The revision counts those observed changes; the epoch is echoed, never invented.
fn text_pump(ui: &Spike, gw: &Rc<RefCell<Option<Gateway>>>, pump: &RefCell<Pump>) {
    let text = lf(&ui.get_buffer());
    let mut pump = pump.borrow_mut();
    if text == pump.last_sent {
        pump.pending_at = None;
        return;
    }
    let entered = *pump.pending_at.get_or_insert_with(Instant::now);
    if entered.elapsed() < AUTOSAVE_IDLE {
        return;
    }
    pump.edits += 1;
    let (revision, epoch) = (pump.edits, pump.epoch);
    pump.last_sent = text.clone();
    pump.pending_at = None;
    let quiet = entered.elapsed();
    let cr = text.matches('\r').count();
    drop(pump);
    report(&format!(
        "flush: sent {} bytes rev={revision} epoch={epoch} edit-to-flush={quiet:?} CR-in-buffer={cr}",
        text.len()
    ));
    send(
        gw,
        Command::Flush {
            text,
            revision,
            epoch,
        },
    );
}

/// The other half of the seam: read the seeded file again and say whether a single
/// byte moved. Print the flush counter too, because "no spurious Flush" is the
/// mechanism by which it should not have moved.
fn do_no_harm(pump: &RefCell<Pump>) {
    let p = pump.borrow();
    let Some(seed) = p.seed.clone() else {
        report("do-no-harm: no seed file was written, nothing to compare");
        return;
    };
    let bytes = std::fs::read(&seed).unwrap_or_default();
    let after = fnv1a(&bytes);
    report(&format!(
        "do-no-harm: {} bytes fnv={after:#x} (seeded {} bytes fnv={:#x}) hash-equal={} flush-since-open={}",
        bytes.len(),
        p.hash_len,
        p.hash_before,
        u8::from(after == p.hash_before && bytes.len() == p.hash_len),
        p.edits - p.flushes_at_open
    ));
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

/// What the pin pump remembers. Deliberately holds no opinion about the window: the
/// only bool in here that is a FACT is the one an `Event::Pinned` wrote.
#[derive(Default)]
struct Pump {
    /// Set once the strip has asked on its own, so the probe needs no hands.
    asked: bool,
    /// Last state the PORT reported. None until it says so - never guessed.
    confirmed: Option<bool>,
    /// When `Event::Pinned(true)` arrived: platform-verified WS_EX_TOPMOST, so t0.
    applied_at: Option<Instant>,
    /// When the same-bit re-ask went out, and whether it was answered.
    reask_at: Option<Instant>,
    answered_after_reask: bool,
    hold_reported: bool,
    // ---- tick evidence (finding 1) ----
    /// How many `tick` needles have been taken, i.e. how many times slint delivered
    /// this callback since `ui.run()` began.
    ticks: u64,
    /// Bucket of the last printed tick, so the needle is ~2s apart, not every 8ms.
    last_bucket: u64,
    /// Events the pump has ever pulled out of the port's channel, in-run or not.
    seen: u64,
    /// `Saved` count, so the first one (the answer to this probe's own `SaveAs`) can
    /// be told apart from a later one, which only the autosave path could have sent.
    saves: u64,
    // ---- S5 ----
    /// The paths behind the rendered recent rows, in the order the port sent them,
    /// so a row index maps back to the file it names.
    recent_paths: Vec<PathBuf>,
    /// The CRLF file this probe seeded beside the exe, and its bytes on the way IN.
    seed: Option<PathBuf>,
    hash_before: u64,
    hash_len: usize,
    /// `edits` as it stood when the open was asked: if the adoption is honest, this
    /// does not move before the run ends.
    flushes_at_open: u64,
    /// Rows the last `RecentsUpdated` carried, so the needle prints on a CHANGE
    /// rather than on every delivery.
    rows: usize,
    /// The synthetic click: fired once, and its answer is matched by path.
    click_done: bool,
    click_pending: Option<PathBuf>,
    /// How many times the close-requested callback has run. Act 1 declines the first,
    /// Act 2 lets the second through - the count IS the act selector.
    closes: u32,
    /// Has the frame act run yet?
    max_toggled: bool,
    /// Times the pump has run. `drains` climbing while `seen` stays 0 is the proof
    /// that the callback and its 8ms poll are live and the port is simply silent.
    drains: u64,
    /// Has ANYTHING come back from the port yet (the registration answer is the gate
    /// the pin ask waits on).
    answered: bool,
    // ---- the text loop ----
    /// The port's last ANNOUNCED generation, echoed back on every Flush. The bridge
    /// counts nothing here: 0 means nothing has ever been announced, which is the
    /// file-less start this probe has.
    epoch: u64,
    /// Observed edits, i.e. the `revision` a Flush carries (bridge-gpui's `edits`).
    edits: u64,
    /// What went out last, so an unchanged buffer is never re-sent.
    last_sent: String,
    /// When this change first entered the debounce; the stamp a Flush reports.
    pending_at: Option<Instant>,
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

/// Name every drained event, apply the pin to the UI, and keep the last-wins status
/// line: the two needles. `Pinned`/`PinFailed` are the ONLY writers of the rendered
/// pin bit, which is C3 - the click asks, the answer is what the strip shows.
fn drain(events: &Receiver<Event>, pump: &RefCell<Pump>, weak: &slint::Weak<Spike>) {
    pump.borrow_mut().drains += 1;
    let mut batch = Vec::new();
    while let Ok(event) = events.try_recv() {
        batch.push(event);
    }
    for event in &batch {
        {
            let mut pump = pump.borrow_mut();
            pump.seen += 1;
            pump.answered = true;
        }
        report(&format!("event: {}", describe(event)));
        // The synthetic click's verdict: the Loaded that answers it is identified by
        // PATH, not by timing, so a stray load can never be credited to the click.
        if let Event::Loaded { path, epoch, .. } = event {
            let hit = {
                let mut p = pump.borrow_mut();
                if p.click_pending.as_deref() == Some(path.as_path()) {
                    p.click_pending = None;
                    true
                } else {
                    false
                }
            };
            if hit {
                report(&format!(
                    "open-by-recents: epoch={epoch} landed a Loaded for {}",
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                ));
            }
        }
        match event {
            Event::Pinned(on) => {
                let mut pump = pump.borrow_mut();
                if pump.reask_at.is_some() {
                    pump.answered_after_reask = true;
                }
                pump.confirmed = Some(*on);
                if *on {
                    pump.applied_at = Some(Instant::now());
                }
                let applied = *on;
                drop(pump);
                if let Some(ui) = weak.upgrade() {
                    ui.set_pinned(applied);
                    ui.set_status(format!("pinned: {applied} (port-reported)").into());
                }
                report(&format!(
                    "pinned: {} WS_EX_TOPMOST={} readback (platform verified at apply)",
                    applied,
                    i32::from(applied)
                ));
            }
            Event::PinFailed { reason } => {
                let mut pump = pump.borrow_mut();
                if pump.reask_at.is_some() {
                    pump.answered_after_reask = true;
                }
                pump.confirmed = Some(false);
                drop(pump);
                if let Some(ui) = weak.upgrade() {
                    ui.set_pinned(false);
                    ui.set_status(format!("pin refused: {reason}").into());
                }
                report(
                    "pinned: false WS_EX_TOPMOST=0 readback (PinFailed: the apply did not stick)",
                );
            }
            // (b) THE GENERATION, captured from the only two events that issue it.
            // grep of crates/api/src/event.rs: `Loaded { path, text, meta, epoch }`
            // (event.rs:308) and `Rebound { path, meta, revision, epoch }` (:369).
            // No other event carries an epoch, and `Saved { path, revision }` (:334)
            // deliberately does not - a save does not change the generation, so a
            // bridge that read epoch from it would be one bump out of step.
            Event::Loaded {
                epoch, text, path, ..
            } => {
                let adopted = lf(text);
                let mut p = pump.borrow_mut();
                p.epoch = *epoch;
                p.last_sent = adopted.clone();
                drop(p);
                if let Some(ui) = weak.upgrade() {
                    ui.set_buffer(adopted.clone().into());
                    publish_title(&ui, Some(path), true, "loaded");
                }
                report(&format!(
                    "load: epoch={epoch} announced, buffer adopted ({} bytes, CR-normalised)",
                    adopted.len()
                ));
            }
            Event::Rebound { epoch, path, .. } => {
                pump.borrow_mut().epoch = *epoch;
                if let Some(ui) = weak.upgrade() {
                    publish_title(&ui, Some(path), true, "rebound");
                }
                report(&format!(
                    "rebind: epoch={epoch} announced; the next Flush echoes it"
                ));
            }
            Event::RecentsUpdated(list) => {
                let rows = list.len();
                let names: Vec<slint::SharedString> = list
                    .iter()
                    .map(|entry| entry.display.clone().into())
                    .collect();
                {
                    let mut p = pump.borrow_mut();
                    p.recent_paths = list.iter().map(|entry| entry.path.clone()).collect();
                }
                // The label is core's (`display`), the cap is core's, the missing-file
                // mark is core's. The bridge renders and keeps the paths beside it.
                if let Some(ui) = weak.upgrade() {
                    // 1.17 finding: `ModelRc` is built from a slice or an `Rc<dyn Model>` - not
                    // from a `Vec` (only `VecModel` takes a Vec), so the borrowed slice it is.
                    ui.set_recents(slint::ModelRc::from(names.as_slice()));
                }
                // Print on a CHANGE only: three updates used to mean three needles.
                let changed = {
                    let mut p = pump.borrow_mut();
                    let changed = p.rows != rows;
                    p.rows = rows;
                    changed
                };
                if changed {
                    report(&format!("recents: rendered {rows} rows"));
                }
            }
            Event::Saved { path, revision } => {
                let which = {
                    let mut p = pump.borrow_mut();
                    p.saves += 1;
                    p.saves
                };
                if which == 1 {
                    report(&format!("saved: rev={revision} {}", path.display()));
                } else {
                    // No SaveAs in sight: a change entered the debounce, the 750 ms
                    // passed, one Flush went out, and the bytes landed. That is the
                    // autosave path proving itself - the loop the gpui bridge owns on
                    // its side of the seam.
                    report(&format!(
                        "autosave: saved rev={revision} ({}th Saved, no SaveAs between)",
                        which
                    ));
                    report(&format!("autosave: file {}", path.display()));
                }
            }
            other => {
                if let Some(ui) = weak.upgrade() {
                    ui.set_status(describe(other).into());
                }
            }
        }
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
        Event::AutosaveSkipped { reason } => {
            format!("AutosaveSkipped reason={reason:?}")
        }
        other => format!("{other:?}")
            .split(['(', ' ', ':'])
            .next()
            .unwrap_or("unknown")
            .to_string(),
    }
}
