//! STRIP-2a: the PLUMBING - how a needle gets printed, how a line ending is normalised, how a
//! command reaches the port, and how the two window decorations (title strip, unsaved dot) are
//! written. These are not decisions, and they are not the probe's either: they are the small
//! mechanical layer every root needs, which is why surface.rs used to reach them with a glob
//! import of the crate root and a comment saying "that is not hygiene". This file is the answer to
//! that comment - the reach is now an import list naming seven things.
//!
//! The rule the moved bodies keep: report() is the ONLY writer of the `notes-gpui: ` prefix (the
//! smoke contract), lf() is the ONLY line-ending normaliser, publish_title()/note_dot() are the
//! only writers of the strip and the dot, send() never blocks. Nothing here decides anything about
//! documents; see surface.rs for that half.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use notes_api::{Command, DropGuard, Event, Gateway, Rect, WindowHandle, arm_file_drop};
// STRIP-2b: hwnd_of came with its trait, which is the whole reason the raw-window-handle
// dependency is used by BOTH roots rather than by the probe alone.
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::Spike;
use crate::surface::Pump;
use crate::title_contract;
/// The same voice as the gpui bridge - see the header for why the name stays.
pub(crate) fn report(why: &str) {
    eprintln!("notes-gpui: {why}");
}
pub(crate) fn dialog_allowed() -> bool {
    DIALOG_ALLOWED_BY_DEFAULT && std::env::var_os("SLINT_NO_DIALOG").is_none()
}
/// THE TITLE, from the port's fact and nowhere else. `title_words` is what the strip
/// centre shows; `window_title` is what Alt+Tab, the taskbar preview and a screen
/// reader speak. Both come from `title_contract`, the same two pure functions
/// bridge-gpui calls (its main.rs:851-861), so the wording cannot drift between
/// bridges - and both are PRINTED, because that print is the only honest measurement
/// of this mount a headless run can make: no screenshot was taken.
pub(crate) fn publish_title(ui: &Spike, path: Option<&Path>, loaded: bool, via: &str) {
    let words = title_contract::title_words(path, loaded);
    let title = title_contract::window_title(path, loaded);
    ui.set_title_words(words.clone().into());
    ui.set_os_title(title.clone().into());
    report(&format!("title: {via} words={words:?} os-title={title:?}"));
}
/// LF-normalise an incoming buffer, byte-for-byte the rule `bridge-gpui` states at
/// editor.rs:2026 - CRLF collapses to LF AND a lone CR becomes LF, because core keeps
/// LF internally and the do-no-harm rule restores the file's own ending at the save
/// layer. The bridge never adds a `\r`, and now never leaks one into a Flush.
pub(crate) fn lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}
/// ONE NEEDLE PER SIGNAL CHANGE: the exact triple Chrome renders, printed, so a run is
/// gradable on transitions instead of on a screenshot. 'via' says which input moved
/// (start / buffer / saved / save-failed), and a transition that prints nothing is a
/// signal that is not wired.
pub(crate) fn note_dot(pump: &RefCell<Pump>, weak: &slint::Weak<Spike>, dirty: bool, via: &str) {
    let (words, save_failed, autosave) = {
        let mut p = pump.borrow_mut();
        p.dirty = dirty;
        (
            format!(
                "dot={} save-failed={} dirty={} autosave={}",
                dot_word(p.save_failed, dirty),
                p.save_failed,
                dirty,
                p.autosave
            ),
            p.save_failed,
            p.autosave,
        )
    };
    // PUBLISH, not just print: the triple Chrome draws is the same three facts, and one
    // writer for both. Set unconditionally - Chrome's dot precedence and its menu check
    // are bindings, so an unchanged value costs nothing and a changed one cannot be lost
    // to a printed-diff heuristic that runs before the property lands.
    if let Some(ui) = weak.upgrade() {
        ui.set_dirty(dirty);
        ui.set_save_failed(save_failed);
        ui.set_autosave(autosave);
    }
    let previous = {
        let mut p = pump.borrow_mut();
        std::mem::replace(&mut p.dot_words, words.clone())
    };
    if previous != words {
        report(&format!("chrome: {via}: {words} (was: {previous})"));
    }
}
pub(crate) fn send(gateway: &Rc<RefCell<Option<Gateway>>>, command: Command) {
    let borrowed = gateway.borrow();
    let Some(gateway) = borrowed.as_ref() else {
        return;
    };
    if gateway.send(command).is_err() {
        report("the engine had already exited: a command came back undelivered");
    }
}
/// The event's name, plus its rect when it has one.
pub(crate) fn describe(event: &Event) -> String {
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

/// The headless defence, same shape as SYNTHETIC_CLOSE_ACTS, with one difference that had to be
/// said out loud: a const cannot be set by an environment, and this gate exists precisely so a
/// CI/smoke/probe run can stand the modal down without an edit. So the const is the shipped
/// DEFAULT (a real bridge does show a dialog) and SLINT_NO_DIALOG=1 is the per-run gate. Every
/// probe run in this slice's evidence was made with the gate ON.
pub(crate) const DIALOG_ALLOWED_BY_DEFAULT: bool = true;

/// The dot's word for the pair, in section 4.4's precedence: a failed save outranks
/// plain dirt, and neither paints at all on a clean buffer.
pub(crate) fn dot_word(save_failed: bool, dirty: bool) -> &'static str {
    if save_failed {
        "amber(save-failed)"
    } else if dirty {
        "red(dirty)"
    } else {
        "none(clean)"
    }
}

