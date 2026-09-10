//! Where the app keeps its state, resolved without touching the filesystem
//! or the environment.

use std::path::{Path, PathBuf};

use crate::SessionError;

/// The directory the app persists state into (window geometry, session).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDir(pub PathBuf);

/// Resolves the state directory from its inputs alone: no environment reads,
/// no filesystem access, fully deterministic — which is what makes it
/// testable on any OS.
///
/// The rule:
///
/// * PORTABLE — state lives next to the executable: <exe_dir>/data. A caller
///   lands on this branch in any of three ways:
///   1. the environment has no per-user roaming profile (appdata is None),
///      so there is nowhere else to put state;
///   2. the caller probed <exe_dir>/data with a single Path::exists() and
///      found the portable marker directory. That probe is deliberately the
///      caller's job — this function must stay pure — and the caller reports
///      the finding by passing appdata = None. This split is the documented
///      caller contract: crates/api and the bridge own the one exists()
///      probe and pass appdata = None for a portable deployment.
///   3. appdata is present but EMPTY. An empty path would resolve the state
///      dir CWD-relative ("notes-gpui" wherever the launcher ran), so a
///      one-word change in the launcher could silently redirect user state;
///      core treats it as no profile at all (a lexical check — still pure).
/// * INSTALLED — state lives in the per-user roaming profile:
///   <appdata>/notes-gpui (on Windows %APPDATA%\notes-gpui). appdata must
///   be non-empty to count as a usable profile.
///
/// The caller-side decision, for reference:
///
/// ```no_run
/// # let exe_dir = std::path::Path::new("C:/apps/notes");
/// # let appdata = Some(std::path::Path::new("C:/Users/u/AppData/Roaming"));
/// let usable_appdata = appdata.filter(|p| !p.as_os_str().is_empty());
/// let state = if exe_dir.join("data").exists() || usable_appdata.is_none() {
///     notes_core::resolve_state_dir(exe_dir, None) // portable
/// } else {
///     notes_core::resolve_state_dir(exe_dir, appdata) // installed
/// };
/// let _ = state;
/// ```
///
/// Inputs are used verbatim: no canonicalisation, no case folding, no
/// separator rewriting beyond Path::join semantics (a trailing separator on
/// either input does not produce a doubled separator).
pub fn resolve_state_dir(exe_dir: &Path, appdata: Option<&Path>) -> StateDir {
    match appdata {
        // Installed: a USABLE roaming profile. An empty path does not count:
        // appdata = "" would resolve state CWD-relative ("notes-gpui" next
        // to wherever the launcher ran), so a one-word launcher change could
        // silently redirect user state.
        Some(appdata) if !appdata.as_os_str().is_empty() => StateDir(appdata.join("notes-gpui")),
        // Portable: no roaming profile at all, the caller-found "data"
        // marker (the caller passes None — see the probe in the doc
        // comment), or an empty appdata path, treated as no profile at all.
        _ => StateDir(exe_dir.join("data")),
    }
}

/// The scratch note's FIXED home: <StateDir>/notes/untitled.notes (D69).
///
/// The name is DETERMINISTIC on purpose — no pid, no timestamp, no counter:
/// * two instances of the app share one scratch, and whole-file-or-nothing
///   atomic rename makes that last-writer-wins — already the sharing model
///   of session.json and settings.toml, since no single-instance guard
///   exists anywhere in this repo;
/// * a generated name is how a state dir becomes a junk drawer: every
///   crashed launch would leave another orphan behind.
///
/// Pure and lexical: no filesystem, no environment — the same contract as
/// resolve_state_dir. The caller (api) owns writing the file; core owns
/// deciding WHERE it lives and proving the place is safe
/// (ensure_scratch_dir), and the name is judged by path_policy like every
/// other name (see the pin test below).
pub fn scratch_note_path(dir: &StateDir) -> PathBuf {
    dir.0.join("notes").join("untitled.notes")
}

