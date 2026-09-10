#![windows_subsystem = "windows"]

//! notes-bridge-gpui - M2 slice 1: the four documented startup steps, DPI-correct.
//!
//! The startup order is fixed (docs/architecture.md 5.5) and numbered in `main`:
//! query the saved session, create the window AT that rect, register the handle
//! with the port, apply topmost. The bridge owns the window; the port never sees a
//! keystroke, and this crate imports nothing in this repo except `notes-api`.
//!
//! Not in this slice, on purpose: the editor widget, the menu, the titlebar chrome
//! and the keymap. What has to be true first is that a window appears where it was
//! last taken down, and that closing the app closes the engine instead of
//! aborting it.
//!
//! # The unit rule at this boundary
//!
//! `Session.rect` is FRAME pixels (Win32 `GetWindowRect` space) while GPUI bounds
//! are logical client-area pixels, so crossing is two steps: divide by the scale
//! the session was saved at (done in `bounds_for`), and subtract the non-client
//! chrome (not done - `bounds_for` says what is missing and why). Client
//! coordinates are never persisted, which is what stops the per-launch 8/19/8/20
//! drift M0 recorded as D1/D9.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyWindowHandle, App, Application, Bounds, Context, IntoElement, Point, Render, Subscription,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, px, rgb, size,
};
use notes_api::{
    Command, EventRx, Gateway, InitialState, Settings, StateDir, WindowHandle, resolve_state_dir,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// The window root view for this slice: an empty surface filling the window.
/// The editor arrives in slice 2; none of the startup order depends on it.
struct Surface;

impl Render for Surface {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(rgb(0x1f1f1f))
    }
}

