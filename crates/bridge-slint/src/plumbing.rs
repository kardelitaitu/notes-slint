//! STRIP-2a: the PLUMBING - how a needle gets printed, how a line ending is normalised, how a
//! command reaches the port, and how the two window decorations (title strip, unsaved dot) are
//! written. These are not decisions, and they are not the probe's either: they are the small
//! mechanical layer every root needs, which is why surface.rs used to reach them with a glob
//! import of the crate root and a comment saying "that is not hygiene". This file is the answer to
//! that comment - the reach is now an import list naming eleven things.
//!
//! The rule the moved bodies keep: report() is the ONLY writer of the `notes-gpui: ` prefix (the
//! smoke contract), lf() is the ONLY line-ending normaliser, publish_title()/note_dot() are the
//! only writers of the strip and the dot, send() never blocks. Nothing here decides anything about
//! documents; see surface.rs for that half.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use notes_api::{
    Command, DropGuard, Encoding, Event, FileMeta, Gateway, LineEnding, Rect, SkipReason,
    WindowHandle, arm_file_drop,
};
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

/// THE POPUP'S EXPLANATION, in the user's words. Two voices for one fact is normally drift, and
/// this split is deliberate rather than sloppy: [`describe`] above prints the Debug shape
/// (`AutosaveSkipped reason=ReadOnly`) because the INSTRUMENT quotes it - probe.rs:540 records a
/// real failure by that exact string, and an instrument's evidence is not edited after its verdict
/// (ADR-0006 §4: "never as probe edits"). The menu is not an instrument. docs/features.md §4.4 asks
/// for "the reason in the menu", on a surface a person reads, and `api` assigns that wording to the
/// UI on purpose: SkipReason carries no Display so the bridge owns the sentence. The strings below
/// are the SAME ones bridge-gpui's `skip_words` chose, deliberately: two bridges explaining one skip
/// two different ways is the drift that is not allowed, and gpui's copy was argued out first.
pub(crate) fn skip_words(reason: SkipReason) -> &'static str {
    match reason {
        SkipReason::AutosaveDisabled => "auto-save is off",
        SkipReason::ForeignFileNotArmed => {
            "a file this app did not create: Save As once (Ctrl+S) and it keeps saving"
        }
        SkipReason::Clean => "nothing changed since the last write",
        SkipReason::Superseded => {
            "the edit belonged to a note that has since been replaced, so it was discarded"
        }
        SkipReason::NeedsPath => "this note has no file yet: use Save As",
        SkipReason::ReadOnly => "the file is read-only",
        SkipReason::Oversize => "the file is over the size guard, so writes are refused",
    }
}

/// WHAT THIS FILE IS - the same sentence bridge-gpui prints (`meta_words`), not a second phrasing
/// of it. This is the product's actual promise: "your file came back identical". Every one of these
/// six facts is decided in `core`, carried on `Event::Loaded`/`Event::Rebound`, and was invisible to
/// the person whose file it is. The `armed` verdict is the important one: it is the difference
/// between "auto-save is off" and "this file will never be saved unless you say so", and ADR-0001's
/// design turns on a user being able to tell those two apart.
pub(crate) fn file_words(meta: &FileMeta) -> String {
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

/// Exhaustive like the first bridge's: a new encoding is a compile error here, not a file whose
/// encoding the menu declines to name.
fn encoding_words(encoding: Encoding) -> String {
    match encoding {
        Encoding::Utf8 => "utf-8".to_string(),
        Encoding::Utf8Bom => "utf-8 with BOM".to_string(),
        Encoding::Utf16Le => "utf-16 le".to_string(),
        Encoding::Utf16Be => "utf-16 be".to_string(),
        // The codepage id is a fact the port carries on the variant, so the line names it:
        // utf-8-without-a-BOM and CP1252 must not read the same (D14).
        Encoding::Ansi(codepage) => format!("ansi cp{codepage}"),
    }
}

fn line_ending_words(line_ending: LineEnding) -> &'static str {
    match line_ending {
        LineEnding::Lf => "lf",
        LineEnding::CrLf => "crlf",
    }
}