/// Creates — and proves usable — <StateDir>/notes, the scratch note's
/// directory (D69). Reuses session::ensure_state_dir WHOLESALE: the same
/// pure metadata judge, the same writability probe, no second copy to drift.
///
/// The scratch verdict is checked against the name-only policy BEFORE any
/// filesystem call, so a UNC or otherwise hostile state dir is refused
/// without a network stall: "save the untitled note" must never become the
/// 2.68s freeze this repo already measured once.
pub fn ensure_scratch_dir(dir: &StateDir) -> Result<(), SessionError> {
    let notes = dir.0.join("notes");
    if crate::path_policy::path_policy(&notes) != crate::path_policy::PathVerdict::Allowed {
        return Err(SessionError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "the scratch location is refused by the name-only policy: {}",
                notes.display()
            ),
        )));
    }
    crate::session::ensure_state_dir(&notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Table test over the documented rule. Cases 1 and 3 both land on the
    /// portable branch — the function is pure, so "a data directory exists
    /// next to the exe" is reported by the caller passing appdata = None
    /// (the one exists() probe is the caller's job, see the doc comment).
    #[test]
    fn state_dir_resolution_table() {
        // 1. Portable deployment: a data dir exists next to the exe. The
        //    caller probed it and passes appdata = None — even though this
        //    machine does have a roaming profile in reality.
        let portable = resolve_state_dir(Path::new("C:/apps/notes"), None);
        assert_eq!(portable.0, Path::new("C:/apps/notes").join("data"));

        // 2. Installed: APPDATA present, no portable marker.
        let installed = resolve_state_dir(
            Path::new("C:/Program Files/Notes"),
            Some(Path::new("C:/Users/u/AppData/Roaming")),
        );
        assert_eq!(
            installed.0,
            Path::new("C:/Users/u/AppData/Roaming").join("notes-gpui")
        );

        // 3. No APPDATA at all: falls back to portable, next to the exe.
        let fallback = resolve_state_dir(Path::new("U:/portable/notes"), None);
        assert_eq!(fallback.0, Path::new("U:/portable/notes").join("data"));

        // 4. Input hygiene: a trailing separator must not produce a doubled
        //    separator, and differently-cased input is used verbatim (pure
        //    lexical rule — no filesystem canonicalisation to fold case).
        let trailing = resolve_state_dir(Path::new("C:/apps/notes/"), None);
        assert_eq!(trailing.0, Path::new("C:/apps/notes").join("data"));
        let upper = resolve_state_dir(Path::new("C:/APPS/NOTES"), None);
        assert_eq!(upper.0, Path::new("C:/APPS/NOTES").join("data"));

        // 5. Empty APPDATA is treated as absent: an empty path would make the
        //    state dir CWD-relative, silently redirecting user state. The
        //    check is lexical — core stays pure (no fs, no env).
        let empty_appdata = resolve_state_dir(Path::new("C:/apps/notes"), Some(Path::new("")));
        assert_eq!(empty_appdata.0, Path::new("C:/apps/notes").join("data"));
    }

    /// THE PIN (D69): our own default must pass our own predicate for every
    /// StateDir shape core can produce. If the scratch name failed
    /// path_policy, the save path would refuse a file we invented — exactly
    /// the bug class this repo has been hunting. D33: this test fails if the
    /// scratch name is changed to "untitled.notes." (StrippedName).
    #[test]
    fn the_scratch_note_passes_the_name_only_policy_for_every_state_shape() {
        let shapes = [
            resolve_state_dir(
                Path::new("C:/Program Files/Notes"),
                Some(Path::new("C:/Users/u/AppData/Roaming")),
            ),
            resolve_state_dir(Path::new("C:/apps/notes"), None),
            StateDir(PathBuf::from("C:/Users/u/AppData/Roaming/notes-gpui")),
            StateDir(PathBuf::from("C:/Apps/Notes Portable/Data with spaces")),
            StateDir(PathBuf::from(
                "C:/Users/multi.dot.user/AppData/Roaming/notes-gpui",
            )),
        ];
        for dir in shapes {
            let scratch = scratch_note_path(&dir);
            assert_eq!(
                crate::path_policy::path_policy(&scratch),
                crate::path_policy::PathVerdict::Allowed,
                "our own scratch home must pass our own predicate: {scratch:?}"
            );
        }
    }

    /// No I/O is attempted, so nonsense paths resolve lexically and never
    /// panic (AGENTS.md: no unwrap/expect on I/O-reachable paths).
    #[test]
    fn state_dir_resolution_never_panics_on_nonsense_paths() {
        let nonsense = Path::new("//??/<>*|?\0::///");
        let _ = resolve_state_dir(nonsense, None);
        let _ = resolve_state_dir(nonsense, Some(Path::new("\0")));
        let _ = resolve_state_dir(Path::new(""), None);
        let _ = resolve_state_dir(Path::new("//"), Some(Path::new("///")));
    }
}
