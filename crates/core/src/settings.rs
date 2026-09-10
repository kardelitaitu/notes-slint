//! settings.toml — user preferences and the recent-files list.
//!
//! The storage split is binding (docs/architecture.md): session.json holds
//! the WINDOW state (rect, monitor, scale, maximised, pinned, last path);
//! settings.toml holds the USER preferences and the recents. No field ever
//! moves between the two files.
//!
//! Read discipline is the same as session.rs: a missing or corrupt file
//! yields Default and the bytes on disk are PRESERVED for diagnosis — a
//! failed read never rewrites, never auto-corrects. Write discipline is the
//! D12/D23 atomic tail via save::atomic_write (same-directory sibling temp,
//! flush, fsync, rename, leftovers swept; no tempfile dependency — D23).

use crate::paths::StateDir;
use crate::recent::RecentEntry;

/// File name of the settings inside the StateDir.
pub const SETTINGS_FILE_NAME: &str = "settings.toml";

/// The persisted user preferences. The recents live here (NOT in
/// session.json) per the storage split; RecentEntry is the MRU vocabulary.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// The product premise is that the app never asks you to save: autosave
    /// starts ON.
    pub autosave_enabled: bool,
    /// The ANSI default code page the bridge supplies from GetACP() and
    /// passes to encoding::detect. Default = Some(1252) is the
    /// Windows-English default and NOT a guess: it is the value the caller
    /// is expected to overwrite with the machine's real code page. D27:
    /// CP1252 is the only code page core can write back — anything else
    /// opens read-only, so detect(bytes, None) (which cannot know the code
    /// page) is never the shipped path.
    #[serde(default = "default_codepage")]
    pub codepage: Option<u16>,
    /// The recent-files list, capped by recent::MAX_RECENTS. Defaults to
    /// empty so a hand-trimmed settings.toml without the table still reads.
    #[serde(default)]
    pub recents: Vec<RecentEntry>,
}

/// The documented default code page: Windows-English ANSI 1252, expected
/// to be overwritten by the bridge's GetACP() value.
fn default_codepage() -> Option<u16> {
    Some(1252)
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            autosave_enabled: true,
            codepage: default_codepage(),
            recents: Vec::new(),
        }
    }
}

/// Settings write failures. The save half keeps save::SaveError intact so
/// the UI can render the same user-actionable reason autosave would.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("settings could not be serialised: {0}")]
    Serialise(String),
    #[error(transparent)]
    Save(#[from] crate::save::SaveError),
}

/// Reads settings from a StateDir. Never fails: a missing or corrupt
/// settings.toml yields Default. A corrupt file's bytes are left exactly as
/// they were — diagnosis beats tidiness.
pub fn read_settings(dir: &StateDir) -> Settings {
    let path = dir.0.join(SETTINGS_FILE_NAME);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Settings::default();
    };
    toml::from_str(&text).unwrap_or_default()
}

