//! Persisted session state: where the window was, how it was scaled, whether
//! it was maximised and pinned, and which document was open. Storage is
//! core's job; applying the state to a real window (restore) is the bridge's.
//!
//! The file is session.json inside the StateDir (see
//! notes_core::paths::resolve_state_dir). A corrupt or missing file is never
//! rewritten by a read — the bytes stay on disk for diagnosis — and startup
//! goes through read_session_or_default, which never fails.

use std::path::{Path, PathBuf};

use crate::geometry::Rect;

/// File name of the session state inside the StateDir.
pub const FILE_NAME: &str = "session.json";

/// The persisted window session.
///
/// Geometry is stored as a frame-pixel Rect (Win32 GetWindowRect space) —
/// see geometry::Rect for why client space is never persisted.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Session {
    /// Last window rect in FRAME pixels.
    pub rect: Rect,
    /// OS identifier of the monitor the window last lived on.
    pub monitor_id: u32,
    /// DPI scale factor of that monitor at save time (1.0, 1.25, 1.5, ...).
    pub scale_factor: f32,
    /// Whether the window was maximised (restored as maximised, not to rect).
    pub maximized: bool,
    /// D10: the pin bit lives ONLY here. A pinned: key in .notes frontmatter
    /// is preserved byte-for-byte by the round-trip rule but is NOT acted on
    /// — one home per bit.
    pub pinned: bool,
    /// The document that was open, if any.
    pub path: Option<PathBuf>,
}

impl Default for Session {
    /// The startup default: an 800x600 window at (120, 90), monitor 0,
    /// scale 1.0, neither maximised nor pinned, no document open.
    fn default() -> Self {
        Session {
            rect: Rect::new(120, 90, 800, 600),
            monitor_id: 0,
            scale_factor: 1.0,
            maximized: false,
            pinned: false,
            path: None,
        }
    }
}

/// Failures of the session file. The UI must be able to render the reason
/// (AGENTS.md), so read/write report typed errors and never panic.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// session.json does not exist (a first launch, not an error to show).
    #[error("session file missing")]
    Missing,
    /// session.json exists but could not be parsed. Its bytes are left
    /// untouched on disk for diagnosis.
    #[error("session file corrupt: {0}")]
    Corrupt(String),
    /// The shared atomic write tail failed. The classified SaveError is
    /// preserved as-is — no string round-trip through io::Error::other — so
    /// the UI renders the real reason (AGENTS.md: the enum IS the contract).
    #[error(transparent)]
    Save(#[from] crate::save::SaveError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Reads the session from a StateDir. Returns SessionError::Missing when
/// session.json does not exist and SessionError::Corrupt when it exists but
/// does not parse; in both cases the bytes on disk are never rewritten.
pub fn read_session(dir: &Path) -> Result<Session, SessionError> {
    let bytes = match std::fs::read(dir.join(FILE_NAME)) {
        Ok(bytes) => bytes,
        // A first launch has no session file; that is Missing, not Io.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(SessionError::Missing);
        }
        Err(e) => return Err(SessionError::Io(e)),
    };
    serde_json::from_slice(&bytes).map_err(|e| SessionError::Corrupt(e.to_string()))
}

/// Startup path: the default session whenever the file is missing or corrupt.
/// Startup never fails because of this file, and a failed read never
/// rewrites — the corrupt bytes stay on disk for diagnosis.
pub fn read_session_or_default(dir: &Path) -> Session {
    // Missing or corrupt: the default. The bytes on disk stay untouched.
    read_session(dir).unwrap_or_default()
}

