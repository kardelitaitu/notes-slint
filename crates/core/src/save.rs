//! The atomic save engine (features §4.2): "Write to a sibling temp file,
//! fsync, then rename over the target. A crash mid-write leaves the previous
//! good file intact."
//!
//! D12/D23, as implemented here and in session.rs:
//! * the temp file is a SIBLING of the target — same directory, therefore
//!   same volume by construction, so the cross-drive rename hazard cannot
//!   occur;
//! * the temp is created exclusively, written, flushed, fsynced
//!   (sync_all), and only then renamed over the target;
//! * on ANY failure the temp is removed and the target is untouched;
//! * at the START of every save into a directory, stale temps of the same
//!   target file (a crash can skip the removal) are swept best-effort with
//!   errors ignored — a sweep failure never blocks a save, and only files
//!   matching "<target file name>.tmp-" are ever touched.
//!
//! The bytes come from encoding::encode — the caller's Detected decides the
//! encoding, the BOM, the line endings and the trailing newline; this engine
//! never normalises anything.
//!
//! Pure std: tempfile is DEV-ONLY in notes-core (D23 — as a normal dep it
//! pulls windows-sys into core's closure and fails the core-no-os arch rule),
//! so the tail below is hand-rolled, exactly like session.rs. session.rs
//! still carries its own copy (its fence was closed when this was written);
//! the temp naming matches ("<file>.tmp-..."), so this module's sweep also
//! cleans session.json leftovers. Folding session.rs onto this tail is a
//! later slice.

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use crate::encoding::{self, Detected};
use crate::session::FILE_NAME;

/// Save failures. Every variant is rendered to the user — an autosave
/// failure has no caller to return Err to (AGENTS.md), so this enum IS the
/// explanation. The Display strings are the shipped UI copy.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SaveError {
    #[error("the file is marked read-only — clear the read-only flag or use Save As")]
    ReadOnly,
    #[error("Windows denied access to the file — check its permissions")]
    PermissionDenied,
    #[error("the disk is full — free up space and try again")]
    DiskFull,
    #[error("the file is open in another program — close it there and try again")]
    Locked,
    #[error("the file or its folder no longer exists")]
    NotFound,
    #[error(
        "the text contains characters that cannot be stored in this file's encoding — use Save As to a .notes file"
    )]
    Unencodable,
    #[error("the path is not a usable file location")]
    InvalidPath,
    #[error("{0}")]
    Other(String),
}

/// The result of a successful save: where the bytes went, how many there
/// were, and which document revision produced them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveOutcome {
    pub path: PathBuf,
    pub bytes_written: usize,
    pub revision: u64,
}

/// Saves text to a file atomically, in the encoding the caller detected.
/// Atomic: never truncates the target before the new bytes are safely on
/// disk. The revision is stamped by the revision-aware form
/// (save_document_revision); the byte-level engine records 0.
pub fn save_document(
    path: &Path,
    text: &str,
    detected: Detected,
) -> Result<SaveOutcome, SaveError> {
    save_document_revision(path, text, detected, 0)
}

/// The general form of save_document: stamps the caller's document revision
/// into the SaveOutcome so the autosave slice can report what it wrote.
pub fn save_document_revision(
    path: &Path,
    text: &str,
    detected: Detected,
    revision: u64,
) -> Result<SaveOutcome, SaveError> {
    // Encode FIRST: an unencodable character must never touch the
    // filesystem, so the previous good file survives an encoding failure
    // byte-for-byte.
    let bytes = encoding::encode(text, detected).map_err(|_| SaveError::Unencodable)?;
    // Pre-flight: a read-only ATTRIBUTE is detectable before the write, and
    // is the common, easily-fixed cause. A denied ACL is NOT distinguishable
    // from the attribute at write time — both surface as ERROR_ACCESS_DENIED
    // — so the classifier maps that code to PermissionDenied (see
    // classify_io_error for the reasoning).
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.permissions().readonly() {
            return Err(SaveError::ReadOnly);
        }
    }
    atomic_write(path, &bytes)?;
    Ok(SaveOutcome {
        path: path.to_path_buf(),
        bytes_written: bytes.len(),
        revision,
    })
}

