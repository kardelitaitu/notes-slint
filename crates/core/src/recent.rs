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

use std::path::{Component, MAIN_SEPARATOR_STR, Path, PathBuf};

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

/// How many parent folders a colliding label may show. The judgement: one
/// folder settles the common "two readme.txt in different folders", two
/// settles nested clones (Project\ vs Project copy\); past that the label
/// is becoming the path, which is what the title bar already shows.
const MAX_LABEL_PARENTS: usize = 2;
/// The hard character cap for any label. The component bound above cannot
/// bound a label by itself — a single folder NAME has no length limit — so
/// this cap is what makes "never an unbounded path in a menu" true.
const MAX_LABEL_CHARS: usize = 64;

/// The menu-label rule (features.md 4.4), owned here because it is a RULE,
/// not a rendering detail — a label decision living in the port would be
/// the silent architecture change AGENTS.md calls out. Pure: names in,
/// labels out, no canonicalisation, no filesystem — the label is judged
/// from the stored spellings alone.
///
/// * One label per entry, parallel to the input. The label is the file
///   NAME, case-preserved: identity is case-insensitive (AGENTS.md) and
///   lives in identity_key, never in the label.
/// * Entries that share a basename CASE-INSENSITIVELY (two "readme.txt")
///   must not both render as the same bare word: the nearest parent
///   folders are appended, growing jointly just until the labels differ.
/// * The suffix is BOUNDED — at most MAX_LABEL_PARENTS folders — and "…"
///   marks any parents above it that were cut; every label is then
///   hard-capped at MAX_LABEL_CHARS chars. Residual ambiguity past the
///   bounds is honest and harmless: a label is never an identity.
/// * A vanished entry (exists = false) labels exactly like a present one:
///   greying is the bridge's rendering of 'exists', not a naming rule.
pub fn display_labels(entries: &[RecentEntry]) -> Vec<String> {
    // The bare, case-preserved file name; the stored display string is the
    // fallback for degenerate paths with no file-name component (a bare
    // drive), which a recent list cannot normally hold.
    let basenames: Vec<String> = entries
        .iter()
        .map(|e| {
            e.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| e.display.clone())
        })
        .collect();
    // Collision groups, keyed case-insensitively (full Unicode lowercase,
    // the same philosophy as the identity key: a label that READS the same
    // is the ambiguity, whatever the case trick).
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, b) in basenames.iter().enumerate() {
        let key = b.to_lowercase();
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, g)) => g.push(i),
            None => groups.push((key, vec![i])),
        }
    }
    // The basenames stay intact as the composition base; labels is the
    // output being grown. (Cloned: a group's take-2 suffix must compose
    // onto the BASENAME, not onto its own take-1 label.)
    let mut labels = basenames.clone();
    for (_, group) in &groups {
        if group.len() < 2 {
            continue;
        }
        let chains: Vec<Vec<String>> = group
            .iter()
            .map(|&i| parent_chain(&entries[i].path))
            .collect();
        // Grow every member's suffix jointly; stop at the FIRST length
        // where the labels come apart, or at the bound.
        let mut shown = 0usize;
        let mut resolved = false;
        for take in 1..=MAX_LABEL_PARENTS {
            let candidate: Vec<String> = group
                .iter()
                .zip(&chains)
                .map(|(&i, chain)| compose(&parent_suffix(chain, take), &basenames[i]))
                .collect();
            let distinct = candidate
                .iter()
                .enumerate()
                .all(|(a, l)| candidate[a + 1..].iter().all(|other| other != l));
            for (&i, l) in group.iter().zip(&candidate) {
                labels[i] = l.clone();
            }
            if distinct {
                shown = take;
                resolved = true;
                break;
            }
        }
        if !resolved {
            shown = MAX_LABEL_PARENTS;
            for (&i, chain) in group.iter().zip(&chains) {
                labels[i] = compose(&parent_suffix(chain, MAX_LABEL_PARENTS), &basenames[i]);
            }
        }
        // Mark the cut whenever real parent FOLDERS above the shown suffix
        // were dropped — the drive and the root separator are never parents
        // a menu needs, so they do not count as truncation.
        for &i in group.iter() {
            if parent_count(&entries[i].path) > shown {
                labels[i] = format!("…{MAIN_SEPARATOR_STR}{}", labels[i]);
            }
        }
    }
    labels.into_iter().map(hard_cap).collect()
}

