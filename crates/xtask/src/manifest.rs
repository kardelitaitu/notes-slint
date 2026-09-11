//! xtask manifest - ensure the SHIPPED exe carries OUR manifest, and prove it.
//!
//! Why a post-link step exists at all: with the toolkit moving to the gpui-kit
//! family, gpui's own \`windows-manifest\` feature is in its DEFAULT feature list
//! and is not subtractable from outside, and every shape that pulls the kit also
//! embeds a manifest - which collides at link time (duplicate resource ID 24) or,
//! in the one shape that links, silently ships GPUI'S manifest instead of ours.
//! Its readback differs from ours in the two ways that matter: \`longPathAware\`
//! and the \`system\` DPI fallback are gone, \`SegmentHeap\` arrives. That is an
//! unannounced change to the assumption under D40/D42, the scale refresh and the
//! clamp, so the manifest is written AFTER the link and then READ BACK.
//!
//! Verdicts, and why there is no skip path: the tool ENSURES and VERIFIES. It
//! never answers "already fine, did nothing", because a reader cannot tell a
//! skipped step from a working one. If the exe already carries our markers the
//! replace still runs (it is idempotent and cheap) and the line says so.
//!
//! Exit codes, for CI:
//! * 0 - replaced and verified: our assemblyIdentity and longPathAware are in the
//!   produced resource, read back from the file.
//! * 20 - mt.exe NOT FOUND (no Windows SDK here). Loud and distinct, because
//!   "success with nothing done" is the failure this whole module exists to kill.
//! * 21 - the read-back refused to judge: the exe could not be read, or the
//!   markers are missing after a run that reported no error.
//! * 22 - the tool could not run the work: no exe at the path given, the source
//!   manifest is unreadable, or mt.exe exited nonzero.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const MANIFEST_OK_EXIT: i32 = 0;
pub const NO_SDK_EXIT: i32 = 20;
pub const VERIFY_FAILED_EXIT: i32 = 21;
pub const CANNOT_RUN_EXIT: i32 = 22;

/// Where the SDK puts the manifest tool. Version-independent on purpose:
/// hard-coding 10.0.26100.0 makes the tool break the day the runner image is
/// updated, which is the same class of drift as a name list that has to be
/// edited on every upstream rename.
/// Joined to Program Files (x86), which is where the kit actually lives. A
/// leftover 'Program Files (x86)' inside this constant makes the search a
/// self-nesting path that finds nothing and reports NO SDK - the false verdict
/// this exit code exists to distinguish from a real absence.
const SDK_BIN_REL: &str = "Windows Kits/10/bin";
/// The resource id a PE's manifest lives under. 24 is CREATEPROCESS_MANIFEST_RESOURCE_ID.
const MANIFEST_RESOURCE_ID: &str = "1";
/// The manifest we ship, relative to the workspace root.
/// Published so smoke can ask the same question of the exe it is about to
/// launch without duplicating the path or the marker list.
pub const APP_MANIFEST_REL: &str = "crates/bridge-gpui/app.manifest";
/// The exe this step protects, relative to the workspace root.
const EXE_REL: &str = "target/debug/notes-gpui.exe";

/// Everything the read-back can say about the produced exe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Markers {
    /// The assemblyIdentity name from OUR source manifest, found in the exe.
    pub identity: bool,
    pub long_path: bool,
    /// PerMonitorV2 is the DPI claim D40/D42 rest on.
    pub per_monitor: bool,
}

impl Markers {
    pub fn all_present(&self) -> bool {
        self.identity && self.long_path && self.per_monitor
    }
}

/// The read-back decision as a function rather than an inline 'if', because an
/// inline check is exactly what a later edit can hollow out unnoticed: this one
/// has a test per marker, so deleting the check turns a test red instead of
/// turning CI green.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadBack {
    Ours,
    Missing(Vec<&'static str>),
}

