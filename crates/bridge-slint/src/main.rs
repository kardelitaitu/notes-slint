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
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use notes_api::{
    Command, DropGuard, Event, Exit, Gateway, Rect, Settings, StateDir, WindowHandle,
    arm_file_drop, resolve_state_dir,
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

/// What the PORT said the window was, straight out of `InitialState`: one string, so the
/// startup print and the first-visible print cannot disagree with each other, and neither
/// one reaches into a state directory the bridge does not own. `measured()` stays where its
/// question is genuinely "what did the port WRITE to disk" - the persistence probes.
fn port_said(rect: &Rect, scale: f32, maximized: bool, pinned: bool) -> String {
    format!(
        "port says {}x{} at {},{} scale {scale} maximized={maximized} pinned={pinned}",
        rect.w, rect.h, rect.x, rect.y
    )
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
fn arm_drop_target(hwnd: i64, holder: &Rc<RefCell<Option<DropGuard>>>) {
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

/// Rect plus show state - the same two halves the gpui bridge diffs, because a
/// maximise can leave the rect alone. Both reads are already PHYSICAL here
/// (`position() -> PhysicalPosition`, `size() -> PhysicalSize`, i-slint-core
/// api.rs:562 / :576), so unlike the restore path there is no scale to apply.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Fingerprint {
    rect: Rect,
    maximized: bool,
    /// S8: the third thing a window can be doing to itself, measured not assumed: minimizing
    /// moves the rect to Windows' parking place (-32000,-32000) AND shrinks the reported size
    /// to 160x28, so a minimized window is visible in the rect after all - but as an OS
    /// convention nobody documented to us, one that a restore-later or a different DPI could
    /// change. The bit is explicit, free, and the thing the caption button needs to be provable
    /// from the log, which is this spike's only eye. 1.17 has both calls (i-slint-core
    /// api.rs:608 is_minimized, :613 set_minimized): no winit workaround, no window-handle FFI,
    /// one call each way.
    minimized: bool,
}

fn fingerprint_of(window: &slint::Window) -> Fingerprint {
    let position = window.position();
    let size = window.size();
    Fingerprint {
        rect: Rect::new(position.x, position.y, size.width, size.height),
        maximized: window.is_maximized(),
        minimized: window.is_minimized(),
    }
}

/// S10: the dirty-witness switch. True means TextEdit.edited is the authority and a tick costs
/// one borrow; false means the pre-S10 full-buffer compare runs. Both paths compile and
/// neither is dead code, deliberately.
// MEASURED, not assumed: with this TRUE a live run printed ZERO flush lines. TextEdit.edited is
// a USER-input signal, and this bridge writes the buffer programmatically (set_buffer) for its
// keystroke acts - which is exactly what a person typing does not do - so the flag never set
// and autosave quietly stopped. That answers item 3: the event is a fine ADDITION (it stamps
// the debounce clock the moment a real key arrives, and the handler stays wired for that) and a
// broken REPLACEMENT, because anything the bridge writes itself - a dialog-suggested body, a
// future insert, a restore into the buffer - would never be saved. So the compare remains the
// authority and the cost item 3 asked about is UNPAID: about 125 buffer reads a second, in
// exchange for a witness that cannot lie about programmatic writes. The way to pay it later is
// a flag the bridge sets wherever it writes the buffer itself, not one the widget withholds.
const EDITED_IS_DIRTY_WITNESS: bool = false;

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
/// S7: the overlay acts - open, clamp at three host widths, click-away, Escape. Spaced, not
/// instantaneous: a resize only reaches Chrome's clamp after the next layout pass, so reading
/// popup-x in the same statement that asked for the new size would print the old one. The
/// window is put back to its starting size before the act ends, because the NEXT launch
/// restores whatever size this one persisted.
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

// ==== S9: THE NATIVE DIALOG, AND THE ONE RULE IT MAY NOT BREAK ====
//
// The rule: never block the Slint event loop. rfd's pick_file/save_file are SYNCHRONOUS and
// modal, so calling one from a handler freezes pump, autosave, drain and every tick needle for
// as long as the user takes - the same class of failure as "never block on a channel inside a
// frame", with a person instead of a deadlock. So the dialog lives on a thread of its own and
// answers over a channel whose Receiver is polled by try_recv in the per-tick drain: no await,
// no new timer, no callback the toolkit has to be alive to deliver.
//
// THE PARENT, and this is the finding the brief predicted. rfd 0.16 offers exactly one way to
// own a dialog to a window - FileDialog::set_parent(&W) where W: HasWindowHandle +
// HasDisplayHandle (file_dialog.rs:96); there is no set_parent_handle and DialogHandle is not
// public. The only handle this bridge can reach is slint::Window::window_handle(), whose
// return type BORROWS the window (hwnd_of at the top of this file lives inside one statement
// for exactly that reason), so it cannot be moved into a spawned thread: the compiler says
// "closure may outlive the current function, but it borrows". Consequence, and it is a real
// one, not cosmetic: THE DIALOG OPENS UNPARENTED - it is not modal to our window, it can be
// left behind by an alt-tab, and it is not the child that would move with the window. A shipped
// bridge fixes this at the platform layer (notes-platform owns the HWND and could take it as a
// raw isize and call the Win32 API itself), which is an ADR conversation, not a line here. So:
// unparented, said here, said in the needle, and rfd still gets the starting directory and the
// suggested name, which is most of what a dialog being owned buys the user.
//
// WHAT THE COMPILER SAID, since the attempt is gone and its words are the evidence: spawning
// rejected slint::WindowHandle three times over - Rc of dyn WindowAdapter cannot be sent between
// threads safely, dyn HasWindowHandle cannot be SHARED between threads safely, and the type is
// not Send either (i-slint-core api.rs:414 WindowHandleInner, :429 WindowHandle), required by
// this bound in spawn. Not a borrow-lifetime complaint but an auto-trait verdict: the handle is
// deliberately not thread-safe, because a window adapter is single-threaded by design.
//
// THE WAY ROUND, named so nobody rediscovers it as a clever idea: carry a plain isize HWND into
// the thread and implement the two traits on a local newtype, which is what rfd actually wants
// (it reads the raw handle at call time, file_dialog.rs:100). That takes a NonNull over a pointer
// nobody here owns - unsafe - and AGENTS.md puts unsafe in notes-platform, not in a bridge. Right
// rule, and it names where the fix belongs: notes-platform already holds the HWND it was given by
// Command::RegisterWindow, so a Send-able owned-handle answer is a port-and-platform conversation,
// not a line in this file. Until then: unparented, on purpose, out loud.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DialogKind {
    Open,
    SaveAs,
}

impl DialogKind {
    fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::SaveAs => "save as",
        }
    }
}

