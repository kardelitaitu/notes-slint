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
use std::time::Instant;

use notes_api::{Command, Event, Gateway};

use crate::Spike;
use crate::plumbing::{
    describe, dialog_allowed, do_no_harm, lf, note_dot, publish_title, report, send,
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

/// S10: the generation, one step. Plus one, not a toggle: the port epochs move one at a time and
/// a toggle would collide after two adoptions. The PARITY is what a conditional recreation would
/// key on, so this makes the parity a fact a test can hold rather than an expression written
/// twice in two languages.
pub(crate) fn next_generation(previous: i32) -> i32 {
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
    /// final observation).
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

#[cfg(test)]
mod tests {

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
        // The refusal half of the same invariant lives in the OTHER file, because this move took the
        // adoptions and left text_pump with the acts it is called from. A guard that spans the
        // boundary has to read across it - and each slice cuts at its own file's test module,
        // `mod tests` here and `mod chords` in the probe.
        let whole_probe = include_str!("../src/probe.rs");
        let probe = &whole_probe[..whole_probe
            .find("mod chords")
            .expect("the probe's test module")];
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
        let guard = &probe[probe.find("S8b: THE REFUSAL").expect("the guard")..];
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
}
