//! Name-only safety decisions about a path. PURE: no filesystem, no
//! canonicalisation, no timeouts. The engine is single-threaded and the
//! bridge must never block in a frame, so an unreachable UNC host — 2.68s to
//! the FIRST event in the reviewer's measurement, tens of seconds possible —
//! is a frozen window. Core owns the DECISION; api owns the CALL: the port
//! consults this predicate before a path reaches the filesystem and renders
//! the verdict as copy.
//!
//! Every rule names the Win32 behaviour it defends against. The predicate is
//! total and conservative: when the name alone cannot prove danger, the
//! verdict is Allowed and the filesystem call answers normally.
//!
//! Guest, by decision of the state-dir review: the B2 reparse-point
//! predicate lives HERE, because link safety is POLICY — what a path may
//! be followed through. It is defined once and consumed by both
//! save::atomic_write and session::ensure_state_dir; it reads metadata,
//! never writes.

use std::path::{Component, Path};

use crate::save::SaveError;

/// The decision for a path, from the NAME ALONE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathVerdict {
    /// No name-level hazard; the filesystem call may proceed.
    Allowed,
    /// A UNC path (\\no-such-host\share\x.notes, also //host/share and the extended \\?\UNC\host\share\x.notes
    /// form — the prefix matched CASE-INSENSITIVELY, as Windows matches all
    /// path prefixes: "\\?\unc\…" is the same prefix, and a case-sensitive
    /// compare would let the cased spelling reach the filesystem). POLICY,
    /// not a bug: the first touch of an unreachable server
    /// blocks for as long as the OS redirector pleases, and this engine is
    /// single-threaded — the app refuses the attempt instead of freezing
    /// the window. If a future build wants network notes, the lift is an
    /// async/timeout layer owned by api, not here.
    UnboundedNetwork,
    /// NT device namespaces (\\.\PhysicalDrive0, \??\C:\x.notes, \.\x — and the device
    /// namespace reached through the \\?\ root that \??\ shares: \\?\PhysicalDrive0
    /// IS \\.\PhysicalDrive0) and the classic DOS
    /// device names (CON, PRN, AUX, NUL, COM1-9, LPT1-9) WITHOUT an
    /// extension as the final component: opening CON reads the console
    /// (hangs, console bytes), writing NUL silently discards. WITH an
    /// extension ("CON.notes") modern Windows creates an ordinary file —
    /// verified by the tester's probe on this very build — so an extended
    /// name is Allowed and only the bare stem is refused.
    ReservedDevice,
    /// An alternate data stream ("a.notes:sneaky"): Win32 would open or
    /// write a STREAM of another file while the app holds a name that
    /// is_notes_path answers differently about — the same lie class as the
    /// trailing-dot bug. The ONE legitimate colon is the drive separator;
    /// any colon after it is a stream.
    StreamName,
    /// A drive-relative name ("C:notes.notes", "a:b.notes"): Win32 resolves
    /// it against the PER-DRIVE current directory — process state a
    /// name-only policy cannot see — so one name can mean two different
    /// files at two different times, and Path::join with such a component
    /// REPLACES the target wholesale. The app must never hold a name it
    /// cannot honestly resolve (BLOCKER-1, measured with two child CWDs).
    DriveRelative,
    /// The final component ends with '.' or ' ': Win32 strips both from the
    /// final component, so "a.notes." writes "a.notes" while the app reports
    /// Ok for a path that does not exist. One shared implementation with the
    /// save engine (any_component_is_stripped, judging every component) so
    /// the two can never drift.
    ///
    /// REVERSAL (MAJOR-4): a previous revision carved the extended-length
    /// prefix out of this rule — past \\?\ Win32 strips nothing, so the
    /// written file IS the one named. That was true about the mechanism and
    /// wrong about the product: the probe measured that the plain spelling
    /// of the saved name is refused by THIS policy, unreadable on disk
    /// (NotFound), and unreachable by Explorer and the file dialog, while
    /// identity_key claims it is the same note. A note the user can never
    /// open again is a lost note, so the rule now applies past the prefix
    /// too. Do not re-add the carve-out without answering the probe.
    StrippedName,
}

