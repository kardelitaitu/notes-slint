//! Events out: the only thing the engine is allowed to say to a UI.
//!
//! Events are delivered on the UI thread, and they include the things that have
//! no caller. That asymmetry is the whole reason the port is command/event and
//! not call/return: an autosave fires seconds after any UI call, on a worker
//! thread, and it can fail — and there is nobody waiting to receive an `Err`
//! (docs/architecture.md §5.4). So failure crosses the port as data,
//! [`Event::SaveFailed`](crate::Event::SaveFailed), already worded for the UI.
//!
//! This module is **data only**: no serializer, no watcher, no debounce. Those
//! are engine and core work.

use std::path::PathBuf;

/// How a file's bytes are encoded, and therefore how they must be written back.
///
/// Part of the do-no-harm rule: load then save a foreign file byte-identically,
/// preserving encoding, BOM, line endings and trailing newline
/// (whitepaper §4.5, AGENTS.md). Detect once on load, write back the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// UTF-8, no byte-order mark.
    Utf8,
    /// UTF-8 with a BOM. The BOM is part of the encoding's identity, not a
    /// detail to strip: dropping it is a visible change to the file.
    Utf8Bom,
    /// UTF-16 little-endian (BOM-present Windows text files).
    Utf16Le,
    /// UTF-16 big-endian.
    Utf16Be,
    /// A legacy single-byte codepage, named by its Windows codepage id —
    /// `Ansi(1252)` is CP1252 (D14).
    ///
    /// ANSI is not one encoding but a family, and there is no encoding that is
    /// its inverse: a byte decoded as CP1252 must be re-encoded as CP1252, or
    /// the file silently corrupts. So the id is carried on the variant and the
    /// write path uses it, rather than the app normalising to UTF-8 behind the
    /// user's back.
    Ansi(u16),
}

/// What a newline looks like in this file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    /// LF — a bare line feed, byte 0x0A.
    Lf,
    /// CRLF — a CR followed by an LF (bytes 0x0D 0x0A), the Windows default.
    CrLf,
}

/// The facts about a loaded file that the UI is allowed to see.
///
/// Six booleans-and-enums of pure data, deliberately `Copy`: it travels inside
/// [`Event::Loaded`](crate::Event::Loaded) and the bridge keeps a copy for the
/// status line. Every field is here because a specific piece of UI has to render
/// it honestly — nothing is speculative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileMeta {
    /// Detected on load, written back unchanged (§4.5).
    pub encoding: Encoding,
    /// Detected on load, written back unchanged (§4.5).
    pub line_ending: LineEnding,
    /// Whether the file ended with a newline. If it did not, the save must not
    /// add one; if it did, the save must not lose it (§4.5).
    pub trailing_newline: bool,
    /// The file is read-only on disk. Drives the status line, and the save path
    /// fails with `SaveError::ReadOnly` rather than clearing the flag.
    pub read_only: bool,
    /// The 8 MiB size guard verdict (D9): the file was opened READ-ONLY because
    /// it is too big to edit safely. Nothing is ever refused — a refused open
    /// is a lost document.
    pub oversize: bool,
    /// ADR-0001, PER DOCUMENT: has this file been saved once explicitly, so
    /// autosave is armed for it? `false` on a freshly opened foreign file,
    /// `true` on a `.notes` file and on anything opened by Save As.
    ///
    /// This is autosave arming, NOT pin state. Pin state has exactly one home —
    /// `session.json` (D10) — and must never be folded in here.
    pub armed: bool,
}

/// Why an autosave attempt did nothing.
///
/// ADR-0001: silence is forbidden. Each variant must be renderable as a short
/// status line — the whole point of the type is that "nothing happened" is
/// explained rather than inferred.
///
/// This mirrors `notes_core::document::Skip` variant for variant, and
/// deliberately so: the reason is core's verdict (rule 1), the name is what the
/// UI may see. `engine.rs` translates one into the other and the mapping is
/// total — adding a variant to either enum without the other breaks the match,
/// not the user's understanding of why nothing saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The global toggle is off ([`Command::SetAutosave(false)`](crate::Command::SetAutosave)).
    AutosaveDisabled,
    /// A file this app did not create, not yet saved once explicitly
    /// (ADR-0001). The UI copy is the ADR's own explanation: press Ctrl+S once
    /// and it keeps saving.
    ForeignFileNotArmed,
    /// The buffer matches what is on disk (D11: the `Flush` revision is at or
    /// below the last saved revision), so there is nothing to write.
    Clean,
    /// Read-only on disk.
    ReadOnly,
    /// Over the 8 MiB guard, so opened read-only (D9).
    Oversize,
}

