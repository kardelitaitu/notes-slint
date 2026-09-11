#![windows_subsystem = "windows"]

//! notes-bridge-gpui - M2 slice 2: the four startup steps, and the event pump.
//!
//! The startup order is fixed (docs/architecture.md 5.5) and numbered in `main`:
//! query the saved session, create the window AT that rect, register the handle
//! with the port, apply topmost. The bridge owns the window; the port never sees a
//! keystroke, and this crate imports nothing in this repo except `notes-api`.
//!
//! What this slice adds: one wake route from the engine thread into the GPUI loop,
//! and an exhaustive render of every `Event` the port can say. Not in this slice,
//! on purpose: the editor widget, the menu, the titlebar chrome, the keymap and the
//! tray - each is its own slice.
//!
//! # The unit rule at this boundary
//!
//! `Session.rect` is FRAME pixels (Win32 `GetWindowRect` space) while GPUI bounds
//! are logical client-area pixels, so crossing is two steps: divide by the scale
//! the session was saved at (done in `bounds_for`), and subtract the non-client
//! chrome (not done - `bounds_for` says what is missing and why). Client
//! coordinates are never persisted, which is what stops the per-launch 8/19/8/20
//! drift M0 recorded as D1/D9.
//!
//! # The wake route, and why GPUI 0.2.2 offers exactly one
//!
//! `EventRx` is a `std::sync::mpsc::Receiver` (crates/api/src/gateway.rs:41) and a
//! std receiver has no waker, so nothing can await it; the sender belongs to the
//! engine thread, which must never learn GPUI exists. GPUI 0.2.2 then only has one
//! legitimate way in, and it is its own dispatcher:
//!
//! * `Context::spawn` (gpui src/app/context.rs:237) delegates to `App::spawn`
//!   (src/app.rs:1417), which hands the future to `ForegroundExecutor::spawn`
//!   (src/executor.rs:471). That executor's schedule function is
//!   `PlatformDispatcher::dispatch_on_main_thread`, so the future is polled on the
//!   loop thread and nothing else can poll it.
//! * `dispatch_on_main_thread` on Windows (src/platform/windows/dispatcher.rs:82)
//!   pushes the runnable onto a flume queue and then `PostMessageW`es
//!   `WM_GPUI_TASK_DISPATCHED_ON_MAIN_THREAD` - which is what pulls the loop out of
//!   `GetMessageW` (src/platform/windows/platform.rs:314) and into
//!   `run_foreground_task`, which drains that queue (platform.rs:748).
//! * `BackgroundExecutor::timer` (src/executor.rs:357) fires on a Win32 thread-pool
//!   timer (`dispatch_after` -> `dispatch_on_threadpool_after`, dispatcher.rs:58 and
//!   :107). Awaiting it from the main-thread Task above therefore means: the timer
//!   completes on a pool thread, and the pool thread wakes the loop by the ONE
//!   sanctioned route. No thread of ours ever touches `App`.
//!
//! And the two routes that look easier and are not: `ForegroundExecutor` is
//! deliberately `!Send` (src/executor.rs:44-49: "This is intentionally `!Send` ...
//! These checks would fail when spawning foreground tasks from background threads")
//! and `AsyncApp` holds one (src/app/async_context.rs:17-21), so a thread cannot
//! carry a way in even if it wanted to; and there is no `append_event` in this
//! version at all (`grep -rn append_event gpui-0.2.2/src` is empty). So the loop
//! POLLS a non-blocking channel on a timer, rather than being pushed into, which is
//! also the only shape that satisfies "never block on a channel inside a GPUI
//! frame".

// S2: the editor model paints and takes focus. It is still NOT wired to the port -
// no Flush, no Loaded text - until S6, so it is a working surface with nothing
// behind it, not a finished M2.
mod editor;
mod menu;

