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

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    AnyWindowHandle, App, Application, AsyncApp, Bounds, Context, IntoElement, Point, Render,
    SharedString, Subscription, Task, TitlebarOptions, WeakEntity, Window, WindowBounds,
    WindowOptions, div, px, rgb, size,
};
use notes_api::{
    Command, Encoding, Event, EventRx, FileMeta, Gateway, InitialState, LineEnding, RecentEntry,
    Settings, SkipReason, StateDir, WindowHandle, resolve_state_dir,
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
    /// Times the cap truncated a drain. Non-zero is a queue running ahead of the
    /// UI, which is a fact worth being able to see rather than a silent case.
    truncations: u64,
    /// The last line the status line showed. Kept here as well as on the view so
    /// the exit trace can name it: this binary is `windows_subsystem`, so after the
    /// window is gone the only place a rendered fact can still be read is stderr.
    last_shown: Option<String>,
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
}

impl Surface {
    fn new(events: Rc<RefCell<EventRx>>, stats: Rc<RefCell<Pump>>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            events,
            stats,
            _pump: None,
            status: SharedString::from("no event from the port yet"),
        };
        this.start_pump(cx);
        this
    }

    /// THE ONE WAKE ROUTE. A single main-thread Task, awaited on GPUI's own timer,
    /// draining on wake. See the module comment for the dispatcher path.
    fn start_pump(&mut self, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(ENGINE_POLL).await;
                let keep_running = match this.update(cx, |this, cx| this.pump(cx)) {
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

    /// One wake: drain, render, and ask for a repaint ONLY if there was something
    /// to show. Notifying on an idle wake would turn 125 polls a second into 125
    /// frames a second, which is the mistake this function exists to avoid.
    fn pump(&mut self, cx: &mut Context<Self>) -> bool {
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
                stats.truncations += 1;
            }
        }

        // LAST event wins the line: `batch` is in queue order, so its final entry
        // is the newest fact the port has given us.
        for event in &batch {
            self.status = SharedString::from(one_line(describe(event)));
        }
        if drain.closed {
            // Terminal, and said out loud rather than left as a stale line.
            self.status = SharedString::from("the port has closed: no more events");
        }
        if drain.taken > 0 || drain.closed {
            let shown = self.status.to_string();
            self.stats.borrow_mut().last_shown = Some(shown.clone());
            // The rendered line goes on the last-resort trace as it is rendered,
            // not only at exit: "what did the status line show" has to be answerable
            // from a real run, and a `windows_subsystem` binary has no other voice.
            // One line per CHANGED line, so an idle app never writes.
            report(&format!("status line: {shown}"));
            cx.notify();
        }
        !drain.closed
    }
}

impl Render for Surface {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Nothing invented here: GPUI 0.2.2 has no `Label` widget (its own text
        // elements are in src/elements/text.rs - `impl Element for &'static str` at
        // :19, `impl IntoElement for String` at :77, and `impl Element for
        // SharedString` at :85 / `impl IntoElement for SharedString` at :135, which
        // lay the string out through `TextLayout`). So a `SharedString` child IS
        // the label, and the `Div` inherits its text style down to children
        // (src/elements/div.rs:1334, `window.with_text_style`).
        let counters = self.counters();
        div()
            .size_full()
            .flex_col()
            .bg(rgb(0x1f1f1f))
            .text_color(rgb(0xe6_e6_e6))
            .child(div().flex_1())
            .child(
                div()
                    .flex_none()
                    .flex_col()
                    .bg(rgb(0x14_14_14))
                    .text_size(px(13.0))
                    .whitespace_nowrap()
                    .child(self.status.clone())
                    .child(counters),
            )
    }
}

impl Surface {
    /// The second line: the pump's own cost, live, because "it is cheap" is not
    /// evidence.
    fn counters(&self) -> SharedString {
        let stats = self.stats.borrow();
        let total = stats.busy + stats.idle;
        SharedString::from(format!(
            "pump: {total} wakes ({busy} with work, {idle} idle) · {events} events · slowest drain {slowest} us · {trunc} capped · cap {cap} · poll {poll} ms",
            busy = stats.busy,
            idle = stats.idle,
            events = stats.events,
            slowest = stats.slowest_us,
            trunc = stats.truncations,
            cap = MAX_DRAIN_PER_WAKE,
            poll = ENGINE_POLL.as_millis(),
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
    // Shared with the view so the cost can be reported after the view is gone.
    let stats = Rc::new(RefCell::new(Pump::default()));
    // A dropped Subscription unsubscribes, so the close handler needs somewhere to
    // live for the whole loop rather than for one call.
    let subscriptions = Rc::new(RefCell::new(Vec::<Subscription>::new()));

    Application::new().run({
        let gateway = Rc::clone(&gateway);
        let events = Rc::clone(&events);
        let stats = Rc::clone(&stats);
        let subscriptions = Rc::clone(&subscriptions);
        move |cx: &mut App| {
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
                move |_, cx| cx.new(|cx| Surface::new(Rc::clone(&events), Rc::clone(&stats), cx))
            });
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

            // The old startup drain is gone: the pump drains on its first wake,
            // 8 ms from now, and renders what the startup produced instead of
            // discarding it.

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
    // The pump died with its window, so anything the engine said during its own
    // shutdown has nowhere to be rendered. It is named in the trace instead of
    // being dropped in silence, which is what the previous two drains did.
    let tail = final_drain(&events);
    let reported = stats.borrow();
    report(&format!(
        "pump: {} wakes, {} had work, {} events, slowest drain {} us, {} capped drains",
        reported.busy + reported.idle,
        reported.busy,
        reported.events,
        reported.slowest_us,
        reported.truncations,
    ));
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

/// The last look at the queue, once there is no loop and no renderer left. Same
/// bounded, non-blocking shape as the pump; the result goes to the trace.
fn final_drain(events: &Rc<RefCell<EventRx>>) -> Vec<Event> {
    let mut out = Vec::new();
    let guard = events.borrow();
    drain_bounded(&guard, &mut out, MAX_DRAIN_PER_WAKE);
    out
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
}