/// Writes the session into a StateDir, ATOMICALLY (D12):
///
/// 1. a sibling temp file is created in the SAME directory as the target, so
///    the final rename can never cross a volume;
/// 2. the bytes are written, flushed, and fsynced;
/// 3. the temp is renamed over session.json (a same-volume rename is atomic);
/// 4. on any failure the temp is removed and the previous file — corrupt or
///    not — is left exactly as it was.
///
/// Two writes of the same Session produce byte-identical files: struct field
/// order is the JSON key order, and floats use shortest round-trip form.
pub fn write_session(dir: &Path, s: &Session) -> Result<(), SessionError> {
    // Serialise first: a Session that JSON cannot represent must never touch
    // the filesystem, so the previous file — corrupt or not — survives.
    let bytes = serde_json::to_vec_pretty(s).map_err(|e| SessionError::Corrupt(e.to_string()))?;
    // The shared D12/D23 tail (save::atomic_write): sibling temp in the same
    // directory, exclusive create, write, fsync, rename, leftovers swept at
    // the start of the next save. The SaveError crosses as itself — variant
    // fidelity is the contract the UI renders.
    crate::save::atomic_write(&dir.join(FILE_NAME), &bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn sample() -> Session {
        Session {
            rect: Rect::new(120, 90, 800, 600),
            monitor_id: 1,
            scale_factor: 1.5,
            maximized: false,
            pinned: true,
            path: Some(PathBuf::from("C:/notes/idea.notes")),
        }
    }

    /// All names inside a directory, sorted — used for the no-litter and
    /// preserved-garbage assertions. Test-only helper, fails via Result.
    fn dir_names(dir: &Path) -> Result<Vec<String>, std::io::Error> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            names.push(entry?.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    fn session_bytes(dir: &Path) -> Result<Vec<u8>, std::io::Error> {
        std::fs::read(dir.join(FILE_NAME))
    }

    #[test]
    fn write_then_read_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let s = sample();
        write_session(dir.path(), &s)?;
        assert_eq!(read_session(dir.path())?, s);
        Ok(())
    }

    #[test]
    fn two_writes_are_byte_identical() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let s = sample();
        write_session(dir.path(), &s)?;
        let first = session_bytes(dir.path())?;
        write_session(dir.path(), &s)?;
        let second = session_bytes(dir.path())?;
        assert_eq!(first, second, "stable field order => byte-identical file");
        Ok(())
    }

    #[test]
    fn missing_file_is_missing_error() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        assert!(matches!(
            read_session(dir.path()),
            Err(SessionError::Missing)
        ));
        Ok(())
    }

    #[test]
    fn garbage_bytes_are_corrupt() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join(FILE_NAME), b"this is not json at all")?;
        assert!(matches!(
            read_session(dir.path()),
            Err(SessionError::Corrupt(_))
        ));
        Ok(())
    }

    #[test]
    fn truncated_json_is_corrupt() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let bytes = serde_json::to_vec(&sample())?;
        std::fs::write(dir.path().join(FILE_NAME), &bytes[..bytes.len() / 2])?;
        assert!(matches!(
            read_session(dir.path()),
            Err(SessionError::Corrupt(_))
        ));
        Ok(())
    }

    #[test]
    fn read_default_preserves_garbage_bytes_on_disk() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let garbage = b"definitely not a session";
        std::fs::write(dir.path().join(FILE_NAME), garbage)?;
        assert_eq!(read_session_or_default(dir.path()), Session::default());
        // A failed read never rewrites: the corrupt bytes must still be there.
        assert_eq!(session_bytes(dir.path())?, garbage);
        assert_eq!(dir_names(dir.path())?, vec![FILE_NAME.to_owned()]);
        Ok(())
    }

    #[test]
    fn successful_write_leaves_no_temp_file() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        write_session(dir.path(), &sample())?;
        assert_eq!(dir_names(dir.path())?, vec![FILE_NAME.to_owned()]);
        Ok(())
    }

    #[test]
    fn write_into_missing_directory_is_err_not_panic() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let missing = dir.path().join("no").join("such").join("dir");
        assert!(write_session(&missing, &sample()).is_err());
        assert!(write_session(dir.path().join("never-created").as_path(), &sample()).is_err());
        Ok(())
    }

    #[test]
    fn maximized_and_rect_round_trip_together() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let s = Session {
            rect: Rect::new(-40, 300, 1280, 720),
            monitor_id: 7,
            scale_factor: 1.25,
            maximized: true,
            pinned: false,
            path: None,
        };
        write_session(dir.path(), &s)?;
        assert_eq!(read_session(dir.path())?, s);
        Ok(())
    }

    /// M3: a read-only session.json must report ReadOnly — the pre-flight
    /// lives in save::atomic_write now — and the SaveError crosses as itself
    /// (no string round-trip through io::Error::other).
    #[test]
    fn readonly_session_file_reports_read_only() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join(FILE_NAME);
        std::fs::write(&target, b"previous")?;
        let mut perms = std::fs::metadata(&target)?.permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&target, perms)?;
        let Err(e) = write_session(dir.path(), &sample()) else {
            panic!("a read-only session file must refuse the write");
        };
        assert!(
            matches!(e, SessionError::Save(crate::save::SaveError::ReadOnly)),
            "got {e:?}"
        );
        assert_eq!(std::fs::read(&target)?, b"previous");
        Ok(())
    }

    #[test]
    fn scale_factors_survive_json_exactly() -> Result<(), Box<dyn std::error::Error>> {
        // JSON floats are a classic drift source; the shipped values
        // (1.0 / 1.25 / 1.5) must survive a file round-trip bit-exactly.
        for factor in [1.0_f32, 1.25, 1.5] {
            let dir = tempfile::tempdir()?;
            let s = Session {
                scale_factor: factor,
                ..Session::default()
            };
            write_session(dir.path(), &s)?;
            let back = read_session(dir.path())?;
            assert_eq!(back.scale_factor, factor, "scale_factor {factor} drifted");
            assert_eq!(back, s);
            // And the textual form pins the exact shortest round-trip literal.
            let json = serde_json::to_string(&s)?;
            assert!(
                json.contains(&format!("\"scale_factor\":{factor}")),
                "unexpected JSON float form: {json}"
            );
        }
        Ok(())
    }

    #[test]
    fn default_session_matches_the_startup_contract() {
        let d = Session::default();
        assert_eq!(d.rect, Rect::new(120, 90, 800, 600));
        assert_eq!(d.monitor_id, 0);
        assert_eq!(d.scale_factor, 1.0);
        assert!(!d.maximized);
        assert!(!d.pinned);
        assert_eq!(d.path, None);
    }
}