/// What the picker thread sends back: a path, or the absence of one. The absence IS the
/// cancel - rfd returns None for it, and the rule below is that it must never be silence.
struct DialogReply {
    kind: DialogKind,
    path: Option<PathBuf>,
}

/// The headless defence, same shape as SYNTHETIC_CLOSE_ACTS, with one difference that had to be
/// said out loud: a const cannot be set by an environment, and this gate exists precisely so a
/// CI/smoke/probe run can stand the modal down without an edit. So the const is the shipped
/// DEFAULT (a real bridge does show a dialog) and SLINT_NO_DIALOG=1 is the per-run gate. Every
/// probe run in this slice's evidence was made with the gate ON.
const DIALOG_ALLOWED_BY_DEFAULT: bool = true;

fn dialog_allowed() -> bool {
    DIALOG_ALLOWED_BY_DEFAULT && std::env::var_os("SLINT_NO_DIALOG").is_none()
}

/// Where the picker starts. The current file's directory first - the port's recents list is
/// ordered most-recent-first, so recents[0] IS the file in the window, and after a Clear it is
/// the next best thing - then nothing, which leaves rfd on its own default.
fn dialog_starting_dir(current: Option<&PathBuf>, recents: &[PathBuf]) -> Option<PathBuf> {
    let from = current.or_else(|| recents.first());
    let dir = from.and_then(|path| path.parent())?;
    (!dir.as_os_str().is_empty()).then(|| dir.to_path_buf())
}

/// The name the save dialog offers. The bridge already computes the title's words for the OS
/// title and the strip, so the dialog asks for the same thing the user can already read rather
/// than inventing a third name; an extension is added only when the word has none, because
/// rfd's suggestion is a whole file name on Windows.
fn suggested_name(title_words: &str) -> String {
    let trimmed = title_words.trim();
    let base = if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        // The path type knows both separators on Windows; a hand-rolled split by one of them
        // is the kind of bug that only shows up on the other machine. A full path in, a bare
        // file name out, and words that are not a path come back as they were.
        std::path::Path::new(trimmed)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| trimmed.to_string())
    };
    if base.contains(".") {
        base.to_string()
    } else {
        format!("{base}.notes")
    }
}