use editor::Editor;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, App, AsyncApp, Bounds, Context, Entity, Focusable, IntoElement, KeyBinding,
    PathPromptOptions, Pixels, Point, Render, SharedString, Subscription, Task, TitlebarOptions,
    WeakEntity, Window, WindowBounds, WindowHandle as GpuiWindowHandle, WindowId, WindowOptions,
    div, px, rgb, size,
};
use notes_api::{
    Command, Encoding, Event, EventRx, FileMeta, Gateway, InitialState, LineEnding, RecentEntry,
    Rect, Settings, SkipReason, StateDir, WindowHandle, resolve_state_dir,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// How long the loop may sit between looks at the port's queue.
///
/// 8 ms is a POLL interval, not a wait, and it is roughly one tenth of a 60 Hz
/// frame's budget per second of uptime. The engine's own cadence is a 750 ms
/// autosave tick (crates/api/src/engine.rs:53), so the pump is far finer than
/// anything the port can produce: an event never visibly waits for this number.
///
/// MEASURED, not assumed: a 15 s idle run counted 936 wakes, i.e. ~16.9 ms per
/// wake in practice, not 8 ms. The Win32 thread-pool timer this rests on
/// (gpui dispatcher.rs:58 `ThreadPoolTimer::CreateTimer`) coalesces onto the OS
/// tick, so the honest number to quote is ~16 ms of worst-case delivery latency.
/// Halving the constant would not halve the latency, so the value is left at a
/// request the OS can round however it likes.
const ENGINE_POLL: Duration = Duration::from_millis(8);

/// Upper bound on the events one wake may take.
///
/// The drain must be bounded work whatever the queue holds: the port enforces the
/// same rule engine-side (crates/api/src/engine.rs, "MAJOR 3: the drain is
/// bounded"), and a previous review of this bridge found a RECURSIVE drain that
/// overflowed the stack at 2000 queued events. So: iterative, capped, and the rest
/// is taken on the next wake. A flood then costs latency, never the frame.
const MAX_DRAIN_PER_WAKE: usize = 64;

/// The bridge-side debounce: how long the buffer must be UNTOUCHED before a `Flush`
/// goes out. D51 puts the decision here, not in the engine, and the engine only ever
/// sees a Flush the bridge decided to send.
///
/// THE SAME NUMBER BY AGREEMENT, NOT BY ACCIDENT: the engine has its own idle of the
/// same length at crates/api/src/engine.rs:63 (`AUTOSAVE_IDLE`), which is PRIVATE and
/// is not re-exported - api only mentions it in a doc comment (crates/api/src/lib.rs:
/// 170). So this bridge declares 750 ms itself, and if core or api ever moves theirs,
/// NOTHING at compile time will say so. The one-line fix is a public constant in api;
/// that is a request, not an edit - 13 Events and 10 Commands are pinned and the port
/// is not mine.
const AUTOSAVE_IDLE: Duration = Duration::from_millis(750);

/// Everything the wire needs to know, in one place, so the rules that keep a user's
/// text safe are auditable rather than spread across the pump.
#[derive(Debug)]
struct Wire {
    /// The editor's mutation counter as last seen. A change here is the ONLY thing
    /// that makes a flush worth considering - a click, a drag, a scroll and a resize
    /// all leave it alone.
    seen_edits: u64,
    /// When it last changed. The debounce measures from here, so a stream of
    /// keystrokes never flushes until the user pauses.
    changed_at: Instant,
    /// The counter of the text that went out last. Equal to `seen_edits` means the
    /// engine has everything the buffer has, which is the cheapest possible answer to
    /// "did anything change": no clone, no compare, one integer.
    flushed_edits: u64,
    /// A `Flush` we sent that no event has answered yet, and its revision. At most one
    /// is ever in the air - see the reasoning on `tick`.
    in_flight: Option<u64>,
    /// Per-document autosave arming, straight from `FileMeta::armed` on `Loaded` and
    /// `Rebound` (ADR-0001). A foreign file the user has never saved is NOT armed, and
    /// the bridge does not decide otherwise on its own.
    armed: bool,
    /// Has the port ever LOADED a document into this buffer? Until it has, what the
    /// editor holds is the session's own scratch note: no file exists to overwrite, and
    /// no `FileMeta` has been sent to obey. Without this flag the untitled note is refused
    /// a save by an arming rule the port never had a chance to state, and the text is lost
    /// on exit - which is exactly what the first live round trip did.
    loaded: bool,
    /// The document the port last told us about, from `Loaded::path` or `Rebound::path` -
    /// NEVER from what the bridge asked for. Save As picks a path in a dialog, but the path
    /// stored here is the one the ENGINE reported back: a bridge that remembers its own
    /// request can name a file the save never landed on. It is used for exactly one thing,
    /// the suggested directory of the next dialog.
    path: Option<std::path::PathBuf>,
    /// The list the port last delivered (`RecentsUpdated`), kept as given so the menu is
    /// rebuilt from the engine's words, not from a bridge-side copy of the truth.
    recents: Vec<RecentEntry>,
    /// The global toggle, as last stated by the ENGINE (see `new`).
    autosave: bool,
}

impl Wire {
    fn new(autosave: bool) -> Self {
        Self {
            seen_edits: 0,
            changed_at: Instant::now(),
            flushed_edits: 0,
            in_flight: None,
            armed: false,
            loaded: false,
            path: None,
            recents: Vec::new(),
            // SEEDED FROM THE PORT, not from a literal: `InitialState::autosave_enabled` is
            // read out of session.json by the engine, so the check mark on the first frame is
            // the only visible proof that the settings reload path works. Same one-way door
            // the pin was: a check that follows what the UI asked for proves nothing.
            autosave,
        }
    }

    /// The revision is the mutation counter. It is monotonic for the life of the
    /// process, it is a count and not a content hash (D11), and the buffer is the only
    /// thing that can raise it - which is exactly the property "a Flush at or below the
    /// last saved revision means clean" needs. The bridge does not own a SECOND
    /// revision: the engine's is whatever it answers with, and the bridge only ever
    /// compares against the one it sent.
    fn dirty(&self) -> bool {
        self.seen_edits != self.flushed_edits
    }
}

/// How many capped passes the exit drain will make before it says it ran out of
/// patience: 64 x 64 = 4096 events, far more than a note app can queue at exit, and a
/// bound is cheaper than a proof that an abandoned engine has stopped sending.
const MAX_EXIT_PASSES: usize = 64;

/// What one bounded look at the queue produced. Counted, not assumed, because
/// objective is the frame cost and not the feeling of it.
#[derive(Debug, Clone, Copy, Default)]
struct Drain {
    /// Events taken this pass.
    taken: usize,
    /// True when the cap was reached, i.e. the queue may hold more.
    capped: bool,
    /// True when the sender is gone: the port has closed and there is no point
    /// waking again.
    closed: bool,
}

/// The pump's own cost, in the numbers the slice is asked to report.
#[derive(Debug, Default)]
struct Pump {
    /// Wakes that found at least one event - the useful ones.
    busy: u64,
    /// Wakes that found nothing: what polling costs when the app is idle.
    idle: u64,
    /// Events rendered in total.
    events: u64,
    /// The longest single bounded drain, in microseconds.
    slowest_us: u128,
    /// Times a pass took the full cap. Named for what it measures: the pass STOPPED
    /// at the cap, which is not a claim that anything was dropped - the rest is taken
    /// on the next wake, and at exit the passes loop until the queue is empty. A queue
    /// of exactly 64 reaches the cap and truncates nothing, and the old name
    /// (truncations) said otherwise.
    caps_reached: u64,
    /// The last line the status line showed. Kept here as well as on the view so
    /// the exit trace can name it: this binary is `windows_subsystem`, so after the
    /// window is gone the only place a rendered fact can still be read is stderr.
    last_shown: Option<String>,
    /// GeometryChanged commands sent, and rect-changes seen: the ratio is the
    /// debounce, measured. A drag that changes the rect 118 times and costs 2 sends
    /// is a 59:1 suppression, and this is where that number comes from.
    rects_sent: u64,
    motions: u64,
    /// True while a run of motion is being held back, waiting for quiet.
    drag_open: bool,
    last_rect: Option<String>,
}

/// Take up to `limit` events, NON-BLOCKING, ITERATIVE.
///
/// `try_recv` only - `recv`/`recv_timeout` are the two calls that would park the
/// loop thread on a channel, which AGENTS.md forbids inside a frame. Lives apart
/// from `Surface` so the bound is testable without a window.
fn drain_bounded(events: &EventRx, out: &mut Vec<Event>, limit: usize) -> Drain {
    let mut drain = Drain::default();
    while drain.taken < limit {
        match events.try_recv() {
            Ok(event) => {
                out.push(event);
                drain.taken += 1;
            }
            // Empty is the normal answer: nothing has arrived since the last look.
            Err(TryRecvError::Empty) => break,
            // Disconnected is terminal: the sender is gone, so nothing will arrive
            // again. The port documents this as how a caller learns the engine
            // finished (crates/api/src/gateway.rs:312).
            Err(TryRecvError::Disconnected) => {
                drain.closed = true;
                break;
            }
        }
    }
    // Reaching the cap is the cap doing its job: it says the queue was deeper than
    // one wake is allowed to take, and the caller counts it.
    drain.capped = drain.taken == limit;
    drain
}

/// How long a moved window must hold still before the bridge tells the port.
///
/// A drag fires a new frame rect on almost every frame - Windows sends WM_MOVE and
/// WM_SIZE continuously (gpui src/platform/windows/events.rs:44-45, and
/// handle_move_msg at :120 re-stores the origin per message) - and there is no
/// public subscription for any of it, so an undebounced bridge would post one
/// command per pixel. The queue is unbounded (api lib.rs: the channels are
/// unbounded), which makes that a leak rather than a stall: nothing blocks, it just
/// grows. 250 ms of stillness is 15x the measured wake cadence (16.9 ms), so no user
/// can feel it, and a drag that never stops is capped by GEOMETRY_FORCE below.
/// CAUTION, pinned by a test: a window that keeps returning to the rect the port
/// already holds resets the episode clock, and an oscillation like that sends nothing
/// at all - which is the design (a window that ends where it started owes no fact), but
/// it means the force interval below is a ceiling on CONTINUOUS motion away from home,
/// not a promise that any given movement is eventually reported.
const GEOMETRY_QUIET: Duration = Duration::from_millis(250);

/// The most a continuous drag may be held back. Without this, a user who resizes
/// slowly forever would never be recorded at all, which is worse than a few extra
/// commands: one send per second of unbroken dragging, and it is measured.
const GEOMETRY_FORCE: Duration = Duration::from_millis(1000);

/// The rect the bridge can see, in physical pixels.
///
/// THIS IS NOT A FRAME RECT, and it cannot be. Window::window_bounds (gpui
/// src/window.rs:1466) is the only public read of where a window is, and on Windows
/// it is GetWindowPlacement().rcNormalPosition through
/// calculate_client_rect(rcNormalPosition, border_offset, scale_factor)
/// (src/platform/windows/window.rs:166-186): the RESTORE position - which is exactly
/// what a maximized close needs, because WindowBounds::Maximized carries the restore
/// size by definition (src/platform.rs:1192) - but with the non-client chrome
/// already SUBTRACTED, in LOGICAL pixels. Putting the chrome back needs border_offset,
/// which is private to GPUI's window state, and asking Win32 directly needs a windows
/// dependency this crate may not have (AGENTS.md: a bridge imports the port and its
/// own toolkit).
///
/// So this number is NOT what gets persisted, and no longer could be. `GeometryChanged`
/// is now a PAYLOAD-FREE SIGNAL ("geometry state changed, go measure"), the port measures
/// the FRAME rect itself through `GetWindowPlacement`, and only a successful measure may
/// write session.json - a failed measure defers the whole write and retries the pending
/// bit, a move in flight defers the tick too, and the guard lifts only at shutdown,
/// because there is no later tick to catch it (5ea2f49d).
///
/// The old contract said this value was a TRIGGER AND A FALLBACK, and the fallback half
/// is exactly what broke the app: a client-space number under gpui's own chrome model was
/// winning every write into a field core documents as FRAME pixels, so each
/// move-then-relaunch cycle shifted the window by the chrome - 16 px across, 39 px down at
/// 100%. The role left for `rect_of` is CHANGE DETECTOR: it is diffed by `Watch`, it is
/// what decides WHEN to send the signal, and it is never sent. A future reader must not
/// re-attach it to the command, and that is why this paragraph is here instead of a
/// shorter one.
fn rect_of(window: &Window) -> Rect {
    let bounds = match window.window_bounds() {
        WindowBounds::Windowed(bounds) | WindowBounds::Maximized(bounds) => bounds,
        // Fullscreen carries the RESTORE size too (gpui src/platform.rs:1195: "the
        // bounds provided here represent the restore size"), so this is the one case
        // where reading it is right: the user's note goes back where it was before it
        // went fullscreen, not to the monitor's full frame.
        WindowBounds::Fullscreen(bounds) => bounds,
    };
    let scale = window.scale_factor();
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let physical = |v: Pixels| f32::from(v).mul_add(scale, 0.0).round();
    Rect::new(
        physical(bounds.origin.x) as i32,
        physical(bounds.origin.y) as i32,
        physical(bounds.size.width).max(0.0) as u32,
        physical(bounds.size.height).max(0.0) as u32,
    )
}

/// The debounce itself: no GPUI, no clock of its own. Time is passed in, so the
/// rules are testable as data (a synthetic drag) rather than as a timing hope.
#[derive(Debug, Default)]
struct Watch {
    /// The rect the port was last told about. None until the first look, which is
    /// the BASELINE: the port put the window there, so reporting it back is noise -
    /// and it would be the client-space number, a worse answer than the measured one
    /// the port already holds.
    reported: Option<Rect>,
    /// A rect that has been seen and not yet sent.
    pending: Option<Rect>,
    /// When the current run of motion began, and when it last changed.
    first: Option<Instant>,
    changed: Option<Instant>,
    /// Counted, because "debounced" is a claim and the number is the proof.
    motions: u64,
    sends: u64,
}

impl Watch {
    /// Look at the current rect; get back the rect to send, or nothing.
    fn observe(&mut self, seen: Rect, now: Instant) -> Option<Rect> {
        let Some(reported) = self.reported else {
            self.reported = Some(seen);
            return None;
        };
        if seen == reported {
            // The window is where the port already thinks it is. Any half-finished
            // run of motion is moot - a drag that ends where it started is not a
            // fact about the window - so drop it rather than send a stale rect.
            self.pending = None;
            self.first = None;
            self.changed = None;
            return None;
        }
        if self.pending != Some(seen) {
            self.motions += 1;
            if self.pending.is_none() && self.first.is_none() {
                self.first = Some(now);
            }
            self.pending = Some(seen);
            self.changed = Some(now);
        }
        // Send only a rect that has STOPPED changing, or one that has been overdue
        // for a whole force window while the drag never stopped.
        let quiet = self
            .changed
            .is_some_and(|at| now.saturating_duration_since(at) >= GEOMETRY_QUIET);
        let overdue = self
            .first
            .is_some_and(|at| now.saturating_duration_since(at) >= GEOMETRY_FORCE);
        (quiet || overdue).then(|| self.commit(seen, now, overdue && !quiet))
    }

    /// Take a rect as sent. A forced send keeps the drag's clock running (the next
    /// force is another GEOMETRY_FORCE away); a quiet send ends the episode.
    fn commit(&mut self, rect: Rect, now: Instant, forced: bool) -> Rect {
        self.reported = Some(rect);
        self.pending = None;
        self.first = forced.then_some(now);
        self.changed = forced.then_some(now);
        self.sends += 1;
        rect
    }
}

/// The window's root view: the status line, and the Task that feeds it.
struct Surface {
    /// Shared with `main` so the exit trace can report what the pump cost after
    /// the view is gone. Main-thread only, hence `Rc`, and read only inside a
    /// `try_recv` that cannot block.
    events: Rc<RefCell<EventRx>>,
    stats: Rc<RefCell<Pump>>,
    /// The pump. Held, never read: dropping a `Task` cancels it (gpui
    /// src/executor.rs:15), so this field IS the pump's lifetime. It dies with the
    /// entity, i.e. with the window, which is why no orphan poller outlives the UI.
    _pump: Option<Task<()>>,
    /// The last event, rendered. Only `describe` writes it.
    status: SharedString,
    /// This window, so the wake can look at where it is. Filled in by `main` right
    /// after `open_window` returns, because the view is BUILT inside that call and
    /// the handle only exists after it: an empty slot simply means "nothing to
    /// compare yet", which is what the window-not-yet-open case must do anyway.
    window: Rc<RefCell<Option<AnyWindowHandle>>>,
    /// Only to send `GeometryChanged` when the watch says the drag has settled.
    gateway: Rc<RefCell<Option<Gateway>>>,
    watch: Watch,
    /// The editor surface (S2). Held as an entity because gpui renders an
    /// `Entity<V>` where `V: Render`, and because that entity is what the IME input
    /// handler is attached to during paint.
    editor: Entity<Editor>,
    /// The wire's state, shared with the close path: an `Rc` rather than a field-
    /// local because the last `Flush` has to be issued by whoever shuts the window,
    /// and that is not this view.
    wire: Rc<RefCell<Wire>>,
    /// Whether we have asked for the keyboard yet - first frame only, so that
    /// focusing the editor cannot fight something the user clicks into later.
    focus_requested: bool,
    /// Has the port been told once that a window exists? See the first-reading rule in
    /// the pump.
    signalled_once: bool,
}

impl Surface {
    fn new(
        events: Rc<RefCell<EventRx>>,
        stats: Rc<RefCell<Pump>>,
        gateway: Rc<RefCell<Option<Gateway>>>,
        window: Rc<RefCell<Option<AnyWindowHandle>>>,
        editor: Entity<Editor>,
        wire: Rc<RefCell<Wire>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            events,
            stats,
            _pump: None,
            status: SharedString::from(format!("{} · no event from the port yet", menu::legend())),
            window,
            gateway,
            watch: Watch::default(),
            editor,
            wire,
            focus_requested: false,
            signalled_once: false,
        };
        this.start_pump(cx);
        this
    }

    /// THE ONE WAKE ROUTE. A single main-thread Task, awaited on GPUI's own timer,
    /// draining on wake. See the module comment for the dispatcher path.
    fn start_pump(&mut self, cx: &mut Context<Self>) {
        let window = Rc::clone(&self.window);
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(ENGINE_POLL).await;
                // The other half of the wake: one read of where the window is now.
                // It is a `Window` method, so it happens on the loop thread; it is
                // GPUI's own cached placement, so it costs no syscall of ours beyond
                // what GPUI already refreshed on WM_MOVE/WM_SIZE. A closed window
                // reads as None, which the pump treats as "nothing to compare".
                let seen = match window.borrow().as_ref() {
                    Some(handle) => handle.update(cx, |_view, window, _cx| rect_of(window)).ok(),
                    None => None,
                };
                let keep_running = match this.update(cx, |this, cx| this.pump(seen, cx)) {
                    Ok(keep) => keep,
                    // The entity is gone with its window, so the pump goes with it.
                    Err(_) => break,
                };
                if !keep_running {
                    break;
                }
            }
        });
        self._pump = Some(task);
    }

    /// One wake: drain, look at the window, render, and ask for a repaint ONLY if
    /// there was something to show. Notifying on an idle wake would turn 125 polls a
    /// second into 125 frames a second, which is the mistake this avoids.
    fn pump(&mut self, seen: Option<Rect>, cx: &mut Context<Self>) -> bool {
        let started = Instant::now();
        let mut batch = Vec::new();
        let drain = {
            let events = self.events.borrow();
            drain_bounded(&events, &mut batch, MAX_DRAIN_PER_WAKE)
        };
        let took_us = started.elapsed().as_micros();

        {
            let mut stats = self.stats.borrow_mut();
            if drain.taken == 0 {
                stats.idle += 1;
            } else {
                stats.busy += 1;
                stats.events += drain.taken as u64;
                stats.slowest_us = stats.slowest_us.max(took_us);
            }
            if drain.capped {
                stats.caps_reached += 1;
            }
        }

        // LAST event wins the line: `batch` is in queue order, so its final entry
        // is the newest fact the port has given us.
        for event in &batch {
            self.status = SharedString::from(one_line(describe(event)));
            self.apply(event, cx);
        }
        if drain.closed {
            // Terminal, and said out loud rather than left as a stale line.
            self.status = SharedString::from("the port has closed: no more events");
        }
        if drain.taken > 0 || drain.closed {
            let shown = self.status.to_string();
            // The rendered line goes on the last-resort trace as it is rendered, not
            // only at exit: "what did the status line show" has to be answerable from a
            // real run, and a `windows_subsystem` binary has no other voice. Gated on a
            // REAL change, so an idle app never writes and a repeated event does not
            // grow a log without bound - the claim this comment used to make without the
            // code behind it. The repaint is NOT gated: the counters line under this one
            // changes every wake and has to keep updating.
            if record_shown(&mut self.stats.borrow_mut().last_shown, &shown) {
                report(&format!("status line: {shown}"));
            }
            cx.notify();
        }

        // The text half of the same wake: the debounce lives here (D51), and a wake is
        // the only clock this bridge has. It costs one integer compare until the buffer
        // is actually quiet, so a 2000-line note is never cloned on an idle tick.
        self.flush_tick(cx);

        // The geometry half of the same wake. ONE read, ONE possible send per wake,
        // and only after the drag has settled - so a two-second drag cannot put 4000
        // commands on an unbounded queue. The counters are what prove that claim.
        if let Some(rect) = seen {
            let settled = self.watch.observe(rect, Instant::now());
            // THE FIRST SETTLED READING IS ITSELF A FACT. `Watch` only reports a
            // CHANGE, which was right under the old contract (the port kept whatever rect
            // it was last told) and is wrong under the new one: a fresh install has no
            // rect to restore, the port now writes session.json only after a SUCCESSFUL
            // measure, and with no signal ever sent there is nothing to measure and
            // nothing to persist - which is exactly how `xtask smoke` came back with
            // `session=FAILED, this run wrote no session.json` tonight (measured:
            // `geometry: 0 rect changes seen, 0 GeometryChanged sent`, and the same
            // binary DOES persist 333,222,500,350 the moment the window is moved).
            let first = !self.signalled_once;
            if first || settled.is_some() {
                self.signalled_once = true;
                let fired = settled.unwrap_or(rect);
                // PAYLOAD-FREE (5ea2f49d): the port measures the FRAME rect itself, and
                // only a successful measure may write session.json. The rect that fired
                // the signal is still named in the trace, so the change stays auditable -
                // it is simply no longer the value that gets persisted, which is the drift
                // bug this replaces.
                send(&self.gateway, Command::GeometryChanged);
                let mut stats = self.stats.borrow_mut();
                stats.rects_sent += 1;
                stats.last_rect = Some(format!(
                    "signal sent{}, change was {}x{} at {},{}",
                    if first { " (first reading)" } else { "" },
                    fired.w,
                    fired.h,
                    fired.x,
                    fired.y
                ));
            }
            let mut stats = self.stats.borrow_mut();
            stats.motions = self.watch.motions;
            stats.drag_open = self.watch.pending.is_some();
        }
        !drain.closed
    }
}

