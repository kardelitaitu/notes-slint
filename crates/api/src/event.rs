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

use notes_core::Rect;
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
    /// The note has no path yet, so the only way to persist it is an explicit
    /// Save As. A SKIP and not a failure: a brand-new note is not an error, and an
    /// error toast about a file that does not exist teaches the user nothing
    /// (M9 - this replaced an [`Event::SaveFailed`] whose text the engine
    /// invented, with an empty path beside it). No copy here on purpose: like every
    /// other reason in this enum, the words belong to the UI.
    NeedsPath,
    /// Read-only on disk.
    ReadOnly,
    /// Over the 8 MiB guard, so writes are refused (D9). Reached only for a
    /// document that is open and became oversize: an [`Open`](crate::Command::Open)
    /// that trips the guard never gets this far, it answers
    /// [`LoadError::TooLarge`] instead, because the port has no buffer to show.
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
    /// Nothing was open to write. The user's action is not "fix the file" but
    /// "choose a location", so this belongs in the enum with its own copy rather
    /// than in a string the engine made up at the call site - AGENTS.md puts the
    /// reason in the type because the UI has to render it, and rule 2 says every
    /// value here is a fact somebody else decided.
    #[error("this note has not been saved to a file yet")]
    NoTarget,
    /// The target is a symlink, junction or other reparse point, so replacing it
    /// would orphan the real file behind it while the app reported success
    /// (OneDrive and other sync clients materialise files this way). The [`String`]
    /// is core's own sentence, naming the link and, where it resolved, the target -
    /// the reason this one variant carries text is that those two paths ARE the
    /// diagnosis, and no enum field could hold them for an arbitrary file.
    #[error("{0} is a link; save to the real file instead")]
    ReparsePoint(String),
    /// The path can never be written as named: no file-name component, or a name
    /// Windows would silently alter - it strips trailing dots and spaces from the
    /// final component, so saving "a.notes." puts the bytes in "a.notes" while the
    /// app reports Ok for a path that does not exist (0db0b69's second data-loss
    /// fix). The [`String`] is core's sentence naming WHICH rule broke, kept
    /// rather than laundered into [`Other`] (D29/D36: add the variant).
    #[error("the path is not a usable file location: {0}")]
    InvalidPath(String),
    /// Anything else. Carries the OS text, because inventing a friendly string
    /// for an unknown failure hides the one clue the user has.
    #[error("{0}")]
    Other(String),
}