/// (identity key, was-canonicalised). The single filesystem touch in this
/// module: canonicalise reads, nothing writes. The policy verdict decides
/// whether canonicalise is SAFE to attempt, per verdict:
/// * UnboundedNetwork — NEVER: canonicalise on an unreachable UNC host froze
///   a quit for 2.68s in measurement.
/// * ReservedDevice — NEVER: canonicalising a device namespace opens the
///   device, which is the harm the verdict exists to refuse.
/// * StrippedName — YES (MAJOR-3): a stripped spelling is a LOCAL mangled
///   name the OS will rewrite on contact, so canonicalising it is exactly
///   "normalise to what the OS would write". Without this, one real file
///   occupies two recent slots and the refused spelling can never be
///   reopened — the list refuses the name it displays.
/// * StreamName / DriveRelative — NO: a stream is another file's storage,
///   and a drive-relative name resolves against invisible process state;
///   both stay lexical (and drive-relative names are refused outright).
///   The lexical fallback is pure and instant, and was-canonicalised=false
///   is the honest answer (core did not touch the filesystem).
fn identity_of(path: &Path) -> (String, bool) {
    use crate::path_policy::PathVerdict;
    let canonicalisable = !matches!(
        crate::path_policy::path_policy(path),
        PathVerdict::UnboundedNetwork | PathVerdict::ReservedDevice
    );
    if !canonicalisable {
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

/// The parent folders of the path, NEAREST first, as display strings. Pure:
/// component inspection only — no canonicalisation, no filesystem.
fn parent_chain(path: &Path) -> Vec<String> {
    // A drive-RELATIVE path ("C:notes.notes") has a prefix but no root: its
    // "parent" is the per-drive CWD, which a menu label must never invent
    // (BLOCKER-1) — the label falls back to the bare name.
    let drive_relative =
        matches!(path.components().next(), Some(Component::Prefix(_))) && !path.has_root();
    if drive_relative {
        return Vec::new();
    }
    let comps: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let names = comps.len().saturating_sub(1);
    comps[..names].iter().rev().cloned().collect()
}

/// How many real parent folders the path has: the Normal components minus
/// the file name itself.
fn parent_count(path: &Path) -> usize {
    path.components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count()
        .saturating_sub(1)
}

/// The nearest 'take' parents, re-joined root-first: ["b", "a"] -> "a\b".
fn parent_suffix(chain: &[String], take: usize) -> String {
    chain
        .iter()
        .take(take)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join(MAIN_SEPARATOR_STR)
}

fn compose(suffix: &str, basename: &str) -> String {
    if suffix.is_empty() {
        basename.to_owned()
    } else {
        format!("{suffix}{MAIN_SEPARATOR_STR}{basename}")
    }
}

/// The hard character cap: keep the tail (the nearest folders and the file
/// name), mark the cut. Bounded is the promise; this is where it is kept.
fn hard_cap(label: String) -> String {
    let count = label.chars().count();
    if count <= MAX_LABEL_CHARS {
        return label;
    }
    let tail: String = label.chars().skip(count - (MAX_LABEL_CHARS - 1)).collect();
    format!("…{tail}")
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

    #[test]
    fn labels_are_the_case_preserved_file_name() {
        let labels = display_labels(&[
            entry(r"C:\Docs\ReadMe.NOTES", "whatever"),
            entry(r"C:\other\idea.notes", "x"),
        ]);
        assert_eq!(
            labels,
            vec!["ReadMe.NOTES".to_owned(), "idea.notes".to_owned()]
        );
    }

    #[test]
    fn colliding_basenames_do_not_both_render_as_the_bare_word() {
        // D33: the collision rule. Two identical basenames must not both
        // render as the same bare word.
        let labels = display_labels(&[
            entry(r"C:\a\readme.txt", "r"),
            entry(r"C:\b\readme.txt", "r"),
        ]);
        assert_eq!(labels, vec![r"a\readme.txt", r"b\readme.txt"]);
    }

    #[test]
    fn colliding_labels_grow_jointly_until_they_differ() {
        // The nearest folders agree; the second one distinguishes. Both
        // folder chains fit inside the bound, so nothing was cut and no
        // truncation marker is honest.
        let labels = display_labels(&[
            entry(r"C:\p\x\readme.txt", "r"),
            entry(r"C:\q\x\readme.txt", "r"),
        ]);
        assert_eq!(labels, vec![r"p\x\readme.txt", r"q\x\readme.txt"]);
    }

    #[test]
    fn the_parent_suffix_is_bounded_and_marks_the_cut() {
        // The distinguishing folder is three up: past the bound the label
        // caps at the two nearest parents and the ellipsis marks the cut.
        // The residual ambiguity is honest: a label is never an identity.
        let labels = display_labels(&[
            entry(r"C:\a\b\c\readme.txt", "r"),
            entry(r"C:\x\b\c\readme.txt", "r"),
        ]);
        assert_eq!(labels, vec![r"…\b\c\readme.txt", r"…\b\c\readme.txt"]);
    }

    #[test]
    fn no_label_exceeds_the_hard_character_cap() {
        // A folder NAME has no length limit, so the component bound alone
        // cannot bound a label; the character cap keeps the promise.
        let long = "f".repeat(100);
        let labels = display_labels(&[
            entry(&format!(r"C:\{long}\readme.txt"), "r"),
            entry(r"C:\g\readme.txt", "r"),
        ]);
        assert_eq!(labels[1], r"g\readme.txt", "short labels untouched");
        assert!(
            labels[0].chars().count() <= 64 && labels[0].starts_with('…'),
            "capped and marked: {labels:?}"
        );
        assert!(
            labels[0].ends_with("readme.txt"),
            "the name survives: {labels:?}"
        );
    }

    #[test]
    fn case_only_twin_names_stay_case_preserved_and_distinct() {
        // Same folder, different case: the suffix cannot help, but the
        // case-preserved names are different words — display is not identity.
        let labels = display_labels(&[
            entry(r"C:\docs\README.txt", "r"),
            entry(r"c:\docs\readme.txt", "r"),
        ]);
        assert_eq!(labels, vec![r"docs\README.txt", r"docs\readme.txt"]);
    }

    #[test]
    fn vanished_entries_get_the_same_label_greyed_elsewhere() {
        let mut entries = vec![
            entry(r"C:\a\one.notes", "one"),
            entry(r"C:\b\one.notes", "one"),
        ];
        entries[1].exists = false;
        let labels = display_labels(&entries);
        assert_eq!(labels, vec![r"a\one.notes", r"b\one.notes"]);
    }

    #[test]
    fn degenerate_paths_fall_back_to_the_stored_display() {
        let labels = display_labels(&[entry("C:", "C:")]);
        assert_eq!(labels, vec!["C:"]);
    }

    #[test]
    fn empty_list_yields_no_labels() {
        assert!(display_labels(&[]).is_empty());
    }
}