impl Render for Surface {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The editor owns the keyboard from the first frame: a notes window that
        // needs a click before it takes typing is not a notes window. One shot only,
        // so it can never steal focus back from something added later.
        if !self.focus_requested {
            self.focus_requested = true;
            let handle = Focusable::focus_handle(self.editor.read(cx), cx);
            window.focus(&handle, cx);
        }
        // Nothing invented here: GPUI 0.2.2 has no `Label` widget (its own text
        // elements are in src/elements/text.rs - `impl Element for &'static str` at
        // :19, `impl IntoElement for String` at :77, and `impl Element for
        // SharedString` at :85 / `impl IntoElement for SharedString` at :135, which
        // lay the string out through `TextLayout`). So a `SharedString` child IS
        // the label, and the `Div` inherits its text style down to children
        // (src/elements/div.rs:1334, `window.with_text_style`).
        let counters = self.counters();
        // A shutdown note outranks the last event, because "the engine did not finish"
        // is the one sentence worth reading before the window disappears.
        let status = SHUTDOWN_NOTE
            .with(|note| note.borrow().clone())
            .unwrap_or_else(|| self.status.clone());
        div()
            .size_full()
            .flex_col()
            .bg(rgb(0x1f1f1f))
            .text_color(rgb(0xe6_e6_e6))
            // The editor above the status line, and the status line STAYS: every
            // pump diagnostic and the smoke harness read that second row, so the
            // layout is stacked rather than replaced. flex_1 gives the editor the
            // space and the status bar its two lines back at the bottom.
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .text_size(px(15.0))
                    .child(self.editor.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .flex_col()
                    .bg(rgb(0x14_14_14))
                    .text_size(px(13.0))
                    .whitespace_nowrap()
                    .child(status)
                    .child(counters),
            )
    }
}

impl Surface {
    /// PUT THE PORT'S WORDS INTO THE BUFFER, and refuse to let a failed load leave a
    /// lie on screen. Everything the wire needs to remember is decided here, once.
    fn apply(&mut self, event: &Event, cx: &mut Context<Self>) {
        match event {
            Event::Loaded { text, meta, path } => {
                // LOAD RESETS THE VIEW, not just the text - see `Editor::load`. The
                // arming comes from the port's own `FileMeta::armed` (ADR-0001): a
                // foreign file nobody has saved once is not armed, and the bridge does
                // not decide otherwise for it.
                self.editor
                    .update(cx, |editor, cx| editor.load(text.clone(), cx));
                let mut wire = self.wire.borrow_mut();
                wire.armed = meta.armed;
                // The path the ENGINE says this buffer now is. Not the one we asked for.
                wire.path = Some(path.clone());
                wire.in_flight = None;
                wire.changed_at = Instant::now();
                let edits = self.editor.read(cx).edits();
                wire.seen_edits = edits;
                wire.flushed_edits = edits;
            }
            Event::Rebound {
                meta,
                revision,
                path,
            } => {
                // The SAME buffer, a new path: Save As. No text crosses back - the
                // bridge owns it - but the write that just happened was this text, so it
                // is clean now, and the arming follows the new path (a file chosen by
                // hand is armed, ADR-0001).
                let mut wire = self.wire.borrow_mut();
                wire.armed = meta.armed;
                // ADOPTED FROM THE PORT: this is the rebind the menu's Save As depends on.
                // Every later Flush goes to the document the engine rebound to, and the
                // bridge learned that from an Event, not from its own request.
                wire.path = Some(path.clone());
                wire.flushed_edits = wire.seen_edits;
                if wire.in_flight.is_some_and(|sent| sent <= *revision) {
                    wire.in_flight = None;
                }
            }
            Event::LoadFailed { .. } => {
                // DECISION, documented because the alternative is data loss: the buffer
                // KEEPS the previous note. The status line above says the open failed, so
                // the screen is not lying about what is loaded, and disarming means the
                // kept text can never be flushed onto the path that just failed - which
                // is how a failed open would otherwise destroy a file it never read. What
                // the user loses is the OLD path's identity, not the words.
                self.wire.borrow_mut().armed = false;
            }
            Event::Saved { revision, .. } | Event::SaveFailed { revision, .. } => {
                let mut wire = self.wire.borrow_mut();
                if wire.in_flight.is_some_and(|sent| sent <= *revision) {
                    wire.in_flight = None;
                }
                // THE ONLY ENGINE ANSWER THAT PROVES THE TOGGLE IS ON: `Saved` reaches the
                // bridge solely from the autosave path (there is no manual Save command), so
                // a save the engine performed says the global toggle was not off. `Event` has
                // no "autosave setting changed" variant, which is why the check mark is driven
                // by outcomes and never by what the menu asked for.
                if matches!(event, Event::Saved { .. }) && !wire.autosave {
                    wire.autosave = true;
                    drop(wire);
                    self.refresh_menus(cx);
                }
            }
            Event::AutosaveSkipped { reason } => {
                // THE ENGINE'S OWN WORDS THAT THE TOGGLE IS OFF. This is the readback the
                // check mark waits for: `SkipReason::AutosaveDisabled` is the port telling us
                // the global switch is off, as opposed to the per-document arming or a clean
                // buffer. Any other reason leaves the setting alone.
                // ANSWERED, even though it was refused. `SkipReason` says why but the
                // variant carries NO revision (crates/api/src/event.rs:344), so this is
                // the second place - after the abandon path - where the bridge has to
                // guess whether the skip was about its own flush. See the request in the
                // commit notes: the port has no way to tie an answer to a revision.
                let mut wire = self.wire.borrow_mut();
                wire.in_flight = None;
                if *reason == SkipReason::AutosaveDisabled && wire.autosave {
                    wire.autosave = false;
                    drop(wire);
                    self.refresh_menus(cx);
                }
            }
            Event::RecentsUpdated(entries) => {
                // The list, as the engine delivered it - capped, labelled and existence-
                // marked by core. The menu is rebuilt FROM THIS and nothing else, so the
                // recent list can never show a file the port did not report.
                self.wire.borrow_mut().recents = entries.clone();
                self.refresh_menus(cx);
            }
            _ => {}
        }
    }

    /// Rebuild the native menu bar from the state the PORT supplied. Cheap and rare: it
    /// happens when a recent list, a save outcome, or a skip arrives - not per frame, and
    /// never from a frame callback. `App::set_menus` takes `&self` and the Windows platform
    /// implementation only stores the vector, so this cannot touch the window, the overlay
    /// stack, or the editor's selection - which is the whole reason the menu was allowed in
    /// without `Root`.
    fn refresh_menus(&mut self, cx: &mut Context<Self>) {
        let wire = self.wire.borrow();
        let menus = menu::build_menus(wire.autosave, &wire.recents);
        drop(wire);
        cx.set_menus(menus);
    }

    /// A sentence the bridge itself wants on the status line - "save as cancelled" is not
    /// something the port can know. The next event from the engine replaces it, because the
    /// engine's words outrank ours; `SHUTDOWN_NOTE` still outranks both.
    fn note(&mut self, text: &str, cx: &mut Context<Self>) {
        self.status = SharedString::from(one_line(text.to_string()));
        report(&format!("menu: {text}"));
        cx.notify();
    }

    /// THE DEBOUNCE, and the one-in-the-air rule. Called from the pump, never from a
    /// frame callback and never from a save thread (AGENTS.md: both deadlock).
    fn flush_tick(&mut self, cx: &mut Context<Self>) {
        let edits = self.editor.read(cx).edits();
        let now = Instant::now();
        {
            let mut wire = self.wire.borrow_mut();
            if edits != wire.seen_edits {
                wire.seen_edits = edits;
                wire.changed_at = now;
            }
        }
        let composing = self.editor.read(cx).is_composing();
        let quiet_for = self.wire.borrow().changed_at.elapsed();
        if !flush_due(&self.wire.borrow(), composing, quiet_for) {
            return;
        }
        let text = self.editor.read(cx).text().to_string();
        {
            let mut wire = self.wire.borrow_mut();
            wire.flushed_edits = edits;
            wire.in_flight = Some(edits);
        }
        report(&format!("flush: revision {edits}, {} bytes", text.len()));
        send(
            &self.gateway,
            Command::Flush {
                text,
                revision: edits,
            },
        );
    }

    /// The second line: the pump's own cost, live, because "it is cheap" is not
    /// evidence.
    fn counters(&self) -> SharedString {
        let stats = self.stats.borrow();
        let total = stats.busy + stats.idle;
        SharedString::from(format!(
            "pump: {total} wakes ({busy} with work, {idle} idle) · {events} events · slowest drain {slowest} us · {caps} reached cap · cap {cap} · poll {poll} ms · geometry: {motions} changes, {sends} sent{drag}",
            busy = stats.busy,
            idle = stats.idle,
            events = stats.events,
            slowest = stats.slowest_us,
            caps = stats.caps_reached,
            cap = MAX_DRAIN_PER_WAKE,
            poll = ENGINE_POLL.as_millis(),
            motions = stats.motions,
            sends = stats.rects_sent,
            drag = if stats.drag_open { " (drag open)" } else { "" },
        ))
    }
}

