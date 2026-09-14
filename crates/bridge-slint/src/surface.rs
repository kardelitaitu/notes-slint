//! STRIP-1, half A: the SHARED SURFACE vocabulary - the pure decisions the product and the
//! probe must never disagree about. Moved VERBATIM out of probe.rs: no logic, no wording, and
//! no doc comment re-cut at the seam, so a diff of these bodies against the file they came from
//! is the review.
//!
//! What is here is chosen by one test: a thing belongs in this module when it answers a question
//! without touching a window, a channel, the pump, or the clock. `route_of` decides where a chord
//! leads; `lock_verdict` turns the port's two bits into one word; `next_generation` steps a
//! counter; `caption_glyph` picks an asset. None of them can observe or be observed by the event
//! loop, which is exactly why the product can adopt them unchanged while the probe that grew
//! around them is deleted.
//!
//! What deliberately stays in probe.rs for now: the drain machinery and `Pump` (they ARE the
//! event loop's state - half B moves them), `text_pump` and the witnesses, every `*_AT` const and
//! every synthetic act, `publish_title` and `arm_drop_target` (both PRINT, and printing is the
//! probe's job until the product owns a status line), and the two `include_str!` drift guards -
//! which is the guard law in one clause: a guard moves in the same commit as the code it polices,
//! so these keep reading ../src/probe.rs, where the text they search still lives.
//!
//! Both [[bin]] targets still compile probe.rs as their root, so every test in this crate - here
//! and in probe.rs - runs once per target. That doubling is real and is reported, not hidden.
#[allow(unused_imports)] // the strip slices re-export this module; half A imports it by name
use std::path::PathBuf;
// The probe's root helpers (report, lf, note_dot, publish_title, send, describe, dialog_allowed) and
// its imports (RefCell, Rc, Duration, Instant, Command, Event, Gateway, Spike) are private TO THE
// ROOT, and a child module may see them: that is Rust's privacy rule, not a loophole. Half B leans
// on it deliberately instead of pub(crate)-ing a dozen printing helpers the product will own again
// later - the boundary this strip is drawing is between DECISIONS and INSTRUMENTATION, and an
// import list that reaches up to the root makes the reach visible in one line.
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use notes_api::{Command, Event, Gateway, RecentEntry};
// C3: the drag writes a PHYSICAL target, so the physical type is named here rather than routed
// through a logical one and converted twice (see `drag_destination` for why once is the whole rule).
use slint::{ComponentHandle, PhysicalPosition};

use crate::Spike;
use crate::plumbing::{
    describe, dialog_allowed, do_no_harm, file_words, hwnd_of, lf, note_dot, publish_explain,
    publish_title, report, send, skip_words,
};
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DialogKind {
    Open,
    SaveAs,
}

impl DialogKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::SaveAs => "save as",
        }
    }
}

/// Where the picker starts. The current file's directory first - the port's recents list is
/// ordered most-recent-first, so recents[0] IS the file in the window, and after a Clear it is
/// the next best thing - then nothing, which leaves rfd on its own default.
pub(crate) fn dialog_starting_dir(
    current: Option<&PathBuf>,
    recents: &[PathBuf],
) -> Option<PathBuf> {
    let from = current.or_else(|| recents.first());
    let dir = from.and_then(|path| path.parent())?;
    (!dir.as_os_str().is_empty()).then(|| dir.to_path_buf())
}