/// Writes settings.toml atomically: sibling temp in the StateDir, flush,
/// fsync, rename, leftovers swept (save::atomic_write). The exact rendered
/// text is pinned by a test so a serde/toml format change is a failure, not
/// a silent file-format migration.
pub fn write_settings(dir: &StateDir, s: &Settings) -> Result<(), SettingsError> {
    let rendered =
        toml::to_string_pretty(s).map_err(|e| SettingsError::Serialise(e.to_string()))?;
    crate::save::atomic_write(&dir.0.join(SETTINGS_FILE_NAME), rendered.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn canonical() -> Settings {
        Settings {
            autosave_enabled: false,
            codepage: Some(1252),
            recents: vec![RecentEntry {
                path: PathBuf::from("C:\\Notes\\idea.notes"),
                display: "idea.notes".to_owned(),
                exists: true,
            }],
        }
    }

    /// TRAP GUARD: the exact rendered TOML for one canonical Settings. A
    /// serde attribute change or toml upgrade that reorders keys or tables
    /// must be a test failure, never a silent file-format migration.
    #[test]
    fn canonical_settings_render_exact_toml() {
        let rendered = toml::to_string_pretty(&canonical())
            .unwrap_or_else(|e| panic!("settings must serialise: {e}"));
        assert_eq!(
            rendered,
            "autosave_enabled = false\ncodepage = 1252\n\n[[recents]]\npath = 'C:\\Notes\\idea.notes'\ndisplay = \"idea.notes\"\nexists = true\n"
        );
    }

    #[test]
    fn write_then_read_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let state = crate::paths::StateDir(dir.path().to_path_buf());
        let s = canonical();
        write_settings(&state, &s)?;
        assert_eq!(read_settings(&state), s);
        Ok(())
    }

    #[test]
    fn missing_settings_file_yields_default() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
        let state = crate::paths::StateDir(dir.path().to_path_buf());
        assert_eq!(read_settings(&state), Settings::default());
        assert!(
            read_settings(&state).autosave_enabled,
            "the premise: autosave on"
        );
    }

    #[test]
    fn corrupt_settings_file_yields_default_and_is_preserved()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let state = crate::paths::StateDir(dir.path().to_path_buf());
        let garbage = b"this is not [ valid toml ===";
        std::fs::write(state.0.join(SETTINGS_FILE_NAME), garbage)?;
        assert_eq!(read_settings(&state), Settings::default());
        // A failed read never rewrites: the corrupt bytes stay for diagnosis.
        assert_eq!(std::fs::read(state.0.join(SETTINGS_FILE_NAME))?, garbage);
        Ok(())
    }

    /// M3: the read-only pre-flight lives in save::atomic_write, so a
    /// read-only settings.toml reports ReadOnly (not PermissionDenied), and
    /// the classified reason crosses the hop without string round-tripping.
    #[test]
    fn readonly_settings_file_reports_read_only() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let state = crate::paths::StateDir(dir.path().to_path_buf());
        let target = state.0.join(SETTINGS_FILE_NAME);
        std::fs::write(&target, b"keep")?;
        let mut perms = std::fs::metadata(&target)?.permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&target, perms)?;
        let Err(e) = write_settings(&state, &Settings::default()) else {
            panic!("a read-only settings file must refuse the write");
        };
        assert!(
            matches!(e, SettingsError::Save(crate::save::SaveError::ReadOnly)),
            "got {e:?}"
        );
        assert_eq!(std::fs::read(&target)?, b"keep");
        Ok(())
    }
    #[test]
    fn toml_without_a_recents_table_still_reads() {
        let s: Settings = toml::from_str("autosave_enabled = false\n")
            .unwrap_or_else(|e| panic!("minimal toml must parse: {e}"));
        assert!(!s.autosave_enabled);
        assert!(
            s.recents.is_empty(),
            "serde default covers the missing table"
        );
        assert_eq!(
            s.codepage,
            Some(1252),
            "a toml without the key defaults to the documented code page"
        );
    }

    #[test]
    fn writing_leaves_no_temp_litter() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let state = crate::paths::StateDir(dir.path().to_path_buf());
        write_settings(&state, &Settings::default())?;
        let mut names = Vec::new();
        for entry in std::fs::read_dir(
            state
                .0
                .join(SETTINGS_FILE_NAME)
                .parent()
                .unwrap_or(state.0.as_path()),
        )? {
            names.push(entry?.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        assert_eq!(names, vec![SETTINGS_FILE_NAME.to_owned()]);
        Ok(())
    }

    #[test]
    fn codepage_is_persisted_and_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let state = crate::paths::StateDir(dir.path().to_path_buf());
        // A machine whose ANSI is not 1252: the stored value is what the
        // bridge will pass to encoding::detect.
        let s = Settings {
            codepage: Some(932),
            ..Settings::default()
        };
        write_settings(&state, &s)?;
        assert_eq!(read_settings(&state), s);
        assert_eq!(read_settings(&state).codepage, Some(932));
        Ok(())
    }

    #[test]
    fn recents_survive_the_toml_round_trip_with_paths_intact()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let state = crate::paths::StateDir(dir.path().to_path_buf());
        let s = Settings {
            autosave_enabled: true,
            codepage: Some(1252),
            recents: vec![
                RecentEntry {
                    path: PathBuf::from("C:\\a dir\\with spaces\\n.notes"),
                    display: "n.notes".to_owned(),
                    exists: true,
                },
                RecentEntry {
                    path: PathBuf::from("\\\\server\\share\\u.notes"),
                    display: "u.notes".to_owned(),
                    exists: false,
                },
            ],
        };
        write_settings(&state, &s)?;
        assert_eq!(read_settings(&state), s);
        Ok(())
    }
}
