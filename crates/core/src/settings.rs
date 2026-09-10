//! settings.toml — user preferences and the recent-files list.
//!
//! The storage split is binding (docs/architecture.md): session.json holds
//! the WINDOW state (rect, monitor, scale, maximised, pinned, last path);
//! settings.toml holds the USER preferences and the recents. No field ever
//! moves between the two files.
//!
//! Read discipline — the three facts are three different answers, because
//! "a missing file" is NOT the same fact as "the factory default":
//! absent file -> Ok(None) (the caller decides; no default is invented
//! behind its back); present file -> Ok(Some(exactly what was written));
//! corrupt file -> Err(SettingsError::Corrupt) with the bytes on disk
//! PRESERVED for diagnosis — a failed read never rewrites, never
//! auto-corrects (D12). Write discipline is the D12/D23 atomic tail via
//! save::atomic_write (same-directory sibling temp, flush, fsync, rename,
//! leftovers swept; no tempfile dependency — D23).

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
    /// The ANSI code page, as RECORDED: None means nobody ever chose one
    /// (a missing file, or a file without the key — TOML cannot say null, so
    /// the absent key IS the null), and the answer is "ask the host" via
    /// resolved_codepage. It is never a hard-coded 1252: a default value
    /// nobody chose would silently override a machine whose ACP is 1251 or
    /// 1254, and D27 ("never guess a codepage") would be unimplementable.
    /// D27 unchanged: CP1252 is the only code page core can write back —
    /// anything else opens read-only. Serde: `default` reads a missing key
    /// as None; `skip_serializing_if` never writes the key for None, since
    /// TOML cannot say null — the absent key IS the null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codepage: Option<u16>,
    /// The recent-files list, capped by recent::MAX_RECENTS. Defaults to
    /// empty so a hand-trimmed settings.toml without the table still reads.
    #[serde(default)]
    pub recents: Vec<RecentEntry>,
}

impl Default for Settings {
    /// The FACTORY settings: no code page decision is made here — the host
    /// supplies the ANSI code page and resolved_codepage composes the two.
    /// A caller that genuinely wants "assume 1252 because there is no host"
    /// must say so at its call site, in words.
    fn default() -> Self {
        Settings {
            autosave_enabled: true,
            codepage: None,
            recents: Vec::new(),
        }
    }
}

impl Settings {
    /// The ONLY way a codepage reaches encoding::detect on a shipped path.
    /// Precedence: explicit user setting wins; else the host's ANSI code
    /// page; else None — and None means ANSI files are REFUSED, never
    /// guessed (D27). Pure and total: no I/O, no GetACP (core has no OS
    /// access; the host value arrives as a parameter).
    pub fn resolved_codepage(&self, host_acp: Option<u16>) -> Option<u16> {
        self.codepage.or(host_acp)
    }
}

/// Settings write failures. The save half keeps save::SaveError intact so
/// the UI can render the same user-actionable reason autosave would.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("settings could not be serialised: {0}")]
    Serialise(String),
    /// The file exists but is not valid TOML. With "absent" no longer
    /// available as a silent fallback, corrupt is a typed error the caller
    /// renders; the bytes are never touched (D12: diagnosis beats tidiness).
    #[error("settings file corrupt: {0}")]
    Corrupt(String),
    #[error(transparent)]
    Save(#[from] crate::save::SaveError),
}

