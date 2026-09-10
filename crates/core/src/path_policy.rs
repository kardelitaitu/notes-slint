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

use std::path::Path;

/// The decision for a path, from the NAME ALONE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathVerdict {
    /// No name-level hazard; the filesystem call may proceed.
    Allowed,
    /// A UNC path (\\no-such-host\share\x.notes, also //host/share and the extended \\?\UNC\host\share\x.notes
    /// form). POLICY, not a bug: the first touch of an unreachable server
    /// blocks for as long as the OS redirector pleases, and this engine is
    /// single-threaded — the app refuses the attempt instead of freezing
    /// the window. If a future build wants network notes, the lift is an
    /// async/timeout layer owned by api, not here.
    UnboundedNetwork,
    /// NT device namespaces (\\.\PhysicalDrive0, \??\C:\x.notes, \.\x) and the classic DOS
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
    /// The final component ends with '.' or ' ': Win32 strips both from the
    /// final component, so "a.notes." writes "a.notes" while the app reports
    /// Ok for a path that does not exist. One shared implementation with the
    /// save engine (final_component_is_stripped) so the two can never drift.
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
    // Network: the extended UNC form first, then the plain ones.
    if s.starts_with("\\\\?\\UNC\\") {
        return PathVerdict::UnboundedNetwork;
    }
    // Extended-length prefix: legitimate, and it BYPASSES Win32 name
    // mangling — so the StrippedName rule does not apply past it (a trailing
    // dot is a real character there). Everything else is judged normally.
    let (body, extended) = match s.strip_prefix("\\\\?\\") {
        Some(rest) => (rest, true),
        None => (s, false),
    };
    if body.starts_with("\\\\") || body.starts_with("//") {
        return PathVerdict::UnboundedNetwork;
    }
    // The drive separator is the ONE legitimate colon; anything after it
    // starts a stream. A relative colon ("a.notes:sneaky") is a stream from
    // the first colon.
    let after_drive = match drive_separator(body) {
        Some(n) => &body[n + 1..],
        None => body,
    };
    if after_drive.contains(':') {
        return PathVerdict::StreamName;
    }
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        // A root, a bare drive ("C:"), or "..": no final component to judge,
        // and none of the hazards live in the prefix.
        return PathVerdict::Allowed;
    };
    // Classic DOS devices, extensionless. With an extension modern Windows
    // makes an ordinary file (the tester's probe created and read back
    // CON.notes), so only the bare stem is refused — stated honestly, not
    // defensively. COM10+ was never in the reserved set.
    if !name.contains('.') && is_dos_device(&name) {
        return PathVerdict::ReservedDevice;
    }
    // Trailing dot/space mangling — skipped under the extended prefix.
    if !extended && final_component_is_stripped(&name) {
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

/// The classic MS-DOS device names. Modern Windows keeps the bare stem
/// reserved in every directory; COM/LPT take single digits 1-9.
fn is_dos_device(name: &str) -> bool {
    let up = name.to_ascii_uppercase();
    matches!(up.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((up.starts_with("COM") || up.starts_with("LPT"))
            && up.len() == 4
            && up.as_bytes()[3].is_ascii_digit())
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
            "..",
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

    #[test]
    fn extended_length_prefix_bypasses_only_the_stripping_rule() {
        // No Win32 mangling past the extended prefix: the dot is real.
        assert_eq!(v(Path::new(r"\\?\C:\a.notes.")), PathVerdict::Allowed);
        // But streams and devices are still judged.
        assert_eq!(v(Path::new(r"\\?\C:\x:ads")), PathVerdict::StreamName);
        assert_eq!(v(Path::new(r"\\?\C:\CON")), PathVerdict::ReservedDevice);
        // And the UNC-inside-extended form is still network.
        assert_eq!(
            v(Path::new(r"\\?\UNC\host\share\x")),
            PathVerdict::UnboundedNetwork
        );
    }

    #[test]
    fn the_shared_stripped_rule_is_the_one_save_uses() {
        for name in ["a.notes.", "a.notes ", "a.notes. "] {
            assert!(final_component_is_stripped(name));
        }
        for name in ["a.notes", "a.notes..x"] {
            assert!(!final_component_is_stripped(name));
        }
    }
}
