//! Where the app keeps its state, resolved without touching the filesystem
//! or the environment.

use std::path::{Path, PathBuf};

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
///   lands on this branch in either of two ways:
///   1. the environment has no per-user roaming profile (appdata is None),
///      so there is nowhere else to put state;
///   2. the caller probed <exe_dir>/data with a single Path::exists() and
///      found the portable marker directory. That probe is deliberately the
///      caller's job — this function must stay pure — and the caller reports
///      the finding by passing appdata = None.
/// * INSTALLED — state lives in the per-user roaming profile:
///   <appdata>/notes-gpui (on Windows %APPDATA%\notes-gpui).
///
/// The caller-side decision, for reference:
///
/// ```no_run
/// # let exe_dir = std::path::Path::new("C:/apps/notes");
/// # let appdata = Some(std::path::Path::new("C:/Users/u/AppData/Roaming"));
/// let state = if exe_dir.join("data").exists() || appdata.is_none() {
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
        // Portable: no roaming profile, or the caller found the portable
        // "data" marker next to the exe (see the probe in the doc comment).
        None => StateDir(exe_dir.join("data")),
        // Installed: per-user roaming profile, namespaced for the app.
        Some(appdata) => StateDir(appdata.join("notes-gpui")),
    }
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
    }

    /// Pure determinism: identical inputs give identical outputs, every call.
    #[test]
    fn state_dir_resolution_is_deterministic() {
        let cases = [
            (Path::new("C:/apps/notes"), None),
            (
                Path::new("C:/Program Files/Notes"),
                Some(Path::new("C:/Users/u/AppData/Roaming")),
            ),
            (Path::new(""), Some(Path::new(""))),
        ];
        for (exe_dir, appdata) in cases {
            assert_eq!(
                resolve_state_dir(exe_dir, appdata),
                resolve_state_dir(exe_dir, appdata),
                "nondeterministic for {exe_dir:?} / {appdata:?}"
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