/// Atomically writes the serialised session bytes to session.json inside a
/// StateDir. The tail is shared with save_document; session.rs keeps its own
/// copy until its fence reopens (see the module docs).
pub fn save_session_bytes(dir: &Path, bytes: &[u8]) -> Result<(), SaveError> {
    atomic_write(&dir.join(FILE_NAME), bytes)
}

/// The D12/D23 tail: sweep stale temps of this target, create an exclusive
/// sibling temp, write, flush, fsync, rename over the target; on any failure
/// remove the temp and classify the error. Never truncates the target.
pub(crate) fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), SaveError> {
    if target.file_name().is_none() {
        return Err(SaveError::InvalidPath);
    }
    let parent = normalize_parent(target);
    if let Some(prefix) = temp_prefix(target) {
        sweep_stale_temps(&parent, &prefix);
    }
    let (temp_path, mut file) = create_sibling_temp(target)?;
    if let Err(e) = write_flush_sync(&mut file, bytes) {
        drop(file);
        // Best effort: a failed write must not leave litter behind.
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }
    drop(file);
    if let Err(e) = std::fs::rename(&temp_path, target) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(classify_io_error(&e));
    }
    Ok(())
}

/// "note.notes" -> "note.notes.tmp-": the prefix that identifies THIS
/// target's temp files (and nothing else's).
fn temp_prefix(target: &Path) -> Option<String> {
    let name = target.file_name()?.to_string_lossy().into_owned();
    Some(name + ".tmp-")
}

