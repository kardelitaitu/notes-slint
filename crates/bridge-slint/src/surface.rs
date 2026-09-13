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
