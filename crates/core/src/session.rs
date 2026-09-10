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
    /// Something that is NOT a directory sits where the state directory
    /// belongs — a plain file, which the fresh-install probe really found
    /// on a real machine. The error names the path.
    #[error("the state path is not a directory: {0}")]
    NotADirectory(PathBuf),
    /// A symlink, junction or other reparse point sits on the state path:
    /// REFUSED by the same B2 predicate the save engine refuses its rename
    /// with — replacing a link orphans the real directory behind it.
    #[error("the state path is a link: {0} — the app does not create through or replace links")]
    Symlink(PathBuf),
    /// The directory exists but cannot be written (an inherited read-only
    /// ACL profile, measured on real machines). Detected by the startup
    /// probe — not at shutdown, when the first save failure is invisible.
    #[error("the state directory is not writable: {0}")]
    NotWritable(PathBuf),
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

/// Creates — or proves usable — the state directory the app persists into.
/// CORE owns this rule: the port had grown its own copy, which duplicated
/// the B2 link refusal and re-derived "is this a directory"; std::fs on
/// this very directory is core's everyday vocabulary already (read_session,
/// atomic_write, write_settings), so the honest home is here and the
/// port's block becomes routing.
///
/// The answers, in order, each with its own test:
/// * absent -> create_dir_all (a missing grandparent is not a surprise);
/// * present directory -> Ok, idempotent, nothing clobbered;
/// * a non-directory on the path -> NotADirectory, naming the path;
/// * a symlink/junction/reparse point -> refused by the SAME predicate the
///   save engine refuses its rename with (one definition, pinned by test);
/// * present but not writable -> NotWritable, by a create-and-delete probe
///   with the same semantics the port used (a uniquely named file), so an
///   ACL failure surfaces at startup. Cost: one create + one delete, the
///   same order as the port's measured 241µs — noise against the
///   cold-start budget, and it buys a failure the user can be told about.
pub fn ensure_state_dir(dir: &Path) -> Result<(), SessionError> {
    match std::fs::symlink_metadata(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(dir).map_err(SessionError::Io)?;
        }
        Err(e) => return Err(SessionError::Io(e)),
        Ok(meta) => judge_state_dir_metadata(dir, &meta)?,
    }
    probe_writable(dir)
}

/// The PURE half of the decision, separable from the creating: given the
/// symlink metadata of a path, is it a usable state directory? No creating,
/// no probing — the verdict falls out of the metadata alone, so it tests
/// without writing anything. The link check is THE shared B2 predicate.
fn judge_state_dir_metadata(path: &Path, meta: &std::fs::Metadata) -> Result<(), SessionError> {
    if crate::path_policy::metadata_is_reparse(meta) {
        return Err(SessionError::Symlink(path.to_path_buf()));
    }
    if !meta.is_dir() {
        return Err(SessionError::NotADirectory(path.to_path_buf()));
    }
    Ok(())
}