/// One row of the recent-files menu.
///
/// The recents rule (D13): cap of 10. Identity is the canonicalised path, with the
/// `\\?\` verbatim prefix stripped and the comparison folded to lower case, so
/// a file does not appear twice
/// because Explorer and Notepad spelled it differently. Display keeps the
/// original case — Windows paths are case-insensitive but case-preserving. A
/// vanished path stays in the list with `exists: false` and still counts
/// toward the cap; a greyed-out entry that tells the truth beats one that
/// quietly disappeared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentEntry {
    pub path: PathBuf,
    /// Pre-formatted for the menu by the engine, so no bridge has to invent its
    /// own truncation or basename rule.
    pub display: String,
    /// False when the path no longer exists. The entry survives.
    pub exists: bool,
}

/// Why a save failed.
///
/// Part of the public contract, because the UI must render the reason
/// (AGENTS.md "the `SaveError` enum is part of the public contract"). **The
/// Display text below IS the user-visible copy**: `to_string()` goes straight
/// into a dialog or the status line. Renaming a variant is a refactor; editing
/// its string is a product change, and the tests in this module exist to make
/// that deliberate.
///
/// Not every variant is reachable from every path: on the async autosave path
/// there is no caller to return this to, so it is wrapped in
/// [`Event::SaveFailed`](crate::Event::SaveFailed).
///
/// `PartialEq`/`Eq` here are not decoration: [`Event`] is specified as
/// `#[derive(PartialEq)]` and `SaveFailed` carries a `SaveError`, so the reason
/// has to be comparable for the event to be comparable — and for a headless
/// "play a session" test to assert on a failure without matching strings.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SaveError {
    /// The file carries a read-only attribute.
    #[error("file is read-only")]
    ReadOnly,
    /// ACL or sharing refused the write.
    #[error("permission denied")]
    PermissionDenied,
    /// No room on the target volume.
    #[error("no space left on device")]
    DiskFull,
    /// Another process holds the file — OneDrive sync, an editor, an indexer.
    #[error("file is locked by another program")]
    Locked,
    /// The path existed when we planned the write and does not now.
    #[error("file no longer exists")]
    NotFound,
    /// The text contains a character the file's own encoding cannot hold — the
    /// escape hatch of the §4.5 do-no-harm rule, and the only case where
    /// refusing is correct. Names the encoding the file was detected as (D14
    /// for ANSI codepages).
    #[error("encoding {0:?} cannot represent this text")]
    Unencodable(Encoding),
    /// Anything else. Carries the OS text, because inventing a friendly string
    /// for an unknown failure hides the one clue the user has.
    #[error("{0}")]
    Other(String),
}