/// Decides from the NAME ALONE, without touching the filesystem, whether
/// opening or saving this path is safe to attempt. Pure: no I/O, no
/// metadata, no canonicalisation — a predicate that touches the filesystem
/// is how the 2.68s freeze moves into the wrong layer.
///
/// "C:" itself (a bare drive, i.e. the per-drive current directory) is
/// Allowed: it names no final component, carries none of the hazards below,
/// and the filesystem call — which api owns — answers for it.
pub fn path_policy(path: &Path) -> PathVerdict {
    // std's Components normalises away some of the shapes we must judge, so
    // the raw string is analysed byte-wise. to_string_lossy cannot invent a
    // colon, dot or space (it only emits U+FFFD for invalid bytes, and
    // U+FFFD is none of those), so the lossy view is sound for these rules.
    let raw = path.as_os_str().to_string_lossy();
    let s = raw.as_ref();
    // Device namespaces: never a regular file. (\\\\?\\ is NOT one of them;
    // the extended-length prefix is legitimate and handled below.)
    if s.starts_with("\\\\.\\") || s.starts_with("\\??\\") || s.starts_with("\\.\\") {
        return PathVerdict::ReservedDevice;
    }
    // Network: the extended UNC form first, then the plain ones. The prefix
    // compare is CASE-INSENSITIVE because Windows matches path prefixes that
    // way: "\\?\unc\host\share\x.notes" is the same path as the uppercase
    // spelling, and a case-sensitive strip would let it fall through to the
    // drive rules and reach the filesystem — the exact 2.68 s freeze this
    // module exists to prevent, unlocked by one changed letter.
    if starts_with_ignore_ascii_case(s, "\\\\?\\UNC\\") {
        return PathVerdict::UnboundedNetwork;
    }
    // Extended-length prefix: legitimate — it bypasses Win32 name mangling,
    // which is exactly why it used to be carved out of the strip rule. The
    // carve-out is REVERSED (MAJOR-4, see StrippedName): a mangled name past
    // the prefix writes a note nothing else can open, so the rule now
    // applies there too. The OTHER extended-prefix judgement stays: a body
    // that starts with anything but a drive, UNC or a root is the device
    // namespace.
    let extended = path.as_os_str().to_string_lossy().starts_with("\\\\?\\");
    let body = s.strip_prefix("\\\\?\\").unwrap_or(s);
    if body.starts_with("\\\\") || body.starts_with("//") {
        return PathVerdict::UnboundedNetwork;
    }
    // Under the extended prefix the body may legitimately start only with a
    // drive ("X:"), a UNC (returned above), or a rooted path ("\dir\x" — the
    // current drive's root). Anything else is the device namespace reached
    // through the same object-manager root "\??\" shares: "\\?\PhysicalDrive0"
    // IS "\\.\PhysicalDrive0", and the NAME proves it — no note is ever named
    // PhysicalDrive0 or GLOBALROOT. Refuse without a filesystem call.
    if extended && drive_separator(body).is_none() && !body.starts_with('\\') {
        return PathVerdict::ReservedDevice;
    }
    // The drive separator is the ONE legitimate colon; anything after it
    // starts a stream. A relative colon ("a.notes:sneaky") is a stream from
    // the first colon.
    let after_drive = match drive_separator(body) {
        Some(n) => &body[n + 1..],
        None => body,
    };
    // A drive RELATIVE name ("C:notes.notes", "a:b.notes"): a drive colon
    // with more name after it and NO separator anywhere. Win32 resolves it
    // against the per-drive current directory — process state this name-only
    // policy cannot see — so the name cannot be honoured honestly
    // (BLOCKER-1, measured: two child CWDs, two different files).
    if drive_separator(body).is_some()
        && !after_drive.is_empty()
        && !after_drive.contains('\\')
        && !after_drive.contains('/')
    {
        return PathVerdict::DriveRelative;
    }
    if after_drive.contains(':') {
        return PathVerdict::StreamName;
    }
    // The final RAW component is "." or "..": std's Components folds both
    // away, so they are judged from the raw spelling. Win32 answers
    // PermissionDenied for a write under either — the name on disk would
    // not be the name spelled, the same lie class as the strip rule.
    if body
        .split(['\\', '/'])
        .next_back()
        .is_some_and(|last| last == "." || last == "..")
    {
        return PathVerdict::StrippedName;
    }
    // The per-component view (BLOCKER-2, measured): Win32 applies its name
    // rules on EVERY step of the way down, not only at the end —
    // "sub.\x.notes" writes "sub\x.notes" while the app names "sub.\x.notes".
    let normals: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    // A device stem as a NON-final component ("<dir>\CON\x.notes"): the
    // traversal cannot resolve, so the name cannot be written as spelled.
    if normals[..normals.len().saturating_sub(1)]
        .iter()
        .any(|c| is_dos_device(c))
    {
        return PathVerdict::ReservedDevice;
    }
    let Some(name) = normals.last() else {
        // A root or a bare drive ("C:"): no component to judge, and none of
        // the hazards live in the prefix.
        return PathVerdict::Allowed;
    };
    // Classic DOS devices, extensionless, as the FINAL component. Measured
    // on this build: std CAN create and reopen "CON.notes" (it auto-prefixes
    // the device path), while an independent "cmd /c echo > CON" witness
    // created NOTHING — so an extended name is Allowed and only the bare
    // stem is refused. The rule is the INTEROP call (every other tool sees
    // nothing, not even a file), not impossibility. COM10+ was never in the
    // reserved set.
    if is_dos_device(name) {
        return PathVerdict::ReservedDevice;
    }
    // Trailing dot/space mangling — EVERY component, INCLUDING past the
    // extended prefix (MAJOR-4 reversal; see StrippedName).
    if normals.iter().any(|n| final_component_is_stripped(n)) {
        return PathVerdict::StrippedName;
    }
    PathVerdict::Allowed
}