/// The one place the wording of a dialog answer lives, so the status line and the log cannot
/// disagree about whether the user picked or cancelled.
fn dialog_words(kind: DialogKind, path: &Option<PathBuf>) -> String {
    match path {
        Some(chosen) => format!(
            "{}: {}",
            kind.label(),
            chosen
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| chosen.display().to_string())
        ),
        None => format!("{}: cancelled", kind.label()),
    }
}

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
            if ostep < 8 {
                let at = OVERLAY_AT + OVERLAY_EVERY * (ostep as u32);
                if now >= at {
                    third_pump.borrow_mut().overlay_step = ostep + 1;
                    let w = ui.window();
                    if ostep > 0 {
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
                        _ => ui.set_close_asks(ui.get_close_asks() + 1),
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
/// S10, ITEM 3: the per-tick cost, fixed. Until now the pump read the WHOLE buffer out of the
/// widget, re-allocated it through lf(), and compared it against last_sent - on every 8 ms
/// tick, about 125 reads a second, including the ~99.9% where nothing had happened. TextEdit
/// has been saying so all along: the edited callback fires on every keystroke, and main.slint
/// now forwards it. So a keystroke sets one bool and stamps the debounce clock, and a tick that
/// finds the flag clear does ONE borrow and returns. The string is read once, when a flush is
/// due, and that read is where the comparison keeps the one job an event cannot do. Text
/// edited and then edited back to identical must not produce a save, and only the bytes can
/// say so.
fn text_pump(ui: &Spike, gw: &Rc<RefCell<Option<Gateway>>>, pump: &RefCell<Pump>) {
    // S8b: THE REFUSAL, deliberately ABOVE the witness dispatch so neither path can escape it.
    // A locked document is one the port has already decided it will not save - the disk flag or
    // D9's 8 MiB verdict - so sending is not merely useless: the log would print a flush for a
    // save whose only possible answer is `SaveError::ReadOnly`, which is a record of an attempt
    // the engine refuses to make. `last_sent` is kept IN SYNC and the edited witness and debounce
    // clock cleared, so no phantom dirty survives the refusal; the buffer is remembered as what
    // the pump WOULD have sent, and the dot is forced clean once, because LOCK beats DIRTY - a
    // file that can never be saved has no unsaved state to advertise. Printed once per lock, not
    // 125 times a second, because noise is how a finding stops being read. The honest cost: this
    // branch reads the buffer every tick while locked, which is exactly the per-tick cost S10
    // removed. Rare by definition, and it buys the no-phantom-dirty property the refusal needs;
    // a locked document that later unlocks must not come back 300 edits behind.
    let locked = {
        let p = pump.borrow();
        p.locked
    };
    if locked {
        let first = {
            let mut p = pump.borrow_mut();
            let first = !p.lock_needled;
            p.lock_needled = true;
            p.last_sent = lf(&ui.get_buffer());
            p.edited_flag = false;
            p.pending_at = None;
            first
        };
        if first {
            note_dot(pump, &ui.as_weak(), false, "locked");
            let word = pump.borrow().lock_word.clone();
            report(&format!(
                "flush[refused]: {word} - the port will not save this document; the buffer is tracked as sent, the dot is clean, and autosave never fires while it is locked"
            ));
        }
        return;
    }
    if !EDITED_IS_DIRTY_WITNESS {
        // The fallback is a const flip, not a redesign: the old path is still in this file,
        // byte for byte, as text_pump_by_compare.
        text_pump_by_compare(ui, gw, pump);
        return;
    }
    let (flag, due) = {
        let mut p = pump.borrow_mut();
        p.invocations += 1;
        if !p.edited_flag {
            return;
        }
        let entered = *p.pending_at.get_or_insert_with(Instant::now);
        (true, entered.elapsed() >= AUTOSAVE_IDLE)
    };
    // The dot and the flush guard are witnesses of one fact, so the event feeds both: the same
    // single source of truth the comparison used to be, unchanged in kind.
    note_dot(pump, &ui.as_weak(), flag, "buffer");
    if !due {
        return;
    }
    let text = lf(&ui.get_buffer());
    let identical = {
        let p = pump.borrow();
        text == p.last_sent
    };
    if identical {
        let mut p = pump.borrow_mut();
        p.edited_flag = false;
        p.pending_at = None;
        drop(p);
        note_dot(pump, &ui.as_weak(), false, "compare");
        report("flush[skipped]: edited and edited back - the bytes are identical, nothing sent");
        return;
    }
    let mut pump = pump.borrow_mut();
    pump.edits += 1;
    let (revision, epoch) = (pump.edits, pump.epoch);
    pump.last_sent = text.clone();
    let quiet = pump.pending_at.map_or_else(
        || String::from("unknown"),
        |at| format!("{:?}", at.elapsed()),
    );
    let cr = text.matches('\r').count();
    pump.pending_at = None;
    pump.edited_flag = false;
    drop(pump);
    report(&format!(
        "flush: sent {} bytes rev={revision} epoch={epoch} edit-to-flush={quiet} CR-in-buffer={cr} (witness=edited)",
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

/// The pre-S10 path, kept whole and reachable: if the edited event ever proves unreliable (a
/// keystroke that does not fire it, or a programmatic set that does), flipping
/// EDITED_IS_DIRTY_WITNESS back to false restores the old behaviour with no other edit. The
/// brief asked for the comparison to survive only as a fallback, and a fallback you have to
/// re-write from memory is not a fallback.
fn text_pump_by_compare(ui: &Spike, gw: &Rc<RefCell<Option<Gateway>>>, pump: &RefCell<Pump>) {
    let text = lf(&ui.get_buffer());
    // The dot's dirty input, by the same comparison the guard below makes - one source
    // of truth for "unsaved", so the dot and the Flush can never disagree.
    // A let, not an inline call: the temporary borrow would still be alive inside
    // note_dot's own borrow_mut and the RefCell would panic (measured on a live run).
    let dirty_now = {
        let p = pump.borrow();
        text != p.last_sent
    };
    note_dot(pump, &ui.as_weak(), dirty_now, "buffer");
    pump.borrow_mut().invocations += 1;
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
///
/// S9 follow-up: this used to run ONLY at the END deadline, which made it invisible to
/// how the run is closed - a granted Quit ends the loop before 25 s and the needle simply
/// never printed, so the do-no-harm rule went unwitnessed in exactly the runs that quit
/// early. It is now called on the SAVE CYCLE instead: after the seed's `Event::Loaded`
/// adoption (the open must not have touched it) and after every `Event::Saved` whose path
/// IS the seed (a save of a CRLF foreign file is the moment the rule can actually break).
/// `via` names the moment, because three identical needles would be ambiguous.
fn do_no_harm(pump: &RefCell<Pump>, via: &str) {
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

/// THE CHORD TABLE, ported row for row from the first bridge's menu.rs SHORTCUTS - the same
/// four commands, the same ten recents, the same alt-0 -> "recent 10" quirk. The table is
/// LOAD-BEARING, not a comment: the synthetic driver walks it to decide what to fire and
/// prints the needle from its own display string, the legend is generated from it, and the
/// tests below fail if a row names an act nothing routes to. That is menu.rs's trick ported
/// - a dead key becomes a test failure instead of a key that does nothing on a machine.
///
/// (binding, display, what, act) - the act is a stable name, resolved by route_of().
const SHORTCUTS: &[(&str, &str, &str, &str)] = &[
    ("ctrl-o", "Ctrl+O", "Open", "open"),
    ("ctrl-s", "Ctrl+S", "Save As", "save-as"),
    ("ctrl-t", "Ctrl+T", "toggle auto-save", "autosave"),
    (
        "ctrl-shift-r",
        "Ctrl+Shift+R",
        "clear recent files",
        "clear-recents",
    ),
    ("alt-1", "Alt+1", "recent 1", "recent-0"),
    ("alt-2", "Alt+2", "recent 2", "recent-1"),
    ("alt-3", "Alt+3", "recent 3", "recent-2"),
    ("alt-4", "Alt+4", "recent 4", "recent-3"),
    ("alt-5", "Alt+5", "recent 5", "recent-4"),
    ("alt-6", "Alt+6", "recent 6", "recent-5"),
    ("alt-7", "Alt+7", "recent 7", "recent-6"),
    ("alt-8", "Alt+8", "recent 8", "recent-7"),
    ("alt-9", "Alt+9", "recent 9", "recent-8"),
    ("alt-0", "Alt+0", "recent 10", "recent-9"),
];

/// What a chord resolves to. The reason this is an enum and not a closure is the rule the
/// whole spike runs on: a key press and a row click must reach the SAME function, so a
/// Route is fired by invoking the callback the row invokes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Open,
    SaveAs,
    Autosave,
    ClearRecents,
    Recent(usize),
}

/// A table act name to a Route. None means the table gained a row nothing implements, which
/// every_chord_routes_somewhere turns into a failing test.
fn route_of(act: &str) -> Option<Route> {
    match act {
        "open" => Some(Route::Open),
        "save-as" => Some(Route::SaveAs),
        "autosave" => Some(Route::Autosave),
        "clear-recents" => Some(Route::ClearRecents),
        other => other
            .strip_prefix("recent-")
            .and_then(|n| n.parse::<usize>().ok())
            .map(Route::Recent),
    }
}

/// THE LEGEND, generated from the same rows (menu.rs::legend() in shape), because on a
/// frameless window with no menu bar there is nothing else that can tell a user the commands
/// exist. Filtered by route_of, so an unbound row cannot advertise itself.
fn legend() -> String {
    SHORTCUTS
        .iter()
        .filter(|(.., act)| route_of(act).is_some())
        .map(|(_, display, what, ..)| format!("{display} {what}"))
        .collect::<Vec<_>>()
        .join("  |  ")
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

/// The clamp's three answers as one string, all READ from Chrome through the mirror
/// bindings in main.slint. Reading is not writing: the single-writer rule is about the
/// menu-open bit, and this function only reports what Chrome computed from host-width.
/// Which asset the maximize cell shows. The markup carries the same ternary; this exists so a
/// test can hold the two together. The risk is not that the condition is wrong, it is that
/// someone renames a file or flips one branch and the button starts lying about the state.
/// Show the dialog OFF THE LOOP. Grabbed on the calling thread: the starting directory, the
/// suggested name and the gate decision, all of which need the component or the pump, then the
/// block happens somewhere else and the answer comes back over the channel.
fn ask_dialog(
    kind: DialogKind,
    weak: &slint::Weak<Spike>,
    pump: &Rc<RefCell<Pump>>,
    tx: &std::sync::mpsc::Sender<DialogReply>,
) {
    let Some(ui) = weak.upgrade() else { return };
    let (starting, suggested, stand_in) = {
        let p = pump.borrow();
        (
            dialog_starting_dir(p.seed.as_ref(), &p.recent_paths),
            suggested_name(&ui.get_title_words()),
            match kind {
                DialogKind::Open => p.seed.clone(),
                DialogKind::SaveAs => p.loop_path.clone(),
            },
        )
    };
    drop(ui);

    if !dialog_allowed() {
        report(&format!(
            "dialog[skipped]: SLINT_NO_DIALOG - no modal to click, so the probe's own path answers through the SAME channel ({} -> {})",
            kind.label(),
            stand_in
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default()
        ));
        let _ = tx.send(DialogReply {
            kind,
            path: stand_in,
        });
        return;
    }

    report(&format!(
        "dialog: spawning the {label} picker on its own thread (UNPARENTED - see the finding; the loop keeps pumping)",
        label = kind.label()
    ));
    let tx = tx.clone();
    // THE PARENT IS NOT CARRIED, and the compiler is the witness for it: rfd's set_parent takes
    // a &impl HasWindowHandle (file_dialog.rs:96) and reads the raw handle out of it at call
    // time (file_dialog.rs:100), so the call has to happen on THIS thread for the dialog to be
    // owned - and this thread is not allowed to block. Owned dialog or responsive loop: the
    // rule picks the loop. The full wording of the rejection is in the finding above.
    std::thread::spawn(move || {
        let mut dialog = rfd::FileDialog::new();
        if let Some(dir) = starting {
            dialog = dialog.set_directory(dir);
        }
        if kind == DialogKind::SaveAs {
            dialog = dialog.set_file_name(suggested);
        }
        // The blocking call, on a thread nobody is waiting on.
        let chosen = match kind {
            DialogKind::Open => dialog.pick_file(),
            DialogKind::SaveAs => dialog.save_file(),
        };
        let _ = tx.send(DialogReply { kind, path: chosen });
    });
}

/// The far end, run from the tick: try_recv, no await, and the SAME doors the rows and the
/// chords already use. Nothing here owns a path or a revision; it hands them to the port.
fn answer_dialog(
    reply: DialogReply,
    gw: &Rc<RefCell<Option<Gateway>>>,
    pump: &Rc<RefCell<Pump>>,
    ui: &Spike,
) {
    let words = dialog_words(reply.kind, &reply.path);
    ui.set_status(words.clone().into());
    match (reply.kind, reply.path) {
        (_, None) => report(&format!(
            "dialog: {words} (nothing sent; cancel is not silence)"
        )),
        (DialogKind::Open, Some(path)) => {
            report(&format!(
                "dialog: {words} -> Command::Open, the recents door"
            ));
            send(gw, Command::Open { path });
        }
        (DialogKind::SaveAs, Some(path)) => {
            // The buffer is read HERE, at the moment the answer arrives, not when the user was
            // asked. gpui snapshots before opening the dialog (main.rs:2160) and pays for it:
            // anything typed while the dialog is up is saved under a name for text that is
            // already gone. Here the pump kept running the whole time, so the freshest text and
            // the revision that describes it are read together, in one pair of statements, and
            // a stale pairing is not reachable.
            let text = lf(&ui.get_buffer());
            let revision = {
                let mut p = pump.borrow_mut();
                p.edits += 1;
                p.last_sent = text.clone();
                p.edits
            };
            report(&format!(
                "dialog: {words} -> Command::SaveAs {} ({} bytes, revision={revision})",
                path.display(),
                text.len()
            ));
            send(
                gw,
                Command::SaveAs {
                    path,
                    text,
                    revision,
                },
            );
        }
    }
}

/// S10c: THE ADOPTION POLICY, one function for both events. It clears the dirty witness (an
/// adoption is not a user edit) and steps the generation - EXCEPT for the adoptions that arrive
/// before the loop has ever ticked, which are the app coming back to the document it was closed
/// with, not a switch. Measured, that distinction is the difference between a fix and a footgun:
/// the S10b run printed `rebind: epoch=1 gen=1` and `load: epoch=2 gen=2` BEFORE
/// `hwnd = 0x… APPEARED at t+84ms, after the loop spun`, so the quarantine armed at
/// generation=2 with ZERO document switches and killed undo on the first keystroke of the first
/// file - the exact opposite of what the capture branch promises.
///
/// S4b, and the second half of that lesson: the witness used to be `invocations > 0`, which is a
/// PUMP counter, not a loop fact. The locked act calls text_pump twice from startup (to make the
/// refusal speak before the seed re-opens), which bumped that counter, faked a tick, and armed the
/// quarantine on the startup open - the S10c bug, re-introduced through the witness itself. So the
/// S4c, and the end of that lesson: the witness is now the FACT itself. The first adoption of a run
/// is the app coming back to its own file, and the second one is the same fact twice (the engine
/// answers startup with a Rebound and then a Loaded, both naming the SAME path), so neither is a
/// switch - and comparing document identity cannot be fooled by timing, which is what defeated both
/// earlier witnesses. The general rule, written where it will be read: a boolean that PROXIES a
/// fact drifts out of sync the moment anyone exercises the proxy; ask for the fact.
///
/// What the rule keeps, on purpose: re-opening the SAME path does NOT step, because the undo stack
/// still holds that file's own edits and native undo is genuinely safe there. Save As DOES step,
/// which is conservative - the text did not change, so nothing is stale - and the price is undo in
/// a case where the user has just performed a document-identity act anyway. Stated rather than
/// hidden, because it is a behaviour a person could notice.
fn note_adoption(pump: &RefCell<Pump>, path: &Path) -> i32 {
    let mut p = pump.borrow_mut();
    p.edited_flag = false;
    p.pending_at = None;
    // The FIRST adoption of a run is the baseline, not a switch: there is no previous document
    // whose bytes could be sitting in the undo stack. Two consequences written down because they
    // are limits, not details: (1) a step needs a path to differ FROM, so `is_some_and`, not a
    // bare inequality - the first draft used `!= Some(path)` and stepped immediately, which the
    // test caught as `left: 1, right: 0`; and (2) this assumes every run BEGINS with an adoption,
    // which is this bridge's behaviour today (every log shows `rebind: epoch=1` then
    // `load: epoch=2` before the user does anything). If a later `New document` act lets a person
    // type into a never-adopted buffer and then Open, that first adoption is a real switch and
    // would not step - the way to see that break is the first assertion below.
    let switching = p
        .adopted_path
        .as_deref()
        .is_some_and(|previous| previous != path);
    if switching {
        p.generation = next_generation(p.generation);
    }
    p.adopted_path = Some(path.to_path_buf());
    p.generation
}

/// S8b: the port's verdict, put into a word. Empty means not locked. The two causes stay
/// distinct because they explain differently to a person: `read_only` is the disk, which the user
/// set (or a file server did), while `oversize` is D9 - the 8 MiB guard opens a big file READ-ONLY
/// instead of refusing it, because a refused open is a lost document. Returning the word rather
/// than a bool is deliberate: the UI must not re-derive a reason the port already gave, and a bare
/// bool invites a status line that says "read-only" about a file that is merely huge.
fn lock_verdict(read_only: bool, oversize: bool) -> &'static str {
    match (read_only, oversize) {
        (false, false) => "",
        (true, false) => "read-only on disk",
        (false, true) => "read-only: too big to edit safely (8 MiB guard)",
        (true, true) => "read-only on disk, and too big to edit safely (8 MiB guard)",
    }
}
/// a toggle would collide after two adoptions. The PARITY is what a conditional recreation would
/// key on, so this makes the parity a fact a test can hold rather than an expression written
/// twice in two languages.
fn next_generation(previous: i32) -> i32 {
    previous + 1
}

/// S10b: THE UNDO QUARANTINE, as a decision a test can hold. The capture handler in
/// main.slint implements the same rule in markup (this file cannot call into it and markup
/// cannot call into here, so the pair is held together by `the_quarantine_is_a_state_rule`,
/// which greps every piece of the condition out of the mount).
///
/// Why a STATE rule and not two more table rows: SHORTCUTS is the legend of *commands*, each
/// with a port Command behind it, and it is held at fourteen rows by tests that mirror
/// menu.rs. Undo and redo are not commands - the port has no Command for them and never will -
/// so rows for them would print them in a legend of commands and break the parity guard the
/// table exists to keep. What is different here is that the rule is armed by generation, not
/// by the key alone: the same keystroke is native undo on the first document and a swallowed
/// hazard on the second, which is precisely the shape a table cannot express.
/// Test-only on purpose, and dead-code-clean because of it: there is no Rust-side key path to
/// call this from - the implementation is the markup branch, and this is the oracle the branch is
/// grepped against. Wiring it into the binary would mean inventing a call site that lies.
#[cfg(test)]
fn undo_quarantined(generation: i32, text: &str, ctrl: bool, shift: bool, alt: bool) -> bool {
    let _ = shift; // allowed, on purpose - see the doc comment above and the Ctrl+Shift+Z hole.
    generation > 0
        && ctrl
        && !alt
        && (text.eq_ignore_ascii_case("z") || text.eq_ignore_ascii_case("y"))
}

fn caption_glyph(maximized: bool) -> &'static str {
    if maximized {
        "icons/restore.svg"
    } else {
        "icons/maximize.svg"
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
    // ---- S10 ----
    /// The dirty witness, set by `TextEdit.edited` and cleared when a flush goes out (or when the
    /// bytes turn out to be identical). True is not a claim about WHAT changed - the callback
    /// carries no text on purpose - so the byte compare still guards the send.
    edited_flag: bool,
    /// Keystrokes witnessed, so the cost claim is a number and not an adjective.
    strokes: u64,
    /// S10: the document generation, stepped at both adoption sites. See `next_generation`.
    generation: i32,
    /// S10b: the arming of the undo quarantine is reported once, when the generation first
    /// leaves zero - the run's own proof that the swallow is live from here on.
    quarantine_reported: bool,
    // ---- S8b ----
    /// The port's lock, mirrored from FileMeta so the pump can refuse without reaching back into
    /// the event that set it. `true` means the engine has already decided it will not save this
    /// document; a bridge that kept flushing anyway was the bug this field exists to close.
    locked: bool,
    /// The word the status line shows and the refusal needle quotes, straight from lock_verdict,
    /// so the two causes (disk vs size) cannot be conflated downstream.
    lock_word: String,
    /// The refusal speaks once per lock, not once per tick.
    lock_needled: bool,
    /// S4c: THE FACT, not a proxy for it. The undo hazard depends on exactly one thing - whether
    /// the document in the buffer changed - so the policy now compares document IDENTITY.
    ///
    /// Both earlier witnesses were temporal, and timing defeated each in turn: `invocations > 0`
    /// (a pump counter) was faked by the locked act calling text_pump from startup, and the
    /// replacement `loop_ticked` was defeated WITHOUT any trick - the 9 MiB fixture open delayed
    /// the startup Loaded past the first tick, so the 4b run printed `load: epoch=2 gen=1` and
    /// `ARMED at generation=1` with zero document switches. That is also why this is not a probe
    /// problem to schedule around: a slow first open - a big file, a network share, OneDrive, a
    /// spinning disk, all normal for this app - would arm the quarantine on a real user's first
    /// document and silently kill undo. A same-path pair cannot arm under ANY timing now, which is
    /// the property the other two lacked.
    adopted_path: Option<PathBuf>,
    /// S4d: every ANSWER to an open - `Loaded` or `LoadFailed`, either way the engine has
    /// replied. The locked act polls this instead of draining once and hoping: the old version
    /// drained microseconds after sending, saw nothing, and printed silence that read like a dead
    /// lever. A count, not a bool, because a run opens several files.
    load_answers: u64,
    /// S4e: the locked act is a once-per-run event, and the one-shot lives with the rest of the
    /// act machine's memory rather than in a captured bool.
    lock_acted: bool,
    /// S4: the read-only fixture this run created, remembered so the exit path can clear the
    /// attribute and delete the file. A probe that leaves a read-only file in its own state dir
    /// makes the NEXT run's write fail, which is a probe lying about the run after it.
    lock_fixture: Option<PathBuf>,
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
    // ---- S4b: the three signals Chrome draws (dirty dot, failed dot, menu check).
    // Each is a port FACT or this bridge's own last ask; none of them is a guess.
    /// dirty means "the buffer differs from what was last sent" - the SAME comparison
    /// the Flush guard makes in text_pump, so the dot cannot disagree with the bytes
    /// that are about to go out.
    dirty: bool,
    /// Latched by Event::SaveFailed, cleared by the next Event::Saved. The failure stays
    /// until the bytes actually land: a dot that blinks off on the next keystroke tells
    /// the user nothing changed, which is the opposite of section 4.4.
    save_failed: bool,
    /// The port has NO event that echoes autosave - engine.rs:601-602 assigns the bool
    /// and says nothing back - so this is InitialState's answer XORed by every
    /// Command::SetAutosave this bridge sends. The bridge's own last ask, named as such
    /// rather than passed off as a report.
    autosave: bool,
    /// The last needle printed, so a signal that did not move stays quiet.
    dot_words: String,
    /// S4b: how many ticks actually reached text_pump. Printed at exit because the
    /// question this answers is arithmetic, not opinion: if the text loop sat inside the
    /// one-shot SAMPLED block, this number would be ~1 and the mid-run flush needle
    /// would be dead. Measured: it is every tick (the block closes at the 't+SAMPLE'
    /// report), and the number below is the proof.
    invocations: u64,
    /// The one in-run keystroke, fired once so the dirty->flush->clean chain is proven on
    /// the bridge's own cadence instead of being inherited from a pre-run probe write.
    late_key: bool,
    // ---- S4c: the save-failure cycle, and the menu ----
    /// The file SaveAs landed on. Autosave rewrites THIS name, so it is where the failure
    /// has to be staged; kept because the tick cannot re-derive it.
    loop_path: Option<PathBuf>,
    /// S6c: the seed's explicit SaveAs (the arming act), once.
    seed_armed: bool,
    /// S8: which caption step has run (nine of them, see CAPTION_AT).
    caption_step: u64,
    /// S7: which overlay step has run - eight of them, each observing the previous act and
    /// then performing its own (open, 400px, 180px, restore, backdrop, reopen, escape, and one
    /// final observation).
    overlay_step: u64,
    /// The window's own size on entry to the overlay act, to be restored at the end.
    start_size: Option<(u32, u32)>,
    /// The three acts, each once.
    ro_done: bool,
    key2_done: bool,
    wb_done: bool,
    /// How many CHORD_DRIVE steps have been driven: 0 none, then one per row of Open, Alt+2,
    /// Auto-save, Save As, Clear recents, and one past the end once Quit has fired. Named for
    /// the rows because a chord and its row reach the same callback; the number records which
    /// step of the table walk ran and nothing more.
    menu_act: u64,
    /// The one keystroke aimed at the seed document.
    seed_key_done: bool,
    /// S5: which drag act has run - 0 none, 1 the plain move, 2 the one aimed at the
    /// maximised state (a refusal when the frame act is on, a second move when it is off).
    drag_step: u64,
    /// The refusal print is once per maximised episode, not once per mouse-move frame.
    drag_refused_shown: bool,
    /// The FRAME-INCLUSIVE target the last drag asked for. The exit check compares this
    /// against what the port actually persisted, which is what the next launch restores
    /// from; kept here because the tick that printed it is long gone.
    drag_target: Option<(i32, i32)>,
    /// How many autosave toggles this bridge has sent. The port echoes NO autosave event,
    /// so the menu check can only follow the ask - printed as 'menu: ...' so the
    /// convention is visible instead of pretending to be a report.
    autosave_asks: u64,
    /// A Quit from the menu is the user's own request, so the close contract must not
    /// decline it the way it declines the FIRST synthetic request to prove declining
    /// works. One door (Window::close), one extra fact in front of it.
    quit_requested: bool,
}

/// The dot's word for the pair, in section 4.4's precedence: a failed save outranks
/// plain dirt, and neither paints at all on a clean buffer.
fn dot_word(save_failed: bool, dirty: bool) -> &'static str {
    if save_failed {
        "amber(save-failed)"
    } else if dirty {
        "red(dirty)"
    } else {
        "none(clean)"
    }
}

/// ONE NEEDLE PER SIGNAL CHANGE: the exact triple Chrome renders, printed, so a run is
/// gradable on transitions instead of on a screenshot. 'via' says which input moved
/// (start / buffer / saved / save-failed), and a transition that prints nothing is a
/// signal that is not wired.
fn note_dot(pump: &RefCell<Pump>, weak: &slint::Weak<Spike>, dirty: bool, via: &str) {
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
                epoch,
                text,
                path,
                meta,
            } => {
                let adopted = lf(text);
                let mut p = pump.borrow_mut();
                p.epoch = *epoch;
                p.last_sent = adopted.clone();
                p.load_answers += 1; // S4d: an answer arrived - what the locked act waits for
                // S8b: THE VERDICT, read at last. This pattern used to end in `..`, which dropped
                // the whole FileMeta - the disk flag, D9's size verdict, ADR-0001's arming bit -
                // on the floor. That is how this bridge came to let a person type into a file the
                // engine had already decided it would never save.
                let word = lock_verdict(meta.read_only, meta.oversize);
                p.locked = !word.is_empty();
                p.lock_word = word.to_string();
                // A new document, a new lock: the refusal gets to speak once about THIS file.
                p.lock_needled = false;
                let (locked, word) = (p.locked, p.lock_word.clone());
                drop(p);
                // An adoption is not a user edit, and the startup one is not a switch either -
                // both facts are decided in one place, note_adoption.
                let generation = note_adoption(pump, path);
                if let Some(ui) = weak.upgrade() {
                    ui.set_buffer(adopted.clone().into());
                    ui.set_doc_generation(generation);
                    // THE WIDGET'S HALF: one property, and 1.17 stops the caret itself
                    // (textedit-base gates its interaction areas on read-only).
                    ui.set_locked(locked);
                    if locked {
                        // The visible word, from the port's own reason - see lock_verdict. The
                        // dot stays clean by the pump's guard, which is the precedence S8b
                        // chose: a document that can never be saved has no unsaved state to
                        // advertise, so LOCK beats DIRTY rather than competing with it.
                        ui.set_status(
                            format!("{word} - typing is off; the engine will not save this file")
                                .into(),
                        );
                    }
                    publish_title(&ui, Some(path), true, "loaded");
                }
                report(&format!(
                    "load: path={} epoch={epoch} gen={generation} meta(read_only={} oversize={} armed={}) locked={locked} announced, buffer adopted ({} bytes, CR-normalised)",
                    path.display(),
                    meta.read_only,
                    meta.oversize,
                    meta.armed,
                    adopted.len()
                ));
                // The open half of the rule: reading a foreign file must not change it.
                let is_seed = {
                    let p = pump.borrow();
                    p.seed.as_deref() == Some(path.as_path())
                };
                if is_seed {
                    do_no_harm(pump, "loaded");
                }
            }
            Event::Rebound {
                epoch, path, meta, ..
            } => {
                // The other half of the same door: a Save As rebinds the document identity, so
                // the generation moves here too - through the SAME policy function, which is what
                // stops a key that must move on both paths from moving on only one. And it reads
                // meta for the same reason Loaded does now: a Save As CAN land on a read-only
                // path, and the bridge may not assume the answer.
                let mut p = pump.borrow_mut();
                p.epoch = *epoch;
                let word = lock_verdict(meta.read_only, meta.oversize);
                p.locked = !word.is_empty();
                p.lock_word = word.to_string();
                p.lock_needled = false;
                let (locked, word) = (p.locked, p.lock_word.clone());
                drop(p);
                let generation = note_adoption(pump, path);
                if let Some(ui) = weak.upgrade() {
                    ui.set_doc_generation(generation);
                    ui.set_locked(locked);
                    if locked {
                        ui.set_status(
                            format!("{word} - typing is off; the engine will not save this file")
                                .into(),
                        );
                    }
                    publish_title(&ui, Some(path), true, "rebound");
                }
                report(&format!(
                    "rebind: path={} epoch={epoch} gen={generation} meta(read_only={} oversize={} armed={}) locked={locked}; the next Flush echoes it",
                    path.display(),
                    meta.read_only,
                    meta.oversize,
                    meta.armed
                ));
            }
            // S4d: EVENT::LOADFAILED, ARMED AT LAST. This event used to fall through the catch-all,
            // which threw away both the path and the reason on the ONE event whose entire job is to
            // say why nothing happened - and "nothing happened" is the load half of the do-no-harm
            // promise, so it is exactly the fact a probe must print. The wording is the PORT's:
            // LoadError is a thiserror enum whose sentences are already human (TooLarge -> "file is
            // too large to open"), so spelling them again here would be a second copy of a verdict
            // the port owns, the same rule PinFailed follows.
            //
            // A REFUSED LOAD IS NOT AN ADOPTION, which is why nothing below touches state: no text
            // crossed the port, so the buffer, the generation, the epoch and the lock all stay as
            // the previous document left them. The engine guarantees the survivor - the session doc
            // is only rewritten from an actual adoption (session.rs:806-835) - and the gpui bridge
            // makes the same decision for the same reason (bridge-gpui/src/main.rs:986-994: keep
            // the text, disarm, and never flush onto a path the open just failed to read).
            Event::LoadFailed { path, reason } => {
                pump.borrow_mut().load_answers += 1;
                let where_ = path.display().to_string();
                report(&format!(
                    "load[refused]: {where_} - {reason} (nothing adopted: the previous document, its buffer, its generation and its lock are untouched)"
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
                    report(&format!(
                        "recents: rendered {rows} row{}",
                        if rows == 1 { "" } else { "s" }
                    ));
                }
            }
            Event::Saved { path, revision } => {
                let which = {
                    let mut p = pump.borrow_mut();
                    p.saves += 1;
                    p.saves
                };
                // The bytes landed, so the failure is over - cleared here and nowhere
                // else: "latched until the next successful Saved", in one line.
                pump.borrow_mut().save_failed = false;
                let dirty = pump.borrow().dirty;
                note_dot(pump, weak, dirty, "saved");
                if which == 1 {
                    report(&format!("saved: rev={revision} {}", path.display()));
                } else {
                    // No SaveAs in sight: a change entered the debounce, the 750 ms
                    // passed, one Flush went out, and the bytes landed. That is the
                    // autosave path proving itself - the loop the gpui bridge owns on
                    // its side of the seam.
                    report(&format!(
                        "autosave: saved rev={revision} (Saved #{which}, no SaveAs between)"
                    ));
                    report(&format!("autosave: file {}", path.display()));
                }
                // The save half, and the one that matters: core is supposed to restore
                // this file's own CRLF endings on the way out. If it ever writes LF, the
                // hash moves HERE, in the run that did it - not 4 s later at a deadline
                // the run may never reach.
                let is_seed = {
                    let p = pump.borrow();
                    p.seed.as_deref() == Some(path.as_path())
                };
                if is_seed {
                    do_no_harm(pump, "saved");
                }
            }
            Event::SaveFailed { reason, .. } => {
                pump.borrow_mut().save_failed = true;
                let dirty = pump.borrow().dirty;
                note_dot(pump, weak, dirty, "save-failed");
                if let Some(ui) = weak.upgrade() {
                    ui.set_status(format!("save failed: {reason:?}").into());
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
        // EOF reaches the popup, whose five rows legitimately DO write menu-open - the first
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
        // and every dismissal route - hamburger, five rows, backdrop, Escape - ends inside
        // chrome.slint.
        let chrome_writes = POPUP.lines().filter(|l| l.contains("menu-open =")).count();
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
            MARKUP.contains("visible: chrome.menu-open")
                && MARKUP.contains("enabled: chrome.menu-open"),
            "a closed popup must leave every pixel to the editor"
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
    fn a_locked_document_is_never_flushed() {
        // The structural half of S8b: the verdict is READ at both adoption sites, the WIDGET is
        // told, and the pump refuses ABOVE the dispatch so neither witness path escapes it. This
        // is grep-grade proof on purpose - the behavioural half needs a file the engine really
        // refuses, see the run's meta(...) needle and the manual list.
        // Cut at the tests module: this file includes ITSELF, so every literal counted below
        // would otherwise match the counting line too - the trap this test walked into on its
        // first run, reporting 3 for 2 real sites. Self-referential greps must say what they
        // exclude.
        let whole = include_str!("../src/main.rs");
        let src = &whole[..whole.find("mod tests").expect("the tests module")];
        assert_eq!(
            src.matches("lock_verdict(meta.read_only, meta.oversize)")
                .count(),
            2,
            "Loaded and Rebound must both read the verdict; one is the bug class again"
        );
        assert_eq!(
            src.matches("ui.set_locked(locked)").count(),
            2,
            "both sites must tell the widget, or a switch can unlock a document the port locked"
        );
        assert_eq!(
            src.matches("armed={}").count(),
            2,
            "armed is printed at both sites: dropping the field is the bug, even unread"
        );
        let guard = &src[src.find("S8b: THE REFUSAL").expect("the guard")..];
        let guard = &guard[..guard
            .find("if !EDITED_IS_DIRTY_WITNESS")
            .expect("the dispatch")];
        assert!(
            guard.contains("return;"),
            "the refusal must return before any send"
        );
        assert!(
            !guard.contains("Command::Flush"),
            "no flush may be reachable from the locked branch"
        );
        assert!(
            guard.contains("p.last_sent = lf(&ui.get_buffer());"),
            "the refusal keeps last_sent in sync, or the unlock resurrects a phantom dirty"
        );
        assert!(
            guard.contains("p.edited_flag = false;") && guard.contains("p.pending_at = None;"),
            "the witness and the debounce clock are cleared too"
        );
        assert!(
            guard.contains("note_dot(pump, &ui.as_weak(), false, \"locked\")"),
            "LOCK beats DIRTY"
        );
        assert!(MARKUP.contains("in-out property <bool> locked"));
        assert!(MARKUP.contains("read-only: root.locked;"));
    }

    #[test]
    fn a_refused_load_adopts_nothing_and_says_why() {
        // S4d, structural: the arm exists, it speaks the port's own reason, and above all it
        // touches NOTHING - a refusal that adopted state would be the data-loss bug this event
        // exists to prevent. Cut at `mod tests` for the reason the S8b test learned: this file
        // greps ITSELF, so the counting lines are inside it.
        let whole = include_str!("../src/main.rs");
        let src = &whole[..whole.find("mod tests").expect("the tests module")];
        assert!(
            src.contains("Event::LoadFailed { path, reason } =>"),
            "the event must have a real arm, not the catch-all"
        );
        let arm = &src[src
            .find("Event::LoadFailed { path, reason } =>")
            .expect("the arm")..];
        let arm = &arm[..arm.find("Event::RecentsUpdated").expect("the next arm")];
        assert!(
            arm.contains("load[refused]:"),
            "the needle must name itself"
        );
        assert!(
            arm.contains("{reason}"),
            "the port's own sentence, not a bridge paraphrase"
        );
        for untouched in [
            "note_adoption(",
            "set_locked",
            "p.locked",
            "set_buffer",
            "Generation",
        ] {
            assert!(
                !arm.contains(untouched),
                "a refused load must not touch {untouched}"
            );
        }
        // The gpui parity claim, checked rather than asserted: it keeps the buffer and says so.
        assert!(
            src.contains("nothing adopted"),
            "the needle must state the survivor"
        );
        // S4d item 3: both adoption needles name the path, because a claim about WHICH adoption
        // was which was previously uncheckable and turned out to be wrong.
        assert!(
            src.contains("\"load: path={} epoch={epoch}"),
            "Loaded must print its path"
        );
        assert!(
            src.contains("\"rebind: path={} epoch={epoch}"),
            "Rebound must print its path"
        );
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
            "one accept per bound chord, plus Escape: fourteen commands in the table and ONE              dismissal, which is not a command and so is not in the legend (menu.rs has no              Escape row either - see the parity warning in main.slint)"
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