/// ONE WRITER for the popup's explanation, in the shape [`note_dot`] uses for the dot triple: read
/// both strings off the pump and set both unconditionally, because these are bindings - an
/// unchanged value costs nothing, and a changed one must not be lost to a printed-diff heuristic.
pub(crate) fn publish_explain(pump: &RefCell<Pump>, weak: &slint::Weak<Spike>) {
    let (why, file) = {
        let p = pump.borrow();
        (p.why.clone(), p.file_words.clone())
    };
    if let Some(ui) = weak.upgrade() {
        ui.set_why_not_saved(why.into());
        ui.set_file_words(file.into());
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

#[cfg(test)]
/// THE COPY TESTS, and they live beside the copy because that is the guard law this crate already
/// follows: a guard moves with the code it polices. What makes these worth writing is not the
/// strings, it is what they cross-check. A sentence in `skip_words` PROMISES an affordance ("Save
/// As once (Ctrl+S)"), and the only thing in this crate that knows what Ctrl+S is bound to is the
/// chord table in `surface.rs` - so the promise is now tested against the table, which is the only
/// way a rebind can be caught before the menu starts telling people to press a key that does
/// something else. A footer nobody can read is not an explanation, so the second test pins that
/// the block is unclickable, and the third that its height and its position are the SAME
/// arithmetic (two numbers that agree today and drift apart tomorrow would clip it or float it).
mod tests {
    use super::{encoding_words, file_words, line_ending_words, skip_words};
    use crate::surface::SHORTCUTS;
    use notes_api::{Encoding, FileMeta, LineEnding, SkipReason};

    fn meta(
        encoding: Encoding,
        line_ending: LineEnding,
        trailing: bool,
        read_only: bool,
        oversize: bool,
        armed: bool,
    ) -> FileMeta {
        FileMeta {
            encoding,
            line_ending,
            trailing_newline: trailing,
            read_only,
            oversize,
            armed,
        }
    }

    #[test]
    fn every_skip_is_explained_and_the_one_with_an_act_names_the_real_one() {
        // Exhaustive by construction: this list is the compiler's check that skip_words has not
        // dropped a variant, and the empty-string check is that none of them is a shrug.
        let all = [
            SkipReason::AutosaveDisabled,
            SkipReason::ForeignFileNotArmed,
            SkipReason::Clean,
            SkipReason::Superseded,
            SkipReason::NeedsPath,
            SkipReason::ReadOnly,
            SkipReason::Oversize,
        ];
        for reason in all {
            let words = skip_words(reason);
            assert!(!words.is_empty(), "{reason:?} would render a blank reason");
            assert!(
                words.chars().next().unwrap().is_lowercase(),
                "{reason:?} is a clause inside a sentence, not a heading: {words}"
            );
        }
        // THE CROSS-CHECK, and the reason this test exists. ForeignFileNotArmed is the only skip
        // that tells the user to DO something, and what it tells them is the chord table's own
        // display string for save-as. Rebind Ctrl+S in SHORTCUTS and this fails, because the menu
        // would then be ordering people to press a key that saves something else.
        let unarmed = skip_words(SkipReason::ForeignFileNotArmed);
        let chord = SHORTCUTS
            .iter()
            .find(|(_, _, _, act)| *act == "save-as")
            .expect("the table still has a save-as row")
            .1;
        assert!(
            unarmed.contains(chord),
            "the reason offers {chord} as the act, and says: {unarmed}"
        );
        assert!(
            unarmed.contains("Save As"),
            "and it names the row by the name the row paints: {unarmed}"
        );
        // The two sentences that must never be confused, because confusing them is the difference
        // between "nothing to do" and "this file will never be saved unless you say so".
        assert_ne!(
            skip_words(SkipReason::AutosaveDisabled),
            skip_words(SkipReason::ForeignFileNotArmed)
        );
    }

    #[test]
    fn the_file_line_states_every_verdict_core_carries() {
        let plain = file_words(&meta(
            Encoding::Utf8,
            LineEnding::Lf,
            true,
            false,
            false,
            true,
        ));
        assert_eq!(
            plain,
            "utf-8 · lf · newline kept · writable · within the guard · autosave armed"
        );
        // The shape this app is actually for: a CRLF file with no final newline, foreign enough to
        // be disarmed. Every one of those four facts used to be invisible.
        let foreign = file_words(&meta(
            Encoding::Utf8,
            LineEnding::CrLf,
            false,
            false,
            false,
            false,
        ));
        assert_eq!(
            foreign,
            "utf-8 · crlf · newline none · writable · within the guard · autosave not armed"
        );
        // D14's rule, tested as a sentence: the two encodings that differ only by a BOM, and the
        // codepage that must carry its number, all read differently.
        assert_ne!(
            encoding_words(Encoding::Utf8),
            encoding_words(Encoding::Utf8Bom)
        );
        assert_eq!(encoding_words(Encoding::Ansi(1252)), "ansi cp1252");
        assert_eq!(line_ending_words(LineEnding::CrLf), "crlf");
        // The two refusals a user can act on, and the one they cannot.
        assert!(
            file_words(&meta(
                Encoding::Utf16Le,
                LineEnding::Lf,
                true,
                true,
                false,
                true,
            ))
            .contains("read-only")
        );
        assert!(
            file_words(&meta(
                Encoding::Utf16Be,
                LineEnding::Lf,
                true,
                false,
                true,
                true,
            ))
            .contains("over the guard")
        );
    }

    #[test]
    fn the_footer_explains_and_asks_for_nothing() {
        let chrome = include_str!("../ui/chrome.slint");
        let main = include_str!("../ui/main.slint");
        let footer = &chrome[chrome
            .find("footer := Rectangle {")
            .expect("the footer block")
            ..chrome
                .find("about := Rectangle {")
                .expect("the panel that ends it")];
        // NOT A ROW. Six is still the row count precisely because this block cannot be clicked, so
        // it inherits none of the row duties - and the popup's existing guards, which count the
        // 6th row's cells, stay true.
        assert!(
            !footer.contains("TouchArea"),
            "an explanation is not an act"
        );
        assert!(!footer.contains("clicked"), "and it has no handler at all");
        assert!(
            footer.contains("color: Theme.amber"),
            "the reason shares the dot's amber, so the two agree about what is wrong"
        );
        // The absent-not-hidden choice, pinned: a hidden child still occupies the layout, which
        // would leave a gap in a popup with nothing to say.
        assert!(
            footer.contains("if root.why-not-saved != \"\":")
                && footer.contains("if root.file-words != \"\":"),
            "each line exists only when the port said the thing behind it"
        );
        // ONE ARITHMETIC, TWO USERS: the popup's height and the footer's y both spend the six rows
        // and their five gaps. Written twice, they must agree, and the only test that can tell is a
        // count of the shared expression.
        let whole = chrome.replace(['\n', ' '], "");
        assert_eq!(
            whole
                .matches("6*Theme.menu-row-height+5*Theme.menu-gap")
                .count(),
            2,
            "the height formula and the footer's offset are the same row arithmetic"
        );
        // The wire, both ends. THIS file writes each setter exactly once - and the count is taken
        // from a slice that stops at the tests module, because a grep that counts its own counting
        // line is the trap this crate has already named. Six arms and hooks in surface.rs publish
        // through publish_explain, and NO arm writes a property directly: one author, so a sticky
        // fact cannot be set in one place and cleared in another that nobody reads.
        let whole = include_str!("../src/plumbing.rs");
        let src = &whole[..whole.find("mod tests").expect("the tests module")];
        assert_eq!(
            src.matches("set_why_not_saved(").count(),
            1,
            "one writer for the reason, and it is this function"
        );
        assert_eq!(
            src.matches("set_file_words(").count(),
            1,
            "one writer for the file line, same door"
        );
        let surface = include_str!("../src/surface.rs");
        assert_eq!(
            surface.matches("set_why_not_saved(").count()
                + surface.matches("set_file_words(").count(),
            0,
            "and no drain arm reaches past publish_explain to write either one"
        );
        assert_eq!(
            surface.matches("publish_explain(").count(),
            6,
            "every sticky change publishes: two adoptions, one save, one skip, one failure, \
             one toggle. A seventh sticky write must publish too, or the footer goes stale."
        );
        assert!(
            main.contains("why-not-saved: root.why-not-saved")
                && main.contains("file-words: root.file-words"),
            "and both are forwarded to Chrome at the one instantiation"
        );
    }
}