/// Reads settings from a StateDir. The round trip, stated as a contract:
/// absent file -> Ok(None) -> the caller decides (there is no default to
/// hide behind); present file -> Ok(Some(exactly what was written)); corrupt
/// file -> Err(Corrupt) with the bytes untouched (D12: never silently
/// correct). Unreadable-but-not-corrupt (an ACL, a locked file, a directory
/// in the way) is Ok(None) too — no file CONTENT was seen, so no fact about
/// the settings exists.
pub fn read_settings(dir: &StateDir) -> Result<Option<Settings>, SettingsError> {
    let path = dir.0.join(SETTINGS_FILE_NAME);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return Ok(None),
    };
    toml::from_str(&text)
        .map(Some)
        .map_err(|e| SettingsError::Corrupt(e.to_string()))
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
        let read = read_settings(&state)?;
        assert_eq!(read, Some(s), "the exact written settings come back");
        Ok(())
    }

    /// The ABSENT fact: no settings.toml at all is Ok(None), NOT
    /// Some(Settings::default()) — "nobody ever ran the app" must stay
    /// distinguishable from "the user chose the factory settings", and an
    /// absent file must not smuggle in a guessed codepage (D27).
    #[test]
    fn missing_settings_file_is_ok_none_not_a_default() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
        let state = crate::paths::StateDir(dir.path().to_path_buf());
        let read = read_settings(&state);
        assert!(
            matches!(&read, Ok(None)),
            "an absent file must read as Ok(None), got {read:?}"
        );
    }

    /// D12: a corrupt file is a typed error, never silently corrected and
    /// never replaced by a default — the bytes stay on disk for diagnosis.
    #[test]
    fn corrupt_settings_file_is_an_error_and_is_preserved() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let state = crate::paths::StateDir(dir.path().to_path_buf());
        let garbage = b"this is not [ valid toml ===";
        std::fs::write(state.0.join(SETTINGS_FILE_NAME), garbage)?;
        let Err(e) = read_settings(&state) else {
            panic!("a corrupt settings file must be Err, not Ok(_)");
        };
        assert!(
            matches!(e, SettingsError::Corrupt(_)),
            "corrupt must be SettingsError::Corrupt, got {e:?}"
        );
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
            s.codepage, None,
            "the absent key is the null: nobody ever chose a codepage"
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
        // A machine whose ANSI is not 1252: the stored value is the user's
        // recorded choice, the one resolved_codepage hands to encoding::detect.
        let s = Settings {
            codepage: Some(932),
            ..Settings::default()
        };
        write_settings(&state, &s)?;
        let read = read_settings(&state)?;
        assert_eq!(read.as_ref(), Some(&s));
        assert_eq!(read.and_then(|s| s.codepage), Some(932));
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
        assert_eq!(read_settings(&state)?, Some(s));
        Ok(())
    }

    /// The PRESENT fact, distinguished from the absent fact above: a file
    /// that EXISTS but carries no codepage key (TOML cannot say null, so
    /// the absent key IS the null) is Ok(Some(...)) with codepage: None —
    /// a readable decision, not a missing file. Same values the factory
    /// default would have had; a different fact.
    #[test]
    fn a_file_without_a_codepage_key_is_some_with_none_not_absent()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let state = crate::paths::StateDir(dir.path().to_path_buf());
        // Written by hand, not via write_settings: the fact under test is
        // the key being absent, not what our serialiser happens to emit.
        std::fs::write(
            state.0.join(SETTINGS_FILE_NAME),
            "autosave_enabled = false\n",
        )?;
        let read = read_settings(&state)?;
        assert_eq!(
            read,
            Some(Settings {
                autosave_enabled: false,
                codepage: None,
                recents: Vec::new(),
            }),
            "a present file with no codepage key = Some with codepage None"
        );
        Ok(())
    }

    /// Unreadable-but-not-corrupt (here: the path is a directory) is
    /// Ok(None) too — no file CONTENT was seen, so no fact about the
    /// settings exists, and nothing is corrected or rewritten (D12).
    #[test]
    fn unreadable_settings_path_is_ok_none_no_content_no_fact()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let state = crate::paths::StateDir(dir.path().to_path_buf());
        std::fs::create_dir(state.0.join(SETTINGS_FILE_NAME))?;
        let read = read_settings(&state);
        assert!(
            matches!(&read, Ok(None)),
            "unreadable must read as absent, got {read:?}"
        );
        Ok(())
    }

    /// D33 mutation guard: the explicit setting is the user's CHOICE — the
    /// host's ACP must never overwrite it. The precedence permutation
    /// "host wins over explicit" is the mutation this test fails on.
    #[test]
    fn resolved_codepage_explicit_setting_beats_the_host_acp() {
        let s = Settings {
            codepage: Some(1252),
            ..Settings::default()
        };
        assert_eq!(
            s.resolved_codepage(Some(932)),
            Some(1252),
            "the user's explicit codepage wins over the host's ACP"
        );
    }

    #[test]
    fn resolved_codepage_falls_back_to_the_host_acp() {
        let s = Settings {
            codepage: None,
            ..Settings::default()
        };
        assert_eq!(
            s.resolved_codepage(Some(932)),
            Some(932),
            "with no explicit choice, the host's ACP is the answer"
        );
    }

    /// D27: None out of both inputs means ANSI files are REFUSED, never
    /// guessed — core has no OS access, so it has no codepage to offer.
    #[test]
    fn resolved_codepage_none_means_ansi_is_refused_never_guessed() {
        let s = Settings::default();
        assert_eq!(
            s.resolved_codepage(None),
            None,
            "no choice and no host: refuse, do not guess"
        );
    }
}
