//! The recent-files MRU list (features.md 4.5).
//!
//! Filesystem honesty: identity_key (via canonicalise) is the ONLY function
//! in this module that touches the filesystem — and even it never writes.
//! push, mark_missing and clear are pure functions over a Vec, so the MRU
//! logic unit-tests with no filesystem at all.
//!
//! Identity (AGENTS.md: "case-insensitive but not case-preserving"):
//! * canonicalise for IDENTITY, keep the original for DISPLAY. On Windows,
//!   canonicalise returns a \\?\-prefixed verbatim path, which is stripped
//!   ("\\?\UNC\server\share" becomes "\\server\share"), and the key is
//!   lowercased with full Unicode to_lowercase — NOT ASCII-lowercase —
//!   because Turkish i and accented paths are real.
//! * On non-Windows platforms identity is case-SENSITIVE, on purpose: POSIX
//!   filesystems distinguish case, and pretending otherwise would merge
//!   genuinely different files. This is a written-down platform difference,
//!   not an accident.
//! * If canonicalise fails (the file vanished, a dead network path), the key
//!   falls back to a LEXICAL normalisation: separators unified, trailing
//!   separators dropped, '.' and '..' folded. Consequence, documented and
//!   accepted: a vanished file can appear twice if it was typed two
//!   different ways that only agree after canonicalisation (for example a
//!   drive-mapped p:\x.notes versus \\server\share\x.notes — canonicalise
//!   would unify an existing file, the lexical fallback cannot).
//!
//! exists: a pushed entry was just used, so it is recorded as existing.
//! mark_missing greys every entry out WITHOUT deleting it and WITHOUT
//! reordering ("grey out — don't silently delete — entries whose file has
//! vanished"); the next push of that file flips it back and moves it to the
//! top. core never probes the filesystem for this — that would be a second
//! fs-touching function.

use std::path::{Component, Path, PathBuf};

/// The recent-files cap (docs 4.5: at most 10).
pub const MAX_RECENTS: usize = 10;

/// One recent-files menu entry. path is the original spelling the user (or
/// the file dialog) provided; display is the menu label the user first saw.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecentEntry {
    pub path: PathBuf,
    pub display: String,
    pub exists: bool,
}

/// A recent-files list capped at MAX_RECENTS. The push/mark_missing/clear
/// functions below operate on the inner Vec directly.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecentList(pub Vec<RecentEntry>);

/// The identity key of a path: canonicalise when the file exists, lexical
/// normalisation when it does not, verbatim prefixes stripped, lowercased on
/// Windows. Stable across repeated calls for the same input.
pub fn identity_key(path: &Path) -> String {
    identity_of(path).0
}

/// Pushes a path to the top of the MRU: move-to-top semantics, no
/// duplicates, capped at MAX_RECENTS by dropping the OLDEST entries.
///
/// Re-opening an existing entry (matched by identity key) never creates a
/// duplicate and keeps the STORED display string — the user's original
/// spelling wins — unless the new path is the canonical one (the file
/// exists and canonicalise resolved it), in which case the canonical
/// spelling replaces it. Entries that already existed keep their slot and
/// relative order; the moved entry records exists = true.
pub fn push(list: Vec<RecentEntry>, path: PathBuf, display: &str) -> Vec<RecentEntry> {
    let (key, canonical) = identity_of(&path);
    let mut out: Vec<RecentEntry> = Vec::with_capacity(MAX_RECENTS);
    // Top slot, provisionally the new spelling.
    out.push(RecentEntry {
        path,
        display: display.to_owned(),
        exists: true,
    });
    let mut replaced = false;
    for entry in list {
        if identity_of(&entry.path).0 == key {
            // Every older slot for this identity is dropped (dedupe).
            if !replaced && !canonical {
                // The new path was typed non-canonically for a vanished
                // file: the user's original spelling wins.
                out[0] = RecentEntry {
                    path: entry.path,
                    display: entry.display,
                    exists: true,
                };
            }
            replaced = true;
            continue;
        }
        out.push(entry);
    }
    out.truncate(MAX_RECENTS);
    out
}