/// Everything the engine can tell the UI.
///
/// Delivered on the UI thread. `RecentsUpdated` carries the whole list rather
/// than a delta so the menu never has to reconstruct state it may have missed
/// while the window did not exist.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A file finished loading: the buffer text plus the facts needed to
    /// preserve its format.
    Loaded {
        path: PathBuf,
        text: String,
        meta: FileMeta,
    },
    /// A save succeeded, and the buffer at `revision` is now what is on disk
    /// (D11 — this is what makes later flushes at that revision `Clean`).
    Saved { path: PathBuf, revision: u64 },
    /// A save failed. The async one: `Flush` has no reply channel, so this is
    /// how the failure reaches the status line (AGENTS.md).
    SaveFailed {
        path: PathBuf,
        revision: u64,
        reason: SaveError,
    },
    /// Something outside the app changed the file on disk. The bridge decides
    /// what to do about it; the port only says it happened.
    ExternalChange { path: PathBuf },
    /// An autosave attempt did nothing — and why. Must be rendered, not merely
    /// defined (ADR-0001).
    AutosaveSkipped { reason: SkipReason },
    /// The recent-files list, in most-recent-first order, capped at 10 (D13).
    RecentsUpdated(Vec<RecentEntry>),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plausible foreign file: CP1252 (D14), CRLF, BOM-less, writable, not
    /// oversized, and — the ADR-0001 point — NOT armed yet.
    fn meta() -> FileMeta {
        FileMeta {
            encoding: Encoding::Ansi(1252),
            line_ending: LineEnding::CrLf,
            trailing_newline: true,
            read_only: false,
            oversize: false,
            armed: false,
        }
    }

    /// Rebuilds every event by destructuring it. Compiler-checked for
    /// exhaustiveness, so a new variant cannot bypass the round-trip test.
    fn rebuild(event: &Event) -> Event {
        match event {
            Event::Loaded { path, text, meta } => Event::Loaded {
                path: path.clone(),
                text: text.clone(),
                meta: *meta,
            },
            Event::Saved { path, revision } => Event::Saved {
                path: path.clone(),
                revision: *revision,
            },
            Event::SaveFailed {
                path,
                revision,
                reason,
            } => Event::SaveFailed {
                path: path.clone(),
                revision: *revision,
                reason: reason.clone(),
            },
            Event::ExternalChange { path } => Event::ExternalChange { path: path.clone() },
            Event::AutosaveSkipped { reason } => Event::AutosaveSkipped { reason: *reason },
            Event::RecentsUpdated(entries) => Event::RecentsUpdated(entries.clone()),
        }
    }

    fn variant(event: &Event) -> &'static str {
        match event {
            Event::Loaded { .. } => "Loaded",
            Event::Saved { .. } => "Saved",
            Event::SaveFailed { .. } => "SaveFailed",
            Event::ExternalChange { .. } => "ExternalChange",
            Event::AutosaveSkipped { .. } => "AutosaveSkipped",
            Event::RecentsUpdated(_) => "RecentsUpdated",
        }
    }

    fn all_events() -> Vec<Event> {
        vec![
            Event::Loaded {
                path: PathBuf::from("C:/notes/a.notes"),
                text: "hi".to_string(),
                meta: meta(),
            },
            Event::Saved {
                path: PathBuf::from("C:/notes/a.notes"),
                revision: 3,
            },
            Event::SaveFailed {
                path: PathBuf::from("C:/notes/a.notes"),
                revision: 4,
                reason: SaveError::DiskFull,
            },
            Event::ExternalChange {
                path: PathBuf::from("C:/notes/a.notes"),
            },
            Event::AutosaveSkipped {
                reason: SkipReason::ForeignFileNotArmed,
            },
            Event::RecentsUpdated(vec![RecentEntry {
                path: PathBuf::from("C:/notes/a.notes"),
                display: "a.notes".to_string(),
                exists: true,
            }]),
        ]
    }

    /// Acceptance (a), second half: every `Event` variant constructs, clones,
    /// compares equal to itself, and survives a match-and-rebuild round trip.
    #[test]
    fn every_event_variant_round_trips() {
        let all = all_events();
        let names: Vec<&'static str> = all.iter().map(variant).collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            all.len(),
            "the fixture must cover each variant exactly once: {names:?}"
        );
        // Loaded, Saved, SaveFailed, ExternalChange, AutosaveSkipped,
        // RecentsUpdated.
        assert_eq!(all.len(), 6, "Event gained or lost a variant");

        for event in &all {
            assert_eq!(event, &event.clone(), "{event:?} clone is not equal");
            assert_eq!(&rebuild(event), event, "round trip changed {event:?}");
        }
    }

    /// `PartialEq` on events must reach inside them: two `Saved` events that
    /// differ only by revision are different news.
    #[test]
    fn events_compare_their_payloads() {
        assert_ne!(
            Event::Saved {
                path: PathBuf::from("a"),
                revision: 1
            },
            Event::Saved {
                path: PathBuf::from("a"),
                revision: 2
            }
        );
        assert_ne!(
            Event::AutosaveSkipped {
                reason: SkipReason::Clean
            },
            Event::AutosaveSkipped {
                reason: SkipReason::Oversize
            }
        );
        let mut armed = meta();
        armed.armed = true;
        assert_ne!(
            Event::Loaded {
                path: PathBuf::from("a"),
                text: String::new(),
                meta: meta()
            },
            Event::Loaded {
                path: PathBuf::from("a"),
                text: String::new(),
                meta: armed
            }
        );
        assert_ne!(
            Event::RecentsUpdated(vec![]),
            Event::RecentsUpdated(vec![RecentEntry {
                path: PathBuf::from("a"),
                display: "a".into(),
                exists: false,
            }])
        );
        assert_eq!(
            Event::SaveFailed {
                path: PathBuf::from("a"),
                revision: 1,
                reason: SaveError::Locked
            },
            Event::SaveFailed {
                path: PathBuf::from("a"),
                revision: 1,
                reason: SaveError::Locked
            }
        );
    }

    /// Every `SkipReason` is renderable, because ADR-0001 forbids silence. The
    /// UI copy is core's or the bridge's to word; the port only has to keep
    /// every cause distinguishable, which the exhaustive `Debug` below gives
    /// the status line as a fallback.
    #[test]
    fn every_skip_reason_is_distinguishable() {
        let all = [
            SkipReason::AutosaveDisabled,
            SkipReason::ForeignFileNotArmed,
            SkipReason::Clean,
            SkipReason::ReadOnly,
            SkipReason::Oversize,
        ];
        let rendered: Vec<String> = all.iter().map(|r| format!("{r:?}")).collect();
        let mut unique = rendered.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 5, "skip reasons collapsed: {rendered:?}");
        for (reason, text) in all.iter().zip(&rendered) {
            assert!(!text.is_empty(), "{reason:?} renders empty");
        }
    }

    /// Acceptance (d): `FileMeta` is `Copy`, it stays inside two machine words,
    /// and its six fields are six independent facts.
    ///
    /// The budget is asserted at COMPILE time because that is the only place a
    /// layout promise means anything; the exact sizes are not asserted, because
    /// layout is the compiler's business. What these guard is the failure mode:
    /// a `String`, `PathBuf` or `Vec` smuggled into a `Copy` DTO that rides
    /// along with every [`Event::Loaded`](crate::Event::Loaded). Measured on
    /// rustc 1.98 / x86_64-pc-windows-msvc: FileMeta 10 bytes = Encoding 4 (a
    /// tag plus the `u16` codepage id of D14) + LineEnding 1 + four bools,
    /// aligned to 2.
    const _: () = assert!(std::mem::size_of::<FileMeta>() <= 16);
    const _: () = assert!(std::mem::size_of::<Encoding>() <= 4);
    const _: () = assert!(std::mem::size_of::<LineEnding>() <= 1);
    const _: () = assert!(std::mem::size_of::<SkipReason>() <= 1);

    /// `Copy` is part of the contract: the bridge keeps one for the status line
    /// without cloning or borrowing it.
    const _: fn() = || {
        fn assert_copy_eq<T: Copy + Eq>() {}
        assert_copy_eq::<FileMeta>();
    };

    #[test]
    fn file_meta_is_copy_and_its_fields_are_independent() {
        let base = meta();
        let copied = base; // Copy: a move, no clone call, no borrow.
        assert_eq!(base, copied);
        assert_eq!(
            copied.encoding,
            Encoding::Ansi(1252),
            "D14: the codepage id round-trips"
        );

        // Each field is its own answer. Flipping one must change the value and
        // nothing else — read_only (the disk says so) and oversize (the 8 MiB
        // guard says so, D9) are separate questions, and armed (ADR-0001,
        // per document) is a third. Pin state is not in here at all: D10 gives
        // it one home, session.json.
        let mut armed = base;
        armed.armed = true;
        let mut oversize = base;
        oversize.oversize = true;
        let mut read_only = base;
        read_only.read_only = true;

        assert!(
            !base.armed,
            "a fresh foreign file starts DISARMED (ADR-0001)"
        );
        assert_ne!(armed, base);
        assert_ne!(oversize, base);
        assert_ne!(read_only, base);
        assert_ne!(
            oversize, read_only,
            "D9: oversized and read-only are distinct"
        );
        assert_ne!(oversize, armed);
    }

    /// Acceptance (b): the shipped UI copy. These strings are user-visible;
    /// changing one is a product decision, so it is pinned here verbatim.
    #[test]
    fn save_error_ui_copy_is_pinned() {
        assert_eq!(SaveError::ReadOnly.to_string(), "file is read-only");
        assert_eq!(SaveError::PermissionDenied.to_string(), "permission denied");
        assert_eq!(SaveError::DiskFull.to_string(), "no space left on device");
        assert_eq!(
            SaveError::Locked.to_string(),
            "file is locked by another program"
        );
        assert_eq!(SaveError::NotFound.to_string(), "file no longer exists");
        assert_eq!(
            SaveError::Unencodable(Encoding::Utf16Be).to_string(),
            "encoding Utf16Be cannot represent this text"
        );
        assert_eq!(
            SaveError::Unencodable(Encoding::Ansi(1252)).to_string(),
            "encoding Ansi(1252) cannot represent this text"
        );
        assert_eq!(
            SaveError::Other("os error 5".to_string()).to_string(),
            "os error 5"
        );
    }

    /// `SaveError` is a real error type on the way to the UI, not just a
    /// display bag: it must survive `&dyn Error`.
    #[test]
    fn save_error_is_an_error() {
        let reason = SaveError::ReadOnly;
        let as_error: &dyn std::error::Error = &reason;
        assert_eq!(as_error.to_string(), "file is read-only");
        assert_eq!(reason, SaveError::ReadOnly, "SaveError compares by value");
    }

    /// A `RecentEntry` keeps its display casing while its identity (computed
    /// in core) is case-folded — D13. The port's job is only to hold both
    /// fields apart so the two can never be confused for one another.
    #[test]
    fn recent_entry_keeps_path_and_display_apart() {
        let entry = RecentEntry {
            path: PathBuf::from("C:/Notes/A.NOTES"),
            display: "A.NOTES".to_string(),
            exists: false,
        };
        assert_eq!(entry.display, "A.NOTES", "display keeps original case");
        assert!(!entry.exists, "a vanished path stays, marked missing");
        assert_ne!(
            entry,
            RecentEntry {
                display: "a.notes".to_string(),
                ..entry.clone()
            }
        );
    }
}