/// A relative target like "x.notes" has an empty parent component; saving
/// next to the current directory is legitimate, so normalise to ".".
fn normalize_parent(target: &Path) -> PathBuf {
    match target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// D12: best-effort sweep of crashed saves' temps for THIS target. Errors
/// are ignored — a sweep failure must never block a save — and only files
/// matching the prefix are touched, never the user's other files.
fn sweep_stale_temps(dir: &Path, prefix: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(prefix) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The sibling temp: exclusive create_new in the target's directory, with a
/// pid + nanos + attempt name so two saves never collide on a name.
fn create_sibling_temp(target: &Path) -> Result<(PathBuf, std::fs::File), SaveError> {
    let parent = normalize_parent(target);
    let file_name = target
        .file_name()
        .ok_or(SaveError::InvalidPath)?
        .to_string_lossy()
        .into_owned();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    for attempt in 0..3u32 {
        let candidate = parent.join(format!(
            "{file_name}.tmp-{}-{nanos}-{attempt}",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(classify_io_error(&e)),
        }
    }
    Err(SaveError::Other(
        "could not create a uniquely named sibling temp file".to_owned(),
    ))
}

fn write_flush_sync(file: &mut std::fs::File, bytes: &[u8]) -> Result<(), SaveError> {
    file.write_all(bytes).map_err(|e| classify_io_error(&e))?;
    file.flush().map_err(|e| classify_io_error(&e))?;
    file.sync_all().map_err(|e| classify_io_error(&e))?;
    Ok(())
}

/// Maps an std::io::Error to the reason a USER can act on. This is the whole
/// value of the enum: autosave failures arrive as events, not results, so
/// the classification must name the fix.
///
/// The Win32 codes below are the ones that actually happen during a save.
/// They are written as named consts with their stable numeric values so core
/// needs no windows crate (core-no-os). The table is gated to Windows: on
/// other OSes the same numbers are errno values with different meanings
/// (32 is EPIPE on Linux, not a sharing violation), and the io::ErrorKind
/// fallback below handles those hosts.
pub fn classify_io_error(err: &std::io::Error) -> SaveError {
    if cfg!(windows) {
        if let Some(code) = err.raw_os_error() {
            const ERROR_INVALID_FUNCTION: i32 = 1;
            const ERROR_FILE_NOT_FOUND: i32 = 2;
            const ERROR_PATH_NOT_FOUND: i32 = 3;
            const ERROR_ACCESS_DENIED: i32 = 5;
            const ERROR_SHARING_VIOLATION: i32 = 32;
            const ERROR_LOCK_VIOLATION: i32 = 33;
            const ERROR_HANDLE_DISK_FULL: i32 = 39;
            const ERROR_BAD_NET_PATH: i32 = 53;
            const ERROR_DISK_FULL: i32 = 112;
            const ERROR_FILE_CORRUPT: i32 = 1392;
            match code {
                ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => {
                    return SaveError::NotFound;
                }
                // DELIBERATE CHOICE, per the review brief: ACCESS_DENIED maps
                // to PermissionDenied, not ReadOnly. A read-only ATTRIBUTE
                // is caught by the pre-flight in save_document (std exposes
                // it as permissions().readonly(), which is exactly
                // FILE_ATTRIBUTE_READONLY on Windows); a denied ACL is not
                // distinguishable from the attribute by code alone, and both
                // reach the classifier as 5. Labelling 5 as ReadOnly would
                // tell a user to clear an attribute that is not set; the
                // pre-flight keeps genuine attribute cases on ReadOnly.
                ERROR_ACCESS_DENIED => return SaveError::PermissionDenied,
                // Another program holds the file open without write sharing
                // — the single most common real autosave failure on Windows.
                ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION => return SaveError::Locked,
                // The ERROR_DISK_FULL family (including what some headers
                // call not-enough-space); io::ErrorKind::StorageFull covers
                // the same condition on other platforms.
                ERROR_HANDLE_DISK_FULL | ERROR_DISK_FULL => return SaveError::DiskFull,
                // ERROR_INVALID_FUNCTION, ERROR_BAD_NET_PATH and
                // ERROR_FILE_CORRUPT, plus anything else unanticipated: no
                // actionable fix to name, so keep the lossless OS message.
                ERROR_INVALID_FUNCTION | ERROR_BAD_NET_PATH | ERROR_FILE_CORRUPT => {
                    return SaveError::Other(err.to_string());
                }
                _ => return SaveError::Other(err.to_string()),
            }
        }
    }
    // Cross-platform fallback: synthetic errors that carry no OS code, and
    // every non-Windows host.
    match err.kind() {
        ErrorKind::NotFound => SaveError::NotFound,
        ErrorKind::PermissionDenied => SaveError::PermissionDenied,
        ErrorKind::StorageFull => SaveError::DiskFull,
        ErrorKind::InvalidInput => SaveError::InvalidPath,
        _ => SaveError::Other(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::{LineEnding, TextEncoding, detect, encode};

    fn det(encoding: TextEncoding, bom: bool) -> Detected {
        Detected {
            encoding,
            line_ending: LineEnding::Lf,
            trailing_newline: false,
            bom_present: bom,
        }
    }

    fn dir_names(dir: &Path) -> Result<Vec<String>, std::io::Error> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            names.push(entry?.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    #[test]
    fn happy_path_writes_the_reported_encoding_byte_exact() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("note.notes");
        // The previous good file: CRLF UTF-16LE with a BOM — a deliberately
        // non-default encoding that must come back untouched.
        let d = det(TextEncoding::Utf16Le, true);
        std::fs::write(&target, encode("old\r\n", d)?)?;
        let outcome = save_document(&target, "new text\r\n", d)?;
        assert_eq!(outcome.path, target);
        let expected = encode("new text\r\n", d)?;
        assert_eq!(outcome.bytes_written, expected.len());
        let on_disk = std::fs::read(&target)?;
        // Byte-exact, not "decodes to something similar": BOM, units, CRLF.
        assert_eq!(on_disk, expected);
        assert_eq!(detect(&on_disk, None).line_ending, LineEnding::CrLf);
        assert!(detect(&on_disk, None).trailing_newline);
        // No temp litter after a successful save.
        assert_eq!(dir_names(dir.path())?, vec!["note.notes".to_owned()]);
        Ok(())
    }

    #[test]
    fn encode_failure_leaves_the_previous_good_file_intact()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("note.notes");
        let original = vec![0xDE, 0xAD, 0xBE, 0xEF];
        std::fs::write(&target, &original)?;
        let Err(e) = save_document(&target, "\u{1F600}", det(TextEncoding::Ansi(1252), false))
        else {
            panic!("an emoji cannot be encoded into 1252");
        };
        assert_eq!(e, SaveError::Unencodable);
        // The crash-mid-write claim, minus the crash: the previous good
        // bytes are untouched and no temp file was left behind.
        assert_eq!(std::fs::read(&target)?, original);
        assert_eq!(dir_names(dir.path())?, vec!["note.notes".to_owned()]);
        Ok(())
    }

    #[test]
    fn missing_directory_is_not_found_with_no_partial_file()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let missing = dir.path().join("no").join("such").join("x.notes");
        let Err(e) = save_document(&missing, "hi", det(TextEncoding::Utf8, false)) else {
            panic!("saving into a missing directory must fail");
        };
        assert_eq!(e, SaveError::NotFound);
        // Nothing was created anywhere along the path.
        assert!(!dir.path().join("no").exists());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn win32_codes_classify_to_user_actionable_reasons() {
        let exact = [
            (2u16, SaveError::NotFound),
            (3u16, SaveError::NotFound),
            (5u16, SaveError::PermissionDenied),
            (32u16, SaveError::Locked),
            (33u16, SaveError::Locked),
            (39u16, SaveError::DiskFull),
            (112u16, SaveError::DiskFull),
        ];
        for (code, want) in exact {
            assert_eq!(
                classify_io_error(&std::io::Error::from_raw_os_error(i32::from(code))),
                want,
                "win32 code {code}"
            );
        }
        // The Other family keeps the lossless OS message.
        for code in [1u16, 53u16, 1392u16] {
            assert!(
                matches!(
                    classify_io_error(&std::io::Error::from_raw_os_error(i32::from(code))),
                    SaveError::Other(_)
                ),
                "win32 code {code} must map to Other with the OS message"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn readonly_attribute_classifies_read_only_and_spares_the_file()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("note.notes");
        std::fs::write(&target, b"keep")?;
        // std's set_readonly maps to FILE_ATTRIBUTE_READONLY on Windows.
        let mut perms = std::fs::metadata(&target)?.permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&target, perms)?;
        let Err(e) = save_document(&target, "nope", det(TextEncoding::Utf8, false)) else {
            panic!("a read-only target must refuse the save");
        };
        assert_eq!(e, SaveError::ReadOnly, "the attribute case, named exactly");
        assert_eq!(std::fs::read(&target)?, b"keep", "file untouched");
        Ok(())
    }

    #[test]
    fn kind_only_errors_classify_cross_platform() {
        let cases = [
            (ErrorKind::NotFound, SaveError::NotFound),
            (ErrorKind::PermissionDenied, SaveError::PermissionDenied),
            (ErrorKind::StorageFull, SaveError::DiskFull),
            (ErrorKind::InvalidInput, SaveError::InvalidPath),
        ];
        for (kind, want) in cases {
            assert_eq!(classify_io_error(&std::io::Error::from(kind)), want);
        }
        assert!(matches!(
            classify_io_error(&std::io::Error::from(ErrorKind::TimedOut)),
            SaveError::Other(_)
        ));
    }

    #[test]
    fn stale_temps_are_swept_and_other_files_are_not() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("note.notes");
        std::fs::write(&target, b"old")?;
        // A temp left behind by a crash of a previous save of THIS file.
        std::fs::write(dir.path().join("note.notes.tmp-999-1"), b"stale")?;
        // The user's own file: a sweep that deletes this is a disaster.
        std::fs::write(dir.path().join("precious.txt"), b"user data")?;
        save_document(&target, "new", det(TextEncoding::Utf8, false))?;
        let names = dir_names(dir.path())?;
        assert!(
            !names.iter().any(|n| n.starts_with("note.notes.tmp-")),
            "stale temp must be swept: {names:?}"
        );
        assert!(
            names.contains(&"precious.txt".to_owned()),
            "other files untouched"
        );
        assert!(names.contains(&"note.notes".to_owned()));
        Ok(())
    }

    #[test]
    fn session_bytes_save_is_atomic_and_replaces_exactly() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        save_session_bytes(dir.path(), b"v1")?;
        assert_eq!(std::fs::read(dir.path().join(FILE_NAME))?, b"v1");
        save_session_bytes(dir.path(), b"v2-longer")?;
        assert_eq!(std::fs::read(dir.path().join(FILE_NAME))?, b"v2-longer");
        assert_eq!(dir_names(dir.path())?, vec![FILE_NAME.to_owned()]);
        Ok(())
    }

    #[test]
    fn outcome_stamps_the_callers_revision() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("note.notes");
        let outcome = save_document_revision(&target, "x", det(TextEncoding::Utf8, false), 7)?;
        assert_eq!(outcome.revision, 7);
        assert_eq!(outcome.path, target);
        Ok(())
    }
}
