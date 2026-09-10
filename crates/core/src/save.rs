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
//!   errors ignored — a sweep failure never blocks a save. The sweep is
//!   deliberately PARANOID (B1): a prefix match is not a capability, so a
//!   file is only reclaimed when its name is EXACTLY our temp shape
//!   ("<file>.tmp-<pid>-<digits>-<digits>") AND it is older than
//!   SWEEP_MIN_AGE_SECS (M4). The user's "session.json.tmp-backup" — the
//!   natural move when D12 preserves a corrupt file for diagnosis — and any
//!   similarly-named file survive untouched.
//!
//! The bytes come from encoding::encode — the caller's Detected decides the
//! encoding, the BOM, the line endings and the trailing newline; this engine
//! never normalises anything.
//!
//! Pure std: tempfile is DEV-ONLY in notes-core (D23 — as a normal dep it
//! pulls windows-sys into core's closure and fails the core-no-os arch rule),
//! so the tail is hand-rolled on std. session.rs and settings.rs share this
//! ONE implementation via atomic_write — there is no second copy anywhere
//! for a future cleanup to break.
//!
//! LINK SAFETY (B2): the rename refuses to run over a symlink, junction or
//! any other reparse point (OneDrive and sync clients materialise files as
//! reparse points). Replacing a link destroys the link and orphans the real
//! file behind it while the app reports success — proven in review. A HARD
//! LINK is not a reparse point and is NOT refused, deliberately: the rename
//! replaces the target's directory entry with the new file, so the saved
//! path always carries the newest bytes while sibling hard links keep the
//! old inode's content.

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
    /// The path can never be written as named — no file-name component, or
    /// a name Windows will alter: Win32 strips trailing dots and spaces from
    /// the final component, so the bytes would land in a DIFFERENT file while
    /// the app reports Ok for a path that does not exist. The message says
    /// which and what to do.
    #[error("{0}")]
    InvalidPath(String),
    /// The target is a symlink, junction or other reparse point (OneDrive
    /// and other sync clients materialise files this way). The message
    /// names the link and, where it could be resolved, the real file behind
    /// it: replacing the link would orphan that file while the app reported
    /// success. The user should save to the real file instead.
    #[error("{0}")]
    ReparsePoint(String),
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
    // The read-only pre-flight and the link refusal live inside atomic_write
    // so the session and settings paths get them too.
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
    let _file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| SaveError::InvalidPath("the path has no file name component".to_owned()))?;
    // Windows path hostility, PROVEN blocker: Win32 strips trailing dots and
    // spaces from the final component, so saving "a.notes." lands in
    // "a.notes" while the app reports Ok for a path that does not exist —
    // and the caller keeps editing the name the app did not write. Refuse
    // with the reason; silently rewriting a user-visible path is the same
    // class of lie as reporting success for a different file.
    // REVERSAL (MAJOR-4): the \\?\ carve-out this check once had is gone.
    // It was true that Win32 strips nothing past the prefix — and wrong
    // about the product: the plain spelling of the saved name is refused by
    // the policy, unreadable, and invisible to the file dialog, so the app
    // reported success for a note the user can never open again. The strip
    // rule now applies to every component, extended prefix or not, through
    // the same shared predicate path_policy judges with.
    if crate::path_policy::any_component_is_stripped(target)
    {
        return Err(SaveError::InvalidPath(
            "a component of the path ends with '.' or a space, which Windows strips — the file written would not be the one named; rename the target"
                .to_owned(),
        ));
    }
    crate::path_policy::refuse_reparse_point(target)?;
    // M3: the read-only pre-flight lives HERE so every caller — documents,
    // sessions, settings — gets the same ReadOnly diagnosis. A denied ACL is
    // not distinguishable at this point (both it and the attribute surface
    // as code 5 once we write/rename); the classifier maps code 5 BY STEP,
    // and the Rename step reports Locked.
    if let Ok(meta) = std::fs::metadata(target) {
        if meta.permissions().readonly() {
            return Err(SaveError::ReadOnly);
        }
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
        return Err(classify_io_error(&e, IoStep::Rename));
    }
    // Unix prep (M7): fsync the parent directory so the rename itself is
    // durable. Windows has no directory-handle fsync; the rename there is
    // already durable, so this is cfg(unix)-only on purpose.
    #[cfg(unix)]
    {
        if let Ok(parent_dir) = std::fs::File::open(target.parent().unwrap_or(Path::new("."))) {
            let _ = parent_dir.sync_all();
        }
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

/// How old a temp must be before the sweep will touch it (M4). A real write
/// — create, write, fsync, rename — completes in far under a second even
/// with antivirus and a sync client in the loop; 60s is orders of magnitude
/// above that, while crash litter sits forever and is always older by the
/// next save. Two instances of a single-window app (portable exe + installed
/// exe, one user, one notes folder) are a real scenario: without this gate
/// instance B would sweep instance A's live temp mid-write and A's rename
/// would then fail for a file that exists.
const SWEEP_MIN_AGE_SECS: u64 = 60;

/// The retry budget of create_sibling_temp, shared with the sweep's shape
/// check so the two can never drift: a temp whose attempt field is not a
/// value this loop could have produced is, by definition, not ours.
const MAX_TEMP_ATTEMPTS: u32 = 3;

/// B1: the EXACT-SHAPE predicate for our own temp names. A prefix match is
/// not a capability — "session.json.tmp-backup" (a user's copy of a corrupt
/// file kept for diagnosis, exactly what D12 invites) would die to a plain
/// starts_with.
///
/// Round 2 (proven by test): even "three all-digit fields" is typeable by a
/// human — "session.json.tmp-2026-09-10" parses as pid/nanos/attempt and a
/// dated backup is by definition older than the age gate. The name must
/// therefore also end in our ".part" marker, and the attempt field must be
/// one the retry loop could actually have produced (0..MAX_TEMP_ATTEMPTS).
/// Matching is case-SENSITIVE on purpose: we only reclaim names we could
/// have created byte-exactly, never "X.notes.TMP-Mixed" (it cannot be ours).
fn is_our_temp(file_name: &str, prefix: &str) -> bool {
    let Some(rest) = file_name.strip_prefix(prefix) else {
        return false;
    };
    let Some(body) = rest.strip_suffix(".part") else {
        return false;
    };
    let parts: Vec<&str> = body.split('-').collect();
    parts.len() == 3
        && parts[0].parse::<u32>().is_ok()
        && parts[1].parse::<u128>().is_ok()
        && parts[2]
            .parse::<u32>()
            .is_ok_and(|attempt| attempt < MAX_TEMP_ATTEMPTS)
}

/// D12: best-effort sweep of crashed saves' temps for THIS target. A file is
/// touched only when it is exactly our temp shape (B1) AND older than
/// SWEEP_MIN_AGE_SECS (M4). Errors are ignored — a sweep failure must never
/// block a save — and a future clock counts as "not old yet".
fn sweep_stale_temps(dir: &Path, prefix: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        if !is_our_temp(&entry.file_name().to_string_lossy(), prefix) {
            continue;
        }
        let old_enough = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age.as_secs() >= SWEEP_MIN_AGE_SECS);
        if !old_enough {
            continue;
        }
        let _ = std::fs::remove_file(entry.path());
    }
}