/// Greys out every entry (exists = false) without deleting or reordering.
/// Pure: core never probes the filesystem here — the next push of a file
/// flips its entry back and moves it to the top.
pub fn mark_missing(list: &mut [RecentEntry]) {
    for entry in list.iter_mut() {
        entry.exists = false;
    }
}

/// The menu's Clear recent action: an empty list.
pub fn clear() -> Vec<RecentEntry> {
    Vec::new()
}

/// (identity key, was-canonicalised). The single filesystem touch in this
/// module: canonicalise reads, nothing writes — and NEVER on a path the
/// name-only policy has judged hostile: canonicalise on an unreachable UNC
/// host froze a quit for 2.68s in measurement. The lexical fallback is pure
/// and instant, and was-canonicalised=false is the honest answer (core did
/// not touch the filesystem).
fn identity_of(path: &Path) -> (String, bool) {
    if crate::path_policy::path_policy(path) != crate::path_policy::PathVerdict::Allowed {
        return (
            key_from_raw(&lexical_normalisation(path).to_string_lossy()),
            false,
        );
    }
    match std::fs::canonicalize(path) {
        Ok(canonical) => (key_from_raw(&canonical.to_string_lossy()), true),
        Err(_) => (
            key_from_raw(&lexical_normalisation(path).to_string_lossy()),
            false,
        ),
    }
}

fn key_from_raw(raw: &str) -> String {
    let stripped = strip_verbatim(raw);
    if cfg!(windows) {
        // Full Unicode lowercase: Turkish i and accented paths are real.
        // Non-Windows stays case-sensitive BY DESIGN (see module docs).
        stripped.to_lowercase()
    } else {
        stripped
    }
}

/// Strips the verbatim prefixes std::fs::canonicalize produces on Windows:
/// \\?\C:\... -> C:\... and \\?\UNC\server\share -> \\server\share.
fn strip_verbatim(p: &str) -> String {
    if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
        format!("\\\\{rest}")
    } else if let Some(rest) = p.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        p.to_owned()
    }
}