/// Why a file could not be opened.
///
/// The load-side sibling of [`SaveError`], added because the port otherwise had
/// no shaped failure for a Command::Open, and an open failure was being
/// reported as a SAVE failure — which is not a refactor of the vocabulary, it is
/// a wrong message: the user read-only'd nothing, filled no disk, and the thing
/// that failed was reading. Same rule as SaveError: **the Display text below IS
/// the user-visible copy** and is pinned verbatim in a test, so editing one of
/// these strings is a product decision (AGENTS.md: the enum is part of the
/// public contract because the UI must render the reason).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    /// Nothing is at that path (deleted, moved, or a typo in a command line).
    #[error("file no longer exists")]
    NotFound,
    /// The ACL or the sharing mode refused the read.
    #[error("permission denied")]
    PermissionDenied,
    /// Another process holds it exclusively.
    #[error("file is locked by another program")]
    Locked,
    /// Over the 8 MiB guard (D9). Reported as a refusal, not as a truncated
    /// load, because a half-loaded buffer whose missing half the user cannot see
    /// is worse than no load: an autosave would write back a shortened file.
    /// Emitted for real now, by the D9 byte guard in Engine::open, from the
    /// stat alone. It replaced a Loaded event that reported an encoding, a
    /// line ending and a read-only flag for bytes nobody had read (B1).
    #[error("file is too large to open")]
    TooLarge,
    /// The bytes are not text in any encoding we can read. the byte_offset field is
    /// where it broke, the encoding_hint field is what the header said — a hostile or
    /// half-written file must produce this, never a panic.
    #[error("cannot decode this file as text (at byte {byte_offset})")]
    Undecodable {
        /// Offset of the first byte that does not decode.
        byte_offset: usize,
        /// The encoding the file's own header claimed, when it claimed one.
        encoding_hint: Option<Encoding>,
    },
    /// Anything else, carrying the OS text for the same reason as
    /// [`SaveError::Other`].
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
    /// A file could not be opened, and why. Async like every other answer: an
    /// [`Open`](crate::Command::Open) has no reply channel to return an Err to.
    /// The bridge shows the reason and keeps the current document loaded rather
    /// than blanking the window into an unsaved-buffer lie.
    LoadFailed { path: PathBuf, reason: LoadError },
    /// A save succeeded, and the buffer at `revision` is now what is on disk
    /// (D11 — this is what makes later flushes at that revision `Clean`).
    Saved { path: PathBuf, revision: u64 },
    /// The port could not put the window where the session said it belongs,
    /// could not measure the rect to persist, or could not apply the pin bit.
    /// Emitted by [`RegisterWindow`](crate::Command::RegisterWindow) - the
    /// moment the handle becomes usable - and by the save tick when the
    /// measurement fails, LATCHED (one report per failure episode, not one per
    /// tick - an IsWindow refusal does not heal). Silence is forbidden here: a
    /// window that opens in the wrong place otherwise looks exactly like the
    /// app forgetting. THIS VARIANT ALSO CARRIES PIN FAILURES for now - one
    /// name, two causes, and the reason string is the only discriminator; the
    /// split is sequenced into the one event-vocabulary wave with the state-
    /// failure consolidation so the bridge's exhaustive match rewrites once.
    /// [`rect`] is the rect the port asked
    /// for, so the copy can name the place it could not reach; [`reason`] is
    /// notes-platform's own sentence (its PlatformError Display) passed
    /// through untranslated, for the same reason [`SaveError::Other`] carries
    /// OS text - it is the one clue the user has, and the port may not invent
    /// a friendlier one.
    GeometryNotRestored {
        /// Where the window was supposed to go, in frame pixels.
        rect: Rect,
        /// The platform's own words for why it refused.
        reason: String,
    },
    /// The open document is now a DIFFERENT file: Save As wrote it elsewhere, so
    /// the path changed, the arming changed with it (ADR-0001 requirement 4), and
    /// the `meta` field describes the file that exists now rather than the one first
    /// opened. Emitted right after [`Event::Saved`] for the same write.
    ///
    /// A variant of its own rather than a second [`Event::Loaded`] because Loaded
    /// carries the whole text and the bridge owns the buffer: re-sending a document
    /// to make a point about its encoding is the expensive way to say "fix your
    /// status line". AGENTS.md: when the bridge needs a fact the port never
    /// reported, add the event instead of reaching around the port - which is how
    /// [`Event::LoadFailed`] came to exist (D29).
    Rebound {
        path: PathBuf,
        meta: FileMeta,
        revision: u64,
    },
    /// A DOCUMENT save failed. The async one: `Flush` has no reply channel,
    /// so this is how the failure reaches the status line (AGENTS.md).
    ///
    /// DOCUMENT-ONLY, and that is the contract this variant's old shape broke
    /// (event.rs:312): `path` is the file the write was attempted on and
    /// `revision` is the buffer revision that write would have anchored (D11),
    /// and both are REAL, because a document has both. State files have
    /// neither, and riding them here forced a `revision: 0` the UI could
    /// render and the user could believe; they now report through their own
    /// family
    /// ([`Event::SessionWriteFailed`], [`Event::SettingsWriteFailed`],
    /// [`Event::StateDirUnusable`]). The port builds this variant at exactly
    /// one site - `Engine::document_save_failed` - whose assert is the
    /// tripwire for the empty-path case core already refuses as InvalidPath
    /// (save.rs:141) before any save outcome exists.
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
    /// settings.toml exists but is not valid TOML. D12: the bytes are left
    /// untouched on disk for diagnosis, the launch proceeds on factory
    /// settings - and this event is the rendering of THAT fact, because a
    /// corrupt file that quietly became defaults would read, a year later,
    /// as the app having lost a user's choice. Emitted once, at startup,
    /// before any engine event: [`Gateway::start`] is where the file is read,
    /// and the queue is the only output the port has. [`reason`] is core's
    /// own sentence (its SettingsError::Corrupt Display), passed through
    /// untranslated - the same rule as [`SaveError::Other`].
    SettingsCorrupt {
        /// What core's parser said, verbatim.
        reason: String,
    },
    /// The session file could not be written. NOT a [`SaveFailed`]: that
    /// event is a document save's answer and carries a `revision`, and a
    /// session file has none - the old shape claimed revision 0, which is a
    /// number the UI could render and the user could believe. Reported ONCE
    /// per failure episode and latched until a write succeeds (M5): the tick
    /// retries a failed session write forever, and without the latch that is
    /// one error toast per tick, for a problem the user cannot act on from
    /// inside the app. [`reason`] is core's own sentence (its SessionError
    /// Display), passed through untranslated.
    SessionWriteFailed {
        /// What core said when the session write was refused, verbatim.
        reason: String,
    },
    /// The state directory could not be created, or cannot be used as a
    /// directory (it is a file, or a link). The launch proceeds - the engine
    /// keeps running, and every state write fails and is reported through its
    /// own channel - but this is the ONE event that says WHY nothing will
    /// persist, at the moment it became true, instead of a status line after
    /// the first save quietly failed (the fresh-install blocker the smoke run
    /// caught: the app looked clean and remembered nothing). Emitted from
    /// [`Gateway::start`], which owns the directory. [`reason`] is the OS's
    /// sentence, or the factual sentence naming the path - never advice,
    /// because there is nothing to advise yet.
    StateDirUnusable {
        /// What the OS said, or which path is not a directory.
        reason: String,
    },
    /// settings.toml could not be written. The settings.toml twin of
    /// [`Event::SessionWriteFailed`] - same latch discipline (one report per
    /// failure episode, retried every tick, cleared on success), same no-
    /// revision honesty: a settings file HAS no revision, and the old shape
    /// claimed `revision: 0` through [`Event::SaveFailed`], which is a
    /// document event. [`reason`] is core's own sentence (SettingsError's
    /// Display), passed through untranslated.
    SettingsWriteFailed {
        /// What core said when the settings write was refused, verbatim.
        reason: String,
    },
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
            Event::Rebound {
                path,
                meta,
                revision,
            } => Event::Rebound {
                path: path.clone(),
                meta: *meta,
                revision: *revision,
            },
            Event::Loaded { path, text, meta } => Event::Loaded {
                path: path.clone(),
                text: text.clone(),
                meta: *meta,
            },
            Event::LoadFailed { path, reason } => Event::LoadFailed {
                path: path.clone(),
                reason: reason.clone(),
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
            Event::GeometryNotRestored { rect, reason } => Event::GeometryNotRestored {
                rect: *rect,
                reason: reason.clone(),
            },
            Event::RecentsUpdated(entries) => Event::RecentsUpdated(entries.clone()),
            Event::SettingsCorrupt { reason } => Event::SettingsCorrupt {
                reason: reason.clone(),
            },
            Event::SessionWriteFailed { reason } => Event::SessionWriteFailed {
                reason: reason.clone(),
            },
            Event::StateDirUnusable { reason } => Event::StateDirUnusable {
                reason: reason.clone(),
            },
            Event::SettingsWriteFailed { reason } => Event::SettingsWriteFailed {
                reason: reason.clone(),
            },
        }
    }

    fn variant(event: &Event) -> &'static str {
        match event {
            Event::Loaded { .. } => "Loaded",
            Event::LoadFailed { .. } => "LoadFailed",
            Event::Saved { .. } => "Saved",
            Event::Rebound { .. } => "Rebound",
            Event::SaveFailed { .. } => "SaveFailed",
            Event::ExternalChange { .. } => "ExternalChange",
            Event::AutosaveSkipped { .. } => "AutosaveSkipped",
            Event::GeometryNotRestored { .. } => "GeometryNotRestored",
            Event::SettingsCorrupt { .. } => "SettingsCorrupt",
            Event::SessionWriteFailed { .. } => "SessionWriteFailed",
            Event::StateDirUnusable { .. } => "StateDirUnusable",
            Event::SettingsWriteFailed { .. } => "SettingsWriteFailed",

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
            Event::LoadFailed {
                path: PathBuf::from("C:/notes/gone.md"),
                reason: LoadError::NotFound,
            },
            Event::Rebound {
                path: PathBuf::from("C:/notes/renamed.notes"),
                meta: FileMeta {
                    encoding: Encoding::Utf16Le,
                    line_ending: LineEnding::CrLf,
                    trailing_newline: true,
                    read_only: false,
                    oversize: false,
                    armed: true,
                },
                revision: 4,
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
            Event::GeometryNotRestored {
                rect: Rect::new(120, 90, 800, 600),
                reason: "Win32 SetWindowPos failed: the fixture's sentence".to_string(),
            },
            Event::RecentsUpdated(vec![RecentEntry {
                path: PathBuf::from("C:/notes/a.notes"),
                display: "a.notes".to_string(),
                exists: true,
            }]),
            Event::SettingsCorrupt {
                reason: "settings file corrupt: expected a value at line 2".to_string(),
            },
            Event::SessionWriteFailed {
                reason: "no space left on device".to_string(),
            },
            Event::StateDirUnusable {
                reason: "Access is denied. (os error 5)".to_string(),
            },
            Event::SettingsWriteFailed {
                reason: "settings could not be serialised: unsupported type".to_string(),
            },
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
        // Loaded, LoadFailed, Saved, GeometryNotRestored, SaveFailed,
        // ExternalChange, AutosaveSkipped, RecentsUpdated, SettingsCorrupt,
        // SessionWriteFailed, StateDirUnusable, SettingsWriteFailed.
        // (GeometryNotRestored still carries BOTH the placement and the pin
        // causes; the split is one vocabulary wave with the state-failure
        // consolidation, after this slice. A bounded-join timeout is reported
        // through close()'s typed Err, not through an Event: a Gateway-held
        // Event sender would delay the Disconnected contract.)
        assert_eq!(all.len(), 13, "Event gained or lost a variant");

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
            Event::LoadFailed {
                path: PathBuf::from("a"),
                reason: LoadError::Locked
            },
            Event::LoadFailed {
                path: PathBuf::from("a"),
                reason: LoadError::NotFound
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

    /// Acceptance (b), load half: the shipped UI copy of every `LoadError`.
    /// Pinned for the same reason as SaveError's — these strings are what the
    /// user reads when their file will not open, so a rename is free and an
    /// edit is a product change.
    #[test]
    fn load_error_ui_copy_is_pinned() {
        assert_eq!(LoadError::NotFound.to_string(), "file no longer exists");
        assert_eq!(LoadError::PermissionDenied.to_string(), "permission denied");
        assert_eq!(
            LoadError::Locked.to_string(),
            "file is locked by another program"
        );
        assert_eq!(LoadError::TooLarge.to_string(), "file is too large to open");
        assert_eq!(
            LoadError::Undecodable {
                byte_offset: 17,
                encoding_hint: Some(Encoding::Utf16Le),
            }
            .to_string(),
            "cannot decode this file as text (at byte 17)"
        );
        assert_eq!(
            LoadError::Other("os error 1450".to_string()).to_string(),
            "os error 1450"
        );
    }

    /// `Undecodable` must carry WHERE it broke and what the header claimed:
    /// `{0}`-style copy hides the offset the user would quote to support, and a
    /// lost encoding hint is how a UTF-19-ish file becomes an unanswerable bug.
    #[test]
    fn undecodable_keeps_its_offset_and_hint() {
        let reason = LoadError::Undecodable {
            byte_offset: 4096,
            encoding_hint: Some(Encoding::Ansi(1252)),
        };
        let same = reason.clone();
        assert_eq!(reason, same);
        assert_ne!(
            reason,
            LoadError::Undecodable {
                byte_offset: 4097,
                encoding_hint: Some(Encoding::Ansi(1252)),
            }
        );
        assert_ne!(
            reason,
            LoadError::Undecodable {
                byte_offset: 4096,
                encoding_hint: None,
            }
        );
        assert!(
            reason.to_string().contains("4096"),
            "the offset is in the copy"
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

#[cfg(test)]
mod honesty_tests {
    use super::*;

    /// M9: the two cases the engine used to describe with strings it invented now
    /// have variants, and the copy is pinned where the rest of it is.
    #[test]
    fn a_missing_target_is_a_skip_and_a_refused_save_is_not_an_invented_string() {
        assert_eq!(
            SaveError::NoTarget.to_string(),
            "this note has not been saved to a file yet"
        );
        // SkipReason carries no Display on purpose (slice 1's ruling: the words are
        // the ADR's and the UI's, not the port's), so what is pinned is that the
        // variant exists as a distinct answer.
        assert_ne!(SkipReason::NeedsPath, SkipReason::Clean);
        assert_ne!(SkipReason::NeedsPath, SkipReason::AutosaveDisabled);
        // Distinct shapes, because one is an event the UI toasts and the other is a
        // status-line note that the user can act on with Ctrl+S.
        assert_ne!(
            SaveError::NoTarget.to_string(),
            SaveError::Other(String::new()).to_string(),
            "an unknown failure must not look like the missing-target case"
        );
        assert_ne!(SaveError::NoTarget, SaveError::NotFound);
        // 0db0b69's two new core reasons are mapped, not laundered.
        assert_eq!(
            SaveError::InvalidPath("the path has no file name component".to_string()).to_string(),
            "the path is not a usable file location: the path has no file name component"
        );
        assert_eq!(
            SaveError::ReparsePoint("C:/one/a.notes".to_string()).to_string(),
            "C:/one/a.notes is a link; save to the real file instead"
        );
        assert_ne!(
            SaveError::ReparsePoint("x".to_string()).to_string(),
            SaveError::Other("x".to_string()).to_string(),
            "a link is actionable advice, not an unclassified failure"
        );
    }
}