fn main() {
    // STEP 1 - query the saved session. `Gateway::start` reads session.json once,
    // on this thread; `startup_state` hands over that snapshot and is consume-once,
    // so it is read here and nowhere else.
    let (mut gateway, events) = Gateway::start(state_dir(), Settings::default());
    let Some(initial) = gateway.startup_state() else {
        // Only None if the snapshot had already been consumed, which one call site
        // cannot do. If it ever can: close is the defined exit, where dropping the
        // Gateway would be an abort with no word said.
        let _ = gateway.close();
        return;
    };

    // The Gateway rides in an Option so that exactly one owner can take it and call
    // the blocking `close`. Both candidate exits (last window closed, loop quit)
    // share it and `take` makes the second one a no-op. It is never reachable from
    // engine code, which is what makes joining in it safe.
    let gateway = Rc::new(RefCell::new(Some(gateway)));
    let events = Rc::new(RefCell::new(events));
    // A dropped Subscription unsubscribes, so the close handler needs somewhere to
    // live for the whole loop rather than for one call.
    let subscriptions = Rc::new(RefCell::new(Vec::<Subscription>::new()));

    Application::new().run({
        let gateway = Rc::clone(&gateway);
        let events = Rc::clone(&events);
        let subscriptions = Rc::clone(&subscriptions);
        move |cx: &mut App| {
            // STEP 2 - create the window AT the saved rect, before anything is
            // drawn. Only the bridge can: the port has no window type at all.
            let options = WindowOptions {
                window_bounds: Some(bounds_for(&initial)),
                // The title is the cheapest proof that the running binary is this
                // build: it carries the package version, which nothing else on
                // screen shows yet (no menu, no status line).
                titlebar: Some(TitlebarOptions {
                    title: Some(format!("notes {}", env!("CARGO_PKG_VERSION")).into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let opened = cx.open_window(options, |_, cx| cx.new(|_| Surface));
            let Ok(handle) = opened else {
                // No window means no UI to run: close the port - drain, final
                // session write, join - instead of letting Drop abort it.
                close(&gateway);
                cx.quit();
                return;
            };

            // STEP 3 - register the window handle with the port. The HWND crosses
            // as an i64 because the port must not know a platform type exists.
            let any: AnyWindowHandle = handle.into();
            match any
                .update(cx, |_view, window, _cx| hwnd_of(window))
                .ok()
                .flatten()
            {
                Some(hwnd) => send(
                    &gateway,
                    Command::RegisterWindow {
                        handle: WindowHandle(hwnd),
                    },
                ),
                None => report("GPUI gave no Win32 window handle: RegisterWindow was not sent"),
            }

            // STEP 4 - apply topmost. The bit came from session.json in step 1, so
            // a pinned note is pinned as it appears rather than 200 ms later.
            //
            // What step 4 WILL do, once the port lands the join: the engine will take
            // the handle registered in step 3 and call notes-platform
            // (`WindowBackend::set_topmost(handle, on)`) on the window it is pinned
            // to, so this same command both stores the bit and raises the real HWND -
            // today it only stores the bit, which is why a pinned session opens
            // unpinned. That join is api work over a handle api already holds; this
            // bridge will not import notes-platform to do it first.
            if initial.pinned {
                send(&gateway, Command::SetPinned(true));
            }

            // The only channel drain in this slice: once, on the thread that owns
            // the receiver, with try_recv, and never inside a frame.
            drain(&events);

            // A quit through the close door: `on_window_closed` fires when the last
            // window goes, and `close` joins the engine, so the final session write
            // has completed before this process leaves main.
            let held = Rc::clone(&subscriptions);
            let closing = Rc::clone(&gateway);
            held.borrow_mut()
                .push(cx.on_window_closed(move |_cx| close(&closing)));
        }
    });

    // And the door that does not come through a window close.
    close(&gateway);
    drain(&events);
}

/// Where the window goes, in GPUI logical client pixels.
///
/// `session.scale_factor` is the scale of the monitor the rect was saved on, so it
/// is the only divisor that can be honest here, and it is applied per component.
/// The chrome step is genuinely missing: GPUI wants client bounds, the session
/// stores frame bounds, and the difference (about 8/19/8/20 at 100%) is not
/// knowable before a window exists. So the window opens one title bar and border
/// low-right of where it was left - visible, constant, and never written back, so
/// it does not accumulate. Fixing it needs a frame-to-client conversion the port
/// does not carry yet (see the report).
fn bounds_for(initial: &InitialState) -> WindowBounds {
    let session = &initial.session;
    let scale = if session.scale_factor.is_finite() && session.scale_factor > 0.0 {
        session.scale_factor
    } else {
        // A garbage scale in a hand-edited session.json must not send the window to
        // the origin or shrink it away: 1.0 is the assumption the file was written
        // under.
        1.0
    };
    let logical = |v: i32| px(v as f32 / scale);
    let extent = |v: u32| px(v as f32 / scale);
    let bounds = Bounds {
        origin: Point {
            x: logical(session.rect.x),
            y: logical(session.rect.y),
        },
        size: size(extent(session.rect.w), extent(session.rect.h)),
    };
    if session.maximized {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    }
}

/// The HWND of a live GPUI window, read out of the toolkit with
/// `raw_window_handle` - the accessor the toolkit itself implements, not a
/// window-title hunt through Win32.
fn hwnd_of(window: &Window) -> Option<i64> {
    // The fully-qualified call is load-bearing: `Window` also has an INHERENT
    // `window_handle()` that returns GPUI`s own AnyWindowHandle, and an inherent
    // method always wins over a trait method, so `window.window_handle()` here
    // silently compiles against the wrong thing.
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return None;
    };
    match handle.as_raw() {
        RawWindowHandle::Win32(win32) => Some(win32.hwnd.get() as i64),
        _ => None,
    }
}

fn send(gateway: &Rc<RefCell<Option<Gateway>>>, command: Command) {
    let borrowed = gateway.borrow();
    let Some(gateway) = borrowed.as_ref() else {
        report("the port was already closed, so a command was dropped");
        return;
    };
    // An Err means the engine is gone and no Event will ever describe this
    // command. Silence is the failure mode AGENTS.md forbids.
    if gateway.send(command).is_err() {
        report("the engine had already exited: a command came back undelivered");
    }
}

fn close(gateway: &Rc<RefCell<Option<Gateway>>>) {
    if let Some(gateway) = gateway.borrow_mut().take() {
        // Blocking by contract: drain, final session write, join. Legal here
        // because this is the UI thread on its way out, not a frame, and not the
        // engine thread.
        if gateway.close().is_err() {
            report("Shutdown was queued behind an engine that had already stopped");
        }
    }
}

/// Non-blocking, on the thread that owns the receiver. Nothing consumes Events
/// usefully yet - there is no editor to tell about a failed save - so this drains
/// what already arrived rather than leaving it queued and pretending otherwise.
fn drain(events: &Rc<RefCell<EventRx>>) {
    let rx = events.borrow();
    while let Ok(event) = rx.try_recv() {
        let _ = event;
    }
}

fn report(why: &str) {
    // `windows_subsystem` means there is no console attached, so this is the
    // last-resort trace until the port grows an Event for an undelivered command.
    eprintln!("notes-gpui: {why}");
}

/// Where app state lives. The D-STATE rule is notes-core's and the port now
/// re-exports it as `resolve_state_dir`, so the duplicated installed-half-of-the-
/// rule that used to sit here is gone: this function keeps only the two things
/// the re-export documents as the CALLER'S job - the existence probe (a `data`
/// directory beside the executable means a portable deployment, and for that one
/// APPDATA must be passed as None, because an installed app must not be redirected
/// by a one-word launcher change) and reading the roaming profile it actually has,
/// never a path assembled from a home directory.
fn state_dir() -> StateDir {
    // No exe path means no probe and no portable marker: fall back to the current
    // directory rather than inventing a profile. It cannot happen from a shipped
    // binary, and `resolve_state_dir` is pure, so nothing is written either way.
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