/// The writability probe, SAME semantics the port used: create and delete a
/// uniquely named file inside the directory. The failure it detects — a
/// read-only ACL profile — would otherwise surface at shutdown, when nobody
/// can see it.
fn probe_writable(dir: &Path) -> Result<(), SessionError> {
    let probe = dir.join(format!(
        ".state-write-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    match std::fs::File::create(&probe) {
        Ok(f) => {
            drop(f);
            // Best effort: a probe we could create we can normally delete;
            // a leftover would be a hidden dotfile, never user data.
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(_) => Err(SessionError::NotWritable(dir.to_path_buf())),
    }
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

    #[test]
    fn absent_state_dir_is_created_even_with_a_missing_grandparent()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let state = dir.path().join("grand").join("state");
        ensure_state_dir(&state)?;
        assert!(
            state.is_dir(),
            "create_dir_all covers the missing grandparent"
        );
        // Idempotent: the second call is Ok and changes nothing.
        ensure_state_dir(&state)?;
        Ok(())
    }

    #[test]
    fn present_state_dir_is_ok_and_clobbers_nothing() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state)?;
        std::fs::write(state.join("keep.txt"), b"kept")?;
        ensure_state_dir(&state)?;
        assert_eq!(
            std::fs::read(state.join("keep.txt"))?,
            b"kept",
            "never clobbers anything"
        );
        Ok(())
    }

    /// D33: a plain file on the state path (the fresh-install probe found
    /// this for real) must be refused BY NAME. This test fails without the
    /// not-a-directory check: the file would fall through to the writability
    /// probe and come back as an unlabelled io error instead.
    #[test]
    fn a_file_on_the_state_path_is_refused_by_name() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let state = dir.path().join("state");
        std::fs::write(&state, b"a plain file")?;
        let Err(e) = ensure_state_dir(&state) else {
            panic!("a file where the state dir belongs must be refused");
        };
        assert!(
            matches!(&e, SessionError::NotADirectory(p) if p == &state),
            "the error must name the path, got {e:?}"
        );
        // And the file is untouched: never clobbered, never rewritten (D12).
        assert_eq!(std::fs::read(&state)?, b"a plain file");
        Ok(())
    }

    /// D33: a link on the state path must be refused by THE SAME predicate
    /// the save engine refuses its rename with (one definition — the
    /// include_str! pin in path_policy tests proves the sharing). A junction
    /// is used because creating one needs no privilege, so this proof runs
    /// on any Windows host (the save engine's own B2 test relies on this).
    #[test]
    #[cfg(windows)]
    fn symlinked_state_dir_is_refused_by_the_shared_predicate()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let real = dir.path().join("real-state");
        std::fs::create_dir_all(&real)?;
        let link = dir.path().join("state");
        // mklink /J (a directory junction) needs no privilege, unlike file
        // symlinks — the same mechanism the save engine's B2 proof uses.
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&real)
            .output()?;
        assert!(made.status.success(), "mklink /J failed: {:?}", made.stderr);
        assert!(
            std::fs::symlink_metadata(&link)?.file_type().is_symlink(),
            "std classifies a junction as a reparse point on this toolchain"
        );
        let Err(e) = ensure_state_dir(&link) else {
            panic!("a junction on the state path must be refused");
        };
        assert!(matches!(e, SessionError::Symlink(_)), "got {e:?}");
        Ok(())
    }

    /// A directory that exists but cannot be written (the read-only ACL
    /// profile measured on real machines: RX-only inheritance) is refused
    /// AT STARTUP by the writability probe. Windows-only: the deny is made
    /// with icacls (std has no ACL vocabulary — core-no-os) and removed
    /// again so the tempdir cleans up.
    #[test]
    #[cfg(windows)]
    fn unwritable_state_dir_is_refused_at_startup() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state)?;
        let denied = std::process::Command::new("icacls")
            .arg(&state)
            .args(["/deny", "Everyone:(OI)(CI)(W)"])
            .output()?;
        assert!(
            denied.status.success(),
            "icacls deny failed: {}",
            String::from_utf8_lossy(&denied.stderr)
        );
        let result = ensure_state_dir(&state);
        let _ = std::process::Command::new("icacls")
            .arg(&state)
            .args(["/remove:d", "Everyone"])
            .output();
        let Err(e) = result else {
            panic!("an unwritable state dir must be refused at startup");
        };
        assert!(matches!(e, SessionError::NotWritable(_)), "got {e:?}");
        Ok(())
    }

    /// The probe's cost, stated for the cold-start budget: one create and
    /// one delete, the same shape the port measured at 241µs. The assert is
    /// a generous regression gate (CI filesystem jitter); the eprintln is
    /// the honest number (visible with --nocapture).
    #[test]
    fn the_writability_probe_costs_startup_noise() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state)?;
        let runs = 100;
        let start = std::time::Instant::now();
        for _ in 0..runs {
            probe_writable(&state)?;
        }
        let per = start.elapsed() / runs;
        eprintln!("writability probe: {per:?} per call");
        assert!(per.as_millis() < 10, "probe regressed: {per:?}");
        Ok(())
    }

    /// The pure half is separable: the judge decides from metadata ALONE —
    /// no creating, no probing — so the verdict is checkable without any
    /// write hitting the disk.
    #[test]
    fn the_pure_judge_decides_from_metadata_alone() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let real = dir.path().join("d");
        std::fs::create_dir_all(&real)?;
        let file = dir.path().join("f");
        std::fs::write(&file, b"x")?;
        assert!(
            judge_state_dir_metadata(&real, &std::fs::symlink_metadata(&real)?).is_ok(),
            "a real directory is usable"
        );
        assert!(
            matches!(
                judge_state_dir_metadata(&file, &std::fs::symlink_metadata(&file)?),
                Err(SessionError::NotADirectory(_))
            ),
            "a plain file is not a directory"
        );
        Ok(())
    }
}
