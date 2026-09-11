//! The volume identity of an open file, as a fact. No policy: the caller
//! gets the OS answer or it gets nothing, and it does its own thinking
//! somewhere that is allowed to think.
//!
//! Why a HANDLE and not a path: the name is resolved from the kernel object,
//! so it is immune to a rename between the open and the query (a test below
//! proves it), and a path-based seam would have to OPEN the path itself -
//! an authorisation act this crate does not perform. Core already holds the
//! open file at the moment identity is needed; it reaches this fact through
//! api when that lane wires the flow (this crate decides nothing about who
//! asks or when).
//!
//! IDENTITY INPUT, NEVER A PERMISSION INPUT: the identity string must not be
//! fed back into any refusal check as though it had been validated by the
//! act of resolution. The function performs no policy and answers no
//! is-allowed question; it hands over the unvarnished OS identity so that a
//! comparison above the seam operates on the truth. An identity that crosses
//! a refusal boundary (a junction, a substituted drive) is returned exactly
//! as it is - visible, never laundered.
//!
//! THE KEY IS NOT A DISPLAY VALUE. The string is a volume-GUID identity of
//! the form \\?\Volume{guid}\real\path\note.notes: no user can type it,
//! no file dialog shows it, and it must never reach a label, a title or a
//! recent-list ROW as the displayed name. Display stays whatever the caller
//! already chose; this is the collapse key that makes a substituted drive, a
//! UNC spelling and a junction path the SAME file.
//!
//! A FACT ABOUT REACHABILITY: the identity is obtainable only while a live
//! handle to the file exists - GetFinalPathNameByHandleW is a handle query,
//! and there is no handle for a nonexistent path. What a caller does with
//! that, including whether the string is kept at all, is policy and belongs
//! above this seam. One consequence is worth stating because it bounds every
//! design above the seam: an identity minted on a live handle can be
//! re-derived or checked only through another live handle to the same file;
//! when the volume is offline, neither is possible - while a DOS path can
//! always at least be re-stated. That is a property of the fact, not an
//! instruction to prefer one design over another.

use ::windows::Win32::Foundation::HANDLE;
use ::windows::core::PWSTR;

use crate::{PlatformError, PlatformResult};

// kernel32's GetFinalPathNameByHandleW, declared locally: the windows crate
// gates this symbol behind the Win32_Storage_FileSystem feature, which the
// root manifest does not enable (the feature list lives in the manager-owned
// root manifest). The generated binding would link the same kernel32 export
// with the same signature - the GetACP precedent in this crate - so here it
// is spelled by hand. NOT marked safe: it writes through the buffer pointer
// the caller sizes, so every call goes through an unsafe block stating that
// invariant.
unsafe extern "system" {
    fn GetFinalPathNameByHandleW(
        hfile: HANDLE,
        lpszfilepath: PWSTR,
        cchfilepath: u32,
        dwflags: u32,
    ) -> u32;
}

/// `FILE_NAME_NORMALIZED` (0x0): the normalized form of the final path.
const FILE_NAME_NORMALIZED: u32 = 0x0;
/// `VOLUME_NAME_GUID` (0x1): the volume as its GUID, not its drive letter.
/// THE reason this seam exists: a substituted drive letter, a UNC spelling
/// and a junction path all resolve to the same volume GUID, so two spellings
/// of one file collapse to one identity. The DOS form would answer a subst
/// drive with its own letter and re-create the two-entries defect.
const VOLUME_NAME_GUID: u32 = 0x1;

/// The first buffer, in UTF-16 code units: long enough for the overwhelming
/// majority of real paths, with the documented grow-and-retry behind it.
const FIRST_BUFFER_CCH: usize = 1024;