/// Collapse the rendered line to one physical line.
///
/// core's `SettingsError::Corrupt` and a Win32 message both arrive as sentences
/// that can carry newlines (a TOML parse error quotes the offending line), and a
/// STATUS line is one line: the bar is laid out in a column whose only other
/// member is the editor that will fill the rest of the window. Words are
/// untouched - whitespace runs become single spaces, which is a shape change, not
/// a rewrite. The full text stays on disk and in the parse error the user has to
/// read; this is only about not breaking the layout to say it.
fn one_line(text: String) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// THE EXHAUSTIVENESS CONTROL: one arm per `Event` variant, NO wildcard, so the
/// next variant added to the port is a COMPILE ERROR in the bridge rather than a
/// silently undelivered fact. This is the reason the status line is a deliverable.
///
/// Copy rule: a `reason` the port already worded passes through UNTRANSLATED - the
/// `String` on `GeometryNotRestored` and `SettingsCorrupt`, and the `Display` of
/// `SaveError`/`LoadError`, which the port documents as the user-visible copy. The
/// bridge only supplies words where the port deliberately left them out, which is
/// `SkipReason` (crates/api/src/event.rs:797: "SkipReason carries no Display on
/// purpose ... the words are" the UI's).
fn describe(event: &Event) -> String {
    match event {
        // `text` is never echoed: the bridge owns the buffer, and the editor is a
        // later slice. Its size is the fact the status line may carry.
        Event::Loaded { path, text, meta } => format!(
            "Loaded {} · {} chars · {}",
            path.display(),
            text.chars().count(),
            meta_words(meta)
        ),
        Event::LoadFailed { path, reason } => {
            format!("LoadFailed {} · {}", path.display(), reason)
        }
        Event::Saved { path, revision } => {
            format!("Saved {} · revision {}", path.display(), revision)
        }
        Event::SaveFailed {
            path,
            revision,
            reason,
        } => format!(
            "SaveFailed {} · revision {} · {}",
            path.display(),
            revision,
            reason
        ),
        Event::Rebound {
            path,
            meta,
            revision,
        } => format!(
            "Rebound {} · revision {} · {}",
            path.display(),
            revision,
            meta_words(meta)
        ),
        Event::ExternalChange { path } => {
            format!(
                "ExternalChange {} · changed outside this app",
                path.display()
            )
        }
        Event::AutosaveSkipped { reason } => format!("AutosaveSkipped · {}", skip_words(*reason)),
        Event::GeometryNotRestored { rect, reason } => format!(
            "GeometryNotRestored · frame {}x{} at {},{} · {}",
            rect.w, rect.h, rect.x, rect.y, reason
        ),
        Event::RecentsUpdated(entries) => format!("RecentsUpdated · {}", recents_words(entries)),
        Event::SettingsCorrupt { reason } => {
            format!("SettingsCorrupt · running on defaults · {}", reason)
        }
        // The drain the exhaustiveness control caught: this variant landed in the
        // port while this arm was being written, and the missing case was a COMPILE
        // ERROR rather than a silently undelivered fact. That is the whole point of
        // having no wildcard arm above.
        //
        // Its own variant rather than a SaveFailed because a session file has no
        // revision to report (api event.rs:338), and it is latched to fire once per
        // failure episode, so the line does not become a per-tick toast.
        Event::SessionWriteFailed { reason } => {
            format!(
                "SessionWriteFailed · the window position was not stored · {}",
                reason
            )
        }
        // Caught the same way, a few commits later: the port grew this variant while
        // the pump was being written, and the missing arm was a compile error here
        // rather than a status line that quietly kept saying something older.
        //
        // The line has to be rendered because the event is the ONLY notice that
        // nothing will persist at all; the `reason` is the OS's own sentence or the
        // port's factual one, so it passes through unchanged (api event.rs:351).
        Event::StateDirUnusable { reason } => {
            format!("StateDirUnusable · nothing can be remembered · {}", reason)
        }
        // Third time this slice's no-wildcard match has refused to build, and the
        // third time it was the port growing a variant the bridge had not been told
        // about. settings.toml has no revision either, so like its session twin this
        // is not a SaveFailed and must not read like one: the file that refused is
        // the preference file, not the user's note.
        Event::SettingsWriteFailed { reason } => {
            format!(
                "SettingsWriteFailed · the preference was not stored · {}",
                reason
            )
        }
    }
}

/// All six `FileMeta` fields: every one is a fact the port decided.
fn meta_words(meta: &FileMeta) -> String {
    let FileMeta {
        encoding,
        line_ending,
        trailing_newline,
        read_only,
        oversize,
        armed,
    } = *meta;
    format!(
        "{} · {} · newline {} · {} · {} · autosave {}",
        encoding_words(encoding),
        line_ending_words(line_ending),
        if trailing_newline { "kept" } else { "none" },
        if read_only { "read-only" } else { "writable" },
        if oversize {
            "over the guard"
        } else {
            "within the guard"
        },
        if armed { "armed" } else { "not armed" },
    )
}

/// Exhaustive like `describe`: a new encoding is also a compile error here.
fn encoding_words(encoding: Encoding) -> String {
    match encoding {
        Encoding::Utf8 => "utf-8".to_string(),
        Encoding::Utf8Bom => "utf-8 with BOM".to_string(),
        Encoding::Utf16Le => "utf-16 le".to_string(),
        Encoding::Utf16Be => "utf-16 be".to_string(),
        // The codepage id is a fact the port carries on the variant, so the line
        // names it: utf-8-without-a-BOM and CP1252 must not read the same (D14).
        Encoding::Ansi(codepage) => format!("ansi cp{codepage}"),
    }
}

fn line_ending_words(line_ending: LineEnding) -> &'static str {
    match line_ending {
        LineEnding::Lf => "lf",
        LineEnding::CrLf => "crlf",
    }
}

/// ADR-0001: a skip is EXPLAINED, never inferred. No `Display` reaches us for this
/// enum, so these words are the bridge's, including the ADR's own sentence for the
/// one case where the user has to act.
fn skip_words(reason: SkipReason) -> &'static str {
    match reason {
        SkipReason::AutosaveDisabled => "auto-save is off",
        SkipReason::ForeignFileNotArmed => {
            "a file this app did not create: save it once and it keeps saving"
        }
        SkipReason::Clean => "nothing changed since the last write",
        SkipReason::NeedsPath => "this note has no file yet: use Save As",
        SkipReason::ReadOnly => "the file is read-only",
        SkipReason::Oversize => "the file is over the size guard, so writes are refused",
    }
}

/// `display` is pre-formatted by the engine, so the bridge joins what it is given
/// instead of re-deriving basenames. An entry whose file has gone is marked, not
/// dropped - the port keeps it deliberately (D13).
fn recents_words(entries: &[RecentEntry]) -> String {
    let mut out = format!("{} in the list", entries.len());
    let shown = entries.len().min(3);
    if shown > 0 {
        let list: Vec<String> = entries[..shown]
            .iter()
            .map(|entry| {
                format!(
                    "{}{}",
                    entry.display,
                    if entry.exists { "" } else { " (gone)" }
                )
            })
            .collect();
        out.push_str(" · ");
        out.push_str(&list.join(", "));
    }
    out
}