pub fn read_back(markers: &Markers) -> ReadBack {
    let mut missing: Vec<&'static str> = Vec::new();
    if !markers.identity {
        missing.push("assemblyIdentity");
    }
    if !markers.long_path {
        missing.push("longPathAware");
    }
    if !markers.per_monitor {
        missing.push("PerMonitorV2");
    }
    if missing.is_empty() {
        ReadBack::Ours
    } else {
        ReadBack::Missing(missing)
    }
}

/// What the source manifest promises. Parsed rather than hard-coded, so the
/// checker cannot disagree with the file it is enforcing: if app.manifest loses
/// longPathAware, that is caught here rather than at the read-back.
pub fn source_promise(text: &str) -> Result<(String, Vec<String>), String> {
    let identity = {
        let Some(at) = text.find("assemblyIdentity") else {
            return Err("no assemblyIdentity in the source manifest".to_string());
        };
        let tail = &text[at..];
        let Some(n) = tail.find("name=\"") else {
            return Err("assemblyIdentity has no name attribute".to_string());
        };
        let rest = &tail[n + "name=\"".len()..];
        let Some(end) = rest.find('"') else {
            return Err("unterminated assemblyIdentity name".to_string());
        };
        rest[..end].to_string()
    };
    let mut wanted: Vec<String> = Vec::new();
    for needle in ["longPathAware", "PerMonitorV2"] {
        if text.contains(needle) {
            wanted.push(needle.to_string());
        }
    }
    if wanted.is_empty() {
        return Err("the source manifest names neither longPathAware nor PerMonitorV2".to_string());
    }
    Ok((identity, wanted))
}

/// Search the raw bytes for a UTF-8 marker. The manifest resource is XML text,
/// so a byte search finds it without a PE parser; a miss is reported as absent,
/// never as "could not tell".
pub fn bytes_contain(hay: &[u8], needle: &str) -> bool {
    let n = needle.as_bytes();
    if n.is_empty() || hay.len() < n.len() {
        return false;
    }
    hay.windows(n.len()).any(|w| w == n)
}

pub fn read_markers(exe_bytes: &[u8], identity: &str, wanted: &[String]) -> Markers {
    let mut m = Markers {
        identity: bytes_contain(exe_bytes, identity),
        long_path: false,
        per_monitor: false,
    };
    for w in wanted {
        if w == "longPathAware" {
            m.long_path = bytes_contain(exe_bytes, w);
        } else if w == "PerMonitorV2" {
            m.per_monitor = bytes_contain(exe_bytes, w);
        }
    }
    m
}

/// Newest-first ordering over mt.exe candidates, by the version directory. A
/// lexicographic sort would put 10.0.26100.0 before 10.0.9000.0 wrongly, so the
/// version is compared numerically where it can be.
pub fn pick_mt_exe(mut candidates: Vec<PathBuf>) -> Option<PathBuf> {
    candidates.sort_by(|a, b| version_key(a).cmp(&version_key(b)).reverse());
    candidates.into_iter().next()
}

fn version_key(path: &Path) -> Vec<u64> {
    let text = path.display().to_string();
    let mut nums: Vec<u64> = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            cur.push(c);
        } else {
            if !cur.is_empty() {
                nums.push(cur.parse().unwrap_or(0));
                cur.clear();
            }
        }
    }
    if !cur.is_empty() {
        nums.push(cur.parse().unwrap_or(0));
    }
    nums
}

/// Where the SDK lives. Read from the environment rather than hard-coded, so the
/// tool survives a runner image that puts Windows Kits elsewhere.
pub fn program_files() -> PathBuf {
    for key in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Ok(value) = std::env::var(key) {
            if !value.is_empty() {
                return PathBuf::from(value);
            }
        }
    }
    PathBuf::from(r"C:\Program Files (x86)")
}

/// Every mt.exe under the SDK bin directory, newest candidate unsorted.
pub fn find_mt_exe(root: &Path) -> (Option<PathBuf>, Vec<PathBuf>) {
    let bin = root.join(SDK_BIN_REL);
    let found: Vec<PathBuf> = match fs::read_dir(&bin) {
        Err(_) => return (None, Vec::new()),
        Ok(entries) => entries
            .flatten()
            .filter_map(|e| {
                let candidate = e.path().join("x64").join("mt.exe");
                candidate.is_file().then_some(candidate)
            })
            .collect(),
    };
    (pick_mt_exe(found.clone()), found)
}