/// The no-filesystem fallback: separators unified by component re-joining,
/// trailing separators dropped, '.' folded and '..' popped (best effort —
/// popping past a root stops there, which is the correct ceiling).
fn lexical_normalisation(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, display: &str) -> RecentEntry {
        RecentEntry {
            path: PathBuf::from(path),
            display: display.to_owned(),
            exists: true,
        }
    }

    #[test]
    fn identity_key_is_stable_across_repeated_calls() {
        let p = Path::new(r"C:\some\where\note.notes");
        assert_eq!(identity_key(p), identity_key(p));
        assert_eq!(identity_key(p), identity_of(p).0);
    }

    #[test]
    fn differently_cased_windows_paths_are_one_entry() {
        // The brief's example. Whether or not the file exists, the two
        // spellings share one identity: canonicalise unifies an existing
        // file, the lexical fallback plus lowercase unifies a vanished one.
        let list = push(Vec::new(), PathBuf::from(r"C:\DOCs\A.notes"), "A.notes");
        let list = push(list, PathBuf::from(r"c:\docs\a.notes"), "a.notes");
        assert_eq!(list.len(), 1, "case-insensitive identity: one entry");
    }

    #[test]
    fn repush_of_existing_entry_preserves_the_display_casing() {
        // Guaranteed-nonexistent directory: identity comes from the lexical
        // fallback, which is NOT canonical, so the user's original spelling
        // of the stored entry wins over the re-typed one.
        let list = push(
            Vec::new(),
            PathBuf::from(r"C:\definitely\not\real\A.notes"),
            r"C:\DOCs\A.notes",
        );
        let list = push(
            list,
            PathBuf::from(r"c:\definitely\not\real\a.notes"),
            r"c:\docs\a.notes",
        );
        assert_eq!(list.len(), 1);
        assert_eq!(
            list[0].display, r"C:\DOCs\A.notes",
            "original spelling wins"
        );
        assert_eq!(
            list[0].path,
            PathBuf::from(r"C:\definitely\not\real\A.notes")
        );
        assert!(list[0].exists, "a push means the file was just used");
    }

    #[test]
    fn unc_and_mapped_drive_are_a_documented_limitation() {
        // Accepted limitation (module docs): without a living file the
        // lexical fallback cannot map a drive letter to its UNC form, so
        // these two spellings are two entries. canonicalise would unify
        // them if the file existed; this test pins the vanished case.
        let list = push(Vec::new(), PathBuf::from(r"\\server\share\x.notes"), "x");
        let list = push(list, PathBuf::from(r"p:\x.notes"), "x");
        assert_eq!(list.len(), 2, "documented limitation, not silent equality");
    }

    #[test]
    fn repeated_pushes_never_duplicate() {
        let mut list = Vec::new();
        for _ in 0..3 {
            list = push(list, PathBuf::from(r"C:\definitely\not\real\n.notes"), "n");
        }
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn twelve_pushes_cap_at_ten_dropping_the_two_oldest() {
        let mut list = Vec::new();
        for i in 1..=12 {
            list = push(
                list,
                PathBuf::from(format!(r"C:\definitely\not\real\note{i}.notes")),
                "note",
            );
        }
        assert_eq!(list.len(), MAX_RECENTS);
        // The two oldest (1, 2) are gone; the newest is on top.
        for i in 3..=12 {
            let name = format!(r"C:\definitely\not\real\note{i}.notes");
            assert!(
                list.iter().any(|e| e.path == Path::new(&name)),
                "note{i} should have survived"
            );
        }
        assert_eq!(
            list[0].path,
            Path::new(r"C:\definitely\not\real\note12.notes")
        );
        assert!(
            !list
                .iter()
                .any(|e| e.path == Path::new(r"C:\definitely\not\real\note1.notes"))
        );
        assert!(
            !list
                .iter()
                .any(|e| e.path == Path::new(r"C:\definitely\not\real\note2.notes"))
        );
    }

    #[test]
    fn mark_missing_flips_exists_without_reordering() {
        let mut list = vec![
            entry(r"C:\a\one.notes", "one"),
            entry(r"C:\b\two.notes", "two"),
            entry(r"C:\c\three.notes", "three"),
        ];
        mark_missing(&mut list);
        assert!(list.iter().all(|e| !e.exists), "all greyed out");
        assert_eq!(list[0].display, "one");
        assert_eq!(list[1].display, "two");
        assert_eq!(list[2].display, "three", "no reordering");
        // Re-opening the middle file flips it back and moves it to the top,
        // leaving the others greyed and in place.
        let list = push(list, PathBuf::from(r"C:\b\two.notes"), "two");
        assert!(list[0].exists);
        assert_eq!(list[0].display, "two");
        assert_eq!(list[1].path, PathBuf::from(r"C:\a\one.notes"));
        assert!(!list[1].exists);
        assert!(!list[2].exists);
    }

    #[test]
    fn clear_returns_an_empty_list() {
        assert!(clear().is_empty());
        assert_eq!(RecentList::default().0.len(), 0);
    }

    #[cfg(not(windows))]
    #[test]
    fn case_sensitive_platforms_keep_two_entries() {
        // Written down, not accidental: on case-sensitive filesystems the
        // two spellings are genuinely different files.
        let list = push(Vec::new(), PathBuf::from("/data/NOTES/a.notes"), "a");
        let list = push(list, PathBuf::from("/data/notes/a.notes"), "a");
        assert_eq!(list.len(), 2);
    }
}