/// Where the drive separator ends, if the string STARTS with one: "X:" with
/// X an ASCII letter (the colon at byte position 1). UNC and device
/// namespaces are decided before this runs.
fn drive_separator(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        Some(1)
    } else {
        None
    }
}

/// The ONE trailing-name rule, shared with save::atomic_write so the engine
/// and this policy can never disagree about what a stripped name is.
pub(crate) fn final_component_is_stripped(name: &str) -> bool {
    name.ends_with('.') || name.ends_with(' ')
}

/// The per-PATH strip rule (BLOCKER-2), shared with save::atomic_write:
/// EVERY component is judged, because Win32 strips a trailing dot or space
/// from every directory component on the way down — "sub.\x.notes" writes
/// "sub\x.notes" while the app names "sub.\x.notes". Policy and the save
/// engine both call this, so they cannot drift again.
pub(crate) fn any_component_is_stripped(path: &Path) -> bool {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .any(|n| final_component_is_stripped(&n))
}

/// Case-insensitive ASCII prefix test. Windows matches path prefixes
/// case-insensitively, so a case-sensitive compare is a hole: "\\\\?\\unc\\…"
/// is the same prefix as "\\\\?\\UNC\\…".
fn starts_with_ignore_ascii_case(s: &str, prefix: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= prefix.len() && b[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// THE reparse-point predicate (B2), defined once: Windows reads the
/// FILE_ATTRIBUTE_REPARSE_POINT bit, which catches symlinks, junctions and
/// cloud placeholders through std alone (no windows crate — core-no-os);
/// other platforms use std's symlink classification.
pub(crate) fn metadata_is_reparse(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    let is_reparse = {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT catches symlinks, junctions and cloud
        // placeholders. std only — no windows crate (core-no-os).
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    };
    #[cfg(not(windows))]
    let is_reparse = meta.file_type().is_symlink();
    is_reparse
}

/// B2: refuse to rename over (or create through) a symlink, junction or any
/// other reparse point (OneDrive and sync clients). Replacing one destroys
/// the link and orphans the real file behind it while the app reports
/// success. The error names the link and, where it could be resolved, the
/// real target. A MISSING target is not a link: Ok — the rename will
/// create it.
pub(crate) fn refuse_reparse_point(target: &Path) -> Result<(), SaveError> {
    let Ok(meta) = std::fs::symlink_metadata(target) else {
        return Ok(()); // no target yet: the rename will create it
    };
    if metadata_is_reparse(&meta) {
        let real = std::fs::read_link(target)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "an unresolvable target".to_owned());
        return Err(SaveError::ReparsePoint(format!(
            "{} is a link to {} — the app will not replace links; save to the real file instead",
            target.display(),
            real
        )));
    }
    Ok(())
}

/// The classic MS-DOS device names. Modern Windows keeps the bare stem
/// reserved in every directory; COM/LPT take single digits 1-9 — COM0 and
/// LPT0 were never in the reserved set, so they are ordinary file names and
/// must not be over-rejected.
fn is_dos_device(name: &str) -> bool {
    let up = name.to_ascii_uppercase();
    matches!(up.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((up.starts_with("COM") || up.starts_with("LPT"))
            && up.len() == 4
            && matches!(up.as_bytes()[3], b'1'..=b'9'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(p: &Path) -> PathVerdict {
        path_policy(p)
    }

    #[test]
    fn ordinary_paths_are_allowed() {
        for p in [
            r"C:\Users\me\a.notes",
            "a.notes",
            "relative/dir/x.txt",
            r"C:\x.notes", // the drive colon is the naive-implementation trap
            "C:",
            "/usr/local/x.notes",
            "CON.notes", // modern Windows: an ordinary file (probe-verified)
            "COM10",     // never in the reserved set
            "CONX",
        ] {
            assert_eq!(v(Path::new(p)), PathVerdict::Allowed, "{p}");
        }
    }

    #[test]
    fn unc_paths_are_refused_by_policy() {
        for p in [
            r"\\no-such-host\share\x.notes",
            "//host/share/x.notes",
            r"\\?\UNC\host\share\x.notes",
            // The BLOCKER's exact input: same prefix, different casing.
            r"\\?\unc\host\share\x.notes",
        ] {
            assert_eq!(v(Path::new(p)), PathVerdict::UnboundedNetwork, "{p}");
        }
    }

    #[test]
    fn streams_are_refused() {
        for p in ["a.notes:sneaky", r"dir\a.notes:s", r"C:\dir\x:ads"] {
            assert_eq!(v(Path::new(p)), PathVerdict::StreamName, "{p}");
        }
    }

    #[test]
    fn device_namespaces_and_bare_devices_are_refused() {
        for p in [
            r"\\.\PhysicalDrive0",
            r"\??\C:\x.notes",
            r"\.\x",
            "CON",
            "con",
            "NUL",
            "COM4",
            "LPT1",
        ] {
            assert_eq!(v(Path::new(p)), PathVerdict::ReservedDevice, "{p}");
        }
    }

    #[test]
    fn stripped_names_are_refused() {
        for p in ["a.notes.", "a.notes ", "a.notes. ", "notes.."] {
            assert_eq!(v(Path::new(p)), PathVerdict::StrippedName, "{p:?}");
        }
    }

    /// SUBJECT: the `\\?\` extended prefix - Win32 spelling and nothing else, so
    /// this case runs where that spelling means something. Its one wrong-machine
    /// assert is `\\?\C:\CON`: to a POSIX path that whole string is ONE
    /// ordinary, mangling-free filename, so the answer there is Allowed, not
    /// ReservedDevice. Every OTHER assert in here is a raw-string law - the stream
    /// colon, the `\\?\UNC` network prefix, the device namespace, the trailing
    /// dot, COM0/LPT0 staying ordinary - and each one now has an un-gated twin in
    /// the hostile table below, including the row moved there for exactly that
    /// reason. So Linux gives up the prefix here, not a rule. Not an #[ignore]: the
    /// case still runs, and still fails, on Windows.
    #[test]
    #[cfg(windows)]
    fn extended_prefix_no_longer_bypasses_the_stripping_rule() {
        // MAJOR-4 reversal: the dot is a real character past the prefix, but
        // the note it writes can never be reopened by a plain spelling — so
        // the stripped spelling is refused there too.
        assert_eq!(v(Path::new(r"\\?\C:\a.notes.")), PathVerdict::StrippedName);
        // But streams and devices are still judged.
        assert_eq!(v(Path::new(r"\\?\C:\x:ads")), PathVerdict::StreamName);
        assert_eq!(v(Path::new(r"\\?\C:\CON")), PathVerdict::ReservedDevice);
        // And the UNC-inside-extended form is still network.
        assert_eq!(
            v(Path::new(r"\\?\UNC\host\share\x")),
            PathVerdict::UnboundedNetwork
        );
        // The device namespace under the same \\?\ root is refused: the
        // name proves it, no filesystem call needed.
        assert_eq!(
            v(Path::new(r"\\?\PhysicalDrive0")),
            PathVerdict::ReservedDevice
        );
        assert_eq!(
            v(Path::new(r"\\?\GLOBALROOT\Device\HarddiskVolume3\x")),
            PathVerdict::ReservedDevice
        );
        // COM0/LPT0 were never reserved: ordinary names.
        assert_eq!(v(Path::new("COM0")), PathVerdict::Allowed);
        assert_eq!(v(Path::new("LPT0")), PathVerdict::Allowed);
    }

    /// The stripped-name fact is spelled exactly ONCE, in
    /// path_policy::final_component_is_stripped, and save::atomic_write
    /// calls it — with the extended-prefix carve-out from the same shared
    /// predicate. Behaviour tests alone cannot pin this (a fork passes them
    /// all), so the pin is SOURCE-level: include_str! reads the sibling
    /// module at test-compile time and fails HERE, not at review time, if
    /// save ever re-forks an inline copy.
    #[test]
    fn save_writes_through_the_shared_stripped_rule_not_a_fork() {
        const SAVE: &str = include_str!("save.rs");
        const POLICY: &str = include_str!("path_policy.rs");
        // save calls THE shared helper for both halves of the decision.
        assert!(
            SAVE.contains("path_policy::any_component_is_stripped(target)"),
            "save::atomic_write must judge EVERY component through the shared helper — an inline fork is the drift this pin exists to catch"
        );
        assert_eq!(
            POLICY
                .matches(concat!("pub(crate) fn any", "_component_is_stripped"))
                .count(),
            1,
            "exactly one definition of the per-path strip rule"
        );
        // The facts themselves are spelled only here. The needles are built
        // with concat! so this test's own source — part of POLICY, via the
        // include_str! above — cannot count as a second occurrence.
        assert_eq!(
            POLICY
                .matches(concat!("pub(crate) fn final", "_component_is_stripped"))
                .count(),
            1,
            "the stripped-name rule must have exactly one definition"
        );
        // And no inline re-derivation in save: the strip predicate's byte
        // shape may not appear outside the helper.
        assert!(
            !SAVE.contains("ends_with('.')") && !SAVE.contains("ends_with(' ')"),
            "the trailing-strip fact may be spelled only in path_policy::final_component_is_stripped"
        );
        // The helper's own behaviour, for completeness.
        for name in ["a.notes.", "a.notes ", "a.notes. "] {
            assert!(final_component_is_stripped(name));
        }
        for name in ["a.notes", "a.notes..x"] {
            assert!(!final_component_is_stripped(name));
        }
    }

    /// THE HEADLINE PIN: the equivalence "path_policy Allowed <=> save
    /// writes" is enforced by a real CALL in atomic_write, not by a comment.
    /// The needle is the exact two-line call shape — a comment cannot
    /// produce it without being written to look like the call, which is
    /// deliberate drift, not an accident. The verdict set itself is pinned
    /// by the hostile tables on both sides of the seam.
    #[test]
    fn the_save_path_consults_the_whole_verdict_set() {
        const SAVE: &str = include_str!("save.rs");
        const POLICY: &str = include_str!("path_policy.rs");
        assert!(
            SAVE.contains(concat!(
                "let verdict = crate::path_policy::path_policy(target);\n",
                "    if verdict != crate::path_policy::PathVerdict::Allowed {"
            )),
            "atomic_write must consult path_policy on the write path — the equivalence is a call, not a comment"
        );
        assert_eq!(
            POLICY
                .matches(concat!(
                    "pub fn path_policy(path: &",
                    "Path) -> PathVerdict"
                ))
                .count(),
            1,
            "exactly one definition of THE predicate"
        );
    }

    /// The reviewer's hostile-input table, kept as a fact sheet: every input
    /// that once slipped through a hole, and the verdict that now names it.
    /// Run against the pure predicate — no filesystem, no elevation — which
    /// is also the only way to judge the \??\ forms without a real device.
    /// One row's expectation when the LAW asserted is Win32 name mangling: the
    /// verdict Windows must give, `Allowed` where no such law exists.
    const fn win32_name_law(under_win32: PathVerdict) -> PathVerdict {
        if cfg!(windows) {
            under_win32
        } else {
            PathVerdict::Allowed
        }
    }

    #[test]
    fn hostile_input_table() {
        let cases: &[(&str, PathVerdict)] = &[
            // The UNC prefix, any casing (the uncased form was ALLOWED once).
            (r"\\?\UNC\host\share\x.notes", PathVerdict::UnboundedNetwork),
            (r"\\?\unc\host\share\x.notes", PathVerdict::UnboundedNetwork),
            (r"\\?\unc\HOST\share\x.notes", PathVerdict::UnboundedNetwork),
            // The device namespace under the \\?\ root (was ALLOWED once).
            (r"\\?\PhysicalDrive0", PathVerdict::ReservedDevice),
            (
                r"\\?\GLOBALROOT\Device\HarddiskVolume3\x",
                PathVerdict::ReservedDevice,
            ),
            // COM0/LPT0 were never reserved (were OVER-REJECTED once).
            ("COM0", PathVerdict::Allowed),
            ("LPT0", PathVerdict::Allowed),
            // BLOCKER-1: drive-relative names are refused, and the bare
            // one-letter+colon corollary with them.
            (r"C:notes.notes", PathVerdict::DriveRelative),
            (r"a:b.notes", PathVerdict::DriveRelative),
            // BLOCKER-2 siblings: the rules hold on EVERY component - and EVERY
            // COMPONENT is the Win32 half of them. A POSIX path finds no separator
            // in these two strings at all, so `C:\CON\x.notes` is one legal
            // filename, CON names no device there, and `sub.` loses no dot:
            // Allowed is the CORRECT verdict on Linux, not a hole in the policy.
            // Every other row here is a raw-string law - a leading prefix, a colon,
            // a drive-relative name, a final `.` or `..` - which is why those stay
            // unconditional and why this table is the thing we want Linux to keep
            // judging.
            (
                r"C:\CON\x.notes",
                win32_name_law(PathVerdict::ReservedDevice),
            ),
            (
                r"C:\x\sub.\x.notes",
                win32_name_law(PathVerdict::StrippedName),
            ),
            (r"C:\x\notes\.", PathVerdict::StrippedName),
            (r"C:\x\notes\..", PathVerdict::StrippedName),
            // MAJOR-4: the EXT trailing-dot row is refused now, so it moved
            // out of the carve-outs list; a LEGITIMATE long path (no stripped
            // component) is still Allowed past the prefix.
            (r"\\?\C:\a.notes.", PathVerdict::StrippedName),
            // `\\?\C:\x:ads` is the raw-string colon law from the Windows-only case
            // above, so it lives where Linux keeps judging it.
            (r"\\?\C:\x:ads", PathVerdict::StreamName),
            (r"\\?\\dir\x.notes", PathVerdict::Allowed), // rooted verbatim form
            // ACCEPTED (audit item 4): Allowed-but-unwritable shapes. Win32
            // refuses both on write; no data moves; refusing them here would
            // need traversal semantics the name-only policy deliberately
            // does not own. Documented so no reader rediscovers them.
            (r"\\?\C:\x\..\y.notes", PathVerdict::Allowed), // mid-path
            (concat!(r"\\?\C:\x.notes", r"\"), PathVerdict::Allowed), // trailing backslash
            (r"\??\C:\x.notes", PathVerdict::ReservedDevice),
            (r"\\.\PhysicalDrive0", PathVerdict::ReservedDevice),
            // VOLUME-IDENTITY GATE: a volume-GUID name can never be
            // authorised for a write. If anyone widens the extended-prefix
            // carve-out so an identity string re-enters as an openable path,
            // this row fails - the guarantee expressed as a gate, not a
            // comment.
            (
                r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\note.notes",
                PathVerdict::ReservedDevice,
            ),
        ];
        for (p, expected) in cases {
            assert_eq!(v(Path::new(p)), *expected, "{p}");
        }
    }

    /// The B2 reparse predicate is defined ONCE here and both consumers
    /// call it: save::atomic_write (rename refusal) and
    /// session::ensure_state_dir (creation refusal). Behaviour tests alone
    /// cannot pin this — a fork passes them all — so the pin is
    /// source-level, exactly like the stripped-name pin.
    #[test]
    fn the_reparse_predicate_is_defined_once_and_shared() {
        const SAVE: &str = include_str!("save.rs");
        const SESSION: &str = include_str!("session.rs");
        const POLICY: &str = include_str!("path_policy.rs");
        assert!(
            SAVE.contains("path_policy::refuse_reparse_point(target)"),
            "the save engine must refuse through the shared predicate"
        );
        assert!(
            SESSION.contains("path_policy::metadata_is_reparse"),
            "ensure_state_dir must judge through the shared predicate"
        );
        assert_eq!(
            POLICY
                .matches(concat!("pub(crate) fn metadata", "_is_reparse"))
                .count(),
            1,
            "exactly one definition of the reparse fact"
        );
        assert_eq!(
            POLICY
                .matches(concat!("pub(crate) fn refuse", "_reparse_point"))
                .count(),
            1,
            "exactly one definition of the refusal"
        );
    }
}