fn main() {
    // A panic in a `windows_subsystem = "windows"` binary is invisible: no console, and
    // the process simply leaves, which is how a crash in the frame loop or the pump
    // would go permanently unreported. Route it into the same last-resort trace every
    // other fact uses. The events still queued at that moment are unrecoverable - the
    // pump dies with the panic and nothing can promise a clean exit drain from inside
    // one - so the second line says that rather than leaving it to be guessed.
    std::panic::set_hook(Box::new(|info| {
        report(&format!("panic: {info}"));
        report("a panic ended the app: queued events were not drained");
    }));
    // STEP 1 - query the saved session. `Gateway::start` reads session.json once,
    // on this thread; `startup_state` hands over that snapshot and is consume-once,
    // so it is read here and nowhere else.
    let (mut gateway, events) = Gateway::start(state_dir(), Settings::default());
    let Some(initial) = gateway.startup_state() else {
        // Only None if the snapshot had already been consumed, which one call site
        // cannot do. If it ever can: close is the defined exit, where dropping the
        // Gateway would be an abort with no word said. Nothing of the user's exists
        // yet - no window, no editor text - so an Err here cannot lose an edit, and the
        // line says that instead of asserting a cause it did not observe.
        if gateway.close().is_err() {
            report(
                "the engine was already gone before a window opened - there was nothing yet to lose",
            );
        }
        return;
    };

    // The Gateway rides in an Option so that exactly one owner can take it and call
    // the blocking `close`. Both candidate exits (last window closed, loop quit)
    // share it and `take` makes the second one a no-op. It is never reachable from
    // engine code, which is what makes joining in it safe.
    let gateway = Rc::new(RefCell::new(Some(gateway)));
    let events = Rc::new(RefCell::new(events));
    // Shared with the view so the cost can be reported after the view is gone.
    let stats = Rc::new(RefCell::new(Pump::default()));
    // A dropped Subscription unsubscribes, so the close handler needs somewhere to
    // live for the whole loop rather than for one call.
    let subscriptions = Rc::new(RefCell::new(Vec::<Subscription>::new()));
    // The pump needs this window's handle to look at its bounds, but the handle only
    // exists once `open_window` has returned, and the view is built INSIDE that call.
    // A one-slot cell is the smallest thing that resolves the ordering; the pump reads
    // `None` until it is filled, and `None` means "nothing to compare yet".
    let window_slot: Rc<RefCell<Option<AnyWindowHandle>>> = Rc::new(RefCell::new(None));

    // THE KIT GENERATION: `Application::new` is gone from the surface this crate is given;
    // gpui_kit::platform::application() is the constructor that picks the platform backend
    // (gpui-pre-platform-0.3.4/src/gpui_platform.rs:13).
    gpui_kit::platform::application().run({
        let gateway = Rc::clone(&gateway);
        let events = Rc::clone(&events);
        let stats = Rc::clone(&stats);
        let subscriptions = Rc::clone(&subscriptions);
        let window_slot = Rc::clone(&window_slot);
        move |cx: &mut App| {
            // REQUIRED, not decoration: without this the app panics at first render with
            // "no state of type gpui_component::theme::Theme exists" (learned by the probe,
            // named in the migration brief). It initializes the kit's app state - it does
            // NOT put a Theme, Root or any component in this window, and none is used.
            gpui_kit::init(cx);
            // The key map, in the examples shape (examples/input.rs:677-692), with
            // the Windows modifiers rather than the mac ones the example carries, and
            // the actions named through the module so `Copy` cannot shadow the trait
            // of the same name. No word motion: the example has none, and guessing a
            // word boundary for combining marks and emoji sequences is a slice of its
            // own. `ctrl-c`/`ctrl-v`/`ctrl-x` are taken because the clipboard code is
            // the three trivially-separable handlers, not because S5 is done.
            cx.bind_keys([
                KeyBinding::new("backspace", editor::Backspace, None),
                KeyBinding::new("delete", editor::Delete, None),
                KeyBinding::new("left", editor::Left, None),
                KeyBinding::new("right", editor::Right, None),
                KeyBinding::new("shift-left", editor::SelectLeft, None),
                KeyBinding::new("shift-right", editor::SelectRight, None),
                KeyBinding::new("home", editor::Home, None),
                KeyBinding::new("end", editor::End, None),
                KeyBinding::new("escape", editor::EscapeSelection, None),
                KeyBinding::new("enter", editor::Newline, None),
                KeyBinding::new("shift-enter", editor::Newline, None),
                KeyBinding::new("up", editor::Up, None),
                KeyBinding::new("down", editor::Down, None),
                KeyBinding::new("shift-up", editor::SelectUp, None),
                KeyBinding::new("shift-down", editor::SelectDown, None),
                KeyBinding::new("ctrl-a", editor::SelectAll, None),
                KeyBinding::new("ctrl-c", editor::Copy, None),
                KeyBinding::new("ctrl-x", editor::Cut, None),
                KeyBinding::new("ctrl-v", editor::Paste, None),
                KeyBinding::new("pageup", editor::PageUp, None),
                KeyBinding::new("pagedown", editor::PageDown, None),
                KeyBinding::new("shift-pageup", editor::SelectPageUp, None),
                KeyBinding::new("shift-pagedown", editor::SelectPageDown, None),
            ]);
            // THE MENU'S CHORDS, from the same table the legend and the menu labels
            // are generated from (menu::SHORTCUTS). On Windows `set_menus` stores the bar and
            // never draws it, so these bindings ARE the reachable form of Open, Save As, the
            // autosave toggle and the ten recents - and they dispatch the SAME actions the
            // menu items carry, so there is one handler per command rather than one for the
            // key and one for the click.
            cx.bind_keys(menu::key_bindings());
            // THE WIRE'S STATE and the editor's address, both out here rather than in
            // the build closure below, because the close path needs them and it is not the
            // view that runs it. The editor crosses as a slot for the same reason the
            // window handle does: it is built inside `open_window`, and a `RefCell<Option<
            // _>>` filled there is readable from here afterwards.
            // Seeded from the port's own setting, so the menu's check mark on the first
            // frame is a readback of session.json and not a bridge-side opinion.
            let wire: Rc<RefCell<Wire>> =
                Rc::new(RefCell::new(Wire::new(initial.autosave_enabled)));
            let editor_slot: Rc<RefCell<Option<Entity<Editor>>>> = Rc::new(RefCell::new(None));
            // The root view's handle, filled below, so a menu handler can put a sentence on
            // the status line - the only place this app can say "save as cancelled".
            let view_slot: Rc<RefCell<Option<GpuiWindowHandle<Surface>>>> =
                Rc::new(RefCell::new(None));

            // STEP 2 - create the window AT the saved rect, before anything is
            // drawn. Only the bridge can: the port has no window type at all. The
            // root view owns the pump, so the wake route starts and stops with the
            // window and there is no poller outliving the UI.
            let options = WindowOptions {
                window_bounds: Some(bounds_for(&initial)),
                // The title is the cheapest proof that the running binary is this
                // build: it carries the package version.
                titlebar: Some(TitlebarOptions {
                    title: Some(format!("notes {}", env!("CARGO_PKG_VERSION")).into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let opened = cx.open_window(options, {
                let events = Rc::clone(&events);
                let stats = Rc::clone(&stats);
                let gateway = Rc::clone(&gateway);
                let wire = Rc::clone(&wire);
                let editor_slot = Rc::clone(&editor_slot);
                let window_slot = Rc::clone(&window_slot);
                move |_, cx| {
                    // The editor view, built before the root view so the root can
                    // own it. Empty, unshaped, and not yet focused: focus is asked
                    // for on the first render, and the first shape happens on that
                    // same first paint - not here, and not at startup.
                    let editor = cx.new(Editor::new);
                    *editor_slot.borrow_mut() = Some(editor.clone());
                    cx.new(|cx| {
                        Surface::new(
                            Rc::clone(&events),
                            Rc::clone(&stats),
                            Rc::clone(&gateway),
                            Rc::clone(&window_slot),
                            editor,
                            Rc::clone(&wire),
                            cx,
                        )
                    })
                }
            });
            let Ok(handle) = opened else {
                // No window means no UI to run: close the port - drain, final
                // session write, join - instead of letting Drop abort it.
                close(&gateway, &events);
                cx.quit();
                return;
            };
            // THE MENU'S HANDS LIVE ON THE APP, not on a view: `App::on_action` is an
            // app-global listener, `impl Fn(&A, &mut App)` (gpui-pre-0.3.4 src/app.rs:2261).
            // That is the third fact behind doing this slice without `Root`: a menu item and a
            // key chord both land in a function holding the App and nothing else, so no
            // overlay stack and no second owner of the window's text selection appears next to
            // the editor's marked range.
            *view_slot.borrow_mut() = Some(handle);
            register_menu_commands(
                cx,
                Rc::clone(&gateway),
                Rc::clone(&wire),
                Rc::clone(&editor_slot),
                Rc::clone(&view_slot),
            );
            let (autosave_on, recents) = {
                let state = wire.borrow();
                (state.autosave, state.recents.clone())
            };
            cx.set_menus(menu::build_menus(autosave_on, &recents));
            // STEP 3 - register the window handle with the port. The HWND crosses
            // as an i64 because the port must not know a platform type exists.
            //
            // THIS IS THE PIN ORDER, and it is worth the four lines. The platform lane
            // measured (442c2a1) that an async topmost reband applied to a HIDDEN window
            // never lands - silently lost - while an async move on the same hidden window
            // does. The port drives restore-and-pin off this registration, so the order
            // matters: `open_window` above has already created the window, applied its
            // placement through `SetWindowPlacement` (which carries the show state; gpui
            // src/platform/windows/window.rs:302-316) and let it draw at least once before
            // returning (src/app.rs:959-961). So the first pin is applied to a window that
            // is up, not merely created, and the very first pinned launch is NOT the broken
            // case. What is still unverified, and named rather than claimed: that a
            // minimize/hide cannot slip between this line and the reband landing. There is
            // no guard here and no new Event asked for, because the bridge has no path that
            // hides a window today - no tray, no hide, no second window - and the pin is
            // applied once, at registration, before any user minimize is possible. If S7 or
            // the menus ever add a hide, this comment is the place that decision has to be
            // revisited.
            let any: AnyWindowHandle = handle.into();
            // Fill the slot the pump reads. The view was built inside open_window and
            // could not have had the handle then; until this line lands the pump has
            // nothing to compare and sends nothing, which is the safe reading.
            *window_slot.borrow_mut() = Some(any);
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

            // STEP 4 - the pin bit. No longer the bridge's job to APPLY.
            //
            // Until 14295b8 the port only stored the bit, and this comment said so.
            // It now acts on it: RegisterWindow above is handled by
            // `restore_and_pin` -> `apply_topmost` (crates/api/src/engine.rs), which
            // calls notes-platform on the handle this bridge just registered, reading
            // the bit straight out of the session it read in step 1. So a pinned
            // session opens pinned with no bridge help, and a duplicate
            // `SetPinned(true)` here would only re-state a value the engine already
            // has (its handler compares before queueing, so it would be a no-op).
            // Removed rather than kept as insurance: storing the bit stays the
            // bridge's job when the user clicks a pin button, and that command
            // belongs to that slice, not to startup.
            //
            // What the pump does with the same registration is the visible half: if
            // the restore or the topmost call is refused, the engine answers
            // `Event::GeometryNotRestored` and the status line says so, in
            // notes-platform's own words.

            // STEP 5 - ASK FOR THE DOCUMENT THAT WAS OPEN. The session carries the path
            // and nothing in the app asked for it, which is why a relaunch came back to an
            // EMPTY editor while the bytes sat on disk (named in b343543). This is one
            // command of EXISTING vocabulary, and its answer is the `Loaded` the wire
            // already applies - so it is a send, not a subsystem. No path, no ask: a first
            // launch and an untitled note that was never written come back the way they
            // should, empty.
            if let Some(path) = initial.session.path.clone() {
                report(&format!("startup: asking the port for {}", path.display()));
                send(&gateway, Command::Open { path });
            }

            // The old startup drain is gone: the pump drains on its first wake,
            // 8 ms from now, and renders what the startup produced instead of
            // discarding it.

            // A quit through the close door: `on_window_closed` fires when the last
            // window goes, and `close` joins the engine, so the final session write
            // has completed before this process leaves main.
            let held = Rc::clone(&subscriptions);
            let closing = Rc::clone(&gateway);
            let wire = Rc::clone(&wire);
            let editor_slot = Rc::clone(&editor_slot);
            //
            // UNREGISTER FIRST, then close. A dead HWND value can be reissued to
            // another process window, and a GeometryChanged still sitting in the queue
            // would then be read as OUR rect: the port would write a stranger's position
            // into session.json and move that stranger on the next launch. Unregister
            // clears the stored handle, so every queued geometry command after it is a
            // no-op. The order the port specifies for a recreate is unregister-then-
            // register, and this site plus STEP 3 above are exactly that pair.
            held.borrow_mut().push(cx.on_window_closed({
                let unregistering = Rc::clone(&closing);
                let events = Rc::clone(&events);
                let editor_slot = Rc::clone(&editor_slot);
                let wire = Rc::clone(&wire);
                // THE KIT GENERATION: the callback now also receives the `WindowId` that
                // closed - `impl FnMut(&mut App, WindowId)` (gpui-pre-0.3.4
                // src/app.rs:2387), where 0.2.2 passed the app context alone. The id is
                // not used: this app has exactly one window, and on_window_closed fires
                // when the LAST one goes, so filtering by id would be a guard against a
                // case the bridge does not have. If a second window ever exists, THIS is
                // the line that must start comparing ids before it closes the engine.
                move |cx: &mut App, _window: WindowId| {
                    // LAST WORDS FIRST: the final Flush goes out before `Shutdown` is
                    // queued by `close`, so the engine's own bounded exit does the write
                    // and the join we already wait on waits for it. Not inside a frame -
                    // this is the window-closed callback, the same place that already
                    // blocks on `close()` by contract.
                    final_flush(&unregistering, &editor_slot, &wire, cx);
                    send(&unregistering, Command::UnregisterWindow);
                    close(&closing, &events);
                }
            }));
        }
    });

    // And the door that does not come through a window close.
    close(&gateway, &events);
    // The pump died with its window, so anything the engine said during its own
    // shutdown has nowhere to be rendered. It is named in the trace instead of
    // being dropped in silence, which is what the previous two drains did.
    let (tail, exhausted) = final_drain(&events);
    // The count first, so the list can be checked: 100 named, 100 counted. Without it
    // "every event was reported" is a claim about a trace nobody is going to count by
    // hand, which is the same silence with more words.
    report(&format!(
        "exit drain: {} event(s) accounted for{}",
        tail.len(),
        if exhausted {
            ""
        } else {
            " (the pass limit was reached - MORE MAY BE QUEUED BEHIND THIS)"
        }
    ));
    let reported = stats.borrow();
    report(&format!(
        "pump: {} wakes, {} had work, {} events, slowest drain {} us, {} drains reached the cap",
        reported.busy + reported.idle,
        reported.busy,
        reported.events,
        reported.slowest_us,
        reported.caps_reached,
    ));
    report(&format!(
        "geometry: {} rect changes seen, {} GeometryChanged sent (quiet {} ms, force {} ms)",
        reported.motions,
        reported.rects_sent,
        GEOMETRY_QUIET.as_millis(),
        GEOMETRY_FORCE.as_millis(),
    ));
    if let Some(rect) = reported.last_rect.as_deref() {
        report(&format!("last GeometryChanged: {rect}"));
    }
    // And the rendered line itself. `windows_subsystem` means there is no console
    // open once the window is gone, so the exit trace is where "what did the status
    // line show" can still be answered from the binary rather than from a reading
    // of the source.
    match reported.last_shown.as_deref() {
        Some(line) => report(&format!("status line last showed: {line}")),
        None => report("status line never changed: the port said nothing"),
    }
    for event in &tail {
        report(&format!("undisplayed: {}", describe(event)));
    }
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

/// THE FLUSH RULE, as one pure function, because the four conditions are the whole of
/// D51 and an if-chain inside a method is a place for one of them to be lost.
///
/// A `Flush` goes out only when: (a) the buffer changed since the last one accepted, (b)
/// no composition is open, (c) autosave is on AND this document is armed, and (d) the
/// engine has answered the last flush we sent. (d) is the revision guard: without it a
/// slow save plus fast typing queues whole-document writes behind one worker, each of
/// which is a temp write, an fsync and a rename.
fn flush_due(wire: &Wire, composing: bool, quiet_for: Duration) -> bool {
    if !wire.dirty() {
        return false;
    }
    if composing || !wire.autosave {
        return false;
    }
    // (c) A LOADED document needs the port's own `armed`; a buffer the port never
    // loaded is the session's scratch note and has no arming to wait for. Foreign and
    // never-armed never flushes, which is ADR-0001 doing its job.
    if wire.loaded && !wire.armed {
        return false;
    }
    if wire.in_flight.is_some() {
        return false;
    }
    quiet_for >= AUTOSAVE_IDLE
}

/// THE LAST FLUSH, issued before `Shutdown` so the engine's own exit drain does the
/// write while we are still waiting for it. Every branch says what it did in the exit
/// trace, because the one outcome that must never be silent is text that is about to
/// be lost - and D51's composition rule CAN cost a user a line if they close mid-IME.
fn final_flush(
    gateway: &Rc<RefCell<Option<Gateway>>>,
    editor_slot: &Rc<RefCell<Option<Entity<Editor>>>>,
    wire: &Rc<RefCell<Wire>>,
    cx: &mut App,
) {
    let Some(editor) = editor_slot.borrow().clone() else {
        report("final flush: there was no editor to flush");
        return;
    };
    let edits = editor.read(cx).edits();
    let state = wire.borrow();
    if edits == state.flushed_edits && state.in_flight.is_none() {
        report("final flush: nothing outstanding, the engine already has the buffer");
        return;
    }
    if edits == state.flushed_edits {
        report("final flush: nothing new to send, one flush is still unanswered");
        return;
    }
    if !state.autosave {
        report("final flush: SKIPPED, autosave is off - the text is not saved");
        return;
    }
    if !state.armed {
        // ADR-0001: an un-armed document has never been saved once by an explicit act,
        // and writing it here would create a file the user never asked for. Exit says
        // so, in words, because the alternative is silence and a lost note.
        report(
            "final flush: SKIPPED, the document is not armed for autosave - the text is NOT saved",
        );
        return;
    }
    if editor.read(cx).is_composing() {
        report("final flush: HELD, a composition was open - that text is LOST");
        return;
    }
    drop(state);
    let text = editor.read(cx).text().to_string();
    wire.borrow_mut().flushed_edits = edits;
    report(&format!(
        "final flush: revision {edits}, {} bytes",
        text.len()
    ));
    send(
        gateway,
        Command::Flush {
            text,
            revision: edits,
        },
    );
}

/// THE FOUR MENU COMMANDS, attached to the App. Each one answers through the native
/// platform prompt (gpui-pre-0.3.4 src/app.rs:1582 and :1595, both `&self` on the App,
/// both returning an awaitable receiver) - no kit dialog, no overlay, no `Root`, and no
/// view to hold a subscription.
fn register_menu_commands(
    cx: &mut App,
    gateway: Rc<RefCell<Option<Gateway>>>,
    wire: Rc<RefCell<Wire>>,
    editor_slot: Rc<RefCell<Option<Entity<Editor>>>>,
    view_slot: Rc<RefCell<Option<GpuiWindowHandle<Surface>>>>,
) {
    cx.on_action({
        let gateway = Rc::clone(&gateway);
        let view_slot = Rc::clone(&view_slot);
        move |_: &menu::OpenFile, cx: &mut App| {
            prompt_open(&gateway, &view_slot, cx);
        }
    });
    cx.on_action({
        let gateway = Rc::clone(&gateway);
        let wire = Rc::clone(&wire);
        let editor_slot = Rc::clone(&editor_slot);
        let view_slot = Rc::clone(&view_slot);
        move |_: &menu::SaveAsFile, cx: &mut App| {
            prompt_save_as(&gateway, &wire, &editor_slot, &view_slot, cx);
        }
    });
    cx.on_action({
        let gateway = Rc::clone(&gateway);
        let wire = Rc::clone(&wire);
        let view_slot = Rc::clone(&view_slot);
        move |_: &menu::ToggleAutosave, cx: &mut App| {
            let asking_for = !wire.borrow().autosave;
            // THE TOGGLE CHANGES NOTHING LOCALLY. Not the check mark, not the flush gate.
            // `Event` has no "the setting is now X" variant, so the only honest sources are
            // the engine's own answers: `AutosaveSkipped { AutosaveDisabled }` proves off and
            // a `Saved` proves on. Both are handled in `apply`. A check mark that flipped when
            // we asked would be the pin bug again - the UI believing its own request.
            send(&gateway, Command::SetAutosave(asking_for));
            note_to(
                &view_slot,
                cx,
                &format!(
                    "autosave toggle sent: asking the engine for {}. the check mark waits for the engine's answer",
                    asking_for
                ),
            );
        }
    });
    cx.on_action({
        let gateway = Rc::clone(&gateway);
        let view_slot = Rc::clone(&view_slot);
        move |_: &menu::ClearRecents, cx: &mut App| {
            send(&gateway, Command::ClearRecents);
            note_to(
                &view_slot,
                cx,
                "clear recent files sent; the list is the engine's to answer with",
            );
        }
    });
    // One handler per slot, because a gpui action's identity is its TYPE (app_menu.rs:92
    // boxes `dyn Action`), so ten entries are ten types and the index is compile-time.
    macro_rules! recent {
        ($($type:ty => $index:expr),* $(,)?) => {
            $(
                cx.on_action({
                    let gateway = Rc::clone(&gateway);
                    let wire = Rc::clone(&wire);
                    let view_slot = Rc::clone(&view_slot);
                    move |_: &$type, cx: &mut App| {
                        open_recent($index, &gateway, &wire, &view_slot, cx);
                    }
                });
            )*
        };
    }
    recent!(
        menu::Recent0 => 0,
        menu::Recent1 => 1,
        menu::Recent2 => 2,
        menu::Recent3 => 3,
        menu::Recent4 => 4,
        menu::Recent5 => 5,
        menu::Recent6 => 6,
        menu::Recent7 => 7,
        menu::Recent8 => 8,
        menu::Recent9 => 9,
    );
}

/// A sentence on the status line from outside the view. The handle is the one the run
/// closure filled; before a window exists there is nowhere to say anything, so this is
/// silent rather than wrong.
fn note_to(view_slot: &Rc<RefCell<Option<GpuiWindowHandle<Surface>>>>, cx: &mut App, text: &str) {
    if let Some(handle) = *view_slot.borrow() {
        let _ = handle.update(cx, |view, _window, cx| view.note(text, cx));
    }
}

/// THE FILE MENU'S ONLY ASK: a path from the real `IFileOpenDialog`, then `Open`. The
/// answer comes back as `Loaded` (or `LoadFailed`) and the pump does the rest - the bridge
/// never reads the file itself, which is the seam holding.
fn prompt_open(
    gateway: &Rc<RefCell<Option<Gateway>>>,
    view_slot: &Rc<RefCell<Option<GpuiWindowHandle<Surface>>>>,
    cx: &mut App,
) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: None,
    });
    let gateway = Rc::clone(gateway);
    let view_slot = Rc::clone(view_slot);
    // Detached on purpose: the task IS the dialog's continuation, and there is nothing to
    // await it for - its whole effect is the command it sends and the sentence it writes.
    // Detached: the task is the dialog's continuation, and its whole effect is the command
    // it sends and the sentence it writes. There is nothing to await it for.
    cx.spawn(async move |cx: &mut AsyncApp| {
        let picked = match receiver.await {
            Ok(Ok(Some(mut paths))) => paths.pop(),
            _ => None,
        };
        let Some(path) = picked else {
            // Cancelled: the buffer and its path stay exactly as they were. The status line
            // says so because silence after a key press reads as a hang.
            with_view(&view_slot, cx, |view, cx| {
                view.note(
                    "open cancelled - nothing was loaded and nothing was flushed",
                    cx,
                )
            });
            return;
        };
        let name = path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        send(&gateway, Command::Open { path });
        with_view(&view_slot, cx, move |view, cx| {
            view.note(&format!("asking the engine to open {name}"), cx)
        });
    })
    .detach();
}

/// SAVE AS, and the ordering rule inside it: the text and the revision are read from the
/// buffer BEFORE the dialog, so what lands at the new path is what was on screen when the
/// user asked - and the rebind we then adopt comes from the engine's `Rebound`, never from
/// the path we typed into the chooser.
fn prompt_save_as(
    gateway: &Rc<RefCell<Option<Gateway>>>,
    wire: &Rc<RefCell<Wire>>,
    editor_slot: &Rc<RefCell<Option<Entity<Editor>>>>,
    view_slot: &Rc<RefCell<Option<GpuiWindowHandle<Surface>>>>,
    cx: &mut App,
) {
    let Some(editor) = editor_slot.borrow().clone() else {
        return;
    };
    let text = editor.read(cx).text().to_string();
    let revision = editor.read(cx).edits();
    let (directory, suggested) = {
        let state = wire.borrow();
        match state.path.as_ref().and_then(|path| {
            path.parent().map(|dir| {
                (
                    dir.to_path_buf(),
                    path.file_name()
                        .map(|name| name.to_string_lossy().into_owned()),
                )
            })
        }) {
            Some((dir, name)) => (dir, name),
            None => (std::env::current_dir().unwrap_or_default(), None),
        }
    };
    if text.is_empty() && revision == 0 {
        // Nothing to save is not nothing to say. An empty fresh buffer asked to be renamed:
        // send it anyway - the user may want the file to exist - but say what it holds.
        note_to(
            &view_slot.clone(),
            cx,
            "save as: the buffer is empty, the new file will be too",
        );
    }
    let receiver = cx.prompt_for_new_path(&directory, suggested.as_deref());
    let gateway = Rc::clone(gateway);
    let view_slot = Rc::clone(view_slot);
    // Detached: the task is the dialog's continuation, and its whole effect is the command
    // it sends and the sentence it writes. There is nothing to await it for.
    cx.spawn(async move |cx: &mut AsyncApp| {
        let picked = match receiver.await {
            Ok(Ok(Some(path))) => Some(path),
            _ => None,
        };
        let Some(path) = picked else {
            with_view(&view_slot, cx, |view, cx| {
                view.note(
                    "save as cancelled - the document keeps its current path",
                    cx,
                )
            });
            return;
        };
        let name = path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        send(
            &gateway,
            Command::SaveAs {
                path: path.clone(),
                text,
                revision,
            },
        );
        with_view(&view_slot, cx, move |view, cx| {
            // NOT "saved as": the engine has only been ASKED. `Rebound` is the word that
            // changes what the bridge believes about this buffer's path.
            view.note(
                &format!("save as sent: {name}, awaiting the engine's rebind"),
                cx,
            )
        });
    })
    .detach();
}

/// A numbered recent entry. The entry is looked up in the list THE PORT LAST SENT, at the
/// same index the menu item was built from, so a stale menu can only miss - never open a
/// different file than the one it named.
fn open_recent(
    index: usize,
    gateway: &Rc<RefCell<Option<Gateway>>>,
    wire: &Rc<RefCell<Wire>>,
    view_slot: &Rc<RefCell<Option<GpuiWindowHandle<Surface>>>>,
    cx: &mut App,
) {
    let entry = menu::entry_for(&wire.borrow().recents, index).cloned();
    match entry {
        Some(entry) if entry.exists => {
            send(
                gateway,
                Command::Open {
                    path: entry.path.clone(),
                },
            );
            note_to(
                view_slot,
                cx,
                &format!("asking the engine to open {}", entry.display),
            );
        }
        // The menu disables these, and the click still should not lie about what happened.
        Some(entry) => note_to(
            view_slot,
            cx,
            &format!("{} no longer exists - nothing was opened", entry.display),
        ),
        None => note_to(
            view_slot,
            cx,
            &format!(
                "recent slot {} is empty - the list changed under the menu",
                index + 1
            ),
        ),
    }
}

/// Update the root view from a spawned task, if the window is still up.
fn with_view(
    view_slot: &Rc<RefCell<Option<GpuiWindowHandle<Surface>>>>,
    cx: &mut AsyncApp,
    f: impl FnOnce(&mut Surface, &mut gpui_kit::Context<Surface>),
) {
    let Some(handle) = *view_slot.borrow() else {
        return;
    };
    let _ = handle.update(cx, |view, _window, cx| f(view, cx));
}

fn close(gateway: &Rc<RefCell<Option<Gateway>>>, events: &Rc<RefCell<EventRx>>) {
    if let Some(gateway) = gateway.borrow_mut().take() {
        // Blocking by contract: drain, final session write, join. Legal here
        // because this is the UI thread on its way out, not a frame, and not the
        // engine thread.
        //
        // The port hands back ONE error for two outcomes and the bridge cannot tell
        // them apart: either the queue was already closed, or the bounded join ran out
        // and the engine was ABANDONED inside its own exit. The second is the one a
        // user can lose characters to - and it names the trade: a 3 s abandoned
        // shutdown beats the permanent hang this replaced, which was measured at 12 s
        // timed out and >15 s lived through once with no panic, no log, and no event.
        if gateway.close().is_err() {
            // Do not leave main on an assumption. The port documents Disconnected on
            // the EventRx as the terminal signal that the engine thread is gone
            // (crates/api/src/gateway.rs:312), so the bridge can ASK instead of guessing
            // which of the two outcomes happened - once more, bounded, because the
            // reviewer's interleaving is real: a Flush queued behind Shutdown runs a
            // genuine save (temp write, fsync, rename), and on slow or network storage
            // that can outrun 3 s and the process would otherwise exit mid-syscall. That
            // is silent loss of the newest edit, the worst failure this app can have.
            //
            // 10 s is the price of not hanging forever, not an expectation: an
            // abandoned-but-healthy engine finishes its exit in tens of milliseconds in
            // practice, so the wait almost never runs anywhere near its budget. Parking
            // here is legal precisely because this is not a frame - the rule AGENTS.md
            // states is no blocking on a channel inside a GPUI frame, and by now the
            // frame loop has ended and the pump died with its window.
            if wait_for_engine_exit(events) {
                note_shutdown(
                    "the engine finished its exit after the join deadline had passed - the final save ran",
                );
            } else {
                note_shutdown(
                    "the engine never answered within 10 s of being abandoned - the last edit may not be saved",
                );
            }
        }
    }
}

/// Wait for the engine thread to actually go away, up to a bounded grace period,
/// returning true only if the queue said Disconnected. Every event that arrives while
/// waiting is kept, not swallowed: the exit trace has to name it later, and dropping
/// it here would be exactly the silence this files contract forbids.
fn wait_for_engine_exit(events: &Rc<RefCell<EventRx>>) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let outcome = {
            let guard = events.borrow();
            match guard.try_recv() {
                Ok(event) => Ok(event),
                Err(TryRecvError::Empty) => Err(false),
                Err(TryRecvError::Disconnected) => Err(true),
            }
        };
        match outcome {
            Ok(event) => HOLDOVER.with(|held| held.borrow_mut().push(event)),
            Err(true) => return true,
            Err(false) => {
                if std::time::Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }
}

thread_local! {
    /// A message that arrived when there may be no frame left to render it in. Kept
    /// so the last frame CAN still show it (render asks for it before the status line)
    /// and traced immediately, because `windows_subsystem` means there is no console by
    /// then and the exit trace is the only surviving witness.
    static SHUTDOWN_NOTE: RefCell<Option<SharedString>> = const { RefCell::new(None) };

    /// Events that arrived while the bridge was waiting out an abandoned engine. They
    /// have no frame left to render them, so `final_drain` reports them alongside the
    /// ones it finds itself rather than letting the wait consume them in silence.
    static HOLDOVER: RefCell<Vec<Event>> = const { RefCell::new(Vec::new()) };
}

fn note_shutdown(words: &str) {
    SHUTDOWN_NOTE.with(|note| {
        if note.borrow().is_none() {
            *note.borrow_mut() = Some(SharedString::from(words.to_string()));
        }
    });
    report(words);
}

/// The last look at the queue, once there is no loop and no renderer left, and it is
/// COMPLETE: pass after pass until the queue says it is empty or the sender is gone.
///
/// One capped pass was a lie about this file's own contract ("named in the trace
/// instead of being dropped in silence"): with more than `MAX_DRAIN_PER_WAKE` events
/// queued at exit, event 65 and everything after it left no trace at all. It is
/// reachable for real, because the engine sends its shutdown events after the pump has
/// stopped - a SaveFailed and a batch of RecentsUpdated are enough. The queue is finite
/// by now because `close` has already joined or abandoned the engine, so looping is
/// safe; `MAX_EXIT_PASSES` is there so an ABANDONED engine that keeps sending cannot
/// turn the exit trace into a loop that never ends, and hitting it is reported rather
/// than assumed. Every event still goes through the same bounded, non-blocking
/// `drain_bounded` - the cap stays, only the passing repeats.
fn final_drain(events: &Rc<RefCell<EventRx>>) -> (Vec<Event>, bool) {
    let mut out = HOLDOVER.with(|held| held.borrow_mut().drain(..).collect::<Vec<Event>>());
    for _ in 0..MAX_EXIT_PASSES {
        let drain = {
            let guard = events.borrow();
            drain_bounded(&guard, &mut out, MAX_DRAIN_PER_WAKE)
        };
        if drain.closed {
            return (out, true);
        }
        if !drain.capped {
            // A short pass means the queue was empty at that instant.
            return (out, true);
        }
    }
    (out, false)
}

fn report(why: &str) {
    // `windows_subsystem` means there is no console attached, so this is the
    // last-resort trace until the port grows an Event for an undelivered command.
    eprintln!("notes-gpui: {why}");
}

/// Print the status line to the trace only when it CHANGED. `last` holds what was
/// printed last, or None at startup. Returns true when it printed.
///
/// The comment above this call site always claimed "one line per CHANGED line" and the
/// code printed on every event-taking wake instead. The distinction is not pedantic:
/// the same event arriving repeatedly is the normal case, not the rare one - a 750 ms
/// AutosaveSkipped with an unchanged reason, or a SaveFailed that retries, would each
/// add a line per wake to a redirected log forever. Two states genuinely ALTERNATING
/// still print every one of them, and correctly so: that is information, and suppressing
/// it would be rate limiting, which this is not.
fn record_shown(last: &mut Option<String>, shown: &str) -> bool {
    if last.as_deref() == Some(shown) {
        false
    } else {
        *last = Some(shown.to_string());
        true
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn skipped() -> Event {
        Event::AutosaveSkipped {
            reason: SkipReason::Clean,
        }
    }

    /// The rule the brief names: a deep queue must be taken in bounded PASSES,
    /// iteratively. A recursive drain blew the stack at 2000 queued events; this
    /// asserts the pass is capped, that repeated passes still empty the queue, and
    /// that order is preserved.
    #[test]
    fn a_deep_queue_is_drained_in_bounded_passes() {
        const QUEUED: usize = 5000;
        let (tx, rx) = channel::<Event>();
        for i in 0..QUEUED {
            tx.send(Event::Saved {
                path: PathBuf::from(format!("C:/notes/{i}.notes")),
                revision: i as u64,
            })
            .expect("unbounded queue");
        }
        drop(tx);

        let mut total = 0usize;
        let mut passes = 0usize;
        let mut last_revision = None;
        loop {
            let mut batch = Vec::new();
            let drain = drain_bounded(&rx, &mut batch, MAX_DRAIN_PER_WAKE);
            assert!(
                drain.taken <= MAX_DRAIN_PER_WAKE,
                "a single pass took {} events, more than the cap",
                drain.taken
            );
            for event in &batch {
                // Queue order: the revision numbers only come out ascending if
                // nothing was reordered or lost between the cap boundaries.
                if let Event::Saved { revision, .. } = event {
                    assert!(
                        last_revision.is_none_or(|last| *revision > last),
                        "the queue came out out of order"
                    );
                    last_revision = Some(*revision);
                }
            }
            total += drain.taken;
            passes += 1;
            if drain.closed {
                break;
            }
            assert!(passes < 1000, "the drain stopped making progress");
        }
        assert_eq!(total, QUEUED, "a bounded pass must not lose events");
        // 5000 events / a cap of 64 = 79 passes: 78 of them capped, the last one
        // taking the remaining 8 AND learning the sender is gone in the same pass.
        assert_eq!(passes, QUEUED.div_ceil(MAX_DRAIN_PER_WAKE));
        assert_eq!(passes, 79);
    }

    /// An empty queue answers Empty and costs one call: this is why no frame can
    /// block on it. A closed queue answers Disconnected and says so.
    #[test]
    fn an_idle_queue_returns_immediately_and_a_closed_one_says_so() {
        let (tx, rx) = channel::<Event>();
        let idle = drain_bounded(&rx, &mut Vec::new(), MAX_DRAIN_PER_WAKE);
        assert_eq!(idle.taken, 0);
        assert!(!idle.closed);
        assert!(!idle.capped);

        tx.send(skipped()).expect("unbounded queue");
        let mut out = Vec::new();
        let busy = drain_bounded(&rx, &mut out, MAX_DRAIN_PER_WAKE);
        assert_eq!(busy.taken, 1);
        assert!(matches!(out[0], Event::AutosaveSkipped { .. }));

        drop(tx);
        let closed = drain_bounded(&rx, &mut Vec::new(), MAX_DRAIN_PER_WAKE);
        assert!(closed.closed, "the pump must learn the port has gone");
    }

    /// The control that keeps the bridge and the port from drifting: every variant
    /// has copy, so this builds one of each and renders it. Adding a variant
    /// without an arm in `describe` is a compile error, and this test is what
    /// proves each arm was actually written rather than wildcarded.
    #[test]
    fn every_event_variant_renders_its_own_fields() {
        let cases = vec![
            Event::Loaded {
                path: PathBuf::from("C:/notes/a.notes"),
                text: "two\nlines".to_string(),
                meta: FileMeta {
                    encoding: Encoding::Ansi(1252),
                    line_ending: LineEnding::CrLf,
                    trailing_newline: false,
                    read_only: true,
                    oversize: false,
                    armed: false,
                },
            },
            Event::LoadFailed {
                path: PathBuf::from("C:/notes/gone.md"),
                reason: notes_api::LoadError::NotFound,
            },
            Event::Saved {
                path: PathBuf::from("C:/notes/a.notes"),
                revision: 3,
            },
            Event::SaveFailed {
                path: PathBuf::from("C:/notes/a.notes"),
                revision: 4,
                reason: notes_api::SaveError::DiskFull,
            },
            Event::Rebound {
                path: PathBuf::from("C:/notes/b.notes"),
                meta: FileMeta {
                    encoding: Encoding::Utf16Le,
                    line_ending: LineEnding::Lf,
                    trailing_newline: true,
                    read_only: false,
                    oversize: true,
                    armed: true,
                },
                revision: 7,
            },
            Event::ExternalChange {
                path: PathBuf::from("C:/notes/a.notes"),
            },
            Event::AutosaveSkipped {
                reason: SkipReason::NeedsPath,
            },
            Event::GeometryNotRestored {
                rect: notes_api::Rect::new(120, 90, 800, 600),
                reason: "the platform's own sentence".to_string(),
            },
            Event::RecentsUpdated(vec![RecentEntry {
                path: PathBuf::from("C:/notes/a.notes"),
                display: "a.notes".to_string(),
                exists: false,
            }]),
            Event::SettingsCorrupt {
                reason: "expected a value at line 2".to_string(),
            },
            Event::SessionWriteFailed {
                reason: "Access is denied".to_string(),
            },
            Event::StateDirUnusable {
                reason: "the state directory C:/x/notes-gpui is not a directory".to_string(),
            },
            Event::SettingsWriteFailed {
                reason: "the file is open in another program".to_string(),
            },
        ];
        // One name per variant, in the order above. A new variant without an arm in
        // `describe` never reaches this list, because the match is already a compile
        // error; this is the half that proves each arm says WHICH variant it rendered.
        let names = [
            "Loaded",
            "LoadFailed",
            "Saved",
            "SaveFailed",
            "Rebound",
            "ExternalChange",
            "AutosaveSkipped",
            "GeometryNotRestored",
            "RecentsUpdated",
            "SettingsCorrupt",
            "SessionWriteFailed",
            "StateDirUnusable",
            "SettingsWriteFailed",
        ];
        assert_eq!(
            names.len(),
            cases.len(),
            "the render list and the name list moved apart"
        );
        for (name, event) in names.iter().zip(&cases) {
            let line = describe(event);
            assert!(!line.is_empty());
            assert!(
                line.starts_with(name),
                "the {name} line does not name its kind: {line}"
            );
        }
        // The two reasons that arrive as the port's own words must survive
        // verbatim: the bridge may not paraphrase a refusal it did not make.
        assert!(describe(&cases[7]).contains("the platform's own sentence"));
        assert!(describe(&cases[9]).contains("expected a value at line 2"));
        for (index, words) in [
            (10usize, "Access is denied"),
            (11, "is not a directory"),
            (12, "open in another program"),
        ] {
            assert!(
                describe(&cases[index]).contains(words),
                "variant {index} lost the sentence it was handed"
            );
        }
        // And the payload facts, not just the kind.
        let loaded = describe(&cases[0]);
        assert!(loaded.contains("ansi cp1252") && loaded.contains("crlf"));
        assert!(loaded.contains("read-only") && loaded.contains("not armed"));
        assert!(loaded.contains("newline none"));
        assert!(loaded.contains("9 chars"));
        assert!(describe(&cases[2]).contains("revision 3"));
        assert!(describe(&cases[8]).contains("a.notes (gone)"));
    }

    fn rect_at(x: i32) -> Rect {
        Rect::new(x, 200, 800, 600)
    }

    /// THE DEBOUNCE, as data. A two-second drag at the MEASURED wake cadence
    /// (16.9 ms, from the 15 s idle run) changes the rect on nearly every wake. Sent
    /// raw, that is ~118 commands on an unbounded queue for one gesture - the leak
    /// this slice exists to close. Through `Watch` it costs one.
    #[test]
    fn a_two_second_drag_costs_one_command_not_one_per_frame() {
        let t0 = Instant::now();
        let mut watch = Watch::default();
        assert_eq!(
            watch.observe(rect_at(100), t0),
            None,
            "the first look is the baseline"
        );
        let mut sends = 0usize;
        for step in 1..=118u64 {
            let at = t0 + Duration::from_micros(16_900 * step);
            if watch.observe(rect_at(100 + step as i32), at).is_some() {
                sends += 1;
            }
        }
        assert_eq!(
            watch.motions, 118,
            "every distinct frame of the drag was seen"
        );
        assert!(
            sends <= 1,
            "a 2 s drag of 118 changes cost {sends} commands during the drag itself"
        );
        assert_eq!(
            watch.pending,
            Some(rect_at(218)),
            "the newest rect is the held one"
        );
        // Stop dragging: the held rect goes out on the first wake after the quiet
        // period, and nothing else follows.
        let settled = t0 + Duration::from_micros(16_900 * 119) + GEOMETRY_QUIET;
        assert_eq!(watch.observe(rect_at(218), settled), Some(rect_at(218)));
        assert_eq!(watch.observe(rect_at(218), settled + GEOMETRY_QUIET), None);
        assert_eq!(watch.sends, sends as u64 + 1);
        assert!(watch.sends <= 2, "whole gesture: {} commands", watch.sends);
    }

    /// A drag that never stops must still be recorded, or a slow resize followed by
    /// a crash loses the window. The ceiling is one command per GEOMETRY_FORCE.
    #[test]
    fn an_endless_drag_is_recorded_once_per_force_window() {
        let t0 = Instant::now();
        let mut watch = Watch::default();
        watch.observe(rect_at(0), t0);
        let mut at_ms = Vec::new();
        for step in 1..=600u64 {
            let now = t0 + Duration::from_millis(10 * step);
            if watch.observe(rect_at(step as i32), now).is_some() {
                at_ms.push(now.duration_since(t0).as_millis() as u64);
            }
        }
        assert_eq!(watch.motions, 600, "6 s of 10 ms steps is 600 changes");
        assert!(
            at_ms.len() >= 4 && at_ms.len() <= 6,
            "6 s of unbroken dragging sent {} commands at {at_ms:?}",
            at_ms.len()
        );
        for pair in at_ms.windows(2) {
            assert!(
                pair[1] - pair[0] >= GEOMETRY_FORCE.as_millis() as u64 - 20,
                "forced sends bunched up: {at_ms:?}"
            );
        }
    }

    /// One move, then stillness: exactly one command, and never before the quiet
    /// period has passed. And a drag that ends where it started is not a fact about
    /// the window at all, so it must cost nothing.
    #[test]
    fn a_stopped_move_sends_once_and_a_return_home_sends_nothing() {
        let t0 = Instant::now();
        let mut watch = Watch::default();
        watch.observe(rect_at(500), t0);
        let moved = t0 + Duration::from_millis(20);
        assert_eq!(
            watch.observe(rect_at(700), moved),
            None,
            "too early: still moving"
        );
        assert_eq!(
            watch.observe(rect_at(700), moved + GEOMETRY_QUIET / 2),
            None,
            "half the quiet period is not the quiet period"
        );
        assert_eq!(
            watch.observe(rect_at(700), moved + GEOMETRY_QUIET),
            Some(rect_at(700)),
            "settled: send it"
        );
        assert_eq!(watch.sends, 1);
        // Stay put: a settled rect is not re-sent, however many wakes pass.
        assert_eq!(
            watch.observe(rect_at(700), moved + GEOMETRY_QUIET * 9),
            None,
            "a still window must never be reported twice"
        );
        assert_eq!(watch.sends, 1, "one move, one command");
    }

    /// A drag that comes back to where the port already has the window is not a fact
    /// about the window, so it must cost NOTHING: the held rect is dropped, not sent.
    /// Without this rule a round trip would persist a window to a place it never was.
    #[test]
    fn a_round_trip_before_the_send_costs_no_command_at_all() {
        let t0 = Instant::now();
        let mut watch = Watch::default();
        watch.observe(rect_at(500), t0);
        let out = t0 + Duration::from_millis(20);
        assert_eq!(
            watch.observe(rect_at(700), out),
            None,
            "outbound, still held"
        );
        assert_eq!(watch.pending, Some(rect_at(700)));
        let back = out + Duration::from_millis(20);
        assert_eq!(watch.observe(rect_at(500), back), None, "home again");
        assert_eq!(watch.pending, None, "a round trip is not a change");
        assert_eq!(
            watch.observe(rect_at(500), back + GEOMETRY_QUIET * 4),
            None,
            "the round trip must never wake the port"
        );
        assert_eq!(watch.sends, 0);
    }
    /// The reviewer's Major: `final_drain` used to be ONE capped pass, so 100 events
    /// queued at exit left 36 of them with no trace at all - while the comment at that
    /// call site promised "named in the trace instead of being dropped in silence". The
    /// claim was false for n > 64. The passes now repeat, and this counts them BY NAME.
    #[test]
    fn the_exit_drain_accounts_for_every_queued_event_and_not_one_less() {
        const QUEUED: usize = 100;
        let (tx, rx) = channel::<Event>();
        for index in 0..QUEUED {
            tx.send(Event::Saved {
                path: PathBuf::from(format!("C:/notes/{index}.notes")),
                revision: index as u64,
            })
            .expect("unbounded queue");
        }
        drop(tx);
        let events = Rc::new(RefCell::new(rx));
        let (tail, exhausted) = final_drain(&events);
        assert!(
            exhausted,
            "a finite queue must be reported as fully drained"
        );
        assert_eq!(tail.len(), QUEUED, "100 in, 100 accounted for");
        let mut revisions: Vec<u64> = tail
            .iter()
            .filter_map(|event| match event {
                Event::Saved { revision, .. } => Some(*revision),
                _ => None,
            })
            .collect();
        revisions.sort_unstable();
        for (index, revision) in revisions.iter().enumerate() {
            assert_eq!(
                *revision, index as u64,
                "event {index} was dropped in silence"
            );
        }
    }

    /// The flap, as numbers. A wake that ends on the SAME line is the normal case, not
    /// the rare one: the 750 ms AutosaveSkipped S6 is about to put on a timer, a retry,
    /// a duplicate. Unconditional printing (what the code did while its comment claimed
    /// otherwise) is one line per wake into a redirected log, forever. A genuine
    /// two-state flap still prints every line, and that is the correct limit of this
    /// fix: this is change gating, not rate limiting.
    #[test]
    fn a_repeated_line_prints_once_and_a_real_alternation_prints_every_time() {
        let mut last: Option<String> = None;
        let mut printed = 0usize;
        for _ in 0..200 {
            if record_shown(&mut last, "autosave skipped: nothing to save") {
                printed += 1;
            }
        }
        assert_eq!(printed, 1, "200 wakes on one line must cost ONE trace line");

        let mut last: Option<String> = None;
        let mut printed = 0usize;
        for step in 0..200u64 {
            let line = if step % 2 == 0 { "saved" } else { "edited" };
            if record_shown(&mut last, line) {
                printed += 1;
            }
        }
        assert_eq!(
            printed, 200,
            "an alternating pair of states is information; suppressing it would be rate limiting"
        );
    }

    /// THE WIGGLE, and the decision. A rect oscillating home to away to home faster
    /// than the quiet window never sends and never stops, because every observation of
    /// the reported rect drops pending/first/changed and so restarts the force clock:
    /// the 1 s ceiling the force rule exists to hold is starved indefinitely. The
    /// reviewer is right that it can be starved; the disagreement is whether that is an
    /// accident.
    ///
    /// It is not. A window that keeps returning to where the port already believes it is
    /// has never left, and the position worth persisting is the one it rests at.
    /// Sending the away-rect on a timer during an endless wiggle would store a placement
    /// the user did not choose. Zero commands is the design, and the cost is named:
    /// during a long drag that happens to pass through the resting rect, the port can go
    /// longer than GEOMETRY_FORCE without an update, and the last rect it holds is the
    /// resting one. Reopen if a real drag is ever observed losing an update it should
    /// have made - the fix is to carry `first` across a home visit, paid for with one
    /// stale send at the start of the next episode.
    #[test]
    fn an_endless_wiggle_through_the_resting_rect_owes_nothing_by_design() {
        let t0 = Instant::now();
        let mut watch = Watch::default();
        watch.observe(rect_at(500), t0);
        for step in 0..6000u64 {
            let at = t0 + Duration::from_millis(step);
            let seen = if step % 2 == 0 {
                rect_at(500)
            } else {
                rect_at(508)
            };
            assert_eq!(
                watch.observe(seen, at),
                None,
                "step {step}: a window that keeps coming home must not be reported as moving"
            );
        }
        assert_eq!(
            watch.motions, 3000,
            "the motion was counted, so the silence is visible on the counters line"
        );
        let stop = t0 + Duration::from_millis(6000);
        assert_eq!(
            watch.observe(rect_at(508), stop),
            None,
            "still inside the quiet window"
        );
        assert_eq!(
            watch.observe(rect_at(508), stop + GEOMETRY_QUIET),
            Some(rect_at(508)),
            "and the same watch reports the moment the movement rests away from home"
        );
    }
    /// Every condition D51 and the revision rule put on a `Flush`, in one place: an
    /// un-armed document, a composition, an idle that has not run out, and an
    /// unanswered flush each hold it back on their own.
    #[test]
    fn a_flush_needs_change_quiet_and_an_answered_predecessor() {
        let quiet = Duration::from_millis(800);
        let mut wire = Wire::new(true);
        wire.armed = true;
        wire.loaded = true;
        assert!(!flush_due(&wire, false, quiet), "clean: nothing to send");
        wire.seen_edits = 1;
        assert!(
            flush_due(&wire, false, quiet),
            "dirty, armed, quiet, nothing in the air"
        );
        assert!(
            !flush_due(&wire, true, quiet),
            "a composition holds it (D51)"
        );
        wire.in_flight = Some(1);
        assert!(
            !flush_due(&wire, false, quiet),
            "one unanswered flush is enough"
        );
        wire.in_flight = None;
        wire.flushed_edits = 1;
        assert!(
            !flush_due(&wire, false, quiet),
            "the engine already has this text"
        );
        wire.seen_edits = 2;
        wire.armed = false;
        wire.loaded = true;
        assert!(
            !flush_due(&wire, false, quiet),
            "an un-armed document never flushes (ADR-0001)"
        );
        wire.armed = true;
        assert!(
            !flush_due(&wire, false, Duration::from_millis(100)),
            "and not before the idle has run out"
        );
    }
}