/// The other half of the seam: read the seeded file again and say whether a single
/// byte moved. Print the flush counter too, because "no spurious Flush" is the
/// mechanism by which it should not have moved.
///
/// S9 follow-up: this used to run ONLY at the END deadline, which made it invisible to
/// how the run is closed - a granted Quit ends the loop before 25 s and the needle simply
/// never printed, so the do-no-harm rule went unwitnessed in exactly the runs that quit
/// early. It is now called on the SAVE CYCLE instead: after the seed's `Event::Loaded`
/// adoption (the open must not have touched it) and after every `Event::Saved` whose path
/// IS the seed (a save of a CRLF foreign file is the moment the rule can actually break).
/// `via` names the moment, because three identical needles would be ambiguous.
pub(crate) fn do_no_harm(pump: &RefCell<Pump>, via: &str) {
    let p = pump.borrow();
    let Some(seed) = p.seed.clone() else {
        report("do-no-harm: no seed file was written, nothing to compare");
        return;
    };
    let bytes = std::fs::read(&seed).unwrap_or_default();
    let after = fnv1a(&bytes);
    report(&format!(
        "do-no-harm[{via}]: {} bytes fnv={after:#x} (seeded {} bytes fnv={:#x}) hash-equal={} flush-since-open={}",
        bytes.len(),
        p.hash_len,
        p.hash_before,
        u8::from(after == p.hash_before && bytes.len() == p.hash_len),
        p.edits - p.flushes_at_open
    ));
}

/// FNV-1a over the file's own bytes. A checksum rather than a hash crate on purpose:
/// this bridge may not add a dependency to prove a byte-for-byte claim, and the
/// question is only ever "did anything change at all".
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Rect plus show state - the same two halves the gpui bridge diffs, because a
/// maximise can leave the rect alone. Both reads are already PHYSICAL here
/// (`position() -> PhysicalPosition`, `size() -> PhysicalSize`, i-slint-core
/// api.rs:562 / :576), so unlike the restore path there is no scale to apply.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Fingerprint {
    pub(crate) rect: Rect,
    pub(crate) maximized: bool,
    /// S8: the third thing a window can be doing to itself, measured not assumed: minimizing
    /// moves the rect to Windows' parking place (-32000,-32000) AND shrinks the reported size
    /// to 160x28, so a minimized window is visible in the rect after all - but as an OS
    /// convention nobody documented to us, one that a restore-later or a different DPI could
    /// change. The bit is explicit, free, and the thing the caption button needs to be provable
    /// from the log, which is this spike's only eye. 1.17 has both calls (i-slint-core
    /// api.rs:608 is_minimized, :613 set_minimized): no winit workaround, no window-handle FFI,
    /// one call each way.
    pub(crate) minimized: bool,
}