/// The sibling temp: exclusive create_new in the target's directory, with a
/// pid + nanos + attempt name so two saves never collide on a name.
fn create_sibling_temp(target: &Path) -> Result<(PathBuf, std::fs::File), SaveError> {
    let parent = normalize_parent(target);
    let file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| {
            SaveError::InvalidPath("the target has no file name component".to_owned())
        })?;
    for attempt in 0..MAX_TEMP_ATTEMPTS {
        // Fresh jitter per attempt: nanos used to be computed once outside
        // the loop, overstating the entropy the comment claimed.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        // The ".part" marker is part of the SAFETY story, not decoration:
        // the sweep's shape check requires it, so no name a human plausibly
        // types (a dated backup, a word) can collide with our temp space.
        let candidate = parent.join(format!(
            "{file_name}.tmp-{}-{nanos}-{attempt}.part",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(classify_io_error(&e, IoStep::Create)),
        }
    }
    Err(SaveError::Other(
        "could not create a uniquely named sibling temp file".to_owned(),
    ))
}

fn write_flush_sync(file: &mut std::fs::File, bytes: &[u8]) -> Result<(), SaveError> {
    file.write_all(bytes)
        .map_err(|e| classify_io_error(&e, IoStep::Write))?;
    // No flush() here: File is unbuffered, so sync_all IS the durability
    // point — flush() was a no-op that implied buffering we do not have.
    file.sync_all()
        .map_err(|e| classify_io_error(&e, IoStep::Write))?;
    Ok(())
}