/// The verdict, so run() can print one line per outcome and pick a code.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Verified {
        exe: PathBuf,
        mt: PathBuf,
        before: u64,
        after: u64,
        markers: Markers,
        was_already_ours: bool,
    },
    NoSdk {
        searched: String,
    },
    Refused(String),
    CouldNotRun(String),
}

pub fn ensure(root: &Path, exe: &Path) -> Outcome {
    let manifest_path = root.join(APP_MANIFEST_REL);
    let source = match fs::read_to_string(&manifest_path) {
        Ok(t) => t,
        Err(e) => {
            return Outcome::CouldNotRun(format!("cannot read {}: {e}", manifest_path.display()));
        }
    };
    let (identity, wanted) = match source_promise(&source) {
        Ok(v) => v,
        Err(e) => {
            return Outcome::CouldNotRun(format!(
                "the source manifest is not readable as ours: {e}"
            ));
        }
    };
    let before = match fs::metadata(exe) {
        Ok(md) => md.len(),
        Err(e) => return Outcome::CouldNotRun(format!("no exe at {}: {e}", exe.display())),
    };
    let before_bytes = match fs::read(exe) {
        Ok(b) => b,
        Err(e) => {
            return Outcome::Refused(format!("cannot read the exe back before touching it: {e}"));
        }
    };
    let was_already_ours = read_markers(&before_bytes, &identity, &wanted).all_present();
    let program_files = program_files();
    let (mt, candidates) = find_mt_exe(&program_files);
    let Some(mt) = mt else {
        return Outcome::NoSdk {
            searched: format!(
                "{}/*/x64/mt.exe ({} candidates)",
                program_files.display(),
                candidates.len()
            ),
        };
    };
    // mt.exe parses this itself and wants the colon form as ONE token: passing
    // "-outputresource" and its value as separate arguments is reported as a
    // missing option (measured: c1010008, exit 31). The semicolon is inside the
    // value, which is why the probe's shell needed quotes and we do not - there
    // is no shell here, so no quoting layer to get wrong.
    let resource = format!(
        "-outputresource:{};#{}",
        exe.display(),
        MANIFEST_RESOURCE_ID
    );
    let status = Command::new(&mt)
        .args(["-manifest", &manifest_path.display().to_string(), &resource])
        .status();
    match status {
        // Exit 0 is not the verdict, it is only permission to look: what counts
        // is the read-back below, which is the whole reason this step exists.
        Ok(s) if s.success() => {}
        Ok(s) => {
            return Outcome::CouldNotRun(format!("mt.exe exited {s}"));
        }
        Err(e) => {
            return Outcome::CouldNotRun(format!("could not run {}: {e}", mt.display()));
        }
    }
    let after = fs::metadata(exe).map(|md| md.len()).unwrap_or(0);
    let bytes = match fs::read(exe) {
        Ok(b) => b,
        Err(e) => {
            return Outcome::Refused(format!(
                "the exe cannot be read back after the replace: {e}"
            ));
        }
    };
    let markers = read_markers(&bytes, &identity, &wanted);
    if let ReadBack::Missing(absent) = read_back(&markers) {
        return Outcome::Refused(format!(
            "mt.exe reported success but the read-back disagrees: no {} in the produced resource - \
             refusing to call this a pass",
            absent.join(", ")
        ));
    }
    Outcome::Verified {
        exe: exe.to_path_buf(),
        mt,
        before,
        after,
        markers,
        was_already_ours,
    }
}