/// The volume identity of the file an OS handle refers to: normalized, with
/// the volume spelled as its GUID - \\?\Volume{guid}\real\path\note.notes.
///
/// `Ok(Some)` is the OS answer VERBATIM, prefix and all: the prefix is part of
/// the identity, not noise this crate trims away. Two different spellings of
/// one file - a drive letter and a junction, a subst and the real letter -
/// answer the SAME string, which is the whole defect this closes.
///
/// The friction, and what each case returns:
/// * **A reparse point the OS refuses to normalize** (some junction and
///   symlink shapes): the call fails with ERROR_NOT_SUPPORTED -> `Ok(None)`.
/// * **A network file**: the volume is the redirector, and a GUID may not
///   exist for it - the call may answer a UNC-spelled device form or fail;
///   either way the answer is whatever the OS said (Ok(Some)) or `Ok(None)`. This
///   crate does not translate one into the other.
/// * **A drive mapping that is gone** (substituted drive removed, share
///   disconnected): the volume GUID SURVIVES the letter, so the identity
///   still comes back - `Some(\\?\Volume{guid}\...)` naming a path no
///   user can open. That is correct: it is an identity, not an openable
///   path, and `Ok(None)` must stay reserved for "the OS could not answer".
///
/// The three verdicts are separated so the caller never has to guess:
/// `Err` means the ARGUMENT was structurally wrong before any OS contact
/// (a null handle - nothing was open, a caller bug worth naming);
/// `Ok(None)` means the OS was asked and could not answer (bad or closed
/// handle at query time, a reparse shape it refuses to normalize) - the
/// caller owns what None means, including whether a DOS key beats no key,
/// and no DOS fallback happens here; `Ok(Some)` is the identity.
///
/// What Some does NOT promise: that the path is openable by a human. A
/// volume GUID survives the removal of its drive letter, and a still-open
/// handle to a disconnected share may or may not resolve - the OS answer is
/// handed over either way. The string's own prefix is the only free signal
/// about its shape; telling "the volume is not here now" from "the OS would
/// not answer" any further would cost a second syscall and a policy this
/// crate does not hold.
///
/// Cost: one syscall on the happy path, ~19 microseconds measured on this
/// host (the test prints it) - a first-open and rename fact, never a
/// per-save tax.
pub fn volume_identity_of(handle: isize) -> PlatformResult<Option<String>> {
    if handle == 0 {
        return Err(PlatformError::InvalidHandle);
    }
    let file = HANDLE(handle as *mut core::ffi::c_void);
    let flags = FILE_NAME_NORMALIZED | VOLUME_NAME_GUID;
    let mut buffer = vec![0u16; FIRST_BUFFER_CCH];
    // SAFETY: GetFinalPathNameByHandleW receives the handle value we were
    // given (the API validates it internally and fails closed on a handle
    // that is not a file, which becomes Ok(None) below), plus `buffer`, a live
    // Vec<u16> whose length is passed as the capacity and whose storage the
    // call may write and nothing else. No pointer is retained past the call.
    // The returned count is inspected, never unwrapped.
    let mut needed = unsafe {
        GetFinalPathNameByHandleW(file, PWSTR(buffer.as_mut_ptr()), buffer.len() as u32, flags)
    };
    if needed == 0 {
        // Any query failure is Ok(None); the caller owns what None means.
        return Ok(None);
    }
    if needed as usize > buffer.len() {
        // The too-small answer counts the null terminator; grow and retry once.
        buffer = vec![0u16; needed as usize];
        // SAFETY: as above - the buffer now has exactly the size the API
        // asked for, and the second call writes at most that many units.
        needed = unsafe {
            GetFinalPathNameByHandleW(file, PWSTR(buffer.as_mut_ptr()), buffer.len() as u32, flags)
        };
        if needed == 0 {
            return Ok(None);
        }
    }
    let len = (needed as usize).min(buffer.len());
    Ok(Some(
        String::from_utf16_lossy(&buffer[..len])
            .trim_end_matches('\0')
            .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::volume_identity_of;
    use crate::PlatformError;
    use std::os::windows::io::AsRawHandle;
    use std::path::PathBuf;
    use std::{fs, process};

    /// A scratch tree under TEMP: <root>/target_dir/note.txt plus a junction
    /// <root>/link_dir pointing at target_dir. Junctions need no privilege
    /// (mklink /J) and are exactly the reparse shape D30 refuses on write.
    struct Scratch {
        root: PathBuf,
        target: PathBuf,
        link: PathBuf,
    }
    impl Scratch {
        // The root is unique per TEST (parallel tests share one process), or a
        // sibling's cleanup would delete this test's tree mid-run.
        fn new(tag: &str) -> Scratch {
            let root =
                std::env::temp_dir().join(format!("notes-platform-final-{}-{tag}", process::id()));
            let _ = fs::remove_dir_all(&root);
            let target = root.join("target_dir");
            fs::create_dir_all(&target).expect("scratch target dir");
            fs::write(target.join("note.txt"), "identity").expect("scratch note");
            let link = root.join("link_dir");
            let out = process::Command::new("cmd")
                .args([
                    "/C",
                    "mklink",
                    "/J",
                    &link.to_string_lossy(),
                    &target.to_string_lossy(),
                ])
                .output()
                .expect("mklink runs");
            assert!(
                out.status.success(),
                "mklink /J failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            Scratch { root, target, link }
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            // The junction is removed by name (rmdir), never recursed into.
            let _ = process::Command::new("cmd")
                .args(["/C", "rmdir", &self.link.to_string_lossy()])
                .status();
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// THE COLLAPSE: the direct spelling and the junction spelling of one
    /// file answer the SAME identity string. This is the defect the recents
    /// lane is fighting from the other side of the seam, closed in one test.
    #[test]
    fn two_spellings_of_one_file_collapse_to_one_identity() {
        let scratch = Scratch::new("collapse");
        let direct = fs::File::open(scratch.target.join("note.txt")).expect("opens");
        let through_link =
            fs::File::open(scratch.link.join("note.txt")).expect("opens through the junction");
        let a = volume_identity_of(direct.as_raw_handle() as isize)
            .expect("direct resolves")
            .expect("direct identity");
        let b = volume_identity_of(through_link.as_raw_handle() as isize)
            .expect("junction resolves")
            .expect("junction identity");
        assert_eq!(a, b, "one file, one identity: {a} vs {b}");
        assert!(a.starts_with(r"\\?\Volume{"), "the GUID form: {a}");
        assert!(a.contains("target_dir"), "the real path is named: {a}");
        assert!(
            !a.contains("link_dir"),
            "the alias is not laundered in: {a}"
        );
    }

    /// THE SECURITY TEST. The identity string is an identity input, never a
    /// permission input: this function takes no policy, answers no is-allowed,
    /// and hands over the true identity even when the real path crosses a
    /// refusal boundary. A hostile resolution therefore cannot WIDEN what is
    /// allowed: an allow-list keyed on a spelling does not match this GUID
    /// key (different strings by construction), and the key names the REAL
    /// path, so a crossing is visible instead of laundered. What is
    /// asserted: the answer is exactly the OS's GUID form over the real path
    /// - never a verdict, never a display value, never a licence.
    #[test]
    fn a_hostile_resolution_is_handed_over_verbatim_and_never_reauthorised() {
        let scratch = Scratch::new("hostile");
        let file =
            fs::File::open(scratch.link.join("note.txt")).expect("opens through the junction");
        let identity = volume_identity_of(file.as_raw_handle() as isize)
            .expect("resolves")
            .expect("identity present");
        assert!(
            identity.starts_with(r"\\?\Volume{") && identity.contains("target_dir\\note.txt"),
            "the real crossing is visible inside the GUID identity: {identity}"
        );
    }

    /// THE RENAME CASE, which is why this takes a handle: the identity is read
    /// from the kernel object, so a rename between the open and the query
    /// still resolves to the file's CURRENT name. A path spelling would have
    /// answered with a name that no longer exists.
    #[test]
    fn a_rename_between_open_and_query_does_not_lie() {
        let scratch = Scratch::new("rename");
        let file = fs::File::open(scratch.target.join("note.txt")).expect("opens");
        fs::rename(
            scratch.target.join("note.txt"),
            scratch.target.join("renamed.txt"),
        )
        .expect("std opens with FILE_SHARE_DELETE, so the rename succeeds");
        let identity = volume_identity_of(file.as_raw_handle() as isize)
            .expect("resolves")
            .expect("identity present");
        assert!(
            identity.contains("renamed.txt") && !identity.contains("note.txt"),
            "the CURRENT name comes back: {identity}"
        );
    }

    /// Refusals are typed, not guesses: a null handle never reached the OS,
    /// so it is the caller-bug error; a handle the OS is asked anyway comes
    /// back `Ok(None)` when it declines. The caller owns what None means,
    /// including whether a DOS key beats no key. No DOS fallback, no guess,
    /// no distinction this crate is not allowed to make.
    #[test]
    fn a_refusal_is_typed_not_a_guess() {
        // The three-verdict split: a null handle is the caller's bug, typed;
        // a garbage value is asked to the OS, which declines -> Ok(None).
        assert!(matches!(
            volume_identity_of(0),
            Err(PlatformError::InvalidHandle)
        ));
        assert!(matches!(volume_identity_of(isize::MIN), Ok(None)));
    }

    /// The cost, measured on a real open handle and printed for the record:
    /// this belongs on a first open or a rename, never on every save.
    #[test]
    fn the_cost_is_measured_and_printed_not_assumed() {
        let scratch = Scratch::new("cost");
        let file = fs::File::open(scratch.target.join("note.txt")).expect("opens");
        let raw = file.as_raw_handle() as isize;
        let rounds = 200u32;
        let started = std::time::Instant::now();
        for _ in 0..rounds {
            volume_identity_of(raw)
                .expect("resolves")
                .expect("identity present");
        }
        let per_call = started.elapsed() / rounds;
        eprintln!("volume_identity_of measured cost: {per_call:?} per call");
        assert!(
            per_call < std::time::Duration::from_millis(5),
            "a single resolution must stay far below a flush tick"
        );
    }
}