/// Where in the atomic write an io::Error came from. The same Win32 code
/// means different things at different steps: ACCESS_DENIED (5) during the
/// RENAME is the locked-file case (the target is open elsewhere without
/// share-delete — the single most common real autosave failure), while 5
/// during Create/Write is an ACL problem. The diagnosis IS the product.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoStep {
    Stat,
    Create,
    Write,
    Rename,
    Remove,
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
pub fn classify_io_error(err: &std::io::Error, step: IoStep) -> SaveError {
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
                // DELIBERATE CHOICE: ACCESS_DENIED is classified BY STEP.
                // A read-only ATTRIBUTE is caught by the pre-flight in
                // atomic_write (std exposes it as permissions().readonly(),
                // which is exactly FILE_ATTRIBUTE_READONLY on Windows), and
                // that path owns ReadOnly. Of the code-5 errors reaching this
                // classifier, the RENAME step is the locked-file case:
                // renaming over a target another program holds open without
                // share-delete reports ACCESS_DENIED, not a sharing
                // violation — proven in review — so Rename+5 maps to Locked
                // ("close it in the other program"), the copy the UI could
                // never show before. Create/Write+5 stays PermissionDenied
                // (a denied ACL on the folder or file).
                ERROR_ACCESS_DENIED if matches!(step, IoStep::Rename) => {
                    return SaveError::Locked;
                }
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
        ErrorKind::InvalidInput => SaveError::InvalidPath(err.to_string()),
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
            (2u16, IoStep::Write, SaveError::NotFound),
            (3u16, IoStep::Write, SaveError::NotFound),
            (5u16, IoStep::Write, SaveError::PermissionDenied),
            (32u16, IoStep::Rename, SaveError::Locked),
            (33u16, IoStep::Rename, SaveError::Locked),
            (39u16, IoStep::Write, SaveError::DiskFull),
            (112u16, IoStep::Write, SaveError::DiskFull),
        ];
        for (code, step, want) in exact {
            assert_eq!(
                classify_io_error(&std::io::Error::from_raw_os_error(i32::from(code)), step),
                want,
                "win32 code {code} at {step:?}"
            );
        }
        // M1, the reviewer's proven case: ACCESS_DENIED arriving from the
        // RENAME step is a LOCKED file (open elsewhere without share-delete),
        // not an ACL problem — the Locked copy must be reachable.
        assert_eq!(
            classify_io_error(&std::io::Error::from_raw_os_error(5), IoStep::Rename),
            SaveError::Locked,
            "rename-time ACCESS_DENIED is the locked-file diagnosis"
        );
        assert_eq!(
            classify_io_error(&std::io::Error::from_raw_os_error(5), IoStep::Stat),
            SaveError::PermissionDenied
        );
        // The Other family keeps the lossless OS message.
        for code in [1u16, 53u16, 1392u16] {
            assert!(
                matches!(
                    classify_io_error(
                        &std::io::Error::from_raw_os_error(i32::from(code)),
                        IoStep::Write
                    ),
                    SaveError::Other(_)
                ),
                "win32 code {code} must map to Other with the OS message"
            );
        }
    }

    /// M1, end to end: a target held open by another PROCESS with share-read
    /// only (PowerShell's File::Open with FileShare::Read — std cannot set
    /// share modes) fails the save with Locked, and the previous bytes
    /// survive. This is the real Windows autosave failure, not a synthesis.
    #[cfg(windows)]
    #[test]
    fn locked_target_reports_locked_end_to_end() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("n.notes");
        std::fs::write(&target, b"previous good bytes")?;
        let path_for_ps = target.display().to_string();
        let mut holder = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "$f = [System.IO.File]::Open('{path_for_ps}', 'Open', 'Read', 'Read'); Start-Sleep -Seconds 30; $f.Close()",
                ),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        // Give the holder a moment to actually open the file.
        std::thread::sleep(std::time::Duration::from_millis(2_000));
        let result = save_document(&target, "new", det(TextEncoding::Utf8, false));
        let _ = holder.kill();
        let _ = holder.wait();
        let Err(e) = result else {
            panic!("a target held open without share-delete must refuse the save");
        };
        assert_eq!(e, SaveError::Locked, "the Locked copy must be reachable");
        assert_eq!(std::fs::read(&target)?, b"previous good bytes");
        Ok(())
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
        ];
        for (kind, want) in cases {
            assert_eq!(
                classify_io_error(&std::io::Error::from(kind), IoStep::Write),
                want
            );
        }
        assert!(matches!(
            classify_io_error(&std::io::Error::from(ErrorKind::TimedOut), IoStep::Write),
            SaveError::Other(_)
        ));
        // InvalidPath carries the OS message now (it has to say WHY a path is
        // unusable), so it is asserted by shape, not equality.
        assert!(matches!(
            classify_io_error(
                &std::io::Error::from(ErrorKind::InvalidInput),
                IoStep::Write
            ),
            SaveError::InvalidPath(_)
        ));
    }

    #[test]
    fn stale_temps_are_swept_and_other_files_are_not() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("note.notes");
        std::fs::write(&target, b"old")?;
        // A REAL crash litter: our exact shape (pid-nanos-attempt.part),
        // with its mtime pushed back past the sweep age so the gate lets go.
        let litter = dir.path().join("note.notes.tmp-4242-1726000000000-1.part");
        std::fs::write(&litter, b"stale")?;
        age_file(&litter, 120)?;
        // The user's own files: a sweep that deletes either is a disaster.
        std::fs::write(dir.path().join("precious.txt"), b"user data")?;
        save_document(&target, "new", det(TextEncoding::Utf8, false))?;
        let names = dir_names(dir.path())?;
        assert!(
            !names.iter().any(|n| n.starts_with("note.notes.tmp-")),
            "aged crash litter must be swept: {names:?}"
        );
        assert!(
            names.contains(&"precious.txt".to_owned()),
            "other files untouched"
        );
        assert!(names.contains(&"note.notes".to_owned()));
        Ok(())
    }

    /// Sets a file's mtime back by the given number of seconds so the sweep's
    /// age gate sees it as old (test-only mtime manipulation, no sleeps).
    fn age_file(path: &Path, seconds_ago: u64) -> Result<(), std::io::Error> {
        let past = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(seconds_ago))
            .ok_or_else(|| std::io::Error::other("cannot compute a past timestamp"))?;
        let f = std::fs::OpenOptions::new().write(true).open(path)?;
        f.set_times(std::fs::FileTimes::new().set_modified(past))
    }

    /// B1 mutation guard, the reviewer's exact scenario: a user keeping a
    /// copy of a corrupt session file as "<target>.tmp-old" (exactly what D12
    /// invites) and another file one character short of our temp shape. The
    /// sweep must touch NEITHER — a prefix match is not a capability.
    #[test]
    fn sweep_never_touches_files_that_are_not_our_temp_shape()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("x.notes");
        std::fs::write(&target, b"current")?;
        let thesis = dir.path().join("x.notes.tmp-old");
        std::fs::write(&thesis, b"USER NOTE: my thesis draft")?;
        age_file(&thesis, 120)?;
        let recovery = dir.path().join("x.notes.tmp-999999999-not-our-shape");
        std::fs::write(&recovery, b"recovery data")?;
        age_file(&recovery, 120)?;
        // Round 2: a dated backup — three digit fields WITHOUT the .part
        // marker, and ancient by definition — plus the .part-less variant.
        let dated = dir.path().join("x.notes.tmp-2026-09-10");
        std::fs::write(&dated, b"MY ONLY COPY")?;
        age_file(&dated, 2_592_000)?;
        let partless = dir.path().join("x.notes.tmp-2026-09-10.part-less");
        std::fs::write(&partless, b"dated and suffixed")?;
        age_file(&partless, 2_592_000)?;
        save_document(&target, "new", det(TextEncoding::Utf8, false))?;
        assert_eq!(
            std::fs::read(&thesis)?,
            b"USER NOTE: my thesis draft",
            "the user's .tmp-backup must survive a save"
        );
        assert_eq!(std::fs::read(&recovery)?, b"recovery data");
        assert!(dated.exists(), "a dated backup must never be swept");
        assert!(partless.exists(), "a .part-less name is not our temp");
        Ok(())
    }

    /// B2 (trailing names): Win32 strips trailing dots and spaces from the
    /// final component, so "a.notes." would land in "a.notes" while the app
    /// reports Ok for a path that does not exist. The save must REFUSE, and
    /// the neighbour must be untouched.
    #[test]
    fn trailing_dot_or_space_names_are_refused_not_rewritten()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let neighbour = dir.path().join("a.notes");
        std::fs::write(&neighbour, b"REAL NOTE")?;
        for hostile in ["a.notes.", "a.notes ", "a.notes. "] {
            let target = dir.path().join(hostile);
            let Err(e) = save_document(&target, "new", det(TextEncoding::Utf8, false)) else {
                panic!("{hostile:?}: a Win32-stripped name must be refused");
            };
            assert!(
                matches!(e, SaveError::InvalidPath(_)),
                "{hostile:?}: expected InvalidPath, got {e:?}"
            );
            assert_eq!(
                std::fs::read(&neighbour)?,
                b"REAL NOTE",
                "{hostile:?}: the neighbour must be untouched"
            );
        }
        Ok(())
    }

    /// REVERSAL (MAJOR-4): the extended prefix no longer buys an exception.
    /// A trailing-dot target is refused in BOTH spellings — the previous
    /// test pinned the carve-out that wrote a note nothing else could open
    /// (plain spelling StrippedName, dialog and Explorer unreachable).
    #[test]
    fn extended_prefix_stripped_target_is_refused_too()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        // canonicalise returns the verbatim \\?\ form, which also rules out
        // 8.3 short names in the composed target.
        let verbatim_dir = std::fs::canonicalize(dir.path())?;
        let verbatim_target = verbatim_dir.join("v.notes.");
        let Err(e) = atomic_write(&verbatim_target, b"kept") else {
            panic!("a trailing-dot target must be refused even past \\\\?\\");
        };
        assert!(matches!(e, SaveError::InvalidPath(_)), "got {e:?}");
        // The plain form: the strip is real there, the write refuses too.
        let plain_target = dir.path().join("plain.notes.");
        let Err(e) = atomic_write(&plain_target, b"x") else {
            panic!("a plain trailing-dot target must still be refused");
        };
        assert!(matches!(e, SaveError::InvalidPath(_)), "got {e:?}");
        Ok(())
    }

    /// M4: the age gate. A temp of OUR shape but written seconds ago (a live
    /// temp of another instance) is never swept; the same shape, old enough,
    /// is.
    #[test]
    fn sweep_age_gate_spares_live_temps_and_reclaims_old_ones()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("n.notes");
        std::fs::write(&target, b"old")?;
        // Instance A's live temp: our exact shape, seconds old.
        let live = dir.path().join("n.notes.tmp-4242-1726000000000-0.part");
        std::fs::write(&live, b"being written right now")?;
        // Instance-from-yesterday's litter: same shape, ancient.
        let ancient = dir.path().join("n.notes.tmp-1717-1726000000000-2.part");
        std::fs::write(&ancient, b"crash litter")?;
        age_file(&ancient, 3_600)?;
        save_document(&target, "new", det(TextEncoding::Utf8, false))?;
        assert_eq!(
            std::fs::read(&live)?,
            b"being written right now",
            "a live temp of another instance must survive (M4)"
        );
        assert!(!ancient.exists(), "aged litter must be reclaimed");
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
    /// B2: a junction target (the OneDrive/sync-client case, creatable
    /// without privileges) must be REFUSED: the link and the real file
    /// behind it both survive, and the message names the link and says what
    /// to do.
    #[cfg(windows)]
    #[test]
    fn junction_target_is_refused_and_link_and_body_survive()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::process::Command;
        let dir = tempfile::tempdir()?;
        let real_dir = dir.path().join("realdir");
        std::fs::create_dir(&real_dir)?;
        let real_file = real_dir.join("n.notes");
        std::fs::write(&real_file, b"REAL CURRENT BYTES")?;
        let jn = dir.path().join("jn.notes");
        // mklink /J (a directory junction) needs no privilege, unlike file
        // symlinks, so this proof runs on any Windows host.
        let out = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&jn)
            .arg(&real_dir)
            .output()?;
        assert!(out.status.success(), "mklink /J failed: {:?}", out.stderr);
        let Err(e) = save_document(&jn, "new bytes", det(TextEncoding::Utf8, false)) else {
            panic!("saving over a junction must be refused");
        };
        let SaveError::ReparsePoint(msg) = &e else {
            panic!("expected ReparsePoint, got {e:?}");
        };
        assert!(
            msg.contains("jn.notes"),
            "the message names the link: {msg}"
        );
        assert!(
            msg.contains("save to the real file"),
            "the message says what to do: {msg}"
        );
        // The junction is still a link and the real file still holds the old
        // bytes — nothing was orphaned.
        assert!(std::fs::symlink_metadata(&jn)?.file_type().is_symlink());
        assert_eq!(std::fs::read(&real_file)?, b"REAL CURRENT BYTES");
        Ok(())
    }

    /// B2: the plain file-symlink case (the Linux/macOS case, and Windows
    /// where creating one needs developer mode or admin). When the host
    /// refuses symlink creation this test says so and passes vacuously; the
    /// junction test above carries the refusal proof for those hosts — both
    /// go through the same reparse check.
    #[cfg(windows)]
    #[test]
    fn symlink_file_target_is_refused_when_creatable() -> Result<(), Box<dyn std::error::Error>> {
        use std::process::Command;
        let dir = tempfile::tempdir()?;
        let real = dir.path().join("real.txt");
        std::fs::write(&real, b"REAL BODY")?;
        let fl = dir.path().join("fl.notes");
        let out = Command::new("cmd")
            .args(["/C", "mklink"])
            .arg(&fl)
            .arg(&real)
            .output()?;
        if !out.status.success() {
            eprintln!(
                "symlink_file_target_is_refused_when_creatable: host cannot create file                  symlinks (no privilege/dev mode); the junction test carries the proof"
            );
            return Ok(());
        }
        let Err(e) = save_document(&fl, "new", det(TextEncoding::Utf8, false)) else {
            panic!("saving over a symlink must be refused");
        };
        assert!(matches!(e, SaveError::ReparsePoint(_)));
        assert!(std::fs::symlink_metadata(&fl)?.file_type().is_symlink());
        assert_eq!(
            std::fs::read(&real)?,
            b"REAL BODY",
            "the real file is untouched"
        );
        Ok(())
    }

    /// B2, hard-link behaviour (documented, chosen): a hard link is NOT a
    /// reparse point, so the save replaces the target's directory entry —
    /// the saved path carries the newest bytes while sibling links keep the
    /// old inode's content. Asserting it exactly so a future change to
    /// refuse is a conscious decision.
    #[cfg(windows)]
    #[test]
    fn hard_link_target_is_replaced_by_design() -> Result<(), Box<dyn std::error::Error>> {
        use std::process::Command;
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("n.notes");
        std::fs::write(&target, b"OLD CONTENT")?;
        let sibling = dir.path().join("sibling.notes");
        let out = Command::new("cmd")
            .args(["/C", "mklink", "/H"])
            .arg(&sibling)
            .arg(&target)
            .output()?;
        assert!(out.status.success(), "mklink /H failed: {:?}", out.stderr);
        save_document(&target, "new bytes", det(TextEncoding::Utf8, false))?;
        assert_eq!(std::fs::read(&target)?, b"new bytes");
        assert_eq!(
            std::fs::read(&sibling)?,
            b"OLD CONTENT",
            "sibling links keep the old inode's content"
        );
        Ok(())
    }
}