/// The name the save dialog offers. The bridge already computes the title's words for the OS
/// title and the strip, so the dialog asks for the same thing the user can already read rather
/// than inventing a third name; an extension is added only when the word has none, because
/// rfd's suggestion is a whole file name on Windows.
pub(crate) fn suggested_name(title_words: &str) -> String {
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
pub(crate) fn dialog_words(kind: DialogKind, path: &Option<PathBuf>) -> String {
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

/// THE CHORD TABLE, ported row for row from the first bridge's menu.rs SHORTCUTS - the same
/// four commands, the same ten recents, the same alt-0 -> "recent 10" quirk. The table is
/// LOAD-BEARING, not a comment: the synthetic driver walks it to decide what to fire and
/// prints the needle from its own display string, the legend is generated from it, and the
/// tests below fail if a row names an act nothing routes to. That is menu.rs's trick ported
/// - a dead key becomes a test failure instead of a key that does nothing on a machine.
///
/// (binding, display, what, act) - the act is a stable name, resolved by route_of().
pub(crate) const SHORTCUTS: &[(&str, &str, &str, &str)] = &[
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
pub(crate) enum Route {
    Open,
    SaveAs,
    Autosave,
    ClearRecents,
    Recent(usize),
}

/// A table act name to a Route. None means the table gained a row nothing implements, which
/// every_chord_routes_somewhere turns into a failing test.
pub(crate) fn route_of(act: &str) -> Option<Route> {
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
pub(crate) fn legend() -> String {
    SHORTCUTS
        .iter()
        .filter(|(.., act)| route_of(act).is_some())
        .map(|(_, display, what, ..)| format!("{display} {what}"))
        .collect::<Vec<_>>()
        .join("  |  ")
}

/// S8b: the port's verdict, put into a word. Empty means not locked. The two causes stay
/// distinct because they explain differently to a person: `read_only` is the disk, which the user
/// set (or a file server did), while `oversize` is D9 - the 8 MiB guard opens a big file READ-ONLY
/// instead of refusing it, because a refused open is a lost document. Returning the word rather
/// than a bool is deliberate: the UI must not re-derive a reason the port already gave, and a bare
/// bool invites a status line that says "read-only" about a file that is merely huge.
pub(crate) fn lock_verdict(read_only: bool, oversize: bool) -> &'static str {
    match (read_only, oversize) {
        (false, false) => "",
        (true, false) => "read-only on disk",
        (false, true) => "read-only: too big to edit safely (8 MiB guard)",
        (true, true) => "read-only on disk, and too big to edit safely (8 MiB guard)",
    }
}

/// THE RECENT ROWS, BUILT IN ONE PLACE. ADR-0006 item 1: the label trio this bridge may cite as
/// a Slint-side proof has to live HERE, on the rows the product renders, and not inside the event
/// arm that hands them to the model.
///
/// The cap is restated on this side of the seam for the same reason bridge-gpui/src/menu.rs:58
/// states it: not because core's list is distrusted, but because a row past the last slot cannot
/// be reached by a chord at all. main.slint binds Alt+1..9 and then Alt+0, which is TEN doors, so
/// an eleventh row would be text a person can read and nothing can open. Taking the first ten is
/// what keeps "slot N" a meaning that survives from the label all the way to the
/// open-at-index(N-1) the row's TouchArea fires.
pub(crate) const MAX_RECENTS: usize = 10;

/// One rendered row of the recents stack: the facts a row carries, built together so no two of
/// them can be about different files. The first bridge has the same property by construction -
/// "the label is built FIRST and then handed to the slot, so the number a user reads and the
/// index the action carries are the same expression" (menu.rs:88-90) - and here it is the same
/// idea spread over a struct instead of a menu item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecentRow {
    /// The slot a person READS, 1-based: "3." is the row Alt+3 opens, and slot 10 is Alt+0's -
    /// the same quirk the chord table carries as ("alt-0", "Alt+0", "recent 10", "recent-9").
    pub(crate) slot: usize,
    /// The whole text of the row: the slot, core's pre-formatted display UNEDITED, and the mark
    /// when the path went away.
    pub(crate) label: String,
    /// The port's fact, kept as a bit so the print and the tests can name WHY a row reads
    /// "missing" without parsing its own words back.
    pub(crate) missing: bool,
    /// The path behind the row, which never crosses into the markup: a row is a label, and an
    /// index is what maps back to a file.
    pub(crate) path: PathBuf,
}

/// THE ROWS BEHIND A RECENTS LIST. Three rules, and every one of them is the port's or the first
/// bridge's - none invented here:
///
/// 1. THE LABEL is core's "display" verbatim behind the slot number. That string is already
///    core/src/recent.rs "visible_label" output - parent folders, basename, escaping and the
///    64-char hard cap included - so a bridge that composed its own from "path" would be a second
///    owner of a truncation rule, which is exactly what menu.rs:80-83 refuses to be.
/// 2. THE MARK. A vanished path STAYS in the list (D13: "a greyed-out entry that tells the truth
///    beats one that quietly disappeared"), it still counts toward the cap, and it says so. GPUI
///    renders this as per-item disabled; these rows are plain strings in a Slint for-loop with no
///    per-row colour hook, so the honest form is the word - APPENDED to the label, never
///    substituted for it. That asymmetry is the parity claim rather than a failure of it: a grey
///    needs a widget that can be greyed, and where the toolkit has no such door the row still has
///    to tell the truth.
/// 3. THE CAP, taken with take() and not filtered, so the first ten ARE the ten slots.
pub(crate) fn recents_rows(entries: &[RecentEntry]) -> Vec<RecentRow> {
    entries
        .iter()
        .take(MAX_RECENTS)
        .enumerate()
        .map(|(index, entry)| {
            // 1-based: the number read and the index fired are one expression, born together.
            let slot = index + 1;
            let label = if entry.exists {
                format!("{slot}. {}", entry.display)
            } else {
                format!("{slot}. {} (missing)", entry.display)
            };
            RecentRow {
                slot,
                label,
                missing: !entry.exists,
                path: entry.path.clone(),
            }
        })
        .collect()
}

/// S10: the generation, one step. Plus one, not a toggle: the port epochs move one at a time and
/// a toggle would collide after two adoptions. The PARITY is what a conditional recreation would
/// key on, so this makes the parity a fact a test can hold rather than an expression written
/// twice in two languages.
pub(crate) fn next_generation(previous: i32) -> i32 {
    previous + 1
}

/// The bridge's declared autosave cadence, the same 750 ms `bridge-gpui` states
/// (its main.rs:122) rather than reading core's - and the same rule with it: a
/// Flush only goes out when the buffer actually changed.
const AUTOSAVE_IDLE: Duration = Duration::from_millis(750);

// S10b: THE UNDO QUARANTINE, as a decision a test can hold. The capture handler in
// main.slint implements the same rule in markup (this file cannot call into it and markup
// cannot call into here, so the pair is held together by `the_quarantine_is_a_state_rule`,
// which greps every piece of the condition out of the mount).
//
// Why a STATE rule and not two more table rows: SHORTCUTS is the legend of *commands*, each
// with a port Command behind it, and it is held at fourteen rows by tests that mirror
// menu.rs. Undo and redo are not commands - the port has no Command for them and never will -
// so rows for them would print them in a legend of commands and break the parity guard the
// table exists to keep. What is different here is that the rule is armed by generation, not
// by the key alone: the same keystroke is native undo on the first document and a swallowed
// hazard on the second, which is precisely the shape a table cannot express.
// Test-only on purpose, and dead-code-clean because of it: there is no Rust-side key path to
// call this from - the implementation is the markup branch, and this is the oracle the branch is
// grepped against. Wiring it into the binary would mean inventing a call site that lies.
// (the fn this text describes is `undo_quarantined`, below, with its own copy of the summary)

// STRIP-2b: the FLUSH DECISION moved here with the pump it reads. It is not instrumentation:
// it is the rule for when the buffer's difference becomes a Command::Flush, the debounce
// clock, and the locked refusal that sits ABOVE the dispatch. The probe's tick and the
// product's tick call the same fn, which is what makes the refusal one rule instead of two.
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
pub(crate) const EDITED_IS_DIRTY_WITNESS: bool = false;

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
pub(crate) fn text_pump(ui: &Spike, gw: &Rc<RefCell<Option<Gateway>>>, pump: &RefCell<Pump>) {
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
pub(crate) fn text_pump_by_compare(
    ui: &Spike,
    gw: &Rc<RefCell<Option<Gateway>>>,
    pump: &RefCell<Pump>,
) {
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

#[cfg(test)]
pub(crate) fn undo_quarantined(
    generation: i32,
    text: &str,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> bool {
    let _ = shift; // allowed, on purpose - see the doc comment above and the Ctrl+Shift+Z hole.
    generation > 0
        && ctrl
        && !alt
        && (text.eq_ignore_ascii_case("z") || text.eq_ignore_ascii_case("y"))
}

pub(crate) fn caption_glyph(maximized: bool) -> &'static str {
    if maximized {
        "icons/restore.svg"
    } else {
        "icons/maximize.svg"
    }
}

// STRIP-1 half B: what moved here in this commit, and why it is the boundary. Everything above
// (half A) answered a question without touching the loop; this is the loop's OWN state - the
// reader of the event queue, the pump it fills, the adoption rule and the dialog mailbox. It
// belongs to the surface because a product bridge needs exactly these decisions and none of the
// acts. `text_pump`, the witnesses, the *_AT clock and every synthetic act stay in probe.rs.
//
// The pump's fields are pub(crate) because the probe's act machine builds and reads them across
// this boundary: that is the coupling, stated, rather than hidden behind a getter per field.
// There was no struct literal to kill - the probe already built Pump::default(), which is why
// moving the struct was mechanical.
//
// The two drift guards moved WITH the code they police, per the guard law, so their include_str!
// now reads this file and their slice anchor resolves to the test module at the foot of it.
// (That anchor is a substring search, so this header says "test module" instead of spelling the
// string out: the first literal occurrence of it in this file has to be the module itself.)
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
/// What the picker thread sends back: a path, or the absence of one. The absence IS the
/// cancel - rfd returns None for it, and the rule below is that it must never be silence.
pub(crate) struct DialogReply {
    kind: DialogKind,
    path: Option<PathBuf>,
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
pub(crate) fn ask_dialog(
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
pub(crate) fn answer_dialog(
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
pub(crate) fn note_adoption(pump: &RefCell<Pump>, path: &Path) -> i32 {
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
/// What the pin pump remembers. Deliberately holds no opinion about the window: the
/// only bool in here that is a FACT is the one an `Event::Pinned` wrote.
#[derive(Default)]
pub(crate) struct Pump {
    /// Set once the strip has asked on its own, so the probe needs no hands.
    pub(crate) asked: bool,
    /// Last state the PORT reported. None until it says so - never guessed.
    pub(crate) confirmed: Option<bool>,
    /// When `Event::Pinned(true)` arrived: platform-verified WS_EX_TOPMOST, so t0.
    pub(crate) applied_at: Option<Instant>,
    /// When the same-bit re-ask went out, and whether it was answered.
    pub(crate) reask_at: Option<Instant>,
    pub(crate) answered_after_reask: bool,
    pub(crate) hold_reported: bool,
    // ---- tick evidence (finding 1) ----
    /// How many `tick` needles have been taken, i.e. how many times slint delivered
    /// this callback since `ui.run()` began.
    pub(crate) ticks: u64,
    /// Bucket of the last printed tick, so the needle is ~2s apart, not every 8ms.
    pub(crate) last_bucket: u64,
    /// Events the pump has ever pulled out of the port's channel, in-run or not.
    pub(crate) seen: u64,
    /// `Saved` count, so the first one (the answer to this probe's own `SaveAs`) can
    /// be told apart from a later one, which only the autosave path could have sent.
    pub(crate) saves: u64,
    /// HYGIENE (P1b, the close-burn fix): a terminal ANSWER to a flush, whatever it was. The
    /// close wait used to watch `saves` alone - and with autosave OFF the port does not go
    /// silent, it answers `AutosaveSkipped` (engine.rs, fn `flush`), after which no `Saved` ever
    /// comes. So every close of an autosave-off session paid the whole SAVE_WAIT for an event
    /// that cannot exist. This counts the ANSWERED family: `Saved`, `SaveFailed`,
    /// `AutosaveSkipped`. A COUNT, not a bool, because the waiter has to see the answer that
    /// arrived after ITS OWN flush and not one left over from a save cycle earlier in the run.
    pub(crate) saves_settled: u64,
    /// Which of the three settled the last cycle, in the bridge's own words, so the close line
    /// can name what it exited on rather than let a reader infer it from the absence of a save.
    pub(crate) saves_answer: &'static str,
    // ---- S10 ----
    /// The dirty witness, set by `TextEdit.edited` and cleared when a flush goes out (or when the
    /// bytes turn out to be identical). True is not a claim about WHAT changed - the callback
    /// carries no text on purpose - so the byte compare still guards the send.
    pub(crate) edited_flag: bool,
    /// Keystrokes witnessed, so the cost claim is a number and not an adjective.
    pub(crate) strokes: u64,
    /// S10: the document generation, stepped at both adoption sites. See `next_generation`.
    pub(crate) generation: i32,
    /// S10b: the arming of the undo quarantine is reported once, when the generation first
    /// leaves zero - the run's own proof that the swallow is live from here on.
    pub(crate) quarantine_reported: bool,
    // ---- S8b ----
    /// The port's lock, mirrored from FileMeta so the pump can refuse without reaching back into
    /// the event that set it. `true` means the engine has already decided it will not save this
    /// document; a bridge that kept flushing anyway was the bug this field exists to close.
    pub(crate) locked: bool,
    /// The word the status line shows and the refusal needle quotes, straight from lock_verdict,
    /// so the two causes (disk vs size) cannot be conflated downstream.
    pub(crate) lock_word: String,
    /// The refusal speaks once per lock, not once per tick.
    pub(crate) lock_needled: bool,
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
    pub(crate) adopted_path: Option<PathBuf>,
    /// S4d: every ANSWER to an open - `Loaded` or `LoadFailed`, either way the engine has
    /// replied. The locked act polls this instead of draining once and hoping: the old version
    /// drained microseconds after sending, saw nothing, and printed silence that read like a dead
    /// lever. A count, not a bool, because a run opens several files.
    pub(crate) load_answers: u64,
    /// S4e: the locked act is a once-per-run event, and the one-shot lives with the rest of the
    /// act machine's memory rather than in a captured bool.
    pub(crate) lock_acted: bool,
    /// S4: the read-only fixture this run created, remembered so the exit path can clear the
    /// attribute and delete the file. A probe that leaves a read-only file in its own state dir
    /// makes the NEXT run's write fail, which is a probe lying about the run after it.
    pub(crate) lock_fixture: Option<PathBuf>,
    // ---- S5 ----
    /// The paths behind the rendered recent rows, in the order the port sent them,
    /// so a row index maps back to the file it names.
    pub(crate) recent_paths: Vec<PathBuf>,
    /// The CRLF file this probe seeded beside the exe, and its bytes on the way IN.
    pub(crate) seed: Option<PathBuf>,
    pub(crate) hash_before: u64,
    pub(crate) hash_len: usize,
    /// `edits` as it stood when the open was asked: if the adoption is honest, this
    /// does not move before the run ends.
    pub(crate) flushes_at_open: u64,
    /// Rows the last `RecentsUpdated` carried, so the needle prints on a CHANGE
    /// rather than on every delivery.
    pub(crate) rows: usize,
    /// The synthetic click: fired once, and its answer is matched by path.
    pub(crate) click_done: bool,
    pub(crate) click_pending: Option<PathBuf>,
    /// How many times the close-requested callback has run. Act 1 declines the first,
    /// Act 2 lets the second through - the count IS the act selector.
    pub(crate) closes: u32,
    /// Has the frame act run yet?
    pub(crate) max_toggled: bool,
    /// Times the pump has run. `drains` climbing while `seen` stays 0 is the proof
    /// that the callback and its 8ms poll are live and the port is simply silent.
    pub(crate) drains: u64,
    /// Has ANYTHING come back from the port yet (the registration answer is the gate
    /// the pin ask waits on).
    pub(crate) answered: bool,
    // ---- the text loop ----
    /// The port's last ANNOUNCED generation, echoed back on every Flush. The bridge
    /// counts nothing here: 0 means nothing has ever been announced, which is the
    /// file-less start this probe has.
    pub(crate) epoch: u64,
    /// Observed edits, i.e. the `revision` a Flush carries (bridge-gpui's `edits`).
    pub(crate) edits: u64,
    /// What went out last, so an unchanged buffer is never re-sent.
    pub(crate) last_sent: String,
    /// When this change first entered the debounce; the stamp a Flush reports.
    pub(crate) pending_at: Option<Instant>,
    // ---- S4b: the three signals Chrome draws (dirty dot, failed dot, menu check).
    // Each is a port FACT or this bridge's own last ask; none of them is a guess.
    /// dirty means "the buffer differs from what was last sent" - the SAME comparison
    /// the Flush guard makes in text_pump, so the dot cannot disagree with the bytes
    /// that are about to go out.
    pub(crate) dirty: bool,
    /// Latched by Event::SaveFailed, cleared by the next Event::Saved. The failure stays
    /// until the bytes actually land: a dot that blinks off on the next keystroke tells
    /// the user nothing changed, which is the opposite of section 4.4.
    pub(crate) save_failed: bool,
    /// STRIP-4b row 1: how many times this bridge has re-sent AFTER a failure. Printed, not
    /// decorative: a save that fails for a PERMANENT reason (locked by another program, disk full,
    /// a path that went away) now retries every autosave idle, so this number is how loud that loop
    /// is, and a live run can be read for it. See the SaveFailed arm for why the loop is the lesser
    /// evil and for who should really own the policy.
    pub(crate) retries: u64,
    /// The port has NO event that echoes autosave - engine.rs:601-602 assigns the bool
    /// and says nothing back - so this is InitialState's answer XORed by every
    /// Command::SetAutosave this bridge sends. The bridge's own last ask, named as such
    /// rather than passed off as a report.
    pub(crate) autosave: bool,
    /// The last needle printed, so a signal that did not move stays quiet.
    pub(crate) dot_words: String,
    /// S4b: how many ticks actually reached text_pump. Printed at exit because the
    /// question this answers is arithmetic, not opinion: if the text loop sat inside the
    /// one-shot SAMPLED block, this number would be ~1 and the mid-run flush needle
    /// would be dead. Measured: it is every tick (the block closes at the 't+SAMPLE'
    /// report), and the number below is the proof.
    pub(crate) invocations: u64,
    /// The one in-run keystroke, fired once so the dirty->flush->clean chain is proven on
    /// the bridge's own cadence instead of being inherited from a pre-run probe write.
    pub(crate) late_key: bool,
    // ---- S4c: the save-failure cycle, and the menu ----
    /// The file SaveAs landed on. Autosave rewrites THIS name, so it is where the failure
    /// has to be staged; kept because the tick cannot re-derive it.
    pub(crate) loop_path: Option<PathBuf>,
    /// S6c: the seed's explicit SaveAs (the arming act), once.
    pub(crate) seed_armed: bool,
    /// S8: which caption step has run (nine of them, see CAPTION_AT).
    pub(crate) caption_step: u64,
    /// S7: which overlay step has run - eight of them, each observing the previous act and
    /// then performing its own (open, 400px, 180px, restore, backdrop, reopen, escape, and one
    /// final observation) - plus ABOUTSLINT's three (open the licence screen through the 6th
    /// row's door, read it back, dismiss it), so eleven, the last one an observation only.
    pub(crate) overlay_step: u64,
    /// The window's own size on entry to the overlay act, to be restored at the end.
    pub(crate) start_size: Option<(u32, u32)>,
    /// The three acts, each once.
    pub(crate) ro_done: bool,
    pub(crate) key2_done: bool,
    pub(crate) wb_done: bool,
    /// How many CHORD_DRIVE steps have been driven: 0 none, then one per row of Open, Alt+2,
    /// Auto-save, Save As, Clear recents, and one past the end once Quit has fired. Named for
    /// the rows because a chord and its row reach the same callback; the number records which
    /// step of the table walk ran and nothing more.
    pub(crate) menu_act: u64,
    /// The one keystroke aimed at the seed document.
    pub(crate) seed_key_done: bool,
    /// S5: which drag act has run - 0 none, 1 the plain move, 2 the one aimed at the
    /// maximised state (a refusal when the frame act is on, a second move when it is off).
    pub(crate) drag_step: u64,
    /// The refusal print is once per maximised episode, not once per mouse-move frame.
    pub(crate) drag_refused_shown: bool,
    /// The FRAME-INCLUSIVE target the last drag asked for. The exit check compares this
    /// against what the port actually persisted, which is what the next launch restores
    /// from; kept here because the tick that printed it is long gone.
    pub(crate) drag_target: Option<(i32, i32)>,
    /// THE DRAG EPISODE IN PROGRESS: the corner the note stood at when this gesture's first
    /// delta arrived, plus the travel accumulated since, plus the one-line-per-gesture print
    /// bit. Pure state (see `DragEpisode`), owned here because this pump is the only memory
    /// the product root keeps across callbacks, and per-gesture state must die with the gesture
    /// rather than live in a bool next to it - which is why the C3 print bit that used to sit in
    /// this list is a field of the episode now.
    pub(crate) drag_episode: DragEpisode,
    /// C4: the corner shape this bridge last ASKED the port for, `None` before the first ask.
    /// The wake that watches maximisation runs every 8 ms, so without this a held maximise
    /// would name an attribute to the OS ~125 times a second; with it, a normal→maximised
    /// transition costs exactly one command. `ask_corners` is the only writer.
    pub(crate) corners_asked: Option<bool>,
    /// C4: the OS said no, and this bridge believes it - the PERMANENT no, and only that one.
    /// The corner attribute is Windows 11+ and the support floor (R11) is Windows 10, where the
    /// SET call itself answers every ask; THAT refusal has to retire the asking, not merely
    /// print. FIX-A (D2, 2026-09-15): a refusal naming any other call is transient - a handle
    /// that went stale after the guard, a read that failed, a read-back contradicting an `S_OK` -
    /// and retiring on one of those retired the SQUARE ask with it, which is how a maximised
    /// note spent a whole run floating round over the taskbar. See
    /// `corner_refusal_is_permanent`, which is where the two are told apart.
    ///
    /// There is still no retry on the SAME shape: `corners_asked` above is what bounds the rate,
    /// and a transient refusal leaves it standing. Nothing else changes - the note stays square,
    /// which is what it was before corners were a question, and the log says why.
    pub(crate) corners_refused: bool,
    /// THE POPUP'S EXPLANATION, sticky; `""` means nothing to say. This exists because
    /// docs/features.md §4.4 asks for the autosave reason IN THE MENU and the status line cannot
    /// carry one: `set_status(describe(..))` has a single writer, so the next event overwrites the
    /// sentence and the reason for a failure that is STILL TRUE is gone within a few hundred
    /// milliseconds. The pump is the only place a sticky fact can live, and
    /// [`publish_explain`](crate::plumbing::publish_explain) is the only writer to the UI.
    pub(crate) why: String,
    /// What the current file IS - the `FileMeta` verdict from the last `Loaded`/`Rebound`, put into
    /// words by [`file_words`](crate::plumbing::file_words). Written on a document change and on
    /// nothing else: saving does not alter a file's encoding, and refreshing it anywhere else would
    /// have the bridge guessing at a verdict only core holds.
    pub(crate) file_words: String,
    /// How many autosave toggles this bridge has sent. The port echoes NO autosave event,
    /// so the menu check can only follow the ask - printed as 'menu: ...' so the
    /// convention is visible instead of pretending to be a report.
    pub(crate) autosave_asks: u64,
    /// A Quit from the menu is the user's own request, so the close contract must not
    /// decline it the way it declines the FIRST synthetic request to prove declining
    /// works. One door (Window::close), one extra fact in front of it.
    pub(crate) quit_requested: bool,
}
/// Name every drained event, apply the pin to the UI, and keep the last-wins status
/// line: the two needles. `Pinned`/`PinFailed` are the ONLY writers of the rendered
/// pin bit, which is C3 - the click asks, the answer is what the strip shows.
pub(crate) fn drain(events: &Receiver<Event>, pump: &RefCell<Pump>, weak: &slint::Weak<Spike>) {
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
            // C4. The refusal-only event of the port's corner contract, and deliberately NOT
            // rendered: the pin answers here because the strip DISPLAYS a pin, and a corner
            // displays itself. What this arm does instead is retire the asking - see
            // `Pump::corners_refused` - so a Windows 10 run prints once rather than once per
            // maximise, which is the only reason the event exists at all. FIX-A (D2): WHICH
            // refusals retire it is decided in `note_corner_refusal`, one call down, because
            // latching on a transient one cost the square ask too.
            Event::CornerRoundingFailed { reason } => note_corner_refusal(pump, reason),
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
                // THE FILE, DESCRIBED, while this borrow is already open: the same FileMeta that
                // decides the lock decides what the menu's footer says about this document.
                p.file_words = file_words(meta);
                p.why.clear();
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
                // The footer follows the document, in the same breath the title does: a new file
                // has both a description and no unresolved failure to explain.
                publish_explain(pump, weak);
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
                // A Save As can land on a different file, so the description is re-read from this
                // event's meta rather than kept from the Loaded - and the arming bit is exactly the
                // thing that flips here, which is the one word in the footer a user acts on.
                p.file_words = file_words(meta);
                p.why.clear();
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
                publish_explain(pump, weak);
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
                // STRIP-4b row 3, ADR-0006 item 1: the words a row carries are built by
                // `recents_rows` - the one place the slot number, core's own label and the
                // missing-file mark are decided - so the product's rows and the tests below read
                // the SAME builder the event arm feeds the model. Everything the arm used to do by
                // hand is in that function, and the reason it is there is its doc comment.
                let rows = recents_rows(&list[..]);
                let shown = rows.len();
                let dropped = list.len() - shown;
                let greyed = rows.iter().filter(|row| row.missing).count();
                let names: Vec<slint::SharedString> =
                    rows.iter().map(|row| row.label.clone().into()).collect();
                {
                    let mut p = pump.borrow_mut();
                    // Same builder, same order: the index Chrome fires for the row it painted, and
                    // the index main.slint fires for the chord, are both the index
                    // of slot N here, because both come out of one pass over the same ten.
                    p.recent_paths = rows.iter().map(|row| row.path.clone()).collect();
                }
                // The label is core's (`display`, pre-formatted for the menu), the slot is this
                // bridge's, and the missing-file MARK is core's FACT (RecentEntry::exists) rendered
                // by the bridge's own hand. The bridge keeps the paths beside the words for
                // Alt+1..0, which routes by path and not by what a row says about itself.
                if let Some(ui) = weak.upgrade() {
                    // 1.17 finding: `ModelRc` is built from a slice or an `Rc<dyn Model>` - not
                    // from a `Vec` (only `VecModel` takes a Vec), so the borrowed slice it is.
                    ui.set_recents(slint::ModelRc::from(names.as_slice()));
                }
                // Print on a CHANGE only: three updates used to mean three needles.
                let changed = {
                    let mut p = pump.borrow_mut();
                    let changed = p.rows != shown;
                    p.rows = shown;
                    changed
                };
                if changed {
                    // The number line names the slots DRAWN, because "ten rows" and "rows 1..=10"
                    // are different claims when a chord is what reaches a row; an empty list says
                    // so in the same breath, which is the no-dangling-separator half - no rows, so
                    // nothing for a gap to divide.
                    let slots = match (rows.first(), rows.last()) {
                        (Some(first), Some(last)) => {
                            format!("slots {}..={}", first.slot, last.slot)
                        }
                        _ => String::from("no slots drawn, so the menu shows its six rows"),
                    };
                    let greyed = if greyed == 0 {
                        String::new()
                    } else {
                        format!(", {greyed} marked (missing)")
                    };
                    let cap = if dropped == 0 {
                        String::new()
                    } else {
                        format!(", {dropped} past slot {MAX_RECENTS} not drawn")
                    };
                    report(&format!(
                        "recents: rendered {shown} row{} - {slots}{greyed}{cap} - the words are core's, the number is the row's",
                        if shown == 1 { "" } else { "s" }
                    ));
                }
            }
            Event::Saved { path, revision } => {
                let which = {
                    let mut p = pump.borrow_mut();
                    p.saves += 1;
                    p.saves_settled += 1;
                    p.saves_answer = "Saved";
                    p.saves
                };
                // The bytes landed, so the failure is over - cleared here and nowhere
                // else: "latched until the next successful Saved", in one line.
                pump.borrow_mut().save_failed = false;
                // So is the menu's explanation. The footer outlives the status line by design,
                // which means it has to be cleared by design too: an "auto-save is off" still
                // hanging under a save that just worked would be the one lie this addition could
                // tell.
                pump.borrow_mut().why.clear();
                let dirty = pump.borrow().dirty;
                note_dot(pump, weak, dirty, "saved");
                publish_explain(pump, weak);
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
            // HYGIENE (P1b): THE ANSWER THAT IS NOT A SAVE. Autosave OFF was never silence -
            // the port answers this event, and it was falling through to the catch-all below,
            // which set the status line and settled NOTHING. That is why the close wait could
            // not see it and burned 2.0 s on every session with autosave off. The arm keeps the
            // catch-all's status line (still one writer of the words, `describe`) and adds the
            // settle, which is the only fact the root is waiting for.
            Event::AutosaveSkipped { reason } => {
                {
                    let mut p = pump.borrow_mut();
                    p.saves_settled += 1;
                    p.saves_answer = "AutosaveSkipped";
                    // THE REASON, made sticky. This is §4.4's "the reason in the menu", and the
                    // pattern used to end in `..`: the port answered, the bridge put the sentence on
                    // the status line, and the next event erased it - so a file that will never be
                    // saved explained itself for one frame. The words are the bridge's by api's
                    // design; plumbing::skip_words says why they are not describe()'s Debug shape.
                    p.why = skip_words(*reason).to_string();
                }
                if let Some(ui) = weak.upgrade() {
                    ui.set_status(describe(event).into());
                }
                publish_explain(pump, weak);
            }
            Event::SaveFailed { reason, .. } => {
                pump.borrow_mut().save_failed = true;
                let dirty = pump.borrow().dirty;
                // THE WITNESS COMES BACK. bridge-gpui clears its in-flight marker on Saved|SaveFailed
                // (bridge-gpui/src/main.rs:996-1000) so a failure never leaves a bridge believing
                // bytes went out. This bridge's equivalent marker is its SEND WITNESS - last_sent
                // plus the edited flag - and the pump cleared BOTH AT THE SEND, which meant: bytes
                // refused, witness clean, and nothing going out again until the user typed. A save
                // failure that cannot retry is a silent loss of the newest edit (AGENTS.md 4.4), so
                // the failure restores the witness and the next quiet tick re-sends.
                //
                // WHAT THAT BUYS AND WHAT IT COSTS, out loud: a permanent failure now retries every
                // AUTOSAVE_IDLE, forever. Each retry is a real attempt that lands the moment the
                // cause clears, and the printed count makes the loop audible instead of mysterious -
                // but a cap wants a back-off, and this bridge has no business inventing that policy
                // alone: gpui retries on the next edit and does not loop, so if the two bridges are
                // to agree, the RETRY RULE BELONGS TO THE PORT. Raised here, not settled here.
                {
                    let mut p = pump.borrow_mut();
                    p.last_sent.clear();
                    p.edited_flag = true;
                    p.retries += 1;
                    // HYGIENE (P1b): a refusal is an ANSWER too - the close wait must not
                    // sit out its deadline because the bytes it waited for are never coming.
                    p.saves_settled += 1;
                    p.saves_answer = "SaveFailed";
                    // The port's OWN sentence, passed through rather than re-worded: SaveError has
                    // a Display precisely because "the UI must render the reason" (AGENTS.md, code
                    // conventions) - unlike SkipReason, which no Display reaches, so the bridge
                    // words that one itself. The prefix is the status line's, character for
                    // character: one hex-owner rule, and the footer outlives the line it copies.
                    p.why = format!("save failed: {reason}");
                    report(&format!(
                        "retry: a failed save restored the send witness (retry #{})",
                        p.retries
                    ));
                }
                note_dot(pump, weak, dirty, "save-failed");
                if let Some(ui) = weak.upgrade() {
                    // STRIP-4b row 2: this printed the Debug of a public-contract enum - a variant
                    // name, and for the inner kinds a couple of numbers, where a person needs a
                    // sentence. The port says otherwise in its own source: "The Display text below
                    // IS the user-visible copy: to_string() goes straight into a dialog or the status
                    // line" (api/src/event.rs:188-191). One hex-owner rule: the port owns the
                    // wording, the bridge owns the sentence around it.
                    ui.set_status(format!("save failed: {reason}").into());
                }
                publish_explain(pump, weak);
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

/// STEP A OF THE PRODUCT WIRING, dated 2026-09-15: the toolkit's asks, hooked in ONE
/// place, so the product root can call a single function instead of re-implementing a menu.
///
/// Every body below is the probe's body (`probe.rs:1313-1442`), LESS the lines that exist only to
/// grade a measurement run - the ask counters, the post-act property reads, the visible /
/// close-allowed witnesses. What remains is the contract a product needs and nothing else: a row or
/// a chord is an ASK, an ask sends ONE command or moves ONE bit, and the answer comes back through
/// `drain` - never from the click. `report()` stays, because stderr is this binary's only voice
/// (product.rs:26-36); what left are the prints whose subject was the probe.
///
/// Three absences are named here rather than quietly omitted, because each is a finding for step B:
///
/// 1. `toggle-menu-asked` has NO Rust body anywhere, and must not gain one. Chrome answers its own
///    ask (chrome.slint:601 flips `menu-open`), and S7's measured finding is that a component
///    callback cannot be invoked from Rust through the parent forward (chrome.slint:113-119) - so a
///    handler here would be a second writer of a bit with exactly one owner. The menu opens fine
///    without this function.
/// 2. The recent row is the ONE body that is not a verbatim move. `open_recent` still lives in
///    probe.rs, and a module may not reach up to a root the product binary does not have - so the
///    row's three lines are restated here WITHOUT the probe's two witnesses: `click_pending` (which
///    exists only so a synthetic click can be credited to a later `Loaded`) and the per-row print.
///    Step B's fix is to move `open_recent` into this file and have BOTH roots call it; this
///    duplication is the price of fencing step A to one file, and it is written down so it cannot be
///    mistaken for the design.
/// 3. `autosave_asks` is not bumped. The counter exists so a run can name which ask #N it watched,
///    nothing renders it, and the bit the menu binds to is `autosave`, which this handler does write.
///
/// C1 (STRIP-5, 2026-09-15): THE DOCUMENT THAT WAS OPEN, ASKED FOR ONCE.
///
/// gpui found this the hard way and says so at its own STEP 5 (bridge-gpui/src/main.rs:1706-1720):
/// the session carries the path of what was open, NOTHING in the bridge asked for it, and a
/// relaunch came back to an empty editor while the bytes sat on disk. The slint product had the
/// identical hole: it cloned the session for the rect and read every field of it except `path`.
///
/// The fix is one command of EXISTING vocabulary and no new machinery: its answer is the
/// `Event::Loaded` arm in `drain` (the buffer adoption, the epoch, the title, the lock verdict),
/// which already existed. So this is a send, not a subsystem - and per the root law it lives
/// HERE, because product.rs owns no `send()` of its own.
///
/// ORDER: the engine queues a command posted before the window is registered, and gpui sends its
/// STEP 5 pre-loop too, so either side of RegisterWindow is correct. What is NOT optional is the
/// ask: no path means no ask, and a first launch or a draft that was never written comes back
/// empty - which is the truth about it.
#[allow(dead_code)] // dead in the PROBE root only: notes-slint calls this from product.rs
pub(crate) fn restore_from_session(gw: &Rc<RefCell<Option<Gateway>>>, path: &Path) {
    report(&format!(
        "startup: asking the port for {} (the path the session was restored with)",
        path.display()
    ));
    send(
        gw,
        Command::Open {
            path: path.to_path_buf(),
        },
    );
}

/// C2 (STRIP-5, 2026-09-15): the caption's remaining asks - pin, clear-recents, max/restore and
/// minimize - are hooked in `wire_callbacks` below, each one a verbatim move of the probe body it
/// mirrors (probe.rs:1316-1320, :1410, :1448, :1453, with :1693-1703 and :1825-1833 behind them),
/// minus the probe's measurement witnesses. Nothing here decides anything the port has not been
/// asked to decide: pin is `Command::SetPinned`, clear is `Command::ClearRecents`, and the two
/// frame acts are `ui.window()` calls, which are the bridge's per the geometry law.
///
/// C3 (2026-09-14) adds a FIFTH row, and it is not a verbatim move: the drag. The band's deltas
/// were already arriving and being forwarded, with no listener - see the hook below for what it
/// does and `drag_destination` for the arithmetic it corrects.
#[allow(dead_code)] // dead in the PROBE root only: probe.rs wires its own handlers
pub(crate) fn wire_callbacks(
    ui: &Spike,
    gw: &Rc<RefCell<Option<Gateway>>>,
    pump: &Rc<RefCell<Pump>>,
    dialog_tx: &std::sync::mpsc::Sender<DialogReply>,
) {
    // Open: the row, the Ctrl+O chord and any future native menu land HERE, and this handler means
    // ask a person. What comes back goes out through Command::Open - the same door the recents rows
    // use, which is the only reason a picked file keeps the epoch, the buffer adoption and the title.
    {
        let weak = ui.as_weak();
        let pump = Rc::clone(pump);
        let tx = dialog_tx.clone();
        ui.on_open_asked(move || {
            ask_dialog(DialogKind::Open, &weak, &pump, &tx);
        });
    }
    // Save As: the same ask with a different verb, answering through the door that already exists -
    // with the buffer read when the ANSWER arrives, not when the user was asked (see answer_dialog).
    {
        let weak = ui.as_weak();
        let pump = Rc::clone(pump);
        let tx = dialog_tx.clone();
        ui.on_save_as_asked(move || {
            ask_dialog(DialogKind::SaveAs, &weak, &pump, &tx);
        });
    }
    // Auto-save: toggle the local mirror, send ONE command, repaint the dot. The port echoes no
    // autosave event, so the menu's check can only follow the ask - printed as `menu:` for that reason.
    {
        let gw = Rc::clone(gw);
        let pump = Rc::clone(pump);
        let weak = ui.as_weak();
        ui.on_autosave_asked(move || {
            let was = {
                let mut p = pump.borrow_mut();
                let was = p.autosave;
                p.autosave = !was;
                // The toggle retires the last refusal. "auto-save is off" is a sentence about the
                // setting the user has just changed, and the next save answers under the new one -
                // leaving it standing would be the footer contradicting the row above it.
                p.why.clear();
                was
            };
            report(&format!(
                "menu: autosave-row toggled {was}->{} - MIRROR, not a report: the port echoes no autosave event",
                !was
            ));
            send(&gw, Command::SetAutosave(!was));
            let dirty = pump.borrow().dirty;
            note_dot(&pump, &weak, dirty, "autosave-row");
            publish_explain(&pump, &weak);
        });
    }
    // A recent row is an ask, the same shape as the pin strip: index in, Command out, and the text
    // comes back through Loaded - never from the click itself.
    {
        let gw = Rc::clone(gw);
        let pump = Rc::clone(pump);
        ui.on_open_at_index(move |index| {
            let Some(path) = pump.borrow().recent_paths.get(index as usize).cloned() else {
                return;
            };
            send(&gw, Command::Open { path });
        });
    }
    // THE POLICY, at the door: reached by the menu's Quit row AND by the caption X, and both set the
    // granted bit. Quit hides nothing and calls no gateway - it bumps close-arm, which runs
    // Window::close() in markup, which lands on the same on_close_requested an OS close reaches,
    // where the final flush and the joined shutdown live. One door, and one extra fact in front of it.
    {
        let pump = Rc::clone(pump);
        let weak = ui.as_weak();
        ui.on_quit_asked(move || {
            let Some(ui) = weak.upgrade() else { return };
            let arm = {
                let mut p = pump.borrow_mut();
                p.quit_requested = true;
                p.closes + 1
            };
            report(&format!("menu: Quit -> close-arm {arm} (single door)"));
            ui.set_close_arm(arm as i32);
        });
    }
    // C2 (1) THE PIN, from the title strip and the ^ menu row. probe.rs:1316-1320 is the body this
    // mirrors: ONE command out, nothing rendered locally, and the state asked for comes from the
    // port's last fact (`confirmed`, written by the Pinned / PinFailed arms in `drain`) rather than
    // from a counter of its own. The widget's guess is dropped on purpose - see main.slint:393-398.
    {
        let gw = Rc::clone(gw);
        let pump = Rc::clone(pump);
        ui.on_toggled_pin(move || {
            let next = !pump.borrow().confirmed.unwrap_or(false);
            report(&format!("pinned: strip click -> SetPinned({next})"));
            send(&gw, Command::SetPinned(next));
        });
    }
    // C2 (2) Clear recents: an ask, one command, and the list redraws from the event that answers
    // it - never from the click. probe.rs:1410 -> clear_recents (probe.rs:1793-1796).
    {
        let gw = Rc::clone(gw);
        ui.on_clear_recents_asked(move || {
            report("recents: menu row -> Command::ClearRecents");
            send(&gw, Command::ClearRecents);
        });
    }
    // C2 (3) Max / restore: the band's double-click and the caption button are the SAME door
    // (main.slint:405), and the body is probe.rs:1693-1703 minus its readback act. The glyph is
    // printed because the window's own bit is the only input to which caption asset it now shows.
    {
        let gw = Rc::clone(gw);
        let weak = ui.as_weak();
        let pump = Rc::clone(pump);
        ui.on_toggle_max(move || {
            let Some(ui) = weak.upgrade() else { return };
            let window = ui.window();
            let want = !window.is_maximized();
            window.set_maximized(want);
            report(&format!(
                "frame: toggle-max -> maximised={} glyph={}",
                window.is_maximized(),
                caption_glyph(window.is_maximized())
            ));
            // The corner follows the frame in the same act, from the same read-back: a
            // maximise that keeps its rounded corners floats over the taskbar like a
            // dialog, and the user watches it happen. Read BACK rather than `want`
            // because this act is the toolkit's, not ours; if the read lags the ask, the
            // dedupe in `ask_corners` lets the next wake correct it by one command.
            //
            // FIX-A (D1): the same gate the product tick passes, read the same way the tick's
            // registration reads it - `hwnd_of` is the one question the port's register command
            // answers. A caption button cannot be clicked before the window exists, so today this
            // is belt-and-braces; it is spelled out because the gate belongs in the policy, not
            // in the memory of whichever caller got there first.
            let wired = hwnd_of(window).is_some();
            ask_corners(&gw, &pump, !window.is_maximized(), false, wired);
            send(&gw, Command::GeometryChanged);
        });
    }
    // C2 (4) Minimize: the caption button's ask, probe.rs:1825-1833. It sends NO command - a
    // minimise is not a resting place the port should store, which is why the hook above sends and
    // this one does not.
    {
        let weak = ui.as_weak();
        ui.on_minimize_requested(move || {
            let Some(ui) = weak.upgrade() else { return };
            let window = ui.window();
            window.set_minimized(true);
            report(&format!(
                "caption: minimize -> is_minimized={}",
                window.is_minimized()
            ));
        });
    }
    // C3 (5) THE DRAG. The band has been emitting since S5 (`chrome.slint:529-550`: sample the
    // pointer on press, `drag-delta` on every move while pressed, `drag-ended` once on release),
    // main.slint has been forwarding since the same slice (:408-412), and NOTHING on the product
    // side was listening - so every title-band drag fell on the floor of an unwired callback.
    // This is the bug the user reported as "we cannot move the window", and it is the same shape
    // as STEP B's unwired chords: markup fires, Rust never registered. C3 wired the handler; the
    // handler it wired was a read-modify-write, and the same gesture still lagged the pointer -
    // so the delta now goes through `DragEpisode`, which holds the gesture's origin and its
    // accumulated travel instead of re-reading the corner the write is meant to move.
    {
        let weak = ui.as_weak();
        let pump = Rc::clone(pump);
        ui.on_drag_delta(move |dx, dy| {
            drag_by(&weak, &pump, dx, dy);
        });
    }
    // The release is the OTHER half, and it is the only half that talks to the port: moving the
    // window is a per-frame act, asking it to STORE a rect is not. main.slint:97-99 states this
    // contract for the drag ("the port is asked to STORE a rect once per drag, on the drop, not
    // once per frame"), and the settle watch in product.rs is the general net that catches every
    // other way a rect changes - so a drag lands two GeometryChanged asks, which the engine
    // answers by queueing one session write. Deterministic beats opportunistic here.
    {
        let gw = Rc::clone(gw);
        let pump = Rc::clone(pump);
        ui.on_drag_ended(move || {
            // TAKE THE GESTURE'S FACTS, THEN KILL THE GESTURE. `end()` is what stops the next
            // press from accumulating onto this one - the hazard an episode exists to create, and
            // the reason the release is not merely a send.
            let (target, travel, moved) = {
                let mut p = pump.borrow_mut();
                let target = p.drag_target.take();
                let travel = p.drag_episode.travel;
                let moved = p.drag_episode.origin.is_some();
                p.drag_episode.end();
                (target, travel, moved)
            };
            // The total is printed ONCE, here, which is where an accumulated gesture belongs: the
            // delta-vs-read question micro-2 has to settle is answerable from this line plus the
            // first-delta line, without a print per mouse-move frame.
            report(&format!(
                "drag: released from {target:?} (travel {:?}, {}), asking the port to store the rect",
                travel,
                if moved { "episode closed" } else { "no episode was open" }
            ));
            // ONE ask to store the rect, and it is the release's - including for a drag the
            // maximised state refused, which asked for nothing and moved nothing: the release is
            // the only door, so a refusal cannot half-open it, and a rect that did not change
            // costs the engine a write of what it already had.
            send(&gw, Command::GeometryChanged);
        });
    }
}

/// THE DRAG'S ARITHMETIC, pure, so the DPI half of it is testable without a window.
///
/// IN: the window's CURRENT position in PHYSICAL pixels, the pointer's delta in LOGICAL pixels
/// (every `length` in the markup is logical), and the scale factor. OUT: the target, in PHYSICAL
/// pixels - which is how it is fed to `set_position`, through `WindowPosition::Physical`. One
/// conversion, in this function, and the read and the write are the same unit by construction.
///
/// WHY NOT INHERIT THE INSTRUMENT'S LINE: `probe.rs:1755-1757` adds a logical delta to a physical
/// position and then writes a LOGICAL target, and says so out loud - "at 1.25 or 1.5 the division
/// by `window.scale_factor()` belongs in THIS function", with the whole probe pinned to scale 1.0
/// where the identity holds. That is the same bug class R1 found from the other direction (a band
/// that reads one kind of rect and writes the other walks the window across the screen): at 150 %
/// a drag would move the window two thirds of the pointer's travel, and the user's own pointer
/// outruns the note. Retiring the debt is cheaper than re-documenting it, and the tests below are
/// the proof that scale 1.0 still behaves exactly as the instrument measured.
pub(crate) fn drag_destination(here: (i32, i32), dx: f32, dy: f32, scale: f32) -> (i32, i32) {
    // A scale of 0 or NaN is not a thing to propagate through a window position: it would freeze
    // the note in place (0) or park it at an undefined point (NaN), and neither is recoverable
    // from the log. Fall back to 1.0, which is what the rest of this crate's startup path does.
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    (
        here.0 + (dx * scale).round() as i32,
        here.1 + (dy * scale).round() as i32,
    )
}

/// How long a delta may be followed by silence before the next one is read as a NEW press
/// rather than the continuation of this one. A real drag emits a callback per mouse-move frame
/// (this bridge wakes every 8 ms), so a quarter second without one is a released band whose
/// release edge never arrived, not a person thinking mid-gesture. Both ways of getting it wrong
/// are named at `DragEpisode::gone_cold`.
const DRAG_EPISODE_GAP: Duration = Duration::from_millis(250);

/// ONE title-band gesture's own state, so the arithmetic never reads the thing it is writing.
///
/// THE BUG THIS KILLS (2026-09-15, the "we cannot move the window" repair, micro-1 of three).
/// `drag_by` used to read `window.position()`, add the delta and write the sum back - a
/// read-modify-write whose INPUT is the very thing the write is supposed to change. Slint's
/// `set_position` is an ASK: what `position()` reports afterwards is whatever the windowing
/// layer has applied, so a delta that arrives before that lands reads the OLD corner and re-asks
/// for a target the note already holds. Two events 6 px apart both reading `<390,278>` both ask
/// for `<396,278>`, and the pointer's second 6 px is gone for good. The ruler rides the thing it
/// measures. It is scale-independent, which is precisely why every DPI needle in this file stayed
/// green while the note lagged the hand.
///
/// WHY THIS SHAPE IS RIGHT WHICHEVER WAY THE MEASUREMENT LANDS. Micro-2 has to decide which of
/// the two quantities is actually starved on a real desktop, and the two candidate answers used
/// to need two different repairs. Accumulating the travel fixes the read-starved case outright -
/// the window's position is sampled AT MOST ONCE PER GESTURE, so nothing downstream can starve
/// it - and it fixes the delta-starved case as far as the bridge can: a delta that is never
/// emitted is lost by any scheme, but a delta that IS emitted can no longer be spent against a
/// stale base. Whichever way the measurement lands, the OS-apply dependency leaves the arithmetic,
/// and the only thing still depending on it is the read-back in the print, which is a WITNESS and
/// not an input.
///
/// WHAT THIS CARRIES AND WHAT IT COULD CARRY: the press's cursor position in physical px would
/// let a gesture be reconstructed from one sample instead of a sum of deltas, and the band has it
/// (`ui/chrome.slint` samples `mouse-x`/`mouse-y` on press). The product cannot see it -
/// `drag-delta(dx, dy)` is the only fact that crosses the seam - so holding it would cost a
/// markup argument. Not taken: accumulated travel needs no press sample to be correct, and
/// reaching into the markup to store a field that changes no arithmetic is the kind of scope the
/// project's own rules ask be raised, not spent.
///
/// THE ONE HAZARD THIS CREATES, priced rather than hidden: an episode whose release never arrives
/// keeps its origin, and the NEXT press would then accumulate onto the LAST gesture - a jump
/// across the screen. `end()` on the release is the reset; `gone_cold` is the net under a
/// release that never came. Both are tested below, because a fix that trades lag for teleporting
/// is not a fix.
#[derive(Default)]
pub(crate) struct DragEpisode {
    /// Where the note's corner stood at this gesture's FIRST delta, in PHYSICAL px - the same
    /// unit `drag_destination` returns, so the write below never converts twice. `None` means
    /// no gesture in progress: the next delta samples a fresh origin.
    origin: Option<(i32, i32)>,
    /// Pointer travel since that origin, PHYSICAL px, summed one rounded step per event by the
    /// one arithmetic function. Zero deltas add nothing and, more importantly, lose nothing.
    /// THE DISPLACEMENT, not the sum. Since S1 it is derived from the same two numbers the ask is
    ///  derived from (`target - origin`), which is what lets the release line keep saying "travel"
    /// and mean the pointer's own travel. See the note on `advance`: with the band holding its
    /// anchor, `here + delta` IS `origin + pointer_travel`, exactly, on every delta - so this field
    /// is now a WITNESS of the arithmetic rather than a second arithmetic that could disagree.
    travel: (i32, i32),
    /// THE SCALE THIS GESTURE WAS MEASURED IN, sampled with its origin and then compared on every
    /// later delta. A DPI change mid-drag makes each subsequent delta a conversion of logical px
    /// by a factor the press never used, and there is no honest way to spend it: re-sampling the
    /// origin throws away the pointer and re-basing the band's anchor throws away the frame, and the
    /// two bridges would then disagree about which one moved. So the episode STOPS WRITING and says
    /// so once; the release re-opens the door at the new scale. `None` while an origin is held is
    /// the frozen state, which is why this is one field and not two - see `frozen`.
    scale: Option<f32>,
    /// When the last delta of this episode arrived, and whether this episode has printed once
    /// yet. Both are per-gesture facts, so both die with the gesture.
    last: Option<Instant>,
    printed: bool,
}

impl DragEpisode {
    /// ONE delta of ONE episode. IN: what the window reads right now (`here`), which is trusted
    /// exactly once per gesture; the pointer's delta in LOGICAL px; the scale; and the clock,
    /// taken as an argument so a stall is testable without waiting for one. OUT: the target in
    /// PHYSICAL px, ready for `set_position`.
    pub(crate) fn advance(
        &mut self,
        here: (i32, i32),
        dx: f32,
        dy: f32,
        scale: f32,
        now: Instant,
    ) -> (i32, i32) {
        // A cold start or a cold gap: sample, and start the travel over. Everything after this
        // line ignores `here` - that ignoring IS the repair.
        if self.origin.is_none() || self.gone_cold(now) {
            self.origin = Some(here);
            self.travel = (0, 0);
            self.printed = false;
            // The press delta settles the gesture's scale, so every later delta has something to be
            // compared against and `None` means exactly one thing for the rest of the episode: the
            // DPI moved and this gesture has stopped writing.
            self.scale = Some(scale);
        }
        let Some(origin) = self.origin else {
            // Unreachable: the block above just filled it. A return rather than an expect(),
            // because a position this bridge cannot read is not worth a panic over.
            return here;
        };
        // THE SCALE LAW, and it is checked BEFORE the arithmetic because a frozen gesture must not
        // move even by the amount it is being asked to freeze. A scale that differs from the
        // gesture's own spends this episode's `scale` on the way in, which is the whole state
        // change: `frozen()` is then true for every later delta until `end()` resets the episode,
        // and the caller is told once. The press delta can never take this branch - it has no
        // recorded scale to disagree with.
        if let Some(at_press) = self.scale {
            if at_press != scale {
                // THE FREEZE, and the state change is the spending: `scale` goes to None, so every
                // later delta of this episode takes the branch below, and the caller - which sees
                // the transition, not just the state - prints once. Nothing else moves.
                self.scale = None;
                self.last = Some(now);
                return here;
            }
        } else {
            // Already frozen. The frame is handed back unchanged: re-asking for where the note
            // stands is honest, while re-deriving that place from a factor nobody agreed to is not.
            self.last = Some(now);
            return here;
        }
        // S1: THE LIVE READ IS THE BASE AGAIN, and that is not the bug this file spent three micro-
        // slices killing. The bug was adding an INCREMENT to a read that had not caught up with the
        // last write. ui/chrome.slint's band has held its press anchor since 1085ef63, so the delta
        // that crosses the seam is no longer an increment - it is the OUTSTANDING ERROR between
        // pointer travel and applied travel. Two cases, and the same expression is right in both:
        // the apply landed, `here` moved with it, and the outstanding error has shrunk to the new
        // pointer step; the apply has not landed, `here` is behind, and the outstanding error has
        // grown to cover exactly what was not applied. `here + delta` is therefore the full
        // pointer-minus-frame decomposition, and it costs no accumulator to get there.
        //
        // WHY THE ACCUMULATOR IS GONE RATHER THAN MERELY UNUSED: it summed cumulative quantities.
        // Five 6 px events asked for 396, 402, 408, 414, 420 - a gesture worth 18 px that outran
        // the pointer by the amount it had already been told twice. The sum was the pre-hold
        // leftover, correct only while the band re-anchored every event, and keeping it next to a
        // held anchor is the quadratic case in disguise.
        let step = drag_destination(here, dx, dy, scale);
        // The displacement the release line reports, taken from the SAME arithmetic rather than
        // beside it: with a held anchor the step already IS origin-plus-pointer-travel, so this is
        // a reading of the answer, not a second answer.
        self.travel = (step.0 - origin.0, step.1 - origin.1);
        self.last = Some(now);
        step
    }

    /// The release: this gesture is over, so its origin is history and its travel is spent.
    pub(crate) fn end(&mut self) {
        *self = Self::default();
    }

    /// True while a gesture owns an origin whose first delta has not printed. The caller prints
    /// one line per episode for the same flood reason `drag_refused_shown` exists for.
    pub(crate) fn unprinted(&self) -> bool {
        self.origin.is_some() && !self.printed
    }

    /// Is this gesture frozen by a DPI move it cannot honestly spend? Derived from the ONE field
    /// the freeze spends, so there is no second bit to keep in step and no way for the answer to
    /// disagree with the state that caused it.
    pub(crate) fn frozen(&self) -> bool {
        self.origin.is_some() && self.scale.is_none()
    }

    /// Mark this episode's one line as printed.
    pub(crate) fn mark_printed(&mut self) {
        self.printed = true;
    }

    /// Is the last delta far enough behind to be a different press? Getting this wrong in the
    /// SHORT direction re-samples mid-gesture, which costs the user at most the travel since the
    /// last wake; in the LONG direction a stranded origin survives and the next press JUMPS. The
    /// cheap side of that asymmetry is where the window sits, which is why the gap is a quarter
    /// second and not a frame.
    fn gone_cold(&self, now: Instant) -> bool {
        match self.last {
            // `duration_since` saturates at zero for a clock that went backwards, which cannot
            // then read as cold - the safe direction for the same reason as above.
            Some(previous) => now.duration_since(previous) > DRAG_EPISODE_GAP,
            None => false,
        }
    }
}

/// THE DRAG, moved. Reading and writing the window is bridge work (AGENTS.md: geometry RESTORE is
/// the bridge's, geometry STORAGE is core's), so this function is the only place in the product
/// that turns a pointer's movement into a position.
fn drag_by(weak: &slint::Weak<Spike>, pump: &RefCell<Pump>, dx: f32, dy: f32) {
    let Some(ui) = weak.upgrade() else { return };
    let window = ui.window();
    // REFUSE while maximised - the one guard the instrument earned the hard way and records at
    // probe.rs:1724-1742: a maximised window reports `position()` as `<-8,-8>` (the invisible
    // border), so "read, add, write" is arithmetically perfect and semantically wrong, and it
    // made the PORT STORE `-8,52` as the normal position, destroying a restore point while every
    // delta needle still read cleanly. The check cannot see that class, because the arithmetic is
    // not the bug: the INPUT is meaningless. Printed once per episode for the same flood reason.
    if window.is_maximized() {
        let mut p = pump.borrow_mut();
        if !p.drag_refused_shown {
            p.drag_refused_shown = true;
            drop(p);
            report("drag[refused]: maximised, window unmoved");
            // And said to the person, not just to stderr: a refused drag is an input that did
            // nothing, which is the class `locked` (S8b) and the pin refusal both answer on the
            // status line. The legend is a lot of small text to overwrite, and it is the only
            // surface a person is already reading. `ui` is already upgraded above - a second
            // `weak.upgrade()` here would be the same handle twice for no reason.
            ui.set_status("drag refused: the note is maximised".into());
        }
        return;
    }
    // S1-b: PARKED, NOT MOVED - the same class of lie as the arm above, and it reads the OS's own
    // bit rather than guessing from the numbers. Minimising puts the frame at (-32000,-32000)
    // (plumbing.rs's Fingerprint::minimized, measured, not assumed), so a delta that arrives while
    // the note is in the lot is an addition to a parking place: the arithmetic is perfect, the
    // answer is a restore point made out of -32000, and this bridge has already had the port store
    // a garbage rect once (probe.rs:1724-1742, the maximised case). It is placed ABOVE the read for
    // the same reason the maximised arm is: a refusal must not sample an origin, must not write a
    // position, and must not reach `advance` at all - the two greps in
    // `a_drag_refused_while_maximised_moves_nothing_and_asks_once` bound exactly that region.
    //
    // ONE LINE PER GESTURE, reusing the refusal latch rather than adding a second bit to keep in
    // step with the first: a delta that lands clears it, so a note restored mid-drag prints its
    // first real move and only then starts refusing again if it is put back in the lot.
    if window.is_minimized() {
        let mut p = pump.borrow_mut();
        if !p.drag_refused_shown {
            p.drag_refused_shown = true;
            drop(p);
            report("drag[refused]: minimised, window unmoved");
        }
        return;
    }
    // READ ONCE PER DELTA, AND BELIEVED. `here` used to be sampled once per gesture and then
    // ignored, because the delta arriving beside it was an INCREMENT and adding an increment to a
    // frame that has not caught up is the moving-ruler bug. The band holds its anchor now, so the
    // delta is the outstanding error against THIS frame and the two of them add up to the pointer.
    // The read-back below stays a witness, as it always was.
    let here = window.position();
    let scale = window.scale_factor();
    let (want, was_refusing, first, froze_now, p_frozen) = {
        let mut p = pump.borrow_mut();
        // Before and after the one call that can spend the gesture's scale - the pair is what
        // turns a STATE into a TRANSITION, and a transition is the only thing that may print.
        let was_frozen = p.drag_episode.frozen();
        let want = p
            .drag_episode
            .advance((here.x, here.y), dx, dy, scale, Instant::now());
        let p_frozen = p.drag_episode.frozen();
        // THE FACT THAT WAS THE POINT OF C3: the frame-inclusive target the last delta asked for,
        // printed at the release and compared by the instrument against what the port stored.
        p.drag_target = Some(want);
        // The refusal bit doubles as "the status line is currently lying about this band": a
        // delta that lands means the note moves after all, so stop saying otherwise.
        let was_refusing = p.drag_refused_shown;
        p.drag_refused_shown = false;
        // One line per gesture, not per mouse-move frame (the flood reason `drag_refused_shown`
        // carries for the refusal print). Owned by the episode now, so a gesture the gap restarts
        // prints again instead of going silent for the rest of the run.
        let first = p.drag_episode.unprinted();
        p.drag_episode.mark_printed();
        // The TRANSITION, decided in here where both halves are in scope: the delta that spent
        // the gesture's scale prints, and every later one of the same episode does not.
        (want, was_refusing, first, !was_frozen && p_frozen, p_frozen)
    };
    // THE SCALE LAW, and the transition is the trigger, not the state: `advance` spends the
    // episode's scale when the factor moves under the drag, so the delta that froze it is the one
    // that prints and every later delta of the same gesture is silent and inert. A release resets
    // the episode, which is why the sentence says what it says.
    if froze_now {
        report(&format!(
            "drag: scale changed mid-gesture, release to re-grab (asked <{},{}>, note left where it stands)",
            want.0, want.1
        ));
    }
    if !p_frozen {
        window.set_position(PhysicalPosition::new(want.0, want.1));
    }
    // What the toolkit reads back is the fact; what was asked for is the intention, and the two
    // are printed together because the gap between them is the DPI/clamp story.
    let back = window.position();
    if was_refusing {
        ui.set_status(legend().into());
    }
    if first {
        // HAMBURGER-3: THE DRAG DISMISSES WHAT IS OPEN, through the door that already exists.
        // THE HAZARD IS MEASURED, not supposed: the click-away catcher is mounted BETWEEN the
        // editor and the bar (ui/main.slint:345-359), so the band paints above it and a press there
        // never reaches it. Which means a press on the band while the menu is open DRAGS WITH THE
        // MENU OPEN, and a row's TouchArea - above the catcher too - answers the RELEASE inside the
        // row: a gesture that starts on the title and ends over row 0 asks to OPEN a file. Closing
        // the popup on THIS wake is what removes that target before the release lands.
        //
        // WHO CLOSES IS NOT THIS FILE'S QUESTION. `close-asks` is the root's in-out number
        // (ui/main.slint:154), forwarded to Chrome (:373), and Chrome's `changed close-asks`
        // (ui/chrome.slint:629-632) writes `menu-open = false` and `about-open = false` - the same
        // two lines a backdrop click and an Escape reach. So menu-open keeps exactly ONE writer, no
        // callback is added, no state is declared, and this line is the whole change. ONCE per
        // gesture, because the latch that gates it is the SAME `unprinted`/`mark_printed` pair that
        // bounds the print below - no second bit to keep in step. A press that produces no delta
        // opens no episode and asks for nothing, and so does the maximised arm above: a refusal
        // never reaches `first`. A drag that finds nothing open costs Chrome two writes of `false`
        // to bits that are already false - no change, no re-render, no second owner to notice.
        ui.set_close_asks(ui.get_close_asks() + 1);
        // THE SAME LINE C3 PRINTED, byte for byte on a gesture's first delta (where the episode's
        // origin and this read are the same number by construction), so every recorded verdict
        // about it keeps meaning what it said. The accumulation it cannot show is the accumulation
        // the tests below show; the release line carries the episode's total, once per gesture.
        report(&format!(
            "drag: from <{},{}> by <{dx},{dy}> at scale {scale} -> asked <{},{}>, reads <{},{}>",
            here.x, here.y, want.0, want.1, back.x, back.y
        ));
    }
}

/// C4: the corner POLICY, in exactly one function so there is one place that says what a
/// window's corners should be. The rule is the one Windows itself follows - round when the
/// window has a shape of its own, square when it fills a monitor - and it lives HERE because
/// this is the crate that can see the window: `api` routes the ask and `platform` makes the
/// call, and neither of them decides anything.
///
/// Why square-when-maximised is not left to the OS: `DWMWCP_DEFAULT` would do that by itself,
/// but this window is `no-frame`, and DWM's default policy does not round an undecorated
/// window at all (measured 2026-09-14: the live note reported preference DEFAULT and a square
/// corner pixel). Asking for `ROUND` is what gets the corner; asking `DONOTROUND` on the way
/// to full-screen is what stops a maximised note from floating with rounded corners over the
/// taskbar, which no decorated window on this OS does.
///
/// DEDUPED, because the caller is an 8 ms wake (`ask_corners` is called from the tick that
/// already reads the window's fingerprint, and from the caption's own toggle). Two calls in a
/// row for the same shape cost the second one nothing at all - not a command, not a print.
///
/// `parked` is the minimised bit: while the window is in the OS's parking lot its maximised
/// bit reads false for a window that is maximised, so an ask made there would be a fact about
/// the park. Skipping is safe because the wake after the restore sees the real state, and the
/// same reasoning already governs the rect two lines from that call site.
///
/// `wired` is the third gate, and the one that used to be missing (FIX-A / D1, 2026-09-15):
/// HAS A WINDOW ACTUALLY REACHED THE PORT. Startup law says the handle may not exist after
/// `show()`, and this wake runs from the first tick, so the first asks of a run used to be made
/// into an engine with nothing registered - and the engine's corner arm, api's engine.rs:838-852,
/// is `if let Some(handle) = self.window` with NO else and NO event: a drop
/// so complete that not even a refusal came back. The bridge latched the shape it had not asked
/// for and printed "round" for it, so the note was square for the whole session behind a log
/// line saying otherwise. THAT is why the gate is a parameter rather than a check inside: the
/// one fact that decides it - a handle the engine holds - is held by the caller's registration,
/// and a policy function that guesses it is the second copy the law forbids. Each call site
/// passes the truth nearest it: the product tick threads its own registration latch, and the
/// caption's toggle reads the handle the same way that latch was set.
///
/// OUT: `Some(round)` when this wake spoke - the shape asked for, which is also the fact the
/// tests below assert. `None` means this function neither sent, nor latched, nor printed: the
/// three are one branch, and no gate can silence one of them while leaving the others.
pub(crate) fn ask_corners(
    gateway: &Rc<RefCell<Option<Gateway>>>,
    pump: &RefCell<Pump>,
    round: bool,
    parked: bool,
    wired: bool,
) -> Option<bool> {
    let verdict = {
        let mut p = pump.borrow_mut();
        // THE GATES, as a pure decision (`corner_says_ask`) so all four are testable with no
        // window and no engine; the latch moves only when the verdict says to send.
        let verdict = corner_says_ask(p.corners_asked, p.corners_refused, round, parked, wired);
        if verdict {
            p.corners_asked = Some(round);
        }
        verdict
    };
    if !verdict {
        return None;
    }
    report(&corner_words(round));
    send(gateway, Command::SetCornerRounding(round));
    Some(round)
}

/// The corner policy's ONE question: does this wake ask? Pure, in the shape `register_says` and
/// `settle_says` already take - four remembered bits in, one bool out - so the guards are
/// testable without a window, an engine, or the 8 ms clock.
///
/// The order is the order of cost, cheapest first, and the two FIX-A gates (`wired`, `parked`)
/// sit BEFORE the dedupe on purpose: they are not repeats, they are wakes that must not even
/// latch, so that the wake after them is free to ask. A gate placed after `asked == Some(round)`
/// would be a gate that a silent first wake had already disarmed.
fn corner_says_ask(
    asked: Option<bool>,
    retired: bool,
    round: bool,
    parked: bool,
    wired: bool,
) -> bool {
    // No handle at the port: the command would be dropped in silence, so nothing is asked,
    // nothing is remembered, and nothing is said. In the OS's parking lot: the shape this wake
    // read is not the window's shape. Retired by a PERMANENT refusal: the asking is over. And
    // the dedupe: one ask per shape change, because the caller is a wake that runs 125 times a
    // second.
    wired && !parked && !retired && asked != Some(round)
}

/// The one line an ask prints, returned rather than printed so a test can name the words the
/// log carries without capturing stderr. Byte-for-byte the sentence this function used to build
/// inline: the smoke lane's corners row depends on it, and D1's whole complaint was that the
/// line could be said without a send behind it - so the fix is the gate, never the wording.
fn corner_words(round: bool) -> String {
    format!(
        "corners: {} (the window is {})",
        if round { "round" } else { "square" },
        if round { "normal" } else { "maximised" }
    )
}

/// The Win32 entry point whose refusal means the OS has never heard of the attribute:
/// `DwmSetWindowAttribute` failing IS the Windows 10 case, and no later ask will go differently.
const SET_CORNER_API: &str = "DwmSetWindowAttribute";

/// Which refusals retire the corner asking: only the one that names the SET call.
///
/// THE BUG THIS KILLS (FIX-A / D2). `Event::CornerRoundingFailed` used to latch
/// `corners_refused` whatever the reason said, and the reasons are not one thing. Three of the
/// four refusals `platform` can return are TRANSIENT - `InvalidHandle` (the handle went stale
/// between the guard and the call: platform/lib.rs says a guard cannot hold a window open),
/// a `DwmGetWindowAttribute` that failed outright, and the read-back contradiction, which is
/// this crate's own sentence about a preference some policy or theme owns. Retiring on any of
/// them also retired the SQUARE ask, so one hiccup in a single round ask left a maximised note
/// floating round over the taskbar for the rest of the run - the exact defect the corner policy
/// exists to prevent. A transient refusal now prints, leaves `corners_asked` standing, and the
/// next shape change asks again; the rate is bounded by that dedupe rather than by a latch.
///
/// WHY A STRING, AND WHAT WOULD REPLACE IT: the port FLATTENS the typed refusal -
/// `engine.rs:1777-1787` does `reason: error.to_string()` - so by the time the bridge sees it,
/// `PlatformError`'s shape is gone and a `match` is not available at this layer (and reaching
/// for `platform`'s type from a bridge is the layering violation check-arch exists to fail).
/// What survives the flattening is the SENTENCE, and `PlatformError::Win32`'s Display is
/// `"Win32 {api} failed: {message}"` with `api` naming the call that refused
/// (platform/lib.rs:96-108; corners.rs:97-98 hands it `"DwmSetWindowAttribute"`, and the
/// read-back branch hands it `"DwmGetWindowAttribute"`). Naming the failing api is therefore
/// documented behaviour of the port's contract, not an accident of formatting - but it is still
/// prose, so the test below feeds the EXACT shapes both ends produce and a Display change that
/// drops the api name fails there rather than silently retiring asks again.
///
/// The right replacement is a TYPED refusal: an `Event::CornerRoundingRejected { api, message }`,
/// or a `permanent: bool` decided down in `platform`, where the variant is still known. Either
/// way this function becomes a `match` on `api`, and this paragraph becomes a line in the port's
/// changelog. That is a request into `api` (15 Events are pinned), not an edit this fence can
/// make.
fn corner_refusal_is_permanent(reason: &str) -> bool {
    reason.contains(SET_CORNER_API)
}

/// What a refusal does to the asking, and what it says. Split out of `drain` so the two kinds
/// of refusal are testable without an engine to emit the event.
fn note_corner_refusal(pump: &RefCell<Pump>, reason: &str) {
    let permanent = corner_refusal_is_permanent(reason);
    if permanent {
        pump.borrow_mut().corners_refused = true;
    }
    report(&format!(
        "corners: the OS refused the request ({reason}); {}",
        if permanent {
            "not asking again this run"
        } else {
            "transient - the next shape change asks again"
        }
    ));
}

#[cfg(test)]
mod tests {
    // The grep guards below need no parent names, so this module had no import until the drag's
    // arithmetic became testable without a window - which is the whole point of it being a
    // separate function. The recents tests are the same argument one step on: a ROW BUILDER that
    // touches no window can be driven by real files, so it is driven by real files.
    use super::{
        DragEpisode, MAX_RECENTS, Pump, RecentEntry, RecentRow, SHORTCUTS, ask_corners,
        corner_refusal_is_permanent, corner_says_ask, corner_words, drag_destination,
        note_corner_refusal, recents_rows,
    };
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const MARKUP: &str = include_str!("../ui/main.slint");

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
        let whole = include_str!("../src/surface.rs");
        let src = &whole[..whole.find("mod tests").expect("the tests module")];
        // STRIP-2b: the refusal half used to live in probe.rs, which is why this guard read TWO
        // files and cut each at its own test module. The flush machinery has since moved in here,
        // so both halves of the invariant are audited in one file, at one anchor. If they ever
        // part again, the two-file read has to come back: a single-file slice that quietly stops
        // covering one half is the failure mode this crate's guard law names.
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
        let whole = include_str!("../src/surface.rs");
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
    fn a_failed_save_rearms_the_witness_and_the_copy_is_the_ports() {
        // STRIP-4b rows 1 and 2, grep-grade: a guard that CALLS the arm cannot exist (drain needs
        // a window and a queue), so the invariant is asserted against the source it is written in,
        // sliced at this file's own test module - the self-grep trap this crate already learned.
        let whole = include_str!("../src/surface.rs");
        let src = &whole[..whole.find("mod tests").expect("the tests module")];
        let arm = &src[src
            .find("Event::SaveFailed { reason, .. } =>")
            .expect("the arm")..];
        let arm = &arm[..arm.find("other =>").expect("the next arm")];
        assert!(
            arm.contains("p.last_sent.clear();"),
            "the send witness must come back"
        );
        assert!(
            arm.contains("p.edited_flag = true;"),
            "and so must the edited-flag witness"
        );
        assert!(
            arm.contains("p.retries += 1;"),
            "the loop is counted and printed, not hidden"
        );
        assert!(
            !arm.contains("{reason:?}"),
            "Debug of a public enum is not the user's copy"
        );
        assert!(
            arm.contains("format!(\"save failed: {reason}\")"),
            "the port's Display sentence, one owner of the wording"
        );
        assert!(
            !src.contains("reason:?"),
            "no Debug-formatted port copy may come back anywhere in the surface"
        );
    }

    #[test]
    fn a_vanished_recent_says_so_and_keeps_cores_label() {
        // STRIP-4b row 3, and ADR-0006 item 1's structural half. The claims are the three the
        // first bridge's tests carry; where they are READ moved, because the labelling rule moved
        // out of the event arm and into recents_rows() so the product's rows, the needle and the
        // behavioural tests below are one pass over ONE builder. The bit is the port's
        // (api/src/event.rs:182), the mark reaches the row, and core's pre-formatted label is
        // preserved rather than rewritten by the bridge.
        let whole = include_str!("../src/surface.rs");
        let src = &whole[..whole.find("mod tests").expect("the tests module")];
        let builder = &src[src.find("pub(crate) fn recents_rows").expect("the builder")..];
        let builder = &builder[..builder
            .find("pub(crate) fn next_generation")
            .expect("the next fn")];
        assert!(
            builder.contains("if entry.exists"),
            "the mark comes from the port's bit"
        );
        assert!(
            builder.contains("(missing)"),
            "and reaches the row a person reads"
        );
        assert!(
            builder.contains("entry.display"),
            "core's label survives unchanged"
        );
        assert!(
            !builder.contains("path.display()"),
            "a row never re-labels itself from the path - that is core's truncation rule, owned twice"
        );
        assert!(
            !src.contains("mark is core's. The bridge renders"),
            "a comment must not claim a mark the code never drew - the old wording did"
        );
        // THE ONE BUILDER CLAIM, grepped: the arm renders rows and does not label them again. A
        // second copy in the arm is how two bridges drift; it is how this file would too.
        let arm = &src[src.find("Event::RecentsUpdated(list) =>").expect("the arm")..];
        let arm = &arm[..arm.find("Event::Saved").expect("the next arm")];
        assert!(
            arm.contains("let rows = recents_rows(&list[..]);"),
            "the arm delegates to the builder these tests drive"
        );
        assert!(
            !arm.contains("if entry.exists"),
            "and holds no second copy of the labelling rule"
        );
    }

    /// A directory of REAL files, because exists is a verdict about a disk and the missing-row
    /// claim is only worth proving if the disk can change under it. No new dependency: std::fs, a
    /// unique name per case, removed on drop - the same shape tests/editor_roundtrip.rs uses.
    struct Scratch {
        root: PathBuf,
    }

    impl Scratch {
        fn new(tag: &str) -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let mut root = std::env::temp_dir();
            root.push(format!(
                "notes-bridge-slint-recents-{tag}-{}-{stamp}",
                std::process::id()
            ));
            std::fs::create_dir_all(&root)
                .unwrap_or_else(|e| panic!("cannot create {root:?}: {e}"));
            Scratch { root }
        }

        /// Lays a real file down and returns its path.
        fn touch(&self, name: &str) -> PathBuf {
            let path = self.root.join(name);
            std::fs::write(&path, b"body\n")
                .unwrap_or_else(|e| panic!("cannot write {path:?}: {e}"));
            path
        }

        /// What the port would send for this path: the basename as the display, and exists READ
        /// FROM THE DISK rather than typed into a test. Deleting the file is what flips the row.
        fn entry(&self, path: &Path) -> RecentEntry {
            RecentEntry {
                display: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string()),
                exists: path.is_file(),
                path: path.to_path_buf(),
            }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn labels(rows: &[RecentRow]) -> Vec<String> {
        rows.iter().map(|row| row.label.clone()).collect()
    }

    #[test]
    fn a_recent_row_reads_the_ports_own_label_at_its_own_slot() {
        // menu.rs:231's claim, over Slint rows: the number a person reads is the slot, and the
        // words behind it are core's string untouched. If the two ever disagree, the list offers
        // "2." and opens the third file.
        let scratch = Scratch::new("labels");
        let first = scratch.touch("alpha.notes");
        let second = scratch.touch("beta.notes");
        let entries = vec![scratch.entry(&first), scratch.entry(&second)];
        let rows = recents_rows(&entries);
        assert_eq!(labels(&rows), vec!["1. alpha.notes", "2. beta.notes"]);
        assert_eq!(
            rows.iter().map(|row| row.slot).collect::<Vec<_>>(),
            vec![1, 2],
            "1-based: slot 1 is Alt+1's row, and nothing in a label is zero-indexed"
        );
        assert_eq!(
            rows.iter().map(|row| row.path.clone()).collect::<Vec<_>>(),
            vec![first.clone(), second.clone()],
            "row N still names the file slot N opens - the index main.slint fires is THIS order"
        );
        assert!(
            rows.iter().all(|row| !row.missing),
            "two live paths, no mark: the word is not decoration"
        );
        // VERBATIM, and provably so: a display core already cut - parent suffix, ellipsis, the
        // works - comes back with only the number added in front of it.
        let cut = RecentEntry {
            path: second,
            display: String::from("..") + "/work/" + "a-very-long-note-name.notes",
            exists: true,
        };
        assert_eq!(
            recents_rows(&[cut])[0].label,
            "1. ../work/a-very-long-note-name.notes",
            "the bridge adds a slot and edits nothing else"
        );
    }

    #[test]
    fn a_path_that_vanishes_keeps_its_row_and_says_why() {
        // menu.rs:249's claim (D13 on the disk): the entry survives its own file, still counts
        // toward the cap, and tells the truth about itself. THE FLIP IS REAL - one builder, run
        // before and after a delete - so no bool in this file chose the outcome.
        let scratch = Scratch::new("vanished");
        let here = scratch.touch("here.notes");
        let gone = scratch.touch("gone.notes");
        let before = recents_rows(&[scratch.entry(&here), scratch.entry(&gone)]);
        assert_eq!(labels(&before), vec!["1. here.notes", "2. gone.notes"]);

        std::fs::remove_file(&gone).unwrap_or_else(|e| panic!("cannot delete {gone:?}: {e}"));
        assert!(!gone.is_file(), "the disk really changed");

        let after = recents_rows(&[scratch.entry(&here), scratch.entry(&gone)]);
        assert_eq!(after.len(), 2, "a vanished path is not quietly dropped");
        assert_eq!(
            labels(&after),
            vec!["1. here.notes", "2. gone.notes (missing)"],
            "the mark is APPENDED to core's label, and the slot number does not move"
        );
        assert!(!after[0].missing, "only the vanished row is marked");
        assert!(
            after[1].missing,
            "and the bit is carried beside the word, not inside it"
        );
        assert_eq!(
            after[1].path, gone,
            "the row still names its own path: greyed is not deleted, and a click on it reaches LoadFailed, which is a truth"
        );
        // A vanished entry still counts toward the cap: ten grey rows are still ten slots, and the
        // eleventh - live or not - is the one that does not get a door.
        let mut ten: Vec<RecentEntry> = (0..10)
            .map(|i| RecentEntry {
                path: scratch.root.join(format!("gone{i}.notes")),
                display: format!("gone{i}.notes"),
                exists: false,
            })
            .collect();
        let rows = recents_rows(&ten);
        assert_eq!(
            rows.len(),
            MAX_RECENTS,
            "ten, even when all ten are missing"
        );
        assert_eq!(rows[9].label, "10. gone9.notes (missing)");
        assert_eq!(
            rows.iter().filter(|row| row.missing).count(),
            MAX_RECENTS,
            "the grey count is a fact about the rows, not about the disk"
        );
        ten.push(scratch.entry(&here));
        let rows = recents_rows(&ten);
        assert_eq!(
            rows.len(),
            MAX_RECENTS,
            "and the eleventh live file does not push a row in"
        );
        assert_eq!(
            labels(&rows).last().expect("row 10"),
            "10. gone9.notes (missing)"
        );
    }

    #[test]
    fn the_rows_stop_where_the_slots_end() {
        // menu.rs:264's claim, with ELEVEN real files - the cap is only interesting if the
        // eleventh exists. Nothing past the tenth slot is built, drawn, or numbered, and every row
        // that IS drawn has a chord that reaches it.
        let scratch = Scratch::new("cap");
        let entries: Vec<RecentEntry> = (1..=11)
            .map(|i| {
                let path = scratch.touch(&format!("note{i}.notes"));
                scratch.entry(&path)
            })
            .collect();
        assert_eq!(entries.len(), 11, "the port asked for eleven rows");
        let rows = recents_rows(&entries);
        assert_eq!(rows.len(), MAX_RECENTS, "ten, and never more");
        assert_eq!(rows.first().expect("row 1").slot, 1);
        assert_eq!(rows.last().expect("row 10").slot, 10);
        assert_eq!(
            labels(&rows).last().expect("row 10"),
            "10. note10.notes",
            "the tenth row reads TEN - Alt+0 is its door, and that quirk is the table's"
        );
        assert!(
            labels(&rows).iter().all(|label| !label.starts_with("11.")),
            "no eleventh slot may be labelled, even by an off-by-one"
        );
        assert_eq!(
            rows[MAX_RECENTS - 1].path,
            entries[MAX_RECENTS - 1].path,
            "and the surviving ten are the port's FIRST ten, not some other ten"
        );
        // THE GUARD THE CAP BUYS, read from the other half of the contract: the chords in the
        // table. A row with no chord is text nobody can open; a chord with no row is a dead key.
        let chords = SHORTCUTS
            .iter()
            .filter(|(.., act)| act.starts_with("recent-"))
            .count();
        assert_eq!(rows.len(), chords, "ten rows, ten doors");
        // The dropped one is counted, not hidden: the needle says so (see the arm's cap tail).
        assert_eq!(
            entries.len() - rows.len(),
            1,
            "one path was past the last slot"
        );
    }

    #[test]
    fn an_empty_recent_list_leaves_nothing_to_divide() {
        // menu.rs:280's "no dangling separator", in the shape this toolkit has. There is no
        // separator ELEMENT in these rows, and since the recents moved into the popup the thing that
        // would dangle is no longer a gap over the EDITOR at all - it is the popup's own height, which
        // chrome.slint grows by one capped block when the list is non-empty and by NOTHING when it is
        // empty. Same claim, new owner: zero rows must cost zero height, and the builder must not
        // invent a placeholder to fill it.
        assert!(recents_rows(&[]).is_empty(), "no entries, no rows");
        assert!(
            recents_rows(&Vec::new()).is_empty(),
            "and an empty list stays empty regardless of how it was spelled"
        );
        let scratch = Scratch::new("empty");
        assert!(
            recents_rows(&[]).is_empty(),
            "a scratch dir being present changes nothing: the builder reads no disk, the CALLER decides what exists"
        );
        let live = scratch.touch("unused.notes");
        assert_eq!(
            recents_rows(&[scratch.entry(&live)]).len(),
            1,
            "one row, when there is one"
        );
        // THE CAP GUARD, read off the file that owns the cap now. This assertion used to read
        // main.slint's stack height; the stack is gone, so reading the mount would prove nothing
        // about an empty list. The cap lives in chrome.slint, which main.slint's MARKUP const does
        // not include - hence the local read below, the same pattern the drag guards use
        // (`the_drag_closes_the_popup_once_per_gesture` and
        // `the_drag_band_stops_where_the_caption_begins` both include_str! the bar).
        let popup = include_str!("../ui/chrome.slint");
        // Read FLAT, because the cap is now a two-branch expression and a guard that spans lines
        // must not care where the lines fall. Both branches still hold five: the first is the old
        // constant, kept for a bar that was never told its window's height; the second lets the
        // window ask for fewer rows, never more.
        let flat = popup.replace(['\n', '\r', ' '], "");
        assert!(
            flat.contains("root.host-height<=0px?Math.min(root.recents.length,5)")
                && flat.contains("Math.min(root.recents.length,Math.min(5,root.recents-fit))"),
            "the popup's height IS the row count, capped at the five it can hang and cut shorter by \
             the window it hangs in - so an empty list cannot leave a gap in the menu, and neither a \
             ten-entry list nor a short window can overflow the popup"
        );
        // AND PERMANENCE, the negative half: the stack must not come back as a second copy of the
        // list. `stack-h`/`row-h` are matched as DECLARATIONS, not as words - this file's own
        // comments name the old shape, and a guard that only passes when nobody explains anything
        // is a guard that punishes documentation.
        assert!(
            !MARKUP.contains("property <length> stack-h"),
            "the editor's offset must not be re-computed from the row count: one list, one home"
        );
        assert!(
            !MARKUP.contains("for name[index] in root.recents"),
            "no second rendering of recents in the mount"
        );
        // The needle names the empty case as empty instead of printing a slot range that does not
        // exist. "slots 1..=0" is the bug this line forbids.
        let whole = include_str!("../src/surface.rs");
        let src = &whole[..whole.find("mod tests").expect("the tests module")];
        let arm = &src[src.find("Event::RecentsUpdated(list) =>").expect("the arm")..];
        let arm = &arm[..arm.find("Event::Saved").expect("the next arm")];
        assert!(
            arm.contains("no slots drawn, so the menu shows its six rows"),
            "the needle says so out loud"
        );
        assert!(!arm.contains("slots 1..=0"), "and never numbers nothing");
    }

    #[test]
    fn a_drag_at_scale_one_is_the_identity_the_instrument_measured() {
        // The instrument's needles were all cut at scale 1.0 (probe.rs:1717-1720 says so), so
        // 1.0 must stay an identity or every recorded drag verdict stops meaning anything.
        assert_eq!(drag_destination((100, 200), 60.0, 40.0, 1.0), (160, 240));
        assert_eq!(drag_destination((-8, -8), -20.0, 0.0, 1.0), (-28, -8));
        // A zero delta is a click on the band, not a move, and must not move anything.
        assert_eq!(drag_destination((390, 278), 0.0, 0.0, 1.0), (390, 278));
    }

    #[test]
    fn a_drag_at_150_percent_lands_where_the_pointer_is() {
        // THE DEBT THIS RETIRES. Reading physical and writing logical made the window cover
        // delta*scale of ground while the pointer covered delta: at 1.5 a 10 px drag asked for
        // 15 px of travel. One conversion, in drag_destination, in the direction that matches.
        assert_eq!(drag_destination((100, 100), 10.0, 10.0, 1.5), (115, 115));
        assert_eq!(drag_destination((100, 100), 5.0, -5.0, 1.5), (108, 92));
        // Rounding, not truncation: 0.4 px rounds down, 0.6 rounds up, both physical px.
        assert_eq!(drag_destination((0, 0), 0.4, 0.6, 1.0), (0, 1));
    }

    #[test]
    fn an_unusable_scale_moves_the_window_rather_than_losing_it() {
        // 0 would freeze the note wherever it is and NaN would park it at an undefined point.
        // Both are recovered as 1.0 - the same fallback product.rs:321-325 applies to the
        // session's stored scale before it places the window at startup.
        assert_eq!(drag_destination((10, 10), 5.0, 5.0, 0.0), (15, 15));
        assert_eq!(drag_destination((10, 10), 5.0, 5.0, f32::NAN), (15, 15));
        assert_eq!(
            drag_destination((10, 10), 5.0, 5.0, f32::INFINITY),
            (15, 15)
        );
    }

    /// Drive a stream of deltas through one episode against a window whose read NEVER advances.
    /// That is the exact condition the repair is about: `set_position` is an ask, so a delta
    /// arriving before the windowing layer applies it reads the corner the last delta already
    /// asked to leave. The clock is a parameter, so a stall is a value here and not a wait.
    fn stalled_gesture(deltas: &[(f32, f32)]) -> (Vec<(i32, i32)>, DragEpisode) {
        let mut episode = DragEpisode::default();
        let t0 = Instant::now();
        // Frozen on purpose, and it is the number the instrument measured as a resting place.
        let frozen_read = (390, 278);
        let asked = deltas
            .iter()
            .enumerate()
            .map(|(i, (dx, dy))| {
                episode.advance(
                    frozen_read,
                    *dx,
                    *dy,
                    1.0,
                    t0 + Duration::from_millis(8 * i as u64),
                )
            })
            .collect();
        (asked, episode)
    }

    #[test]
    fn a_stalled_position_read_never_starves_a_gesture() {
        // THE STALL SIMULATION, and the inputs are no longer increments: five events carrying 6,
        // 12, 12, 12, 18 logical px of OUTSTANDING ERROR - pointer travel minus travel the frame
        // has applied - against a window that reads <390,278> every single time. The pointer
        // travelled 18 px and the note travelled nothing, so the last ask must be 390+18.
        //
        // WHY THIS IS A STRONGER CLAIM THAN THE ONE IT REPLACES. The old stream was 6+6+0+0+6 and
        // it proved that a still-pointer event does not end a gesture. This one proves something
        // a sum could not: a read that lies - that never moves at all, for the whole gesture,
        // cannot starve a drag whose delta is already the outstanding error, because the delta
        // grew to cover the lie. The asks are identical to the old test's, line for line, and the
        // mechanism that produces them is not: 396, 402, 402, 402, 408 used to come out of
        // origin + sum(increments) and now come out of read + error, with no sum anywhere.
        let (asked, episode) = stalled_gesture(&[
            (6.0, 0.0),
            (12.0, 0.0),
            (12.0, 0.0),
            (12.0, 0.0),
            (18.0, 0.0),
        ]);
        assert_eq!(
            asked,
            vec![(396, 278), (402, 278), (402, 278), (402, 278), (408, 278)],
            "zero deltas hold the note still without costing it the travel it already has"
        );
        assert_eq!(episode.travel, (18, 0), "the sum, not the last sample");
        assert_eq!(episode.origin, Some((390, 278)));
        // And the 12, measured rather than asserted away: the same episode cut off at the first
        // still-pointer event is worth 12 px, which is the number the full gesture is not.
        let (short, _) = stalled_gesture(&[(6.0, 0.0), (12.0, 0.0)]);
        assert_eq!(short.last(), Some(&(402, 278)));
        assert_ne!(
            episode.travel.0,
            short.last().unwrap().0 - 390,
            "18 px of pointer, not the 12 px of the truncated stream"
        );
    }

    #[test]
    fn an_episode_origin_survives_an_apply_that_refused() {
        // Mid-episode failures are all silent: `set_position` returns nothing, so a clamp
        // against the work area, a monitor that changed under the drag, or a windowing layer that
        // simply had not applied the last ask all look identical from here. The origin must not
        // care: it is a fact the episode sampled once, and the release line's displacement claim
        // is measured from it. The INPUTS below are cumulative outstanding errors, and the three
        // asks they produce are the same 110/120/130 this test has always expected - which is the
        // point. The third delta is the interesting one: the frame jumped +300 out from under the
        // gesture, so the band's own error shrank by exactly that much, and believing the read
        // still lands on 130. Before S1 that line passed because the read was ignored; now it
        // passes because the two halves of the decomposition add back up.
        let mut episode = DragEpisode::default();
        let t0 = Instant::now();
        assert_eq!(episode.advance((100, 100), 10.0, 0.0, 1.0, t0), (110, 100));
        assert_eq!(
            episode.origin,
            Some((100, 100)),
            "the first read is the base"
        );
        // The apply refused: the window still reads where it was, so the error is now 20.
        assert_eq!(
            episode.advance((100, 100), 20.0, 0.0, 1.0, t0 + Duration::from_millis(8)),
            (120, 100)
        );
        // The apply refused AND something moved the note out from under the drag: the read is
        // believed now, and it is believed TOGETHER WITH the error, which the band has already
        // shrunk by the same 300 px. <400,400> plus <-270,-300> is <130,100> - the pointer was
        // never wrong about where it wanted the note, and the frame is no longer ignored.
        assert_eq!(
            episode.advance(
                (400, 400),
                -270.0,
                -300.0,
                1.0,
                t0 + Duration::from_millis(16)
            ),
            (130, 100)
        );
        assert_eq!(episode.origin, Some((100, 100)), "the base never moved");
        assert_eq!(episode.travel, (30, 0));
    }

    #[test]
    fn releasing_the_band_resets_the_episode_for_the_next_press() {
        let mut episode = DragEpisode::default();
        let t0 = Instant::now();
        assert_eq!(episode.advance((500, 500), 20.0, 5.0, 1.0, t0), (520, 505));
        episode.end();
        assert_eq!(
            episode.origin, None,
            "the gesture is over, so its base is history"
        );
        assert_eq!(episode.travel, (0, 0), "and its travel is spent");
        assert_eq!(episode.last, None);
        // The next press samples again from wherever the note now stands. Without the reset it
        // would accumulate onto the last gesture: the jump an episode buys, and the reason the
        // release is not just a send.
        assert_eq!(episode.advance((520, 505), 30.0, 0.0, 1.0, t0), (550, 505));
        assert_eq!(episode.origin, Some((520, 505)));
        assert_eq!(episode.travel, (30, 0));
    }

    #[test]
    fn a_stranded_release_cannot_carry_over_and_a_repeated_error_is_paid_once() {
        // `drag-ended` is the reset and the band's `changed pressed` is the only thing that fires
        // it. A release the markup never reports - the button let go off-window, a grab stolen by
        // the OS - would otherwise leave the note accumulating across two gestures forever. The
        // gap is the net: a delta a quarter second behind the last one is a NEW press.
        let mut episode = DragEpisode::default();
        let t0 = Instant::now();
        assert_eq!(episode.advance((200, 200), 10.0, 0.0, 1.0, t0), (210, 200));
        // Still hot twenty-four wakes later, and the SAME error again - which is what the band
        // emits when the pointer has not moved but the frame has not caught up either. The flip
        // law says an assertion that changes value gets said out loud: this line used to expect
        // 220, because the episode summed a cumulative quantity a second time and paid the frame
        // for travel it had already been asked for. It now expects 210 - the same place, because
        // the same outstanding error is the same destination. Paying once is the whole cure.
        assert_eq!(
            episode.advance((200, 200), 10.0, 0.0, 1.0, t0 + Duration::from_millis(192)),
            (210, 200),
        );
        // a full second cold, and the note has since been parked somewhere else: fresh origin
        assert_eq!(
            episode.advance((900, 900), 10.0, 0.0, 1.0, t0 + Duration::from_millis(1200)),
            (910, 900)
        );
        assert_eq!(
            episode.travel,
            (10, 0),
            "no carry-over from the press that never ended"
        );
        assert_eq!(episode.origin, Some((900, 900)));
        // and the print bit dies with the episode, so the new gesture gets its one line
        assert!(episode.unprinted());
    }

    #[test]
    fn a_gesture_whose_delta_is_an_outstanding_error_cannot_be_starved() {
        // S1, THE POINTER-FAITHFUL CASE - and it is the case the accumulator could not express.
        // Scale 1, a read that is stale by exactly ONE apply (the note lands at the last ask one
        // wake late, which is what Slint's ask-not-tell `set_position` actually does), and a
        // pointer stepping 6 logical px per event. The band holds its press anchor, so the error
        // on each event is the whole pointer travel minus the travel the frame has applied - 6,
        // 12, 18, 24 - and a frame that reports the same corner four times is owed four bigger
        // asks, not four equal ones.
        let mut episode = DragEpisode::default();
        let t0 = Instant::now();
        let reads = [(390, 278), (390, 278), (390, 278), (390, 278)];
        let errors = [6.0, 12.0, 18.0, 24.0];
        let asked: Vec<(i32, i32)> = (0..4)
            .map(|i| {
                episode.advance(
                    reads[i],
                    errors[i],
                    0.0,
                    1.0,
                    t0 + Duration::from_millis(8 * i as u64),
                )
            })
            .collect();
        assert_eq!(
            asked,
            vec![(396, 278), (402, 278), (408, 278), (414, 278)],
            "STRICTLY monotone: every event asks further than the last, because each one carries
             the whole outstanding error and the read cannot hold it back"
        );
        for pair in asked.windows(2) {
            assert!(
                pair[1].0 >= pair[0].0,
                "a gesture that is only ever asked forwards: {asked:?}"
            );
        }
        for pair in asked.windows(2) {
            assert!(
                pair[1].0 > pair[0].0,
                "never a repeat, never a step back while the pointer keeps moving: {asked:?}"
            );
        }
        assert!(
            asked.iter().all(|a| a.0 > 390),
            "and not one event asks for where the gesture started - the old failure"
        );
        // THE EXACT-APPLY TWIN: same pointer, no lag, and the error collapses to one step every
        // time. Two different worlds, one expression, and the asks are the gesture the user made.
        let mut exact = DragEpisode::default();
        let twin: Vec<(i32, i32)> = (0..3)
            .map(|i| {
                exact.advance(
                    (390 + 10 * i, 278),
                    10.0,
                    0.0,
                    1.0,
                    t0 + Duration::from_millis(8 * i as u64 + 1),
                )
            })
            .collect();
        assert_eq!(
            twin,
            vec![(400, 278), (410, 278), (420, 278)],
            "no lag: the error is one 10 px step each time, so the ask is one step each time"
        );
        assert_eq!(
            exact.travel,
            (30, 0),
            "and the witness still reads as pointer travel"
        );
    }

    #[test]
    fn a_frame_that_teleported_under_the_drag_is_paid_for_once() {
        // THE TELEPORT, at the numbers the design gave. Press: frame at <1000,500>, cursor at
        // <1050,512>. The frame is then TAKEN +300,+150 by something the bridge did not ask for -
        // a snap zone, a monitor change, a restore - and the pointer steps 10 logical px LEFT.
        // The band's error is therefore <1050-10 minus 1300+... >, spelled out: pointer travel
        // -10, applied travel +300, so the outstanding error is -310 in x and -150 in y.
        let mut episode = DragEpisode::default();
        let t0 = Instant::now();
        let ask = episode.advance((1300, 650), -310.0, -150.0, 1.0, t0);
        assert_eq!(
            ask,
            (990, 500),
            "press-frame + pointer travel: 1000 + -10, 500 + 0 - the teleport is NOT collected twice"
        );
        // One more step left, and the frame has not moved again.
        let ask2 = episode.advance(
            (1300, 650),
            -320.0,
            -150.0,
            1.0,
            t0 + Duration::from_millis(8),
        );
        assert_eq!(ask2, (980, 500), "and the next 10 px is the next 10 px");
        assert_eq!(
            episode.origin,
            Some((1300, 650)),
            "the origin is the frame as first read - the teleport is already inside it"
        );
        // The witness, on the LAST ask: 980 - 1300, 500 - 650. It reads as a step BACKWARD
        // because the origin already contains the teleport the bridge never asked for, and that
        // is the truth the release line is supposed to tell - a gesture that fought a snap zone
        // moved the note less than the pointer travelled, and printing the pointer instead would
        // be the second copy of a fact that is already in the ask.
        assert_eq!(
            episode.travel,
            (-320, -150),
            "ask minus origin, both of them the frame's"
        );
    }

    #[test]
    fn a_clamped_frame_gains_debt_linearly_and_not_quadratically() {
        // THE CLAMP DEBT, which is the hazard the hold prices out loud (chrome.slint:591-594): a
        // move the OS refuses is never applied, so the outstanding error GROWS while the note is
        // pinned, and it is paid back as one jump when the constraint clears. Under the retired
        // accumulator that growth was summed a second time: 10 events against a pinned frame asked
        // for 110, 130, 160, 200 - triangle numbers, and the note would have flown off the screen
        // the instant it came free. Now the debt is linear and bounded by the pointer itself.
        let mut episode = DragEpisode::default();
        let t0 = Instant::now();
        let asked: Vec<(i32, i32)> = (0..4)
            .map(|i| {
                // The read never moves: the frame is pinned against the work area at x=100.
                episode.advance(
                    (100, 100),
                    10.0 * (i as f32 + 1.0),
                    0.0,
                    1.0,
                    t0 + Duration::from_millis(8 * i as u64),
                )
            })
            .collect();
        assert_eq!(
            asked,
            vec![(110, 100), (120, 100), (130, 100), (140, 100)],
            "one px of ask per px of pointer, not one px per px per event"
        );
        assert_eq!(
            episode.travel,
            (40, 0),
            "the debt is the pointer travel, nothing more"
        );
        assert_eq!(
            asked[3].0 - asked[0].0,
            episode.travel.0 - 10,
            "the growth is LINEAR in the events - a sum-of-sums would show 30 here and 90 there"
        );
    }

    #[test]
    fn a_dpi_move_mid_gesture_freezes_the_episode_and_says_so_once() {
        // THE SCALE LAW. `drag_destination` converts every logical px by the scale it is handed,
        // and the factor is a fact about the press: after a mid-gesture DPI change the same 10 px
        // of pointer travel means a different number of physical px, and the frame the delta is
        // measured against changed size under it. Re-sampling the origin throws away the pointer;
        // re-basing the band's anchor throws away the frame. So NEITHER is written: the episode
        // stops asking and the caller prints once, because a gesture whose arithmetic has gone
        // ambiguous is worse than a note that waits for the next press.
        let mut episode = DragEpisode::default();
        let t0 = Instant::now();
        assert_eq!(episode.advance((200, 200), 10.0, 0.0, 1.0, t0), (210, 200));
        assert!(
            !episode.frozen(),
            "the press settled the scale and nothing else can spend it"
        );
        // The DPI moves under the drag - 1.0 to 1.5, the case a monitor change actually makes.
        assert_eq!(
            episode.advance((210, 200), 10.0, 0.0, 1.5, t0 + Duration::from_millis(8)),
            (210, 200),
            "the delta that discovers the change asks for where the note already stands"
        );
        assert!(
            episode.frozen(),
            "and the episode is frozen for the rest of the gesture"
        );
        assert_eq!(
            episode.travel,
            (10, 0),
            "the freeze spent NO travel: the witness still says what the note was told to do"
        );
        // Later deltas of the same gesture stay inert, and they stay inert at the NEW factor too -
        // the freeze is not a comparison that can be satisfied by drift back.
        for i in 0..4u32 {
            assert_eq!(
                episode.advance(
                    (210, 200),
                    20.0 + i as f32,
                    0.0,
                    1.5,
                    t0 + Duration::from_millis(16 + 8 * i as u64),
                ),
                (210, 200)
            );
        }
        // The release is the re-grab: the next press samples the new scale and moves again.
        episode.end();
        assert!(
            !episode.frozen(),
            "the gesture is over, so so is the freeze"
        );
        assert_eq!(
            episode.advance((210, 200), 10.0, 0.0, 1.5, t0 + Duration::from_millis(600)),
            (225, 200),
            "and the new press converts at the factor it was pressed under: 10 logical is 15 px"
        );
    }

    #[test]
    fn a_drag_refused_while_maximised_moves_nothing_and_asks_once() {
        // THE REFUSAL PATH, still exactly one shape: a maximised note reports its position as
        // <-8,-8> (the invisible border), so the arithmetic would be perfect and the answer
        // wrong, and this bridge has already had a port store -8,52 as a restore point (see the
        // comment above the guard in `drag_by`). The episode is on this file's other side of the
        // same fact: a refusal must not OPEN an episode, because an episode whose origin is a lie
        // would then be accumulated against for the rest of the gesture.
        let episode = DragEpisode::default();
        assert_eq!(episode.origin, None, "nothing has been sampled");
        assert_eq!(episode.travel, (0, 0), "and there is no travel to carry");
        assert!(
            !episode.unprinted(),
            "an episode that never opened has nothing to print"
        );
        // The grep half, which is the only way to see an ASK without a window. Cut at the tests
        // module: this file includes ITSELF, so a literal counted below would otherwise match the
        // line doing the counting.
        let whole = include_str!("../src/surface.rs");
        let src = &whole[..whole.find("mod tests").expect("the tests module")];
        let hooks = &src[src
            .find("C3 (5) THE DRAG")
            .expect("the two drag hooks, by their heading")
            ..src
                .find("/// THE DRAG'S ARITHMETIC")
                .expect("the arithmetic, which ends the hook block")];
        assert_eq!(
            hooks.matches("Command::GeometryChanged").count(),
            1,
            "the release is the ONLY door the port is asked through - a refusal adds no second ask"
        );
        assert!(
            hooks.contains("drag_episode.end()"),
            "and the release closes the episode, not just the send"
        );
        let by = &src[src.find("fn drag_by(").expect("the drag body")
            ..src
                .find("/// C4: the corner POLICY")
                .expect("the next section")];
        let refusal = &by[..by
            .find("let here = window.position()")
            .expect("the read, which is the end of the refusal arm")];
        assert!(
            !refusal.contains("window.set_position("),
            "the maximised arm writes no position"
        );
        assert!(
            !refusal.contains("drag_episode.advance("),
            "and opens no episode"
        );
        assert!(
            by.contains(".advance("),
            "while the arm that moves does go through the episode"
        );
    }

    #[test]
    fn the_hold_is_what_makes_the_read_safe() {
        // S1-d, and the reason it is a CROSS-guard rather than a comment: from micro-1 to micro-3
        // this file held its own base because the read could not be trusted. Since S1 it trusts
        // the read every delta, and that is only correct while ui/chrome.slint holds the press
        // anchor - `last-x` written on down, NEVER re-written on move. Advance that anchor and the
        // emitted delta becomes an increment again, the arithmetic here turns quadratic, and the
        // failure shows up as a note that outruns the hand: exactly the class of bug every needle
        // in this module would still read as green. So the precondition lives in the markup, and
        // the only proof available without a window is a grep of the block that emits the delta.
        // Same shape as `the_drag_band_stops_where_the_caption_begins`: read the bar, slice the
        // block, assert what the block must NOT contain.
        let chrome = include_str!("../ui/chrome.slint");
        let at = chrome
            .find("moved => {")
            .expect("the band's move handler - the thing that emits drag-delta");
        let moved = &chrome[at..at
            + chrome[at..]
                .find("\n    }")
                .expect("the end of the move handler")];
        // COMMENTS OUT FIRST, and here the reason is not tidiness: this block carries a ten-line
        // note quoting the very line it refuses to write - "Writing 'last-x = mouse-x' here (what
        // this did before)" - so a grep over the raw text finds a re-anchor in the prose and fails
        // the commit that documented the fix. Same discipline as the licence guard in probe.rs,
        // which cuts the comment lines out of chrome.slint for exactly this reason.
        let moved_code: String = moved
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !moved_code.contains("last-x =") && !moved_code.contains("last-y ="),
            "the band must HOLD its press anchor: re-anchoring on move turns the outstanding
             error back into an increment and makes `here + delta` a sum of increments again"
        );
        assert!(
            moved_code.contains("max-band.mouse-x - max-band.last-x"),
            "and it keeps emitting the pointer-minus-anchor, which is the delta this file can add
             to a live read"
        );
        // The Rust side of the same contract, in one line: the base of the ask is the read.
        let whole = include_str!("../src/surface.rs");
        let src = &whole[..whole.find("mod tests").expect("the tests module")];
        let adv = &src[src.find("pub(crate) fn advance(").expect("the delta")..];
        let adv = &adv[..adv.find("/// The release:").expect("the next method")];
        assert!(
            adv.contains("drag_destination(here, dx, dy, scale)"),
            "the ask is computed FROM THE READ - the accumulator is gone, not merely unused"
        );
        assert!(
            !adv.contains("self.travel.0 +"),
            "and nothing sums a cumulative quantity any more"
        );
    }
    #[test]
    fn a_drag_asks_for_the_dismissal_once_per_episode_and_not_per_frame() {
        // HAMBURGER-3. THE HAZARD IS MEASURED, NOT SUPPOSED: the click-away catcher is mounted
        // BETWEEN the editor and the bar (ui/main.slint:345-359), so the band paints above it and a
        // press there never reaches it - which means a press on the band while the menu is open
        // DRAGS WITH THE MENU OPEN, and a row's TouchArea, above the catcher too, answers the
        // RELEASE inside the row: a gesture that starts on the title and ends over row 0 asks to
        // OPEN a file. The repair is to dismiss the popup on the gesture's FIRST delta, so there is
        // no row left to release onto.
        //
        // 1. THE LATCH, driven with no window. The close path rides the episode's own
        // once-per-gesture bit - the same unprinted()/mark_printed() pair that bounds the report
        // line - so "once per episode, not per delta" is a property of THIS state machine and not
        // of a second bool parked next to it. That is also why no new state was needed: the
        // gesture already knew which delta was its first.
        let mut episode = DragEpisode::default();
        let t0 = Instant::now();
        let mut asks = 0u32;
        for i in 0..5u64 {
            episode.advance((390, 278), 6.0, 0.0, 1.0, t0 + Duration::from_millis(8 * i));
            if episode.unprinted() {
                asks += 1;
            }
            episode.mark_printed();
        }
        assert_eq!(asks, 1, "five mouse-move frames, ONE dismissal");
        // The release, and then a SECOND press: a new gesture, a new ask. The dismissal is
        // per-gesture, not a once-per-run latch that goes stale and leaves row 0 armed again.
        episode.end();
        episode.advance((420, 278), 6.0, 0.0, 1.0, t0 + Duration::from_millis(100));
        assert!(
            episode.unprinted(),
            "the gesture after a release is a fresh episode, so it dismisses again"
        );
        // And the release that NEVER arrived, which is the hazard an episode created: the gap
        // restarts the episode, so the next real press still gets its dismissal instead of
        // finding the bit spent.
        let mut stranded = DragEpisode::default();
        stranded.advance((100, 100), 5.0, 0.0, 1.0, t0);
        stranded.mark_printed();
        assert!(
            !stranded.unprinted(),
            "the second delta of the same gesture asks for nothing"
        );
        stranded.advance((100, 100), 5.0, 0.0, 1.0, t0 + Duration::from_millis(400));
        assert!(
            stranded.unprinted(),
            "a press a quarter second later is a NEW press, and it asks again"
        );

        // 2. THE GREPS, which are the only way to see an ASK without a window. Four claims: the
        // bump sits in the arm that runs once per gesture and NOWHERE on the per-delta path; it is
        // the only close door this file touches; Rust writes NONE of Chrome's bits; and the door
        // still reaches Chrome, whose handler is the writer. That last pair is the whole point -
        // the ask is a number (ui/main.slint:154 -> :373) and Chrome answers it
        // (ui/chrome.slint:629-632), so menu-open keeps exactly one owner and neither a callback
        // nor a state property had to be added anywhere.
        let whole = include_str!("../src/surface.rs");
        let src = &whole[..whole.find("mod tests").expect("the tests module")];
        assert_eq!(
            src.matches("set_close_asks(").count(),
            1,
            "ONE door, bumped in exactly one place"
        );
        let by = &src[src.find("fn drag_by(").expect("the drag body")
            ..src
                .find("/// C4: the corner POLICY")
                .expect("the next section")];
        let arm = &by[by
            .find("if first {")
            .expect("the once-per-gesture arm, which is where the ask belongs")..];
        assert!(
            arm.contains("set_close_asks("),
            "the ask rides the episode's FIRST delta"
        );
        let per_delta = &by[..by.find("if first {").expect("the once-per-gesture arm")];
        assert!(
            !per_delta.contains("set_close_asks("),
            "and NOTHING on the per-delta path bumps it - that gating IS the once-per-gesture claim"
        );
        for forbidden in ["set_menu_open(", "set_about_open("] {
            assert!(
                !src.contains(forbidden),
                "Rust writes none of Chrome's state - no {forbidden}"
            );
        }
        // THE OTHER HALF OF THE DOOR, read from the markup: the number still travels to Chrome, and
        // Chrome still owns the close. If the markup lane ever renames the property or moves the
        // handler, this goes red HERE instead of in a live run that drags the note with the menu
        // open and finds the menu still there.
        assert!(
            MARKUP.contains("close-asks: root.close-asks"),
            "the mount still forwards the door to Chrome (ui/main.slint)"
        );
        let chrome = include_str!("../ui/chrome.slint");
        let handler = &chrome[chrome
            .find("changed close-asks =>")
            .expect("Chrome's answer to the door")..];
        assert!(
            handler.contains("root.menu-open = false"),
            "and Chrome, not this crate, is the one that closes"
        );
    }

    #[test]
    fn the_band_that_moves_the_window_is_listened_to_and_its_two_reads_are_the_base_and_the_witness()
     {
        // C3, and the shape of the bug it guards against is not hypothetical: the markup has
        // emitted `drag-delta` since S5 and main.slint has forwarded it since S5, and the product
        // still did not move, because nothing on the Rust side had ever registered a handler -
        // exactly how the chords behaved before STEP B (product.rs:425-432). Emission is not
        // wiring, and a grep is the only cheap proof that both halves exist.
        let whole = include_str!("../src/surface.rs");
        let src = &whole[..whole.find("mod tests").expect("the tests module")];
        assert!(
            MARKUP.contains("drag-delta(dx, dy)"),
            "the band still asks (ui/chrome.slint -> ui/main.slint)"
        );
        assert!(src.contains("ui.on_drag_delta("), "and the product listens");
        assert!(
            src.contains("ui.on_drag_ended("),
            "the release too - a drag that never tells the port stores no rect"
        );
        // ONE arithmetic, one writer of a position through the drag path. The startup restore in
        // product.rs is the other legitimate writer, and it lives in the other file.
        assert_eq!(
            src.matches("set_position(").count(),
            1,
            "drag_destination must be the only place this file moves the window"
        );
        // THE TWO READS, and what they are now. This assertion used to carry a prohibition -
        // "sample once, never base a sum on the read" - and S1 retired the prohibition while
        // keeping the count: the corner is still read exactly twice per delta, once as the BASE of
        // the ask and once as the WITNESS beside it. The reason the base is safe to use again is
        // one property of the markup, not of this file: the band holds its press anchor, so the
        // delta arriving next to the read is the outstanding ERROR, not an increment, and
        // base + error is the pointer in either direction the frame moved. What would walk the
        // moving-ruler bug back in is therefore NOT a third read - it is a third read paired with
        // an increment, which is what `the_hold_is_what_makes_the_read_safe` below watches for.
        assert_eq!(
            src.matches("= window.position()").count(),
            2,
            "the base and the witness: two reads, one per delta, and no third anywhere"
        );
        // Token-shaped needles, not whole-call ones: rustfmt is free to break a method chain
        // across lines, and a guard that only passes on one particular wrap is a test that fails
        // on a format run rather than on a regression.
        assert!(
            src.contains(".advance("),
            "the delta goes through the EPISODE, which owns the base"
        );
        assert!(
            src.contains("drag_episode.end()"),
            "and an episode is only safe because the release closes it"
        );
    }

    #[test]
    fn the_drag_band_stops_where_the_caption_begins() {
        // THE MINIMIZE-CELL BUG, grepped off the markup because it is a markup fact: the caption
        // slot is 1.5 slot-widths (chrome.slint's own caption-width), yet the drag band was cut
        // 'root.width - Theme.slot-width * 2' - a whole slot-width short of the edge, which is
        // 30px INSIDE that slot. Declared after the caption, so the band sat above it and every
        // press meant for cap-min was swallowed as a drag start: right-75 went dead, right-50
        // moved the window. The fix is the very subtraction the bar's own title row already uses,
        // so the guard is that the two expressions match - one geometry, one owner of the seam.
        let chrome = include_str!("../ui/chrome.slint");
        let at = chrome
            .find("max-band := TouchArea {")
            .expect("the drag band that moves the window");
        let band = &chrome[at..];
        let width = band
            .lines()
            .find(|l| l.trim_start().starts_with("width:"))
            .expect("the band's own width")
            .trim();
        assert!(
            width.contains("root.caption-width"),
            "the drag band's width must subtract the caption slot, or it presses out the              minimize cell it is declared on top of: {width}"
        );
        // And the subtraction is the SAME one the title row uses - not a second magic number
        // invented to make this test pass.
        let at = chrome
            .find("centre := HorizontalLayout {")
            .expect("the centred title row");
        let row = &chrome[at..];
        let row_width = row
            .lines()
            .find(|l| l.trim_start().starts_with("width:"))
            .expect("the title row's own width")
            .trim();
        let tail = |s: &str| s[s.find("- Theme.slot-width").unwrap()..].to_string();
        assert_eq!(
            tail(width),
            tail(row_width),
            "band and title row must end at the same x, one expression (band: {width} / row: {row_width})"
        );
        // The band still starts one slot in, so the hamburger and pin keep their clicks: it is
        // the RIGHT edge that was wrong, and only the right edge may change.
        assert!(
            band.contains("x: Theme.slot-width;"),
            "the band's left edge stays where the left slot ends"
        );
        // A shortened band must still be a band: it has to reach the whole title, which is the
        // centred row's own measure, and the double-click door must survive the edit.
        assert!(
            band.contains("double-clicked =>") && band.contains("root.toggle-max-requested();"),
            "maximise on a double-click is the band's reason to exist"
        );
    }

    // ---- FIX-A: the CORNERS POLICY's gates, driven with no window and no engine. ----
    // The shape is the one `register_says` takes: what a wake DECIDES comes off the pump as a
    // pure verdict, so the decision is testable and only the SENDING needs a live note. What
    // these four cannot prove is that the corners are round on the screen - that claim stays
    // with the eye-pass in .agents/notes/implemented/2026-09-14-rounded-corners-dwm.md.

    #[test]
    fn a_wake_that_sees_no_window_latches_nothing_and_says_nothing() {
        // D1, the startup law applied to corners: winit has not materialised the platform
        // window yet, the port holds no handle, and the engine's corner arm drops the command
        // with no event at all (crates/api/src/engine.rs:838-852). Fifteen normal-shape wakes
        // like that - well over one second of 8 ms ticks - must leave the pump EXACTLY as it
        // was. The bug this pins latched Some(true) and printed "round" on the FIRST of them.
        let gw = Rc::new(RefCell::new(None));
        let pump = RefCell::new(Pump::default());
        for _ in 0..15 {
            assert_eq!(
                ask_corners(&gw, &pump, true, false, false),
                None,
                "an unwired wake asked"
            );
        }
        assert_eq!(
            pump.borrow().corners_asked,
            None,
            "an unwired wake latched a shape - the square-note-for-the-session bug, back"
        );
        // The latch, the line and the send are ONE branch behind ONE verdict, so a test that
        // sees no verdict has seen no print: no path prints without latching, and none
        // latches without sending. The gate the caller obeys is this one:
        assert!(!corner_says_ask(None, false, true, false, false));
        // The wake that CAN name a handle asks, once, and remembers what it really sent.
        assert_eq!(ask_corners(&gw, &pump, true, false, true), Some(true));
        assert_eq!(pump.borrow().corners_asked, Some(true));
        // And the words are pinned, because a smoke lane reads this row and D1's complaint
        // was the LINE'S TRUTH, never its wording.
        assert_eq!(corner_words(true), "corners: round (the window is normal)");
        assert_eq!(
            corner_words(false),
            "corners: square (the window is maximised)"
        );
    }

    #[test]
    fn a_transient_refusal_leaves_the_next_shape_askable() {
        // D2, against the three refusals `platform` really produces that are NOT the OS
        // saying it has never heard of attribute 33. These strings are the shapes, not
        // paraphrases: `PlatformError::InvalidHandle`'s Display, and `Win32 { api, message }`
        // rendered by that Display for the two api names the corners seam fills in - a failed
        // read, and a read-back that contradicts an S_OK (crates/platform/src/lib.rs:88-108,
        // crates/platform/src/windows/corners.rs:60-103).
        let transient = [
            "invalid window handle",
            "Win32 DwmGetWindowAttribute failed: The parameter is incorrect. (0x80070057)",
            "Win32 DwmGetWindowAttribute failed: the window reports DWMWCP_DEFAULT, not the requested DWMWCP_ROUND (attribute 33)",
        ];
        for reason in transient {
            assert!(
                !corner_refusal_is_permanent(reason),
                "{reason} is not the SET call refusing"
            );
            let gw = Rc::new(RefCell::new(None));
            let pump = RefCell::new(Pump::default());
            assert_eq!(ask_corners(&gw, &pump, true, false, true), Some(true));
            note_corner_refusal(&pump, reason);
            assert!(
                !pump.borrow().corners_refused,
                "{reason} retired the asking for the whole run"
            );
            // THE DEFECT: the old latch killed the SQUARE ask with it, so one transient "no"
            // left a maximised note floating round over the taskbar until the process ended.
            // A shape change now gets its command, and the dedupe - not a latch - is what
            // bounds the rate.
            assert_eq!(
                ask_corners(&gw, &pump, false, false, true),
                Some(false),
                "{reason} silenced the next shape change"
            );
        }
    }

    #[test]
    fn a_set_call_refusal_is_silence_for_the_rest_of_the_run() {
        // D2's other half, and the one case the retirement exists for: Windows 10 - R11's
        // support floor - has no attribute 33, so the SET call itself refuses and no later ask
        // can go differently (crates/platform/src/windows/corners.rs:33-40 says so on the
        // seam). String-matching it is the port's own flattening, not this bridge's choice;
        // see `corner_refusal_is_permanent` for the typed event that would replace the match.
        let reason = "Win32 DwmSetWindowAttribute failed: The parameter is incorrect. (0x80070057)";
        assert!(corner_refusal_is_permanent(reason));
        let gw = Rc::new(RefCell::new(None));
        let pump = RefCell::new(Pump::default());
        assert_eq!(ask_corners(&gw, &pump, true, false, true), Some(true));
        note_corner_refusal(&pump, reason);
        assert!(
            pump.borrow().corners_refused,
            "a permanent no left asking alive"
        );
        // Silence for the rest of the run, in BOTH shapes and across many wakes: one print was
        // the whole reason the event exists, one per maximise is the noise it was cut for, and
        // one per wake is the storm P1b retired everywhere else in this loop.
        for shape in [false, true, false] {
            for _ in 0..8 {
                assert_eq!(
                    ask_corners(&gw, &pump, shape, false, true),
                    None,
                    "a retired asking spoke again"
                );
            }
        }
        // A permanent no is silent on the parked and unwired wakes too.
        assert!(!corner_says_ask(Some(true), true, false, false, true));
        assert!(!corner_says_ask(Some(true), true, false, true, true));
    }

    #[test]
    fn the_corner_dedupe_still_collapses_repeats() {
        // (d) The gate that predates FIX-A, still the only thing bounding the rate: 100 wakes
        // of a held shape cost ONE ask, not a hundred attribute calls ~125 times a second.
        let gw = Rc::new(RefCell::new(None));
        let pump = RefCell::new(Pump::default());
        let wakes: Vec<Option<bool>> = (0..100)
            .map(|_| ask_corners(&gw, &pump, true, false, true))
            .collect();
        assert_eq!(
            wakes.iter().filter(|asked| asked.is_some()).count(),
            1,
            "a held normal window must cost exactly one ask"
        );
        // A held maximisation: one more, and it is the square one.
        let held: Vec<Option<bool>> = (0..100)
            .map(|_| ask_corners(&gw, &pump, false, false, true))
            .collect();
        assert_eq!(
            held.iter().filter(|asked| asked.is_some()).count(),
            1,
            "a held maximise must cost exactly one ask"
        );
        // Through the park and back: a parked wake says nothing (the maximised bit there is a
        // fact about the OS's parking lot), the wake after the restore asks once, and its own
        // repeat asks not at all.
        assert_eq!(
            ask_corners(&gw, &pump, true, true, true),
            None,
            "a parked wake asked"
        );
        assert_eq!(ask_corners(&gw, &pump, true, false, true), Some(true));
        assert_eq!(ask_corners(&gw, &pump, true, false, true), None);
        // And the pure gate agrees with every call above: the caller keeps no second copy of
        // the rule that decides it.
        assert!(!corner_says_ask(Some(true), false, true, false, true));
        assert!(corner_says_ask(Some(true), false, false, false, true));
    }

    // ================= THE PIN'S ROUND TRIP (ADR-0006 item 6, last sub-claim) ==========
    //
    // The claim owed is "the pin check mark's round trip": the strip asks, the port applies,
    // the answer flips the model, the model is what the glyph is painted FROM, and the bit
    // survives a restart. Every link a machine WITHOUT INPUT can prove is proven in these two
    // tests plus crates/api/tests/pin_roundtrip.rs. The chain, by leg:
    //   ask   -> Command::SetPinned(next)   [pin_roundtrip.rs leg 1 - a real Gateway, real cmd]
    //   apply -> Event::Pinned(next)        [pin_roundtrip.rs legs 0-3 - the recording host]
    //   model -> ui.set_pinned(answer)      [TEST 1 BELOW: the real Spike, the real `drain`]
    //   bind  -> the caption cell reads it  [TEST 2 BELOW: census over both markup files]
    //   paint -> the glyph on a real bar    [NOT PROVEN - the one half left, named just below]
    //
    // WHICH HALF OF THE ROUND TRIP REMAINS WINDOW-BOUND, exactly: the LAST ARROW. Everything
    // up to and including the model flip is proven here on the product's own objects; that a
    // live window turns those 24 px of icons/unpin.svg into icons/pin.svg in amber, inside a
    // caption cell that is actually where the strip draws it and actually hit-testable (and no
    // input means no press can even reach the cell) is a pixel claim, not a bit claim. A human
    // eye owes three things and nothing else: (a) the cell is where the caption draws it and
    // the click lands, (b) the glyph swaps AND colourises amber, (c) the bar's ground follows
    // to Theme.bar-bg-pinned. All three are asserted below as BINDINGS on the very bit the
    // model carries - so the eye confirms a painting, never a wiring.
    //
    // WHY `Spike::new()` IS LEGAL HERE, which is what makes test 1 more than a twin:
    // it is the same constructor `product.rs:313` calls, and Slint builds the component and its
    // property storage without ever materialising a platform window until something asks to
    // show it. Nothing below shows, renders or pumps a loop: no window, no taskbar entry, no
    // flake - and no re-implementation either. `set_pinned` / `get_pinned` are the GENERATED
    // accessors the answer arm itself calls, and `drain` is the function the wake calls, so the
    // code under test is the shipped code. Unlike editor_roundtrip.rs (whose header admits the
    // Spike-copy weakness because a test target cannot reach bin internals), NOTHING here
    // re-implements a production function, and no production code was edited to be testable.

    #[test]
    fn the_answer_leg_of_the_pins_round_trip_flips_the_real_model_the_check_mark_reads() {
        use super::{Spike, drain};
        use notes_api::Event;
        use slint::ComponentHandle;

        let ui = Spike::new().expect("the product's own constructor, windowless");
        let pump = RefCell::new(Pump::default());
        let (tx, rx) = std::sync::mpsc::channel::<Event>();
        let weak = ui.as_weak();

        // START: the painted bit is false, and the pump has NOT guessed anything yet.
        assert!(!ui.get_pinned(), "a fresh strip is unpinned");
        assert_eq!(pump.borrow().confirmed, None, "and the fact is not guessed");

        // LEG ON. `drain` is the product function the wake runs; the arm inside it is the
        // only thing in this crate allowed to write the rendered bit (test 2 fails the day a
        // second writer appears).
        tx.send(Event::Pinned(true)).expect("into the channel");
        drain(&rx, &pump, &weak);
        assert!(ui.get_pinned(), "the model flipped");
        assert_eq!(
            pump.borrow().confirmed,
            Some(true),
            "and the pump latched it"
        );
        assert!(
            pump.borrow().applied_at.is_some(),
            "a confirmed pin stamps t0"
        );
        assert!(
            ui.get_status().as_str().contains("port-reported"),
            "the status line names where the fact came from: {}",
            ui.get_status()
        );

        // LEG OFF: a refusal is an answer too, and it un-flips the SAME bit - never silently.
        tx.send(Event::PinFailed {
            reason: "the pin did not stick".into(),
        })
        .expect("into the channel");
        drain(&rx, &pump, &weak);
        assert!(!ui.get_pinned(), "a refused apply paints OFF");
        assert_eq!(pump.borrow().confirmed, Some(false));
        assert!(
            ui.get_status().as_str().contains("pin refused"),
            "and it says why: {}",
            ui.get_status()
        );
        // The field the ask reads, right now, means the NEXT click asks for true. That is
        // the arithmetic `on_toggled_pin` holds, and test 2 censitises it word for word.
        assert!(
            !pump.borrow().confirmed.unwrap_or(false),
            "so the strip's next ask is the opposite of the last fact"
        );

        // AND THE BIT FOLLOWS EVERY ANSWER, both ways, on the same object.
        tx.send(Event::Pinned(true)).expect("into the channel");
        drain(&rx, &pump, &weak);
        assert!(ui.get_pinned(), "the model follows the port, not the wish");
        assert!(
            pump.borrow().confirmed.unwrap_or(false),
            "and the next ask is therefore false - the toggle is the port's answer, twice over"
        );

        // Three answers in, one windowless model, ZERO platform calls: the leg that needed a
        // window is exactly the leg that stayed out, and the leg that needed input is the leg a
        // person still owns.
        assert_eq!(pump.borrow().seen, 3, "three answers were taken");
        assert_eq!(pump.borrow().drains, 3, "each wake drained once");
    }

    #[test]
    fn the_caption_pin_cell_binds_the_very_bit_the_answer_writes() {
        // THE BIND CENSUS, which is what makes the pixel half a PAINTING claim and not a
        // wiring claim: the property the answer writes is the only thing the cell's glyph,
        // colour and bar background read, the click writes nothing, and the names are counted
        // so a second writer or a re-bound cell fails HERE. Cut at `mod tests` for the law this
        // file already states: a self-referential grep excludes its own counting lines.
        let whole = include_str!("../src/surface.rs");
        let src = &whole[..whole.find("mod tests").expect("the tests module")];
        let chrome = include_str!("../ui/chrome.slint");

        // (1) ONE WRITER, and it is the answer rather than the ask.
        assert_eq!(
            src.matches("ui.set_pinned(").count(),
            2,
            "Pinned and PinFailed only - a third writer is the click-believes-itself bug"
        );
        assert_eq!(
            src.matches("confirmed = ").count(),
            2,
            "and those same two arms are the only writers of the fact the ask reads"
        );
        let on = &src[src.find("Event::Pinned(on) =>").expect("the Pinned arm")
            ..src
                .find("Event::PinFailed { reason } =>")
                .expect("the PinFailed arm")];
        assert!(on.contains("let applied = *on;"), "the bit is the port's");
        assert!(on.contains("pump.confirmed = Some(*on);"), "latched...");
        assert!(
            on.contains("ui.set_pinned(applied);"),
            "...and painted from the latch"
        );
        assert!(
            !on.contains("Command::SetPinned"),
            "an answer sends nothing back"
        );
        let off = &src[src
            .find("Event::PinFailed { reason } =>")
            .expect("the PinFailed arm")
            ..src
                .find("Event::CornerRoundingFailed")
                .expect("the next arm")];
        assert!(
            off.contains("ui.set_pinned(false);"),
            "a refusal paints off"
        );
        assert!(
            off.contains("pump.confirmed = Some(false);"),
            "and latches the refusal"
        );

        // (2) THE ASK: one closure, the opposite of the last fact, one command out, and not
        // one local write - which is what leaves (1) as the only painter in the crate.
        let ask = &src[src
            .find("ui.on_toggled_pin(move || {")
            .expect("the pin handler")
            ..src.find("// C2 (2)").expect("the next handler")];
        assert!(
            ask.contains("let next = !pump.borrow().confirmed.unwrap_or(false);"),
            "the ask is the negation of the port's last fact, read from the pump"
        );
        assert!(
            ask.contains("send(&gw, Command::SetPinned(next));"),
            "one command out, and it asks for `next`, never for a wish"
        );
        assert!(!ask.contains("set_pinned"), "the click paints NOTHING");

        // (3) THE CELL: glyph, colour and bar ground all read `root.pinned` - the caption's
        // in-property - and the click asks instead of flipping.
        assert!(
            chrome.contains("in property <bool> pinned: false;"),
            "in-only, so the UI cannot write it outward"
        );
        assert!(
            chrome.contains("source: root.pinned ? @image-url(\"icons/pin.svg\") : @image-url(\"icons/unpin.svg\");"),
            "THE CHECK MARK itself: one image, then the other, on that bit"
        );
        assert!(
            chrome.contains("colorize: root.pinned ? Theme.amber : Theme.icon-muted;"),
            "and its colour rides the same bit"
        );
        assert!(
            chrome.contains("background: root.pinned ? Theme.bar-bg-pinned : Theme.bar-bg;"),
            "and the bar's ground"
        );
        assert!(
            chrome.contains("root.toggled-pin(!root.pinned);"),
            "the cell asks"
        );
        assert_eq!(
            chrome.matches("root.pinned").count(),
            4,
            "four reads, no fifth"
        );
        assert_eq!(
            chrome.matches("root.pinned =").count(),
            0,
            "and zero writes in markup"
        );

        // (4) THE WIRE from the root property test 1 flips to the cell that reads it: exactly
        // two mentions at the root - the declaration and the one-way mount, no expression.
        assert!(
            MARKUP.contains("in property <bool> pinned: false;"),
            "the root's bit"
        );
        assert!(
            MARKUP.contains("pinned: root.pinned;"),
            "mounted into the caption, unchanged"
        );
        assert_eq!(
            MARKUP.matches("pinned:").count(),
            2,
            "declaration + binding only"
        );
        assert!(
            MARKUP.contains("callback toggled-pin();"),
            "the ask leaves the root"
        );
        assert!(
            MARKUP.contains("toggled-pin(asked) => {"),
            "and the caption's ask is forwarded"
        );
        assert!(
            !MARKUP.contains("set_pinned"),
            "the root's markup never writes the bit"
        );

        // (5) BOTH GLYPHS SHIP: a binding to a missing image is a paint bug no assertion here
        // can see, so the two files the cell swaps between are checked on disk.
        let icons = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/icons");
        for icon in ["pin.svg", "unpin.svg"] {
            assert!(
                icons.join(icon).is_file(),
                "{icon} must ship for the cell to swap it",
            );
        }
    }
}