/// S6b: THE DROP TARGET, armed the only way the port allows it to be armed.
///
/// WHY HERE AND NOT IN THE ENGINE. `arm_file_drop` executes on the calling thread, and the
/// contract that comes back with it is blunt: the caller must be the thread that pumps the
/// window, because `OleInitialize` builds the COM apartment of whoever calls it and
/// `RegisterDragDrop` binds the target to the window's owner. The engine looks like its natural
/// home - `Command::RegisterWindow` already receives this exact handle - and arming there would
/// register from a thread that pumps nothing. The callbacks would then wait for a pump inside the
/// drag loop Windows drives for the SENDER: somebody else's Explorer frozen mid-drag, with no
/// error returned anywhere in this process. A wrong thread here is not a degraded window, so the
/// bridge arms it, on the thread that owns the window, at the moment it already holds the handle.
///
/// WHAT A DROP WILL MEAN, the semantics this call buys. The engine turns a dropped path into
/// `Command::Open`, so a drop is BIT-IDENTICAL to every other way this app opens a file: the same
/// unsaved-buffer policy, the same autosave arming rules (ADR-0001 - a drop does not arm
/// autosave on a foreign file any more than Ctrl+O does), the same recents, the same epoch and
/// generation bookkeeping, the same undo quarantine. That is consistency, not an oversight:
/// §4.4 promised a drop opens a note, and an open that behaved differently depending on which
/// door it came in through is the bug a user would call us for. Nothing in this file decides what
/// a path means - the port deliberately has no Event and no PathBuf in this signature, so it
/// could not decide even by accident.
///
/// NO ENV GATE, unlike the dialog: an `IDropTarget` nobody drags onto costs one registration and
/// says nothing, so arming is harmless headless - and gating it would mean the code path that
/// matters in production is not the code path any run ever exercises.
pub(crate) fn arm_drop_target(hwnd: i64, holder: &Rc<RefCell<Option<DropGuard>>>) {
    if holder.borrow().is_some() {
        // Both registration sites can fire in one run (first-visible, then appeared-in-the-loop).
        // A second arm would install a new guard while the old one is still held, and the old
        // guard's drop calls the platform's disarm on the SAME hwnd - revoking the target the
        // second arm had just registered. So the second site reports instead of unwinding.
        report(&format!(
            "drop: already armed (hwnd {hwnd:#x} was named twice; a second arm would have disarmed the first)"
        ));
        return;
    }
    match arm_file_drop(WindowHandle(hwnd)) {
        Ok(guard) => {
            // S4 (bridge side): WHOSE TARGET IT TOOK, read off the live guard before it moves
            // into the holder. The bool is the platform's answer, and the bridge does not
            // interpret it: `false` covers two different worlds - nothing was registered at all,
            // and we displaced our own target on a re-arm (file_drop.rs:321 says so, and calls
            // that stealing rather than taking over) - so the word below is the port's wording,
            // chosen to stay true under either reading rather than to claim a cause we cannot
            // see. `true` is the interesting case for THIS bridge: winit registers its own
            // OLE target first, so taking it over means our copy of the cursor rules from here
            // on, and a drag that once did nothing now arrives as Command::Open.
            let took_over = guard.took_over();
            *holder.borrow_mut() = Some(guard);
            let whose = if took_over {
                "took the toolkit drop target; our copy cursor is the law now"
            } else {
                "quiet - nothing was registered"
            };
            report(&format!(
                "drop: armed hwnd={hwnd:#x} ({whose}) - a file dragged onto this window now arrives as Command::Open, exactly like a menu Open"
            ));
        }
        Err(e) => {
            // Rendered honestly, never silence. A window that cannot receive drops is one missing
            // convenience, not a broken app, so the answer is a line the reader can act on - and
            // the guard stays None, which is what makes the exit needle say so too.
            report(&format!(
                "drop: ARM FAILED - {e}, dragging files onto the window will do nothing"
            ));
        }
    }
}

/// What the PORT said the window was, straight out of `InitialState`: one string, so the
/// startup print and the first-visible print cannot disagree with each other, and neither
/// one reaches into a state directory the bridge does not own. `measured()` stays where its
/// question is genuinely "what did the port WRITE to disk" - the persistence probes.
pub(crate) fn port_said(rect: &Rect, scale: f32, maximized: bool, pinned: bool) -> String {
    format!(
        "port says {}x{} at {},{} scale {scale} maximized={maximized} pinned={pinned}",
        rect.w, rect.h, rect.x, rect.y
    )
}

/// The HWND. Slint's own `window_handle()` is infallible and returns ITS handle
/// object; the raw-window-handle question is the INNER call, which is a `Result`.
/// That inner answer is what risk 3 is about, so it is reported separately.
pub(crate) fn hwnd_of(window: &slint::Window) -> Option<i64> {
    // Two steps, and the first must be a binding: the outer handle owns what the inner
    // one borrows.
    let outer = window.window_handle();
    let handle = outer.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(win32) => Some(win32.hwnd.get() as i64),
        _ => None,
    }
}

pub(crate) fn fingerprint_of(window: &slint::Window) -> Fingerprint {
    let position = window.position();
    let size = window.size();
    Fingerprint {
        rect: Rect::new(position.x, position.y, size.width, size.height),
        maximized: window.is_maximized(),
        minimized: window.is_minimized(),
    }
}