/// Entry point for "cargo xtask manifest [path-to-exe]".
pub fn run(args: &[String]) -> i32 {
    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("manifest: cannot read the current directory: {e}");
            return CANNOT_RUN_EXIT;
        }
    };
    let root = match crate::metadata::find_workspace_root(&cwd) {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("manifest: {e}");
            return CANNOT_RUN_EXIT;
        }
    };
    let mut positional: Option<&String> = None;
    for arg in args {
        if arg.starts_with('-') {
            eprintln!("manifest: unsupported flag '{arg}' - this subcommand takes no flags");
            eprintln!("manifest: usage: cargo xtask manifest [PATH-TO-EXE]");
            return CANNOT_RUN_EXIT;
        }
        if positional.is_some() {
            eprintln!("manifest: more than one exe path given ('{arg}')");
            return CANNOT_RUN_EXIT;
        }
        positional = Some(arg);
    }
    let exe: PathBuf = match positional {
        Some(p) => PathBuf::from(p),
        None => root.join(EXE_REL),
    };
    match ensure(&root, &exe) {
        Outcome::Verified {
            exe,
            mt,
            before,
            after,
            markers,
            was_already_ours,
        } => {
            println!(
                "manifest: PASS - {} | mt.exe {} | size {before} -> {after} | already-ours={} | \
                 identity={} longPathAware={} PerMonitorV2={}",
                exe.display(),
                mt.display(),
                was_already_ours,
                markers.identity,
                markers.long_path,
                markers.per_monitor
            );
            MANIFEST_OK_EXIT
        }
        Outcome::NoSdk { searched } => {
            eprintln!("manifest: NO SDK - mt.exe was not found searching {searched}");
            eprintln!(
                "manifest: nothing was written, so this is NOT a pass: the exe still carries \
                       whatever the link embedded"
            );
            NO_SDK_EXIT
        }
        Outcome::Refused(why) => {
            eprintln!("manifest: REFUSED TO JUDGE - {why}");
            VERIFY_FAILED_EXIT
        }
        Outcome::CouldNotRun(why) => {
            eprintln!("manifest: CANNOT RUN - {why}");
            CANNOT_RUN_EXIT
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OURS: &str = r#"<assemblyIdentity version="1.0.0.0" name="NotesGpui.App" type="win32"/>
      <dpiAware>true</dpiAware><dpiAwareness>PerMonitorV2, system</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>"#;

    #[test]
    fn the_source_manifest_promise_is_parsed_not_assumed() {
        let (identity, wanted) = source_promise(OURS).expect("promise");
        assert_eq!(identity, "NotesGpui.App");
        assert_eq!(
            wanted,
            vec!["longPathAware".to_string(), "PerMonitorV2".to_string()]
        );
        assert!(
            source_promise("<xml/>").is_err(),
            "no identity is a refusal, not a default"
        );
        assert!(source_promise(r#"name="X""#).is_err());
    }

    #[test]
    fn markers_are_read_from_the_exe_bytes() {
        let exe = OURS.as_bytes().to_vec();
        let m = read_markers(
            &exe,
            "NotesGpui.App",
            &["longPathAware".to_string(), "PerMonitorV2".to_string()],
        );
        assert!(m.all_present(), "{m:?}");
        let theirs = exe.iter().take(80).cloned().collect::<Vec<u8>>();
        let m2 = read_markers(
            &theirs,
            "NotesGpui.App",
            &["longPathAware".to_string(), "PerMonitorV2".to_string()],
        );
        assert!(
            !m2.all_present(),
            "a truncated exe must not read as ours: {m2:?}"
        );
        assert!(
            !read_markers(b"", "X", &[]).identity,
            "an empty file is not a pass"
        );
    }

    #[test]
    fn the_newest_mt_exe_wins_not_the_lexicographic_one() {
        let older = PathBuf::from(r"C:\Sdk\bin\10.0.19041.0\x64\mt.exe");
        let newer = PathBuf::from(r"C:\Sdk\bin\10.0.26100.0\x64\mt.exe");
        assert_eq!(
            pick_mt_exe(vec![older.clone(), newer.clone()]),
            Some(newer.clone())
        );
        assert_eq!(
            pick_mt_exe(vec![newer.clone(), older.clone()]),
            Some(newer),
            "order in must equal order out"
        );
        assert_eq!(
            pick_mt_exe(vec![
                PathBuf::from(r"C:\Sdk\bin\10.0.9000.0\x64\mt.exe"),
                PathBuf::from(r"C:\Sdk\bin\10.0.26100.0\x64\mt.exe"),
            ]),
            Some(PathBuf::from(r"C:\Sdk\bin\10.0.26100.0\x64\mt.exe")),
            "9 > 2 lexicographically; the version must be compared numerically"
        );
        assert_eq!(
            pick_mt_exe(Vec::new()),
            None,
            "no candidates is no tool, not a panic"
        );
    }

    #[test]
    fn discovery_against_a_fake_tree_finds_only_real_files() {
        let root = std::env::temp_dir().join(format!("xtask-mt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let good = root.join(SDK_BIN_REL).join("10.0.26100.0").join("x64");
        fs::create_dir_all(&good).expect("dir");
        fs::write(good.join("mt.exe"), b"not really mt").expect("exe");
        let empty = root.join(SDK_BIN_REL).join("10.0.99999.0").join("x64");
        fs::create_dir_all(&empty).expect("dir2");
        let (picked, all) = find_mt_exe(&root);
        assert_eq!(picked, Some(good.join("mt.exe")), "{all:?}");
        assert_eq!(
            all.len(),
            1,
            "a version dir without mt.exe is not a candidate: {all:?}"
        );
        let (none, empty_list) = find_mt_exe(&root.join("nowhere"));
        assert_eq!(none, None);
        assert!(empty_list.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_exe_is_cannot_run_and_a_bad_source_manifest_refuses() {
        let root = std::env::temp_dir().join(format!("xtask-mf-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("crates/bridge-gpui")).expect("dir");
        fs::write(root.join(APP_MANIFEST_REL), OURS).expect("manifest");
        let out = ensure(&root, &root.join("nope.exe"));
        assert!(matches!(out, Outcome::CouldNotRun(_)), "{out:?}");
        fs::write(root.join(APP_MANIFEST_REL), b"<xml/>").expect("junk");
        assert!(matches!(
            ensure(&root, &root.join("nope.exe")),
            Outcome::CouldNotRun(_)
        ));
        let _ = fs::remove_dir_all(&root);
    }

    /// The whole point of the module, asserted where a hollow verification would
    /// be caught: if the read-back is not consulted, this test goes red.
    #[test]
    fn success_from_mt_without_the_markers_is_never_a_pass() {
        let markers = read_markers(
            b"<?xml version=\"1.0\"?><assemblyIdentity name=\"Other\"/>",
            "NotesGpui.App",
            &["longPathAware".to_string(), "PerMonitorV2".to_string()],
        );
        assert!(!markers.identity);
        assert!(!markers.all_present(), "a foreign manifest must not pass");
    }

    /// Each marker has its own red: a verification that can be hollowed out
    /// without any test noticing is the decorative-instrument class, and this is
    /// the third time tonight that class has been the subject.
    #[test]
    fn the_read_back_names_what_is_absent_rather_than_trusting_mt() {
        let all = Markers {
            identity: true,
            long_path: true,
            per_monitor: true,
        };
        assert_eq!(read_back(&all), ReadBack::Ours);
        let no_long = Markers {
            identity: true,
            long_path: false,
            per_monitor: true,
        };
        assert_eq!(
            read_back(&no_long),
            ReadBack::Missing(vec!["longPathAware"])
        );
        let theirs = Markers {
            identity: false,
            long_path: false,
            per_monitor: true,
        };
        let ReadBack::Missing(absent) = read_back(&theirs) else {
            panic!("a foreign manifest is never Ours");
        };
        assert_eq!(absent, vec!["assemblyIdentity", "longPathAware"]);
        let bare = Markers {
            identity: false,
            long_path: false,
            per_monitor: false,
        };
        assert_eq!(
            read_back(&bare),
            ReadBack::Missing(vec!["assemblyIdentity", "longPathAware", "PerMonitorV2"]),
            "nothing present must name all three, not fail quietly"
        );
    }
}
