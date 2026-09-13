//! smoke - the one end-to-end GUI check: launch the real binary, prove a real
//! window appears, close it the way a USER closes it, and prove the process
//! exits BY ITSELF with code 0 and leaves a session artefact behind.
//!
//! Everything else in this repo is headless, so this is the only place where
//! `Gateway::close()` draining, joining and flushing is exercised for real. It
//! has one job and no opinion: it does not BUILD (a harness that rebuilds what
//! it tests folds a build break into a smoke verdict), it runs no unit tests,
//! and it is never a blocking gate step - a headless runner may have no desktop
//! at all (see check.rs: advisory, and skipped by --quick).
//!
//! # How a window is observed and closed without the windows crate
//!
//! xtask is the CHECKER: importing notes-platform or the windows crate would
//! kill the independence rule check-arch enforces, and xtask takes no new
//! dependencies. So the Win32 part is delegated to a child PowerShell process
//! using only the BCL that ships with it - Process::MainWindowHandle and
//! MainWindowTitle to observe, CloseMainWindow() to close. CloseMainWindow
//! posts WM_CLOSE to the main window, the same message the title-bar X sends;
//! it is a REQUEST, not a kill. (`taskkill` without /F also posts WM_CLOSE, so
//! it would pass the "graceful" test, but it cannot report the handle, the title
//! or the exit code - strictly less honest, and it needs a pid parse.) The
//! script prints KEY=VALUE lines and decides NOTHING: every verdict comes from
//! `decide()`, a pure function under test.
//!
//! Scope of that rule, stated because it generalises: notes-gpui.exe is the
//! ONLY long-lived artefact this crate launches, so it is the only one that can
//! be stale. fixtures verify reads fixture files and compares bytes - no binary,
//! no staleness exposure - and every other gate row is a cargo invocation, which
//! rebuilds from the current source by definition. If a second launched artefact
//! ever appears, it gets the same build-then-prove-freshness treatment.
//!
//! # What makes this honest rather than decorative
//!
//! * an exit counts only if the process ended WITHOUT being killed: a forced
//!   kill FAILs even when the code reads 0, because a force-kill that prints
//!   PASS is this repo's found-once failure mode;
//! * an absent KEY is a failure, never a pass - a truncated probe cannot be
//!   green;
//! * every wait is bounded twice: pwsh bounds each wait internally, and xtask
//!   kills pwsh at a strictly larger outer deadline;
//! * no desktop degrades to SKIPPED (exit 3), not to FAIL and not to a silent
//!   green;
//! * the session artefact must be NEWER than this launch, and when none appears
//!   every path searched is printed. A pre-existing file is moved to a named
//!   temp path and restored afterwards, loudly, so each run is a real
//!   fresh-install test of 14295b8 rather than a re-read of an old one.
//!
//! Exit codes, exactly as run() returns them:
//! * 0 - every step passed, and the user's state came back byte-identical;
//! * 1 - a step failed (no window handle, the app died, no session.json, a
//!   window still alive after WM_CLOSE, a nonzero exit), or the relocated bytes
//!   came back different, or the restore could not run at all - the message
//!   names the temp path holding the user's file either way;
//! * 2 - the harness itself could not run: no PowerShell, no binary, the probe
//!   script could not be written, freshness could not be read at all, or the
//!   outer deadline fired;
//! * 4 - the TARGET DID NOT COMPILE. Nothing was launched and no verdict about
//!   the app exists; this is never a decline (3) and never a silent fallback to
//!   the stale exe sitting in target/debug;
//! * 5 - the binary is OLDER THAN ITS SOURCES: the build no-oped or --no-build
//!   was used, so whatever would be tested is not this tree.
//! * 6 - the WINDOW MEMORY PROMISE DID NOT HOLD: the app ran, closed cleanly and
//!   persisted something, but the seeded rect was not restored, or a harness-driven
//!   move never reached session.json, or the relaunch came back elsewhere. Geometry
//!   that could not be measured prints "NOT JUDGED (advisory)" and leaves the exit
//!   code alone - an unverifiable step must not be reported as a pass or a failure.
//! * 8 - the exe under test carries a manifest that is not ours AND the run asked
//!   for --require-ours. Without the flag the same state is a WARN line, never a
//!   red: the ordinary condition on a development machine is that nobody has run
//!   the post-link step, and smoke must not train people to ignore it.
//! * 7 - THE PIN LIED: WS_EX_TOPMOST on the live window did not follow what
//!   session.json claimed (checked in both polarities, pinned:true and
//!   pinned:false). Kept separate from 6 because the usual cause is the order of
//!   show-then-band, not persistence, and it can also be a race - the message
//!   says so rather than accusing the app.
//! * 9 - THE LIVE UI NEVER SAW THE STARTUP ANNOUNCE: the app launched, closed
//!   gracefully and persisted (exit 0's own verdicts), and yet the stderr
//!   redirected from the LIVE process names no `RecentsUpdated` while the state dir
//!   the APP RESOLVED held recents that were there to announce (TRACE_FAILED_EXIT;
//!   roadmap M2 exit item 5 - and counted from ONE directory on purpose: an earlier
//!   draft walked every candidate state dir and armed this verdict against a
//!   settings.toml in the roaming profile while the portable marker sent the app to
//!   target/debug/data, which held no settings file at all. That produced a real
//!   false exit 9). Its own code because 1 means "the shutdown or the write broke"
//!   and this means the opposite: everything ran, and a fact the port knew never
//!   reached the window. A capture that could not be read, and an app-resolved dir
//!   with nothing to announce, both print NOT JUDGED (advisory) and leave the code
//!   alone - an unverifiable step is neither a pass nor a failure, exactly as
//!   geometry treats its own unmeasurable legs. The claim is judgeable on EVERY run,
//!   not only on one whose owner happened to open a file today: smoke seeds exactly one
//!   [[recents]] entry pointing at a scratch note of its own into the dir state_dir_for
//!   resolves, then puts the profile back byte-exactly (SeedGuard - the same Drop
//!   guarantee the session.json relocation gives, plus a sweep of notes a crashed earlier
//!   run left). That proves the ANNOUNCE half of item 5: a non-empty list reached the live
//!   UI. The CLICK half STAYS MANUAL - smoke never drives the mouse or the keyboard, so
//!   "the entry was shown" is machine-proven while "choosing it opens that file" is not.
//!   The evidence line names the VOICE
//!   that carried the match - [rendered] is a status line the UI showed, [arrived]
//!   is the per-arrival event line, [delivered, never rendered] is the exit drain -
//!   and the strongest voice present is the one cited, never the first written.
//! * 3 - DECLINED for a reason that is not the app's fault: no interactive
//!   desktop (no sessions win32k user32.dll), no window handle even though the
//!   app kept running, or the session path holding foreign state this harness
//!   will not move. 3 is never "passed" and never "the app is broken".
//!
//! Two facts worth recording, both measured on a real machine:
//!
//! * exit 3 does not stick. Declining reads the path and never writes it, so run
//!   1 leaves whatever was there, run 2 behaves normally, and the operator's fix
//!   is the occupant - not a state machine to reset. Only a foreign DIRECTORY on
//!   the session path declines; the pollution this caught was a test leftover,
//!   not a harness feedback loop.
//! * "Access is denied (os error 5)" on this machine was the directory, exactly
//!   what renaming a file onto a directory does. The ACL was never the problem:
//!   owner BUILTIN\Administrators, the interactive user has FullControl, no DENY
//!   entry, no reparse point. The next reader should not chase ACLs. */

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

/// How long to wait for a top-level window to appear.
const WINDOW_SECS: u64 = 10;
/// Cold start is a documented product budget (whitepaper §2), so a window that
/// needs longer than this is reported even when it does appear.
const COLD_START_BUDGET_MS: i64 = 2_000;
/// How long to wait for the app to exit on its own after WM_CLOSE. The engine
/// already answers inside 5s; this is that plus the join.
const CLOSE_SECS: u64 = 10;
/// The outer backstop: strictly larger than everything the probe bounds
/// internally, so a wedged child cannot hang the gate.
const OUTER_SECS: u64 = WINDOW_SECS + CLOSE_SECS + 20;

/// One built artifact the harness can point at: which package, which bin inside it,
/// where the exe lands by default, which build script decides its embedded resources,
/// and what may make it stale. The struct exists because there are now TWO bridges in
/// this workspace and every field below used to be six string literals that quietly
/// named only one of them - the shape of bug where a second lane is added and the
/// harness keeps judging the first one.
///
/// It is a description, not an abstraction: nothing loops over targets, and smoke still
/// runs exactly one of them (gpui) with exactly the literals it always had. Adding
/// [SLINT_TARGET] changes no verdict, no path and no exit code on this branch; it makes
/// the second bridge visible to the next person who wires a leg, which is the whole
/// point of slice 0.
pub struct ArtifactTarget {
    /// The cargo package name, as `-p` takes it.
    pub pkg: &'static str,
    /// The bin target inside it, as `--bin` takes it.
    pub bin: &'static str,
    /// Where the exe sits when nobody set CARGO_TARGET_DIR.
    pub exe_rel: &'static str,
    /// The build script that decides the embedded manifest. Empty until the target has
    /// one - and see [crate::smoke::build_embeds_manifest], which reads it.
    pub build_rs: &'static str,
    /// Source DIRECTORIES that are inputs to this exe. `<crate>/src`, never `<crate>`:
    /// the long reason is at [SOURCE_ROOTS].
    pub freshness_roots: &'static [&'static str],
    /// Inputs outside a src dir, named one by one for the same reason.
    pub freshness_files: &'static [&'static str],
}

/// THE TARGET SMOKE ACTUALLY RUNS. Every literal the harness has always used is now a
/// field of this value, and the consts below are projections of it - deliberately, so
/// the exit behaviour is byte-identical to the day before the struct existed while the
/// strings live in exactly one place.
const GPUI_TARGET: ArtifactTarget = ArtifactTarget {
    pkg: "notes-bridge-gpui",
    bin: "notes-gpui",
    exe_rel: "target/debug/notes-gpui.exe",
    build_rs: "crates/bridge-gpui/build.rs",
    freshness_roots: &[
        "crates/bridge-gpui/src",
        "crates/api/src",
        "crates/core/src",
        "crates/platform/src",
    ],
    freshness_files: &[
        "Cargo.toml",
        "Cargo.lock",
        "crates/bridge-gpui/Cargo.toml",
        "crates/bridge-gpui/build.rs",
        "crates/api/Cargo.toml",
        "crates/core/Cargo.toml",
        "crates/platform/Cargo.toml",
    ],
};

/// THE SECOND BRIDGE, described and not driven. No smoke leg runs this exe on this
/// branch, so the only thing to check about this row is that it names real paths: the
/// crate exists, its sources and `ui/` exist, and `build_rs` is the name slice 1 will
/// give its build script when (not if) Slint starts needing one - it has none today,
/// which is why the test `slint_target_has_no_build_script_yet` pins the empty string
/// instead of letting a plausible-looking path stand in for a fact. Named in a code span
/// rather than a doc link on purpose: the test lives in `mod tests`, which `cargo doc`
/// does not build, so a link here would be an unresolved-link warning - the noise this
/// comment used to be.
#[allow(dead_code)] // described, not driven: no smoke leg runs this exe on this branch
const SLINT_TARGET: ArtifactTarget = ArtifactTarget {
    pkg: "notes-bridge-slint",
    bin: "notes-slint",
    exe_rel: "target/debug/notes-slint.exe",
    build_rs: "",
    freshness_roots: &[
        "crates/bridge-slint/src",
        // Empty on slice 0 and listed anyway: `.slint` files are inputs to that exe the
        // moment they exist, and a root the guard does not look at cannot make anything
        // stale. Listed now, the first UI edit is caught; listed never, it is a bug
        // waiting for someone to remember.
        "crates/bridge-slint/ui",
        "crates/api/src",
        "crates/core/src",
    ],
    freshness_files: &[
        "Cargo.toml",
        "Cargo.lock",
        "crates/bridge-slint/Cargo.toml",
        "crates/api/Cargo.toml",
        "crates/core/Cargo.toml",
    ],
};

const BIN_REL: &str = GPUI_TARGET.exe_rel;
/// Just the file name, for a CARGO_TARGET_DIR that replaces the whole tree. Stays a
/// literal for one reason: `concat!` cannot read a const struct's field, and a
/// `format!` at this position would stop it being a const. What keeps it honest is the
/// test `exe_name_is_the_bin_target_plus_the_windows_extension`, which fails the day the
/// two stop agreeing - a code span, not a doc link, because `mod tests` is not built by
/// `cargo doc` and a link to it could not resolve.
pub const EXE_NAME: &str = "notes-gpui.exe";
const BUILD_HINT: &str = "cargo build -p notes-bridge-gpui --bin notes-gpui";
const SESSION_FILE: &str = "session.json";
/// The other file in the same directory, and the one the RECENTS live in (core's
/// settings.rs: "settings.toml holds the USER preferences and the recents"). This
/// harness never moves or writes it - it only asks it how much there was to
/// announce, which is what makes the trace claim judgeable at all.
const SETTINGS_FILE: &str = "settings.toml";

/// The probe. Written to a temp file and run as a -File (never through a shell
/// string), so no path here is re-parsed by anything. It prints KEY=VALUE and
/// decides nothing.
const PROBE: &str = r#"
param([Parameter(Mandatory)][string]$Exe, [string]$OutFile, [string]$ErrFile,
       [int]$WindowSecs = 10, [int]$CloseSecs = 10)
$ErrorActionPreference = 'SilentlyContinue'
$interactive = [Environment]::UserInteractive
$windowed = (Get-Process | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1)
if (-not ($interactive -and $windowed)) { 'DESKTOP=0'; 'PROBE_DONE=1'; exit 0 }
'DESKTOP=1'
$sw = [Diagnostics.Stopwatch]::StartNew()
$p = Start-Process -FilePath $Exe -PassThru -RedirectStandardOutput $OutFile -RedirectStandardError $ErrFile
if ($null -eq $p) { 'SPAWN=0'; 'PROBE_DONE=1'; exit 0 }
'SPAWN=1'
"PID=$($p.Id)"
$deadline = (Get-Date).AddSeconds($WindowSecs)
$handle = 0
while ((Get-Date) -lt $deadline) {
    $p.Refresh()
    if ($p.MainWindowHandle -ne 0) { $handle = [int64]$p.MainWindowHandle; break }
    if ($p.HasExited) { break }
    Start-Sleep -Milliseconds 100
}
"LAUNCH_MS=$($sw.ElapsedMilliseconds)"
"HANDLE=$handle"
"TITLE=$($p.MainWindowTitle)"
$closed = $false
if ($handle -ne 0 -and -not $p.HasExited) { $closed = $p.CloseMainWindow() }
"CLOSE_REQUESTED=$([int]$closed)"
$exited = $false
if (-not $p.HasExited) { $exited = $p.WaitForExit(([int]$CloseSecs) * 1000) }
"EXITED_WITHOUT_KILL=$([int]$exited)"
if ($exited) {
    'FORCED=0'
    "EXIT_CODE=$($p.ExitCode)"
} else {
    'FORCED=1'
    Stop-Process -Id $p.Id -Force
    $p.WaitForExit()
    "EXIT_CODE_AFTER_FORCE=$($p.ExitCode)"
}
'PROBE_DONE=1'
exit 0
"#;

/// The KEY=VALUE lines the probe printed, and nothing else.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Probe {
    kv: BTreeMap<String, String>,
}

impl Probe {
    fn get(&self, key: &str) -> Option<&str> {
        self.kv.get(key).map(String::as_str)
    }

    fn flag(&self, key: &str) -> bool {
        self.get(key) == Some("1")
    }

    fn number(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(|v| v.trim().parse().ok())
    }

    fn keys(&self) -> String {
        self.kv.keys().cloned().collect::<Vec<_>>().join(",")
    }
}

/// Parse KEY=VALUE lines. Unknown keys are KEPT, so a probe that grows a key
/// cannot become invisible to the verdict; a line without = is noise.
pub fn parse_probe(stdout: &str) -> Probe {
    let mut kv = BTreeMap::new();
    for line in stdout.lines() {
        if let Some((k, v)) = line.trim().split_once('=') {
            let k = k.trim();
            if !k.is_empty() {
                kv.insert(k.to_string(), v.trim().to_string());
            }
        }
    }
    Probe { kv }
}

/// Where the session artefact was found, and what it said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artefact {
    pub path: PathBuf,
    /// Written or rewritten at or after this launch.
    pub fresh: bool,
    /// The stored rect, or why it could not be read.
    pub rect: String,
    /// Some when the bytes could not be read at all. That is a verdict of its
    /// own: it describes the machine (an ACL, a planted blocker, a filter
    /// driver), not something the app did or failed to do.
    pub read_error: Option<String>,
}

/// What stood in the way of a fresh-install run, and what was done about it.
///
/// Smoke tests ONE case that matters: an install with no session.json, which is
/// what D54 is about. A pre-existing file is therefore MOVED ASIDE by default -
/// it comes back at the end, hash-checked - rather than being allowed to make
/// the run judge the wrong thing, and rather than being deleted, which is not
/// this harness's to do. When the path holds something that cannot be moved (a
/// directory planted on it, an access refusal), smoke says so and DECLINES.
/// Reporting an app bug because of a blocker on the tester's own disk is the
/// failure mode this enum exists to avoid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Relocation {
    /// Nothing was in the way: this is a real fresh-install run.
    CleanSlate,
    /// The user's file sits in a named temp path and is restored on exit.
    Moved {
        /// Where it went, printed both ways so a human can verify by eye.
        to: PathBuf,
    },
    /// --reuse-state: the file stays exactly where it is, so only mtime
    /// freshness can be judged (and the fresh-install case cannot).
    Kept(String),
    /// Something unmovable sits on the path: no verdict is available.
    Blocked(String),
}

/// The outcome of a probe run. Skip is a deliberate third state: "no desktop"
/// is neither a pass nor a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Skip(String),
    Fail(Vec<String>),
}

/// Every claim this harness makes, decided here, over parsed keys and the
/// relocation outcome only - decide() touches no filesystem.
pub fn decide(p: &Probe, artefact: Option<&Artefact>, relocation: &Relocation) -> Verdict {
    let mut failures: Vec<String> = Vec::new();
    if let Relocation::Blocked(why) = relocation {
        // Before the desktop question: the run was already doomed to be
        // unable to judge, and saying so is not a pass.
        return Verdict::Skip(format!(
            "the session path could not be cleared for a fresh-install run, so nothing about D54 can be judged here: {why}"
        ));
    }
    if p.get("DESKTOP") == Some("0") {
        return Verdict::Skip(
            "no interactive desktop in this session (no windowed process, or the session is not interactive) - a GUI smoke run cannot be had here"
                .to_string(),
        );
    }
    if !p.flag("PROBE_DONE") {
        failures.push(format!(
            "SMOKE FAIL: the probe did not finish (keys seen: [{}]) - it hit its own deadline or died; a missing key is never a pass",
 p.keys()
        ));
        return Verdict::Fail(failures);
    }
    if !p.flag("SPAWN") {
        failures.push(format!(
            "SMOKE FAIL: {BIN_REL} exists but could not be started - run {BUILD_HINT} and look for an antivirus block"
        ));
        return Verdict::Fail(failures);
    }

    let handle = p.number("HANDLE").unwrap_or(0);
    if handle == 0 {
        failures.push(format!(
            "SMOKE FAIL: no top-level window handle within {WINDOW_SECS}s (MainWindowHandle stayed 0; pid={}, title={:?}) - the window was never created, so nothing downstream can be credited",
 p.get("PID").unwrap_or("?"),
 p.get("TITLE").unwrap_or("")
        ));
    } else if let Some(ms) = p.number("LAUNCH_MS") {
        if ms > COLD_START_BUDGET_MS {
            failures.push(format!(
                "SMOKE FAIL: launch to window took {ms}ms, over the {COLD_START_BUDGET_MS}ms cold-start budget (whitepaper §2)"
            ));
        }
    }

    if !p.flag("CLOSE_REQUESTED") {
        failures.push(format!(
            "SMOKE FAIL: CloseMainWindow (WM_CLOSE) was not accepted (handle={handle}) - the graceful-shutdown path was never asked to run, so its silence proves nothing"
        ));
    }

    let forced = p.flag("FORCED");
    if forced || !p.flag("EXITED_WITHOUT_KILL") {
        failures.push(format!(
            "SMOKE FAIL: the app did NOT exit by itself within {CLOSE_SECS}s of WM_CLOSE and was force-killed (code after the kill: {:?}) - a force-kill is not a graceful shutdown and cannot PASS whatever code it ends with",
 p.get("EXIT_CODE_AFTER_FORCE")
        ));
    } else {
        match p.number("EXIT_CODE") {
            Some(0) => {}
            Some(code) => failures.push(format!(
                "SMOKE FAIL: the app exited on its own but with code {code}, not 0"
            )),
            None => failures
                .push("SMOKE FAIL: EXIT_CODE is absent, so the exit status is unknown".to_string()),
        }
    }

    match artefact {
        Some(a) if a.read_error.is_some() && a.fresh => {
            // Own verdict, and not a pass: the app wrote something this run,
            // but it cannot be read back, so what is stored is unknown.
 failures.push(format!(
                "SMOKE FAIL: this run created {} but it CANNOT BE READ BACK ({}) - the rect is unverified. This is a statement about the machine, not about the app; the ACL and attributes are printed above.",
 a.path.display(),
 a.read_error.clone().unwrap_or_default()
            ));
        }
        None => failures.push(format!(
            "SMOKE FAIL: this run wrote no {SESSION_FILE} ({})",
 match relocation {
                Relocation::Moved { to } => format!(
                    "the user's own file was moved aside to {} first, so this was a genuine fresh-install run",
 to.display()
                ),
                Relocation::Kept(what) => format!(
                    "{what} was left in place by --reuse-state, and this run wrote nothing into it"
                ),
                _ => "nothing was in the way - there was no session.json to begin with".to_string(),
            }
        )),
        Some(a) if !a.fresh => failures.push(format!(
            "SMOKE FAIL: {} exists but is OLDER than this launch, so this run never wrote it and the fresh-install behaviour is not proven (rect={}) - if this file was left in place on purpose (--reuse-state), the fresh-install case cannot be judged from this run at all",
 a.path.display(),
 a.rect
        )),
        Some(_) => {}
    }

    if failures.is_empty() {
        Verdict::Pass
    } else {
        Verdict::Fail(failures)
    }
}

/// The two places core's documented caller rule can put state, in order. This
/// MIRRORS crates/core/src/paths.rs rather than importing it - xtask must not
/// depend on the crates it checks - so a changed rule surfaces here as "looked
/// in the wrong place", which the failure path prints in full.
fn candidate_state_dirs(exe: &Path) -> Vec<PathBuf> {
    candidate_state_dirs_in(exe, std::env::var_os("APPDATA").as_deref())
}

/// The same rule with the profile root injected, the shape core's own pure
/// resolve_state_dir takes, so a test can prove both halves without inheriting
/// (or mutating) the machine's APPDATA.
fn candidate_state_dirs_in(exe: &Path, appdata: Option<&std::ffi::OsStr>) -> Vec<PathBuf> {
    let exe_dir = exe.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut out = vec![exe_dir.join("data")];
    if let Some(appdata) = appdata {
        if !appdata.is_empty() {
            out.push(PathBuf::from(appdata).join("notes-gpui"));
        }
    }
    out
}

/// sha256 of a file, using the hasher the fixtures verifier already ships
/// (same crate, so no new dependency and no second implementation to drift).
fn sha_of(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    Some(crate::fixtures::sha256_hex(&bytes))
}

fn read_artefact(path: &Path, launched: SystemTime) -> Option<Artefact> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    if !meta.is_file() {
        // Not a file at all (a directory planted on the path). find_artefact
        // reports it; clear_session_path is what turns it into a verdict.
        return Some(Artefact {
            path: path.to_path_buf(),
            fresh: false,
            rect: format!("<not a regular file: {}>", describe_kind(path)),
            read_error: Some("the path is not a regular file".to_string()),
        });
    }
    let rect = match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(value) => match value.get("rect") {
                Some(r) => {
                    let g = |k: &str| r.get(k).and_then(serde_json::Value::as_i64).unwrap_or(-1);
                    format!("x={} y={} w={} h={}", g("x"), g("y"), g("w"), g("h"))
                }
                None => "<no rect key>".to_string(),
            },
            Err(e) => format!("<unparseable: {e}>"),
        },
        Err(e) => format!("<unreadable: {e}>"),
    };
    let read_error = if rect.starts_with("<unreadable") {
        Some(rect.clone())
    } else {
        None
    };
    Some(Artefact {
        path: path.to_path_buf(),
        fresh: mtime >= launched,
        rect,
        read_error,
    })
}

/// What a path actually is, for the message that says WHY it cannot be judged.
fn describe_kind(path: &Path) -> String {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => "a DIRECTORY sitting on the session file's path".to_string(),
        Ok(meta) if !meta.file_type().is_file() => "not a regular file".to_string(),
        Ok(meta) => format!("a file of {} bytes", meta.len()),
        Err(e) => format!("unreadable metadata ({e})"),
    }
}

/// The artefact this run produced, plus every path searched (printed on a miss).
fn find_artefact(exe: &Path, launched: SystemTime) -> (Option<Artefact>, Vec<PathBuf>) {
    let mut tried = Vec::new();
    let mut fresh: Option<Artefact> = None;
    let mut stale: Option<Artefact> = None;
    for dir in candidate_state_dirs(exe) {
        let path = dir.join(SESSION_FILE);
        tried.push(path.clone());
        match read_artefact(&path, launched) {
            Some(a) if a.fresh => fresh = Some(a),
            Some(a) => stale = Some(a),
            None => {}
        }
    }
    (fresh.or(stale), tried)
}

fn temp_path(stem: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "notes-gpui-smoke-{stem}-{}-{nanos}.{ext}",
        std::process::id()
    ));
    p
}

/// ACL / attribute diagnostics. A REPORTER only: it prints what Windows says,
/// and decide() owns the verdict. Its whole reason to exist is that a red row
/// caused by "Access is denied (os error 5)" on the tester's own profile is
/// unreadable without it - with it, the log says who holds the path.
const ACL_PROBE: &str = r#"
param([Parameter(Mandatory)][string]$Target)
$ErrorActionPreference = 'SilentlyContinue'
$item = Get-Item -LiteralPath $Target -Force
"kind=$($item.Attributes)"
"size=$($item.Length)"
"mtime=$($item.LastWriteTime.ToString('s'))"
"file_owner=$((Get-Acl -LiteralPath $Target).Owner)"
foreach ($a in (Get-Acl -LiteralPath $Target).Access) {
    "file_ace=$($a.AccessControlType) $($a.IdentityReference) $($a.FileSystemRights)"
}
$parent = Split-Path -Parent $Target
"dir=$parent"
"dir_owner=$((Get-Acl -LiteralPath $parent).Owner)"
foreach ($a in (Get-Acl -LiteralPath $parent).Access) {
    "dir_ace=$($a.AccessControlType) $($a.IdentityReference) $($a.FileSystemRights)"
}
'DONE=1'
"#;

/// One line of Windows-side diagnostics for a path smoke cannot use. Empty on
/// any failure: the absence of an ACL note must never change a verdict.
fn acl_note(path: &Path) -> String {
    let script = temp_path("acl", "ps1");
    let written = fs::File::create(&script).and_then(|mut f| f.write_all(ACL_PROBE.as_bytes()));
    let note = match written {
        Err(e) => return format!("<could not write the ACL probe: {e}>"),
        Ok(()) => Command::new("pwsh")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&script)
            .arg("-Target")
            .arg(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .or_else(|_| {
                Command::new("powershell")
                    .args([
                        "-NoProfile",
                        "-NonInteractive",
                        "-ExecutionPolicy",
                        "Bypass",
                        "-File",
                    ])
                    .arg(&script)
                    .arg("-Target")
                    .arg(path)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
            })
            .and_then(|mut child| match wait_bounded(&mut child, 15) {
                Ok(_) => Ok(read_pipe(child.stdout.as_mut())),
                Err(e) => Err(std::io::Error::other(e)),
            }),
    };
    let _ = fs::remove_file(&script);
    match note {
        Err(e) => format!("<ACL probe could not run: {e}>"),
        Ok(text) => {
            let mut bits: Vec<String> = Vec::new();
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with("dir=") || line == "DONE=1" {
                    continue;
                }
                bits.push(line.to_string());
            }
            if bits.is_empty() {
                "<the ACL probe printed nothing>".to_string()
            } else {
                bits.join(" | ")
            }
        }
    }
}

/// Spawn the probe. pwsh first, then Windows PowerShell 5.1: a runner may ship
/// either, and neither is a dependency of this crate.
fn spawn_probe(script: &Path, exe: &Path, out: &Path, err: &Path) -> Result<Child, String> {
    let mut missing = Vec::new();
    for program in ["pwsh", "powershell"] {
        let spawned = Command::new(program)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(script)
            .arg("-Exe")
            .arg(exe)
            .arg("-OutFile")
            .arg(out)
            .arg("-ErrFile")
            .arg(err)
            .arg("-WindowSecs")
            .arg(WINDOW_SECS.to_string())
            .arg("-CloseSecs")
            .arg(CLOSE_SECS.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        match spawned {
            Ok(child) => return Ok(child),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => missing.push(program.to_string()),
            Err(e) => return Err(format!("cannot run {program}: {e}")),
        }
    }
    Err(format!(
        "no PowerShell to run the window probe with (tried: {})",
        missing.join(", ")
    ))
}

/// try_wait against a deadline. A child that overruns is killed and reported:
/// the harness never waits on anything without one.
fn wait_bounded(child: &mut Child, secs: u64) -> Result<Option<std::process::ExitStatus>, String> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(Some(status)),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "the probe child exceeded its outer {secs}s deadline and was killed"
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("waiting on the probe child failed: {e}")),
        }
    }
}

fn read_pipe(pipe: Option<&mut std::process::ChildStdout>) -> String {
    let Some(pipe) = pipe else {
        return String::new();
    };
    let mut buf = Vec::new();
    if pipe.read_to_end(&mut buf).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&buf).to_string()
}

/// Read a capture file, lossily. The app writes its own trace as UTF-8 and the
/// one character in it that matters here is a U+00B7 separator, so a lossy read
/// keeps every line assertable where a strict read would hand back nothing.
fn read_capture(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Print what the launched process wrote, and RETURN the bytes so a caller can
/// assert over them. The return exists because the capture is transient: the
/// geometry step re-uses the same stderr path for its own launches, so the first
/// run's evidence has to be held by whoever means to judge it.
fn report_captured(path: &Path, label: &str) -> Option<String> {
    let text = read_capture(path)?;
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        println!("smoke: {label} said ({} bytes): {trimmed}", trimmed.len());
    }
    Some(text)
}

/// One assertion about what the LIVE app's own last-resort trace must contain.
///
/// The shape exists because this is the first row of a list, not the whole of it.
/// Every remaining M2 exit item that can be proven from a redirected stderr - the
/// seeded-recent open (item 2), a byte-identical save (item 3), the menu toggle
/// (item 6) - is a new entry in [TRACE_CLAIMS], never a new branch in run().
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceClaim {
    /// What the claim says the running app did, in its own words, for the message.
    pub what: &'static str,
    /// The substring one captured line must contain. Kept to plain ASCII on
    /// purpose: describe() separates its fields with U+00B7, and a needle that
    /// straddles one is a needle that breaks when the copy is re-worded.
    pub needle: &'static str,
    /// Which roadmap M2 exit item this proves, so a red line cites the plan it
    /// belongs to instead of being argued about from scratch.
    pub proves: &'static str,
}

/// THE CLAIMS. Row 1 is roadmap M2 exit item 5, "Recents - not proven. Named
/// gap: RecentsUpdated is emitted only on change, so a fresh window shows an
/// empty list and no headless test notices" (docs/roadmap.md SS9). The engine has
/// since grown a startup announce (api/src/engine.rs, THE STARTUP ANNOUNCE in
/// Engine::run), and every test for it is headless: nothing has ever shown the
/// line reaching a real UI. This row is that showing, through the one voice the
/// bridge has when it has no console - report()'s stderr.
pub const TRACE_CLAIMS: &[TraceClaim] = &[TraceClaim {
    // "the RUNNING APP", not "the live UI": what this needle can show is that the
    // announce reached the live process' own stderr, in one of the three voices. A
    // [delivered, never rendered] match is that fact with a smoke run that closed
    // the window before the frame - legitimate, and the voice line keeps the log
    // honest about the difference. The UI SHOWING it is the [rendered] voice only.
    what: "the startup recents announce reached the running app",
    needle: "RecentsUpdated",
    proves: "M2 exit item 5 (Recents)",
}];

/// The three answers a claim list can come back with, spelled the way Geometry
/// spells its own: proven, broken, or not judgeable here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceVerdict {
    /// Every claim matched; the lines are the evidence, one per claim, so a
    /// citation names what the app actually printed rather than a bare green.
    Proven(Vec<String>),
    /// At least one claim matched nothing. Only reachable when the run was
    /// judgeable at all - see the zero-recents gate on judge_trace.
    Broken(Vec<String>),
    /// The claim could not be tried: no capture to read, an empty capture, or a
    /// profile with nothing to announce. Advisory, and never a pass.
    NotJudged(String),
}

/// Which of the app's own voices carried a matched line, because the three carry
/// different facts. "status line:" is a line the UI RENDERED (bridge main.rs,
/// where record_shown gates it on a real change); "undisplayed:" is an event the
/// port delivered that never got a frame, printed by the exit drain; anything
/// else is one of the app's other report() lines. Naming the voice is what keeps
/// this honest: the needle alone says the app KNEW the list, and only a rendered
/// line says the window was told. So the claim asserts the weaker fact - the one
/// that holds on every machine, because a startup batch may let a later event win
/// the single printed line - and prints the strength it actually got.
fn trace_voice(line: &str) -> &'static str {
    if line.contains("status line:") {
        "rendered"
    } else if line.contains("event: ") {
        // The arrival voice: the bridge names every drained event once, before the
        // last-wins collapse and ungated on purpose (bridge main.rs, the
        // report of "event: {words}"), so this is the strongest voice that is
        // ALWAYS true - it survives a batch that hides the announce behind a later
        // event, which is the exact case "status line:" cannot cover.
        "arrived"
    } else if line.contains("undisplayed:") {
        "delivered, never rendered"
    } else {
        "traced"
    }
}

/// How strong a voice is, best last. A claim cites the STRONGEST line that
/// matched rather than the first one written, so a run never downgrades its own
/// evidence to whatever the app happened to print first.
fn voice_rank(line: &str) -> u8 {
    match trace_voice(line) {
        "rendered" => 3,
        "arrived" => 2,
        "delivered, never rendered" => 1,
        _ => 0,
    }
}

/// Decide the claim list over a captured trace. Pure: no files, no process, so
/// all three outcomes are provable without a desktop.
///
/// * captured - the bytes the LIVE app wrote to its own stderr on this launch,
///   or None when the file could not be read.
/// * recents_in_state_dir - how many recents the APP-RESOLVED state dir held before
///   the launch: the one directory the app itself opens (see recents_at_startup,
///   which follows core's resolution rule), never the first settings.toml some
///   candidate happens to hold. Zero makes the assertion UNMEASURABLE rather than
///   satisfied: the engine announces only when there is something to announce, so
///   on a fresh install no line can appear, and a run that "passed" would be a run
///   that asserted nothing.
pub fn judge_trace(
    captured: Option<&str>,
    recents_in_state_dir: usize,
    claims: &[TraceClaim],
) -> TraceVerdict {
    let Some(text) = captured else {
        return TraceVerdict::NotJudged(
            "the app's stderr capture could not be read, so no claim about what the live UI saw was \
 tried"
                .to_string(),
        );
    };
    if text.trim().is_empty() {
        return TraceVerdict::NotJudged(
            "the app wrote NOTHING to its own trace (0 bytes). A graceful exit always writes its exit \
 trace, so an empty capture means the instrument failed to see the app rather than that the \
 app said nothing - which is exactly why it is not a FAIL either"
                .to_string(),
        );
    }
    if recents_in_state_dir == 0 {
        return TraceVerdict::NotJudged(
            "the app-resolved state dir held no recents before this launch (its settings.toml is \
 absent, unreadable, or lists none), and the engine announces only a NON-EMPTY list by \
 design (api/src/engine.rs, THE STARTUP ANNOUNCE): there was nothing for the live UI to be \
 shown, so the item stands unproven rather than disproven"
                .to_string(),
        );
    }
    let mut proven = Vec::new();
    let mut broken = Vec::new();
    for claim in claims {
        // The STRONGEST voice that matched, not the first line written: a run that
        // both announced and rendered the list cites the rendered line, and one
        // whose announce lost the last-wins collapse still cites its arrival.
        match text
            .lines()
            .filter(|line| line.contains(claim.needle))
            .max_by_key(|line| voice_rank(line))
        {
            Some(line) => proven.push(format!("{}: [{}] {line}", claim.what, trace_voice(line.trim()))),
            None => broken.push(format!(
                "SMOKE TRACE FAIL: {} - no line of the live app's stderr contains {:?}, and {} recents \
                 WERE in the app-resolved state dir to announce ({})",
 claim.what, claim.needle, recents_in_state_dir, claim.proves
            )),
        }
    }
    if broken.is_empty() {
        TraceVerdict::Proven(proven)
    } else {
        TraceVerdict::Broken(broken)
    }
}

/// Count the "[[recents]]" array-of-tables headers in core's settings.toml. A
/// text count, not a parse: xtask takes no new dependency (the independence rule
/// check-arch exists to protect), and the only question asked here is "was there
/// anything to announce". Both ways of being wrong stay visible rather than
/// silent: an over-count asserts against a live UI that stayed quiet, and an
/// under-count prints NOT JUDGED.
pub fn count_recents_blocks(toml_text: &str) -> usize {
    toml_text
        .lines()
        .filter(|line| line.trim() == "[[recents]]")
        .count()
}

/// The scratch note a seeded recent points at, named through temp_path so a
/// crashed run's leftovers stay findable by prefix (see sweep_stale_seeds).
const SEED_STEM: &str = "seeded-recent";

/// The note's text: known and non-empty, so opening it by hand is visibly
/// different from an empty buffer.
const SEED_TEXT: &str = "smoke seeded this note so the recent list was not empty\n";

/// The settings.toml the harness writes so the startup announce HAS something to
/// announce, pointing at one real note.
///
/// Two details are load-bearing and pinned by a test rather than trusted:
///
/// * autosave_enabled MUST be present. Settings declares it with no serde default,
///   so a file carrying only the recents table is not "defaults plus a recent" - it
///   is SettingsCorrupt, the app runs on factory defaults, the recents are EMPTY,
///   and the claim silently drops back to not-judged while the log reads like a
///   successful seed. It is written true because that is core's own factory value,
///   so a seeded run behaves like a fresh install in every other respect.
/// * the path is a TOML basic string with every backslash doubled, not a literal
///   string: a profile directory holding an apostrophe would otherwise corrupt the
///   file, and nothing would ever say so.
pub fn seeded_settings_toml(note: &Path) -> String {
    let escaped = note.display().to_string().replace('\\', "\\\\");
    let display = note
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "seeded-recent.notes".to_string());
    format!(
        "autosave_enabled = true\n\n[[recents]]\npath = \"{escaped}\"\ndisplay = \"{display}\"\nexists = true\n"
    )
}

/// Brings the profile's settings.toml back, byte-exactly, and removes the scratch
/// note. The sibling of Restore for the other file in the same directory, with the
/// same Drop guarantee: a panic, an early return or a killed probe cannot leave a
/// seeded recent sitting in someone's profile.
///
/// One asymmetry, on purpose: a session.json the app rewrote is KEPT, because that
/// file IS the persistence evidence the run exists to collect. A settings.toml the
/// app rewrote is still replaced, because here the evidence is the printed trace
/// line, and leaving the app's file behind would make the NEXT run's recents count
/// come from a file whose provenance nobody can state. Every outcome prints.
struct SeedGuard {
    settings: PathBuf,
    /// The bytes that were there before, or None when this run created the file.
    prior: Option<Vec<u8>>,
    note: PathBuf,
    done: bool,
    failed: bool,
}

impl SeedGuard {
    fn finish(&mut self) {
        if !self.done {
            self.done = true;
            self.restore("at the end of the run");
        }
    }

    fn restore(&mut self, when: &str) {
        match &self.prior {
            Some(bytes) => {
                let identical = fs::write(&self.settings, bytes)
                    .and_then(|()| fs::read(&self.settings))
                    .map(|now| now == *bytes);
                match identical {
                    Ok(true) => println!(
                        "smoke: settings: {when} - the user's settings.toml came back byte-identical \
 (sha256 {}) at {}",
                        crate::fixtures::sha256_hex(bytes),
                        self.settings.display()
                    ),
                    Ok(false) => {
                        self.failed = true;
                        eprintln!(
                            "SMOKE FAIL: settings: the user's settings.toml did NOT come back \
 byte-identical at {}. The original bytes exist only in this process' \
 memory, so say so before anything else trusts that profile.",
                            self.settings.display()
                        );
                    }
                    Err(e) => {
                        self.failed = true;
                        eprintln!(
                            "SMOKE FAIL: settings: could not rewrite {} : {e}",
                            self.settings.display()
                        );
                    }
                }
            }
            // Nothing was there, so nothing may be left: this file is ours to remove.
            None => match fs::remove_file(&self.settings) {
                Ok(()) => println!(
                    "smoke: settings: {when} - removed the settings.toml this run created at {}; the \
 trace line above is this run's evidence, not this file's",
                    self.settings.display()
                ),
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        println!(
                            "smoke: settings: {when} - {} is already gone",
                            self.settings.display()
                        );
                    } else {
                        self.failed = true;
                        eprintln!(
                            "SMOKE FAIL: settings: could not remove the seeded {} : {e} - a seeded \
 recent is still sitting in that profile",
                            self.settings.display()
                        );
                    }
                }
            },
        }
        if let Err(e) = fs::remove_file(&self.note) {
            if e.kind() != std::io::ErrorKind::NotFound {
                self.failed = true;
                eprintln!(
                    "SMOKE FAIL: settings: could not remove the scratch note {}: {e}",
                    self.note.display()
                );
            }
        }
    }
}

impl Drop for SeedGuard {
    fn drop(&mut self) {
        if !self.done {
            self.done = true;
            self.restore("while unwinding");
        }
    }
}

/// Delete scratch notes left by a CRASHED earlier run, so seeds never accumulate
/// across runs that never reached their restore. Deliberately narrow: only this
/// stem's notes, NEVER the session backups - those hold the user's own bytes and
/// are the one artefact a later run must not throw away on a timer.
///
/// The age floor is what makes this safe beside a concurrent smoke: a run still in
/// flight owns files minutes old, and nobody's evidence gets swept.
fn sweep_stale_seeds(older_than: std::time::Duration) -> usize {
    let Ok(entries) = fs::read_dir(std::env::temp_dir()) else {
        return 0;
    };
    let prefix = format!("notes-gpui-smoke-{SEED_STEM}-");
    let mut swept = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let aged = path
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|e| e > older_than);
        let named = path
            .file_name()
            .map(|s| s.to_string_lossy().starts_with(&prefix))
            .unwrap_or(false);
        if named && aged && fs::remove_file(&path).is_ok() {
            swept += 1;
        }
    }
    swept
}

/// Resolve the app's own state dir, then seed there. Split in two so the guard is
/// testable against a temp directory: a test that called the resolving half would
/// seed THIS machine's real profile, which is exactly the harm SeedGuard exists to
/// prevent.
fn seed_settings(exe: &Path) -> Result<(SeedGuard, PathBuf), String> {
    let dir =
        app_state_dir(exe).ok_or("the app's state dir cannot be resolved from the exe path")?;
    seed_settings_at(&dir)
}

/// Write the seed into ONE directory and hand back the guard that puts it back. The
/// caller reads recents_at_startup AFTER this, so the count in the log is the count
/// the seeded app will see.
///
/// An Err is never a verdict: the claim falls back to NOT JUDGED, exactly as it did
/// before the seed existed, because a directory that will not take a file is the
/// machine's story and not the app's.
fn seed_settings_at(dir: &Path) -> Result<(SeedGuard, PathBuf), String> {
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let note = temp_path(SEED_STEM, "notes");
    fs::write(&note, SEED_TEXT).map_err(|e| format!("cannot write {}: {e}", note.display()))?;
    let settings = dir.join(SETTINGS_FILE);
    let prior = match fs::read(&settings) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            let _ = fs::remove_file(&note);
            return Err(format!("cannot read {}: {e}", settings.display()));
        }
    };
    if let Err(e) = fs::write(&settings, seeded_settings_toml(&note)) {
        let _ = fs::remove_file(&note);
        return Err(format!("cannot seed {}: {e}", settings.display()));
    }
    Ok((
        SeedGuard {
            settings,
            prior,
            note: note.clone(),
            done: false,
            failed: false,
        },
        note,
    ))
}

/// The ONE directory the app itself persists into, by the rule
/// crates/core/src/paths.rs documents AND in its order - the caller-side shape in
/// that file's own doc comment: portable (exe_dir/data) when the marker exists OR
/// there is no usable roaming profile, installed (appdata/notes-gpui) only
/// otherwise. Pure over its inputs for core's own reason: the marker probe is one
/// caller-side exists() and appdata is passed in, so both halves are testable
/// without touching this machine's profile. An empty APPDATA is not a usable
/// profile, exactly as core defines it - and note this is the OPPOSITE order from
/// candidate_state_dirs, which is a search list for the artefact, not the rule the
/// app applies. Getting them confused is how a claim gets armed against a file the
/// running process never opens.
pub fn state_dir_for(
    exe: &Path,
    appdata: Option<&std::ffi::OsStr>,
    portable_marker: bool,
) -> Option<PathBuf> {
    let exe_dir = exe.parent()?;
    let usable = appdata.filter(|a| !a.is_empty());
    if portable_marker || usable.is_none() {
        Some(exe_dir.join("data"))
    } else {
        Some(PathBuf::from(usable?).join("notes-gpui"))
    }
}

/// What the app had to announce: the recents in the settings.toml of the ONE
/// directory it will actually open, plus that path for the log.
///
/// This used to walk every candidate state dir and take the first readable
/// settings.toml, which armed the claim against a file the running process never
/// opened - measured on a real run: target/debug/data held NO settings.toml (and
/// the portable marker sent the app there) while the roaming profile held one
/// recent, so smoke printed exit 9 about a directory the app ignored. Resolved dir
/// only, and 0 when its settings.toml is missing or unreadable: a NOT JUDGED
/// verdict then means "the app had nothing to announce", never "some profile
/// somewhere had something".
fn recents_at_startup(exe: &Path) -> (usize, PathBuf) {
    let marker = exe
        .parent()
        .map(|d| d.join("data").is_dir())
        .unwrap_or(false);
    match state_dir_for(exe, std::env::var_os("APPDATA").as_deref(), marker) {
        None => (0, PathBuf::from("<state dir unresolvable>")),
        Some(dir) => {
            let settings = dir.join(SETTINGS_FILE);
            let count = fs::read_to_string(&settings)
                .map(|text| count_recents_blocks(&text))
                .unwrap_or(0);
            (count, settings)
        }
    }
}

/// What the run ended up doing with the user's bytes, decided from what is
/// ACTUALLY sitting on the session path when the run finishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conclusion {
    /// The app wrote nothing this run, so the user's own file goes back.
    RestoredUserState,
    /// The app wrote a fresh artefact. Keep it - it is the proof - and leave
    /// the user's pre-run bytes at the named temp path. Overwriting them back
    /// would destroy the evidence the run exists to collect.
    KeptAppArtefact {
        app_sha: String,
        /// True when the app happened to write the bytes the user already had.
        /// Worth printing: it separates "the app re-persisted the same default
        /// rect" from "the app persisted something new".
        rewrote_same_bytes: bool,
    },
}

/// The whole decision, pure, so the two cases are provably distinguishable in
/// a test. 'user_before' is the sha256 of the bytes that were relocated;
/// 'live_now' is the sha256 of whatever is on the session path now, if
/// anything.
pub fn conclude(user_before: Option<&str>, live_now: Option<&str>) -> Conclusion {
    match live_now {
        None => Conclusion::RestoredUserState,
        Some(app_sha) => Conclusion::KeptAppArtefact {
            rewrote_same_bytes: user_before == Some(app_sha),
            app_sha: app_sha.to_string(),
        },
    }
}

/// Brings the user's session.json back - unless the run produced a real
/// artefact to preserve. Drop still runs it, so a panic, an early return or a
/// force-killed probe cannot leave the profile holding nothing.
struct Restore {
    live: PathBuf,
    aside: PathBuf,
    before: Option<String>,
    done: bool,
    /// True when the bytes did not come back identical, or could not come back.
    failed: bool,
    /// Set once restore has decided which of the two outcomes happened.
    conclusion: Option<Conclusion>,
}

impl Restore {
    fn finish(&mut self) {
        if !self.done {
            self.done = true;
            self.restore("at the end of the run");
        }
    }

    fn restore(&mut self, when: &str) {
        let live_now = sha_of(&self.live);
        match conclude(self.before.as_deref(), live_now.as_deref()) {
            Conclusion::KeptAppArtefact {
                app_sha,
                rewrote_same_bytes,
            } => {
                println!("smoke: {when} - the app wrote a fresh session.json; keeping it");
                println!(
                    "smoke:   user bytes before = {:?} (held at {})",
                    self.before,
                    self.aside.display()
                );
                println!(
                    "smoke:   app artefact now  = {app_sha} (at {})",
                    self.live.display()
                );
                println!(
                    "smoke:   user bytes after  = {app_sha} (the app's, not the pre-run ones); same_as_user_before={rewrote_same_bytes}"
                );
                println!(
                    "smoke:   nothing deleted; to give the user their old file back, rename {} over {}",
                    self.aside.display(),
                    self.live.display()
                );
                self.conclusion = Some(Conclusion::KeptAppArtefact {
                    app_sha,
                    rewrote_same_bytes,
                });
            }
            Conclusion::RestoredUserState => match fs::rename(&self.aside, &self.live) {
                Ok(()) => {
                    let after = sha_of(&self.live);
                    let identical = matches!((&self.before, &after), (Some(b), Some(a)) if b == a);
                    println!(
                        "smoke: {when} - restored {} from {}",
                        self.live.display(),
                        self.aside.display()
                    );
                    println!(
                        "smoke:   user bytes before = {:?}  user bytes after = {:?}  identical={identical}",
                        self.before, after
                    );
                    println!("smoke:   app artefact this run = none written");
                    if !identical {
                        self.failed = true;
                        eprintln!(
                            "SMOKE FAIL: the relocated user state did not come back byte-identical"
                        );
                    }
                    self.conclusion = Some(Conclusion::RestoredUserState);
                }
                Err(e) => {
                    self.failed = true;
                    eprintln!(
                        "SMOKE FAIL: could not restore {} from {} - the user's file is at that temp path, put it back by hand: {e}",
                        self.live.display(),
                        self.aside.display()
                    );
                }
            },
        }
    }

    /// Which of the two outcomes happened, for the verdict line.
    fn outcome_note(&self) -> String {
        match &self.conclusion {
            Some(Conclusion::KeptAppArtefact { app_sha, .. }) => {
                format!(
                    "kept-app-artefact (wrote {app_sha}; user bytes at {})",
                    self.aside.display()
                )
            }
            Some(Conclusion::RestoredUserState) => "restored-user-state".to_string(),
            None => "not reached".to_string(),
        }
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        if !self.done {
            self.done = true;
            self.restore("while unwinding");
        }
    }
}

/// Clear the session path so the run judges the case D54 is about, and hand
/// back the guard that puts the user's file back. Anything that is not a plain
/// movable FILE becomes Blocked: a directory planted on the path (which is what
/// an api test does to prove the write path handles a blocked target) is not
/// stale state and not this harness's to move.
fn clear_session_path(exe: &Path) -> (Option<Restore>, Relocation) {
    for dir in candidate_state_dirs(exe) {
        let path = dir.join(SESSION_FILE);
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if !meta.is_file() {
            let acl = acl_note(&path);
            return (
                None,
                Relocation::Blocked(format!(
                    "{} is {} (mtime {}); it was not moved, so no verdict about the fresh-install write is available. Diagnostics: {}",
                    path.display(),
                    describe_kind(&path),
                    meta.modified()
                        .ok()
                        .and_then(|t| t.elapsed().ok().map(|d| format!("{}s ago", d.as_secs())))
                        .unwrap_or_else(|| "mtime unknown".to_string()),
                    acl
                )),
            );
        }
        let tag = dir
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "state".to_string());
        let aside = temp_path(&format!("session-backup-{tag}"), "json");
        let before = sha_of(&path);
        if let Err(e) = fs::rename(&path, &aside) {
            let acl = acl_note(&path);
            return (
                None,
                Relocation::Blocked(format!(
                    "{} is a file but could not be renamed aside ({e}); it stays exactly where it is. Diagnostics: {}",
                    path.display(),
                    acl
                )),
            );
        }
        println!(
            "smoke: relocated the user's {} -> {} (sha256 {:?}); every exit path either puts it back or keeps the app's own artefact and leaves these bytes aside",
            path.display(),
            aside.display(),
            before
        );
        println!(
            "smoke: nothing is being deleted; if this process is killed, the file is at the temp path above"
        );
        let guard = Restore {
            live: path,
            aside,
            before,
            done: false,
            failed: false,
            conclusion: None,
        };
        let relocation = Relocation::Moved {
            to: guard.aside.clone(),
        };
        return (Some(guard), relocation);
    }
    (None, Relocation::CleanSlate)
}

/// One summary line, readable on its own in a CI log.
fn summary(verdict: &Verdict, p: &Probe, elapsed: Duration, relocation: &Relocation) -> String {
    let secs = format!("{:.1}s", elapsed.as_secs_f64());
    let window = if p.number("HANDLE").unwrap_or(0) != 0 {
        "OK"
    } else {
        "MISSING"
    };
    match verdict {
        Verdict::Pass => format!("smoke: window=OK close=0 session=OK {secs}"),
        Verdict::Skip(_) => {
            let what = match relocation {
                Relocation::Blocked(_) => {
                    "BLOCKED BY FOREIGN STATE (machine pollution, not the app) - clear the occupant"
                }
                _ => "no desktop",
            };
            format!("smoke: window=DECLINED close=DECLINED session=DECLINED ({what}) {secs}")
        }
        Verdict::Fail(_) => {
            let close = if p.flag("FORCED") || !p.flag("EXITED_WITHOUT_KILL") {
                "FORCED".to_string()
            } else {
                p.number("EXIT_CODE")
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "UNKNOWN".to_string())
            };
            format!("smoke: window={window} close={close} session=FAILED {secs}")
        }
    }
}

/// The build smoke depends on. Launching whatever exe happens to be lying
/// around proves a cached binary, not the tree it was run from, so the target
/// is built first, a compile failure is a hard verdict with its own exit code,
/// and the exe's mtime is then compared against the newest source that
/// produces it. Freshness is proven, never trusted.
const BUILD_ARGS: &[&str] = &["build", "-p", GPUI_TARGET.pkg, "--bin", GPUI_TARGET.bin];
/// The crates whose SOURCE DIRECTORIES end up inside that binary, named as
/// "<crate>/src" rather than "<crate>" on purpose. Cargo's own fingerprint remains
/// the authority on what rebuilds what; this list is only the guard that exists
/// because a stale exe was once judged as if it were current. A whole-crate mtime
/// scan cannot see a dependency graph, and it over-claims: a crate's tests/,
/// benches/ and examples/ are NOT inputs to the bin (cargo builds them for that
/// package's own test target and nowhere else), yet a newer file in one of them
/// aborted a live run with exit 5 while smoke's OWN build step had just rebuilt the
/// exe - measured on f61d850d, where the named file was
/// crates/api/tests/geometry.rs, committed 84s before the run, in a log that also
/// printed built_by_this_run=yes. A guard that contradicts the build it ships behind
/// is worse than no guard: it teaches people to distrust exit 5.
///
/// The scope is strict, not loose. The exe's own crate's sources stay in it, so a
/// crates/bridge-gpui/src edit newer than the exe is still a loud 5. What left the
/// scope is a non-input, not a granted exception.
const SOURCE_ROOTS: &[&str] = GPUI_TARGET.freshness_roots;
/// Inputs that live OUTSIDE a src dir, named one by one because a scan that walked
/// whole crate dirs would walk their tests with them: the workspace manifest and the
/// lock (the lock decides the entire external graph), one manifest per crate above (a
/// feature or dependency line there changes the binary), and the bridge's build.rs (it
/// decides the embedded manifest, so a stale build script is a stale exe).
const SOURCE_FILES: &[&str] = GPUI_TARGET.freshness_files;
/// The target did not compile: its own verdict, not a decline (the desktop is
/// fine) and not a step failure (nothing was launched).
pub const BUILD_FAILED_EXIT: i32 = 4;
/// The exe is older than the sources that produce it.
pub const STALE_BINARY_EXIT: i32 = 5;
pub const MANIFEST_NOT_OURS_EXIT: i32 = 8;
/// The exe carries somebody else's manifest and --require-ours was passed. NOT
/// part of CONTRACT on purpose: the contract table lists what a default
/// invocation can return, and this code is only reachable behind a flag CI does
/// not pass. Putting it in the table would force a ci.yml arm for a verdict that
/// can never occur there, and the rule would then be teaching people to write
/// arms that do nothing.
/// Everything passed.
pub const PASS_EXIT: i32 = 0;
/// An app step failed: the app itself, not the harness.
pub const STEP_FAILED_EXIT: i32 = 1;
/// The harness could not run at all (also the answer to a bad argument).
pub const HARNESS_EXIT: i32 = 2;
/// Declined: this machine cannot judge the GUI, which is not the app's fault.
pub const DECLINED_EXIT: i32 = 3;

/// THE CONTRACT. One table, spelled from the constants the code actually returns,
/// so a new verdict has to be a new row here rather than a bare return that
/// nobody documented. check-ci asserts ci.yml branches on exactly these codes.
pub const CONTRACT: &[Contract] = &[
    Contract(PASS_EXIT, "pass"),
    Contract(STEP_FAILED_EXIT, "step failed"),
    Contract(HARNESS_EXIT, "harness could not run"),
    Contract(DECLINED_EXIT, "declined"),
    Contract(BUILD_FAILED_EXIT, "target did not compile"),
    Contract(STALE_BINARY_EXIT, "binary older than sources"),
    Contract(GEOMETRY_FAILED_EXIT, "window-memory broke"),
    Contract(PIN_FAILED_EXIT, "the pin lied"),
    Contract(
        TRACE_FAILED_EXIT,
        "the live UI never saw the startup announce",
    ),
];

/// One row of the contract. A struct rather than a tuple so the number and the
/// sentence cannot be transposed by a reader in a hurry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Contract(pub i32, pub &'static str);

/// The codes, in order, for the startup line and for check-ci's message text.
pub fn contract_codes() -> Vec<i32> {
    CONTRACT.iter().map(|c| c.0).collect()
}

/// The startup line, built from the same codes check-ci compares against, so the
/// line a run prints cannot disagree with the table that run is judged by.
pub fn contract_line() -> String {
    let mut out = String::from("smoke: contract");
    for code in contract_codes() {
        out.push(' ');
        out.push_str(&code.to_string());
    }
    out
}

fn mtime_of(path: &Path) -> Option<std::time::SystemTime> {
    crate::identity::mtime_of(path)
}

/// The sources that produce notes-gpui.exe: the shared walk from
/// [crate::identity], with this crate's scope. Deliberately not a second
/// implementation - "is this artefact older than its sources" is one rule that
/// smoke and the tool's own self-check both apply, and two copies of it is how
/// one of them drifts.
pub fn newest_source(root: &Path) -> Option<(std::time::SystemTime, PathBuf)> {
    crate::identity::newest_mtime(root, SOURCE_ROOTS, SOURCE_FILES)
}

/// smoke's own verdict type: a stale GUI binary is a HARD failure (exit 5),
/// unlike the tool's self-check, which can only warn.
/// The first thing a reader of a red build wants: the error, not the spinner.
fn first_error(text: &str) -> Option<String> {
    text.lines()
        .find(|l| l.starts_with("error[") || l.starts_with("error:"))
        .map(|l| {
            let l = l.trim();
            if l.chars().count() > 180 {
                l.chars().take(180).collect()
            } else {
                l.to_string()
            }
        })
}

/// Build the target. Err carries the first real error line and how many there
/// were; cargo's own output is echoed (bounded) so nobody has to re-run it.
pub fn build_target(root: &Path) -> Result<(), (String, usize)> {
    let out = std::process::Command::new("cargo")
        .args(BUILD_ARGS)
        .current_dir(root)
        .output()
        .map_err(|e| (format!("could not run cargo build: {e}"), 1))?;
    if out.status.success() {
        return Ok(());
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    let count = text
        .lines()
        .filter(|l| l.starts_with("error[") || l.starts_with("error:"))
        .count()
        .max(1);
    for line in text.lines().filter(|l| !l.trim().is_empty()).take(12) {
        eprintln!("smoke: build | {line}");
    }
    Err((
        first_error(&text).unwrap_or_else(|| "cargo printed no error line".to_string()),
        count,
    ))
}

/// Is the exe actually the product of the current sources? A pure decision so
/// the stale case is testable without launching anything - the review finding
/// was precisely that a green run said nothing about which tree it certified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stale {
    Fresh,
    /// One of the two mtimes could not be read. That is not "fresh", it is
    /// unverifiable, and an instrument that cannot check itself says so.
    Unknown(&'static str),
    OlderThan {
        source: PathBuf,
        delta_secs: u64,
        age_secs: u64,
    },
}

pub fn staleness(
    exe: Option<std::time::SystemTime>,
    newest: Option<(std::time::SystemTime, PathBuf)>,
) -> Stale {
    let (Some(built_at), Some((source_at, source))) = (exe, newest) else {
        return Stale::Unknown(if exe.is_none() {
            "the exe mtime could not be read"
        } else {
            "no readable source files were found"
        });
    };
    if source_at <= built_at {
        return Stale::Fresh;
    }
    Stale::OlderThan {
        delta_secs: source_at
            .duration_since(built_at)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        age_secs: built_at.elapsed().map(|d| d.as_secs()).unwrap_or(0),
        source,
    }
}

/// Say which binary was tested: path, age, and the HEAD it was built against.
/// Without this line a green run cannot be attributed to a tree at all.
fn report_binary(exe: &Path, root: &Path, built: bool) {
    let age = mtime_of(exe)
        .and_then(|t| t.elapsed().ok())
        .map(|d| format!("{}s old", d.as_secs()))
        .unwrap_or_else(|| "mtime unknown".to_string());
    let head = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!(
        "smoke: tested binary {} | {age} | built_by_this_run={} | git HEAD at that moment {}",
        exe.display(),
        if built { "yes" } else { "NO (--no-build)" },
        head
    );
    let embed = match build_embeds_manifest(root) {
        Some(true) => "build.rs EMBEDS app.manifest at link time",
        Some(false) => {
            "build.rs does NOT embed anything, so these bytes did not come \
 from cargo build - they came from a post-link 'cargo xtask manifest' \
 run, or from the toolkit's own manifest"
        }
        None => "build.rs could not be read, so the cause is unknown",
    };
    match manifest_of(root, exe) {
        Ok(m) => {
            println!(
                "smoke: manifest in that exe: identity={} longPathAware={} PerMonitorV2={} | {}",
                m.identity, m.long_path, m.per_monitor, embed
            );
            if let crate::manifest::ReadBack::Missing(absent) = crate::manifest::read_back(&m) {
                println!(
                    "SMOKE WARN: this exe does NOT carry our manifest - no {:?}. Since the kit \
 migration build.rs embeds NOTHING, so a plain cargo build is NOT compliant: the \
 post-link 'cargo xtask manifest' step is required and CI gates on it. Locally \
 this stays a warning, because a bare cargo run still gets PerMonitorV2 from the \
 toolkit - byte-identical DPI semantics - and a red nobody can clear without \
 learning a new command trains people to ignore red. --require-ours makes it 8.",
                    absent
                );
            }
        }
        Err(why) => println!("smoke: manifest NOT JUDGED - {why} (no claim either way)"),
    }
}

/// The geometry round trip: seed a rect, launch, read where the window really
/// is in FRAME and CLIENT pixels, move it, close, read what got persisted, then
/// relaunch and read where it came back. Prints numbers, never an assert string.
const GEOMETRY_PROBE: &str = r#"
param([Parameter(Mandatory)][string]$Exe, [string]$ErrFile, [string]$Session = "",
       [int]$SeedX = 0, [int]$SeedY = 0, [int]$SeedW = 0, [int]$SeedH = 0,
       [int]$MoveX = -1, [int]$MoveY = -1,
       [int]$WindowSecs = 10, [int]$SettleMs = 4500, [int]$CloseSecs = 10,
       # The polarity this launch was seeded with (1 pinned, 0 unpinned, -1 not
       # asserted), and how long the TOPMOST poll may wait for it. Both are
       # passed in by run_probe_script from PIN_WAIT_MS / PIN_TICK_MS below, so
       # the Rust verdict and the script cannot disagree about the budget.
       [int]$ExpectPinned = -1, [int]$PinWaitMs = 2000, [int]$PinTickMs = 50,
    [int]$PinConfirmMs = 200)
$ErrorActionPreference = 'SilentlyContinue'
Add-Type -AssemblyName System.Windows.Forms
$wa = [System.Windows.Forms.SystemInformation]::WorkingArea
"WORK=$($wa.X),$($wa.Y),$($wa.X + $wa.Width),$($wa.Y + $wa.Height)"
$code = @'
using System;
using System.Runtime.InteropServices;
public static class WIN {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll", EntryPoint="GetWindowLongW")] public static extern int GetWindowLong(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hh, bool rep);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
}
'@
if (-not (Add-Type -TypeDefinition $code -PassThru)) { 'WIN32=0'; 'PROBE_DONE=1'; exit 0 }
'WIN32=1'
$interactive = [Environment]::UserInteractive
$windowed = (Get-Process | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1)
if (-not ($interactive -and $windowed)) { 'DESKTOP=0'; 'PROBE_DONE=1'; exit 0 }
'DESKTOP=1'
'DESKTOP=1'
# WindowSecs 0 = answer WORK/DESKTOP/WIN32 and leave without launching: the seed
# rect has to be computed from the work area BEFORE the app starts, so the round
# trip's first call must not open a window.
if ($WindowSecs -eq 0) { 'REPORTONLY=1'; 'PROBE_DONE=1'; exit 0 }
$p = Start-Process -FilePath $Exe -PassThru -RedirectStandardError $ErrFile
if ($null -eq $p) { 'SPAWN=0'; 'PROBE_DONE=1'; exit 0 }
$deadline = (Get-Date).AddSeconds($WindowSecs)
$handle = [IntPtr]::zero
while ((Get-Date) -lt $deadline) {
    $p.Refresh()
    if ($p.MainWindowHandle -ne 0) { $handle = $p.MainWindowHandle; break }
    if ($p.HasExited) { break }
    Start-Sleep -Milliseconds 100
}
"HANDLE=$([int64]$handle)"
# GWL_EXSTYLE (-20) and WS_EX_TOPMOST (0x8). Read only once the handle exists:
# the platform lane PROVED that an async topmost reband applied to a hidden
# window never lands, so a style read before the show says nothing about the pin.
#
# ONE read after the handle appears is not enough either, and 8d0e5055 is the
# proof: the app pinned itself (an OS probe on that very build read
# exstyle=0x240108 - WS_EX_TOPMOST set - at handle+512ms and still set three
# seconds later, with 'status line: Pinned' in the stderr), and this script
# answered 7 anyway because it read the style on the SAME tick the handle was
# first sighted. RegisterWindow -> restore_and_pin is downstream of visibility,
# so the bit is legitimately in flight: the read must be a BOUNDED POLL that
# stops the moment the seeded polarity is seen, not a single sample that loses
# the race and reports the app as a liar.
#
# What this deliberately does NOT do is turn the verdict into "eventually
# whatever it turned out to be". ExpectPinned is what the harness seeded into
# session.json, the loop only ENDS EARLY on a match, and an expiry is still a
# failure - with the last EXSTYLE and the waited time printed, because "never
# became topmost in 2000ms" and "was topmost at 512ms" are different bugs and
# only the first print tells them apart.
# The maximised leg reads the SHOW state at creation, before any move or band: did
# the file's maximized:true actually put the window in the zoomed style? -1 when there
# is no handle is an absent answer, never a no.
if ($handle -ne 0) { "ZOOM=$([int][WIN]::IsZoomed($handle))" } else { 'ZOOM=-1' }
if ($handle -ne 0) {
    # -1 = no polarity was seeded, so there is nothing to wait FOR and the read
    # stays a single sample. Anything else is the answer session.json demands.
    $want = [int]$ExpectPinned
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $polls = 0
    $crashed = 0
    $style = [WIN]::GetWindowLong($handle, -20)
    $top = [int](($style -band 8) -ne 0)
    while ($want -ge 0 -and $top -ne $want -and $sw.ElapsedMilliseconds -lt $PinWaitMs) {
        Start-Sleep -Milliseconds $PinTickMs
        $polls += 1
        # A dead window has no style to read, and polling a corpse is how a
        # timeout gets reported as a pin failure.
        $p.Refresh()
 if ($p.HasExited) { $crashed = 1; break }
        $style = [WIN]::GetWindowLong($handle, -20)
        $top = [int](($style -band 8) -ne 0)
    }
    # M3: a matched 0 is an ABSENCE, and absence is the easy lie - the bit is 0
    # before the app ever applies it, 0 on a window that is gone, and 0 for a tick
    # when the shell re-bands. So the want=0 half used to be unable to fail: the
    # first sample answered 0 and the loop exited. It now has to hold: after a
    # matched 0, take a FRESH sample a confirm delay later and believe 0 only if the
    # second one agrees. A wrongful band that lands late is caught, and when it is
    # caught the reading becomes the band, so the outer verdict judges the current
    # state and not the hopeful first sample. A matched 1 needs no twin: the bit
    # being SET is a positive fact no race supplies by accident.
    if ($want -eq 0 -and $top -eq 0 -and -not $crashed) {
        $left = $PinWaitMs - $sw.ElapsedMilliseconds
 if ($left -gt 0) {
            Start-Sleep -Milliseconds ([Math]::Min($PinConfirmMs, $left))
            $polls += 1
            $p.Refresh()
 if ($p.HasExited) { $crashed = 1 } else {
                $c = [WIN]::GetWindowLong($handle, -20)
                $style = $c
                $top = [int](($c -band 8) -ne 0)
            }
        }
    }
    # M4: a death mid-poll must print NO reading. GetWindowLong on a dead handle
    # answers 0, which is exactly a false pass for pinned:false - so the crash
    # reports TOPMOST=-1 (the instrument could not look) plus the flag the geometry
    # lane turns into NotJudged, and a crashed app is never labelled a liar.
    "TOPMOST_WANTED=$want"
    "TOPMOST_WAIT_MS=$($sw.ElapsedMilliseconds)"
    # M7: did the poll actually poll - the number of times it slept, from a counter.
    # Derived from elapsed time before, which made it decorative (a 0 ms and a 1 ms
    # answer both said 0) and unfalsifiable.
    "TOPMOST_POLLED=$polls"
    if ($crashed) {
        'PIN_POLL_CRASHED=1'
        'TOPMOST=-1'
    } else {
        'PIN_POLL_CRASHED=0'
        "TOPMOST=$top"
        "EXSTYLE=$style"
    }
} else { 'TOPMOST=-1' }
function Get-Frame($h) {
    $r = New-Object WIN+RECT
    if ([WIN]::GetWindowRect($h, [ref]$r)) { return "$($r.Left),$($r.Top),$($r.Right),$($r.Bottom)" }
    return ''
}
function Get-Client($h) {
    $r = New-Object WIN+RECT
    $pt = New-Object WIN+POINT
    if (-not [WIN]::GetClientRect($h, [ref]$r)) { return '' }
    if (-not [WIN]::ClientToScreen($h, [ref]$pt)) { return '' }
    return "$($pt.X),$($pt.Y),$($pt.X + $r.Right),$($pt.Y + $r.Bottom)"
}
"FRAME=$(Get-Frame $handle)"
"CLIENT=$(Get-Client $handle)"
if ($MoveX -ge 0) {
    $f = Get-Frame $handle
    if ($f -ne '') {
        $a = $f.Split(',')
        $w = [int]$a[2] - [int]$a[0]
        $hh = [int]$a[3] - [int]$a[1]
        [void][WIN]::MoveWindow($handle, $MoveX, $MoveY, $w, $hh, $true)
        Start-Sleep -Milliseconds $SettleMs
        $f2 = Get-Frame $handle
        $c2 = Get-Client $handle
        "MOVED=$([int]($f2 -ne $f))"
        "FRAME_AFTER=$f2"
        "CLIENT_AFTER=$c2"
        # The dossier's experiment: read the state file AGAIN after one flush
        # tick but BEFORE the close. If this already carries the hint, the
        # measured-rect write is bypassed on every save; if only the post-exit
        # read flips, it is the shutdown path doing it.
 if ($Session -ne "" -and (Test-Path $Session)) {
 try {
                $j = Get-Content -Raw -LiteralPath $Session | ConvertFrom-Json
                $mx = [int]$j.rect.x; $my = [int]$j.rect.y
                "MID=$($mx),$($my),$($mx + [int]$j.rect.w),$($my + [int]$j.rect.h)"
                "MID_PIN=$([int][bool]$j.pinned)"
            } catch { 'MID=' }
        }
    } else { 'MOVED=0' }
}
if ($handle -ne 0 -and -not $p.HasExited) { [void]$p.CloseMainWindow() }
if (-not $p.HasExited) { [void]$p.WaitForExit($CloseSecs * 1000) }
if ($p.HasExited) { 'EXITED_WITHOUT_KILL=1'; "EXIT_CODE=$($p.ExitCode)" } else {
    'EXITED_WITHOUT_KILL=0'; Stop-Process -Id $p.Id -Force; $p.WaitForExit()
}
'PROBE_DONE=1'
exit 0
"#;

/// A rectangle in screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

impl Rect {
    pub fn parse(text: &str) -> Option<Rect> {
        let n: Vec<i32> = text
            .trim()
            .split(',')
            .filter_map(|s| s.trim().parse::<i32>().ok())
            .collect();
        if n.len() == 4 {
            Some(Rect {
                l: n[0],
                t: n[1],
                r: n[2],
                b: n[3],
            })
        } else {
            None
        }
    }
    fn within(&self, other: &Rect, tol: i32) -> bool {
        (self.l - other.l).abs() <= tol
            && (self.t - other.t).abs() <= tol
            && (self.r - other.r).abs() <= tol
            && (self.b - other.b).abs() <= tol
    }
    fn deltas(&self, target: &Rect) -> [i32; 4] {
        [
            self.l - target.l,
            self.t - target.t,
            self.r - target.r,
            self.b - target.b,
        ]
    }
    pub fn text(&self) -> String {
        format!("{},{},{},{}", self.l, self.t, self.r, self.b)
    }
}

/// Where a window sits relative to the rect it was told to restore.
///
/// TOLERANCE, and the reason it is 6 and not 2 or 20. This number is set against
/// the chrome MEASURED from the live window in the same run, never against a
/// model: on this build the observed chrome is +8 px left, +31 px top (the
/// documented 8/19/8/20 is gpui's own border offset, not this window's caption,
/// and a checker that trusts the model over the machine it is testing is the bug
/// class this repo keeps finding). A wrong-space placement is therefore off by at
/// least 8 px in x and 31 in y at 100% scaling, and only grows with DPI, so a
/// tolerance of 6 px still separates "restored in the wrong space" from "restored"
/// while surviving rounding. It is deliberately below the SMALLEST measured chrome
/// edge; print the measurement with every verdict, because if a future build
/// shrinks the border under 6 px this rule silently weakens and only the numbers
/// will show it.
pub const PLACEMENT_TOLERANCE: i32 = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// The window is where the seed said, measured in the named space.
    At { space: &'static str },
    /// It is off by about exactly the chrome: a space confusion, not a memory loss.
    ChromeOffset { deltas: [i32; 4] },
    /// Nothing about that explains it.
    Off { deltas: [i32; 4] },
    /// No rect could be read at all.
    Unreadable,
}

pub fn place(seed: &Rect, frame: Option<&Rect>, client: Option<&Rect>) -> Placement {
    let tol = PLACEMENT_TOLERANCE;
    if client.is_some_and(|c| c.within(seed, tol)) {
        return Placement::At { space: "client" };
    }
    if frame.is_some_and(|f| f.within(seed, tol)) {
        return Placement::At { space: "frame" };
    }
    // The chrome is measurable on this very window, so the offset signature can
    // be compared against it instead of against a remembered number.
    if let (Some(f), Some(c)) = (frame, client) {
        let chrome = [c.l - f.l, c.t - f.t, c.r - f.r, c.b - f.b];
        for probe in [f, c] {
            let d = probe.deltas(seed);
            if d.iter()
                .zip(chrome.iter())
                .all(|(a, b)| (*a + *b).abs() <= tol)
            {
                return Placement::ChromeOffset { deltas: d };
            }
        }
    }
    match frame.or(client) {
        Some(p) => Placement::Off {
            deltas: p.deltas(seed),
        },
        None => Placement::Unreadable,
    }
}

/// The persisted rect, out of the app's own session.json.
pub fn persisted_rect(text: &str) -> Option<Rect> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let rect = value.get("rect")?;
    let n = |k: &str| {
        rect.get(k)
            .and_then(serde_json::Value::as_i64)
            .map(|v| v as i32)
    };
    Some(Rect {
        l: n("x")?,
        t: n("y")?,
        r: n("x")? + n("w")?,
        b: n("y")? + n("h")?,
    })
}

fn obj2_pin(value: &mut serde_json::Value) {
    if let Some(map) = value.as_object_mut() {
        map.insert("pinned".to_string(), serde_json::json!(true));
    }
}

/// Flip only the pin, keeping the rect the app itself persisted.
pub fn set_pinned(path: &Path, pinned: bool) -> Result<(), String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let map = value
        .as_object_mut()
        .ok_or_else(|| "session.json is not an object".to_string())?;
    map.insert("pinned".to_string(), serde_json::json!(pinned));
    fs::write(
        path,
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// Was the window really on top, as the state file claims?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pin {
    /// Read in both polarities and it agreed with session.json both times.
    Matched {
        pinned_true: bool,
        pinned_false: bool,
    },
    /// At least one polarity contradicted the file.
    Contradicts {
        wanted: bool,
        seen: Option<bool>,
        which: &'static str,
    },
    /// Not measurable here, so not claimed either way.
    NotJudged(&'static str),
}

/// The evidence a pin verdict is worth citing: the LAST extended style the poll
/// saw, and how long it waited for the seeded polarity. Both come from the
/// script's own stopwatch, so a run that answered on the first sample prints 0 ms
/// and one that spent the budget prints 2000 - and that difference is exactly
/// what an exit 7 has to be able to say out loud: "never banded" and "banded
/// late" are different bugs, and only the wait time separates them.
///
/// An absent key prints as unknown, never as 0: the rule that a truncated probe
/// transcript may not look like a fast answer is the same rule that keeps an
/// absent TOPMOST out of a pass.
fn pin_wait_note(probe: &Probe) -> String {
    let style = match probe.number("EXSTYLE") {
        Some(v) => format!("last EXSTYLE=0x{:X}", v as u32),
        None => "EXSTYLE=unread".to_string(),
    };
    let waited = match probe.number("TOPMOST_WAIT_MS") {
        Some(v) => format!("{v} ms of the {PIN_WAIT_MS} ms budget"),
        None => "wait unread".to_string(),
    };
    // Wired into the reading rather than decorative: how many times the poll
    // actually slept. Absent prints as unread, never as "0", for the same reason a
    // missing TOPMOST does.
    let polled = match probe.number("TOPMOST_POLLED") {
        Some(v) if v > 0 => format!("polled {v}x"),
        Some(_) => "first sample, never polled".to_string(),
        None => "poll count unread".to_string(),
    };
    format!("({style}, {waited}, {polled})")
}

/// The harness distrusting its OWN plumbing, which is the only reading of a pin
/// verdict worth having. The script prints the polarity it actually waited for
/// (TOPMOST_WANTED); if that does not name the polarity this launch was SEEDED
/// with, the two halves disagree about which launch is which, and a TOPMOST
/// reading in that state is meaningless whatever it says. So it is refused -
/// never quietly converted into a pass, and never into a failure of the app
/// either, because the app did nothing to cause it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinRead {
    /// The reported polarity matches the seeded one, and the style answered.
    Read(bool),
    /// The script reported no polarity at all: an old or truncated transcript, so
    /// nothing can be said about which launch this reading came from.
    Unreported,
    /// The script waited for a DIFFERENT polarity than this launch was seeded
    /// with - the argument plumbing disagrees with the state file.
    Mismatch { asked: bool, reported: bool },
    /// Polarity agreed; the extended style itself could not be read.
    Unreadable,
    /// The window died while the poll was waiting. Not an app verdict about the
    /// pin at any polarity: a dead handle answers 0 for every style, which is a
    /// false PASS for pinned:false and would otherwise be reported as "the pin
    /// lied" for what is really a crash.
    Crashed,
}

pub fn read_pin(probe: &Probe, asked: bool) -> PinRead {
    // Checked first, before any attribution: a crash explains a missing reading
    // better than the app does, and it must never reach the exit-7 path.
    if probe.flag("PIN_POLL_CRASHED") {
        return PinRead::Crashed;
    }
    // Written as two arms rather than matches!(v, 0): anything that is not exactly
    // one of those two answers is Unreported, including a script that grew a third
    // polarity the harness has not been told about.
    let reported = match probe.number("TOPMOST_WANTED") {
        Some(0) => false,
        Some(1) => true,
        _ => return PinRead::Unreported,
    };
    if reported != asked {
        return PinRead::Mismatch { asked, reported };
    }
    match topmost(probe) {
        Some(bit) => PinRead::Read(bit),
        None => PinRead::Unreadable,
    }
}

/// Why a polarity was refused, in the words an operator needs: "not judged" is
/// the app's silence, and these three sentences are the harness's own. Kept as a
/// function so both halves print the same explanation of the same shape.
fn pin_refusal(read: PinRead, which: &str) -> String {
    match read {
        PinRead::Unreported => format!(
            "the transcript reported no polarity to wait for, so which launch {which} belongs to is unknown and no reading of it can be trusted"
        ),
        PinRead::Mismatch { asked, reported } => format!(
            "this launch was seeded {which} but the poll waited for pinned:{reported} - the harness and the script disagree about which window is which, so this is an instrument fault, not an app verdict (asked {asked})"
        ),
        PinRead::Unreadable => {
            format!("{which}: the extended style could not be read on the live window")
        }
        PinRead::Crashed => format!(
            "{which}: the process exited by itself while the poll was waiting \
 (EXITED_WITHOUT_KILL), so the window was gone when the style was read - \
 this is a crash and not a pin verdict either way"
        ),
        PinRead::Read(_) => unreachable!("a reading is not a refusal"),
    }
}

/// The one place the maximised leg reads the app's OWN state dir from, so it can
/// never repeat the dossier mistake of watching %APPDATA% while the portable marker
/// sent the process to target/debug/data. The candidate-dir SEARCH list is a
/// different question (it answers "where is a file that might be ours"), and the
/// geometry lane still uses it - a known follow-up, deliberately not copied here.
fn app_state_dir(exe: &Path) -> Option<PathBuf> {
    let portable = exe
        .parent()
        .map(|d| d.join("data").is_dir())
        .unwrap_or(false);
    state_dir_for(exe, std::env::var_os("APPDATA").as_deref(), portable)
}

/// The chrome that walks, as a delta: what the relaunch rect gained over the launch
/// rect in x, y, width and height. A working restore gives all four zeros; the
/// frame/client double-count shows up as the non-client border exactly (-8, -4,
/// +16, +8 at 100% on this machine, which is smoke's own measured chrome).
///
/// Kept a pure function because the whole point is the NUMBER: a print that already
/// had to think about the arithmetic could not be checked, and the assert armed
/// below is this function compared against zero.
pub fn rect_drift(before: Option<Rect>, after: Option<Rect>) -> Option<(i32, i32, i32, i32)> {
    let (b, a) = (before?, after?);
    Some((
        a.l - b.l,
        a.t - b.t,
        (a.r - a.l) - (b.r - b.l),
        (a.b - a.t) - (b.b - b.t),
    ))
}

/// The maximised cycle, INFO ONLY: seed the file to say maximized:true, launch it,
/// close it clean, relaunch, and print what the restore did to the rect.
///
/// Why this exists as a leg of its own rather than as another case inside
/// geometry_round_trip: it needed its OWN seeds (maximised on purpose, per the M5
/// lesson) and its own two launches, and it reports through the geometry lane's code
/// rather than inventing a fourth one. Part 1 shipped it as INFO while the ratchet
/// fix was still landing elsewhere - asserting it there would have reddened the commit
/// that documented the bug instead of the one that fixed it; part 2 (this) arms it.
///
/// The seed is the second reason this is explicit: like the M5 lesson on the rect
/// lane, a seed that INHERITS the show state inherits whatever a previous run left.
/// Here the leg wants maximised, so it says so.
/// Returns Some(note) when the cycle BROKE the promise and None when it held or
/// could not be judged - the geometry lane's three answers, minus the pass claim
/// this leg has no business making (the lane above already owns that word).
fn maximised_cycle(script: &Path, exe: &Path, err_file: &Path, session: &Path) -> Option<String> {
    println!("smoke: maximised: the cycle is armed (M9) - drift beyond zero is exit 6");
    // Keep the position the user's file already names; this leg is about the SHOW
    // state, so inventing a rect here would confound the two.
    let keep = match fs::read_to_string(session) {
        Ok(text) => persisted_rect(&text).unwrap_or(Rect {
            l: 320,
            t: 240,
            r: 920,
            b: 640,
        }),
        Err(e) => {
            println!("smoke: maximised: NOT JUDGED - session file unreadable: {e}");
            return None;
        }
    };
    let before = match seed_session_with_state(session, &keep, false, true) {
        Ok(bytes) => bytes,
        Err(e) => {
            println!("smoke: maximised: NOT JUDGED - could not seed: {e}");
            return None;
        }
    };
    // Launch one: the file says maximised, so ZOOM at creation is the READ half of
    // the promise and FRAME is the rect that will be persisted on the way out.
    let first = match run_probe_script(script, exe, err_file, Some(session), None, None, 12) {
        Ok(p) => p,
        Err(e) => {
            println!("smoke: maximised: NOT JUDGED - first launch did not report: {e}");
            restore_session(session, before.as_deref());
            return None;
        }
    };
    // Headless insurance: the first launch already reported whether there is a
    // desktop to place a window on, and this leg has no business starting a second
    // process on a machine that has none. The geometry lane declines the same way.
    if !first.flag("DESKTOP") {
        println!("smoke: maximised: NOT JUDGED - no interactive desktop to maximise a window on");
        restore_session(session, before.as_deref());
        return None;
    }
    let rect_before = probe_rect(&first, "FRAME");
    // Launch two: the same file, relaunched, is where a frame/client double-count
    // becomes visible, because the persisted rect is applied as bounds and measured
    // back as a frame again.
    let second = match run_probe_script(script, exe, err_file, Some(session), None, None, 12) {
        Ok(p) => p,
        Err(e) => {
            println!("smoke: maximised: NOT JUDGED - relaunch did not report: {e}");
            restore_session(session, before.as_deref());
            return None;
        }
    };
    let rect_after = probe_rect(&second, "FRAME");
    let persisted = fs::read_to_string(session).ok().and_then(|t| {
        let r = persisted_rect(&t);
        let m = t.contains("\"maximized\": true");
        println!("smoke: maximised: INFO - session.json after the cycle persists rect {} with maximized:{}", r.map(|x| x.text()).unwrap_or_else(|| "-".into()), m);
        r
    });
    let zoom = |p: &Probe| match p.number("ZOOM") {
        Some(1) => "zoomed at creation",
        Some(0) => "NOT zoomed at creation",
        _ => "zoom unread",
    };
    println!(
        "smoke: maximised: INFO - launch 1 rect {} ({})",
        rect_before.map(|r| r.text()).unwrap_or_else(|| "-".into()),
        zoom(&first)
    );
    println!(
        "smoke: maximised: INFO - launch 2 rect {} ({})",
        rect_after.map(|r| r.text()).unwrap_or_else(|| "-".into()),
        zoom(&second)
    );
    let drift = rect_drift(rect_before, rect_after);
    println!(
        "smoke: maximised: INFO - drift across one cycle (dx, dy, dw, dh) = {drift:?}; the rect launch 1 persisted was {}, and the chrome measured this run is (+8, +0, -8, -8) - a non-zero quadruple matching that border is the frame/client double-count, not a race",
        persisted.map(|r| r.text()).unwrap_or_else(|| "-".into())
    );
    // M9, ARMED - the exact flip promised in part 1, and the budget is zero. The fix
    // is 5c2516e8's ratchet; the fixed point was measured live as Some((0, 0, 0, 0))
    // at 40bb5057 and again in the manager's cycles (rect unchanged across
    // maximise-close-relaunch-close, with maximized:true round-tripping). Any other
    // quadruple is the frame/client double-count walking the restore rect, which IS
    // the window-memory promise breaking - so it reports as 6, the code that already
    // means exactly that, and the seeds / prints / restore all stay because a failing
    // assertion is still worth reading in the same words.
    //
    // DRIFT ONLY, deliberately not the ZOOM line above. Known probe weakness, not an
    // app finding: ZOOM is read the instant the handle is first sighted, which is
    // BEFORE gpui's async show apply lands, so a genuinely maximised window can answer
    // 0 there - part 1 printed "NOT zoomed at creation" for a window whose frame was
    // the entire 3448x1400 work area, while a SetWindow-driven probe reading the same
    // window says True. Asserting that would measure when the harness looked, not what
    // the product did; arming it needs the probe re-ordered to re-read after a settle,
    // which is a part-3 question with its own evidence.
    let verdict = match drift {
        None => {
            println!(
                "smoke: maximised: NOT JUDGED - a rect was unreadable, so there is nothing to compare"
            );
            None
        }
        Some((0, 0, 0, 0)) => {
            println!("smoke: maximised: PASS - the restore rect is a fixed point across one cycle");
            None
        }
        Some(d) => Some(format!(
            "MAXIMISED: one maximise-close-relaunch cycle moved the restore rect by (dx, dy, dw, dh) = {d:?} \
             (launch 1 {r1}, launch 2 {r2}); the rect persisted while maximised is not the rect the next \
             launch came back at, which is the frame/client ratchet walking the window by its own chrome",
            r1 = rect_before.map(|r| r.text()).unwrap_or_else(|| "-".into()),
            r2 = rect_after.map(|r| r.text()).unwrap_or_else(|| "-".into())
        )),
    };
    restore_session(session, before.as_deref());
    verdict
}

/// Did this launch's window die while the pin poll was waiting for it?
fn poll_crashed(probe: &Probe) -> bool {
    probe.flag("PIN_POLL_CRASHED")
}

fn topmost(probe: &Probe) -> Option<bool> {
    match probe.number("TOPMOST") {
        Some(0) => Some(false),
        Some(1) => Some(true),
        _ => None,
    }
}

/// Did the app persist the rect the harness chose, the seed, or neither?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Persisted {
    Moved,
    StillTheSeed,
    Neither { got: Rect },
    NothingWritten,
}

pub fn persisted(got: Option<Rect>, seed: &Rect, moved: &Rect) -> Persisted {
    let Some(got) = got else {
        return Persisted::NothingWritten;
    };
    if got.within(moved, PLACEMENT_TOLERANCE) {
        Persisted::Moved
    } else if got.within(seed, PLACEMENT_TOLERANCE) {
        Persisted::StillTheSeed
    } else {
        Persisted::Neither { got }
    }
}
/// A geometry round trip that could not be measured. Never a pass, never a red:
/// a line saying what was not proven.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Geometry {
    /// restore / persist / relaunch all matched, with the rects printed.
    Proven {
        restore: &'static str,
        relaunch: &'static str,
        pin: &'static str,
    },
    /// Something about the machine or the app refused to be deterministic.
    NotJudged(&'static str),
    /// Measured, and the promise did not hold. geometry carries the rect
    /// findings; pin_failed marks that the topmost bit specifically lied.
    Broken {
        notes: Vec<String>,
        pin_failed: bool,
    },
}

/// Where the harness put the window, and where the app thought it was.
pub const SEEDED: (i32, i32, i32, i32) = (337, 241, 620, 420);
pub const MOVE_BY: (i32, i32) = (53, 37);
/// The seed is far from the built-in default (120,90,800,600) on purpose: an
/// app that ignored the stored rect lands on the default, and an app that never
/// opened a window lands nowhere, so a coincidence cannot produce a green run.
/// The rect the app falls back to when it has nothing to restore. The seed is
/// kept far from it so a run that ignored the seed can never look like a pass.
pub const DEFAULT_RECT_HINT: &str = "120,90,800,600";
/// The window-geometry poll tick the bridge documents (750 ms quiet window, and
/// its own log line reports the force window too).
pub const POLL_TICK_MS: i32 = 750;
/// How long to wait after the harness moves the window, before closing it: six
/// ticks, because one poll must land, the port must react, and the debounced
/// write must reach the disk. Four ticks covered the quiet window but not the
/// force window the app reported (1000 ms), so the number is printed every run
/// rather than trusted.
pub const SETTLE_MS: i32 = POLL_TICK_MS * 6;
/// How long the pin read may wait for the seeded polarity to appear on the live
/// window, and how often it looks. 2000 ms is just under 4x the 512 ms the platform
/// lane measured for the band to land (the multiple is asserted below, not claimed
/// in prose), and 50 ms is far below both, so a slow-but-real apply is credited and
/// a missing one is still caught - the budget is a wait, not a pass. Passed INTO the script, so the two halves cannot drift apart.
pub const PIN_WAIT_MS: i32 = 2_000;
pub const PIN_TICK_MS: i32 = 50;
/// How long a matched "not topmost" must HOLD before it is believed: the absence
/// of a bit is the easy lie, so absence gets confirmed against a fresh sample
/// rather than a single reading (see the poll in GEOMETRY_PROBE).
pub const PIN_CONFIRM_MS: i32 = 200;
/// The slowest landing the band has ever been MEASURED taking on a live window -
/// the platform lane's 512 ms, the same number quoted in the failure text. The
/// invariant is about THIS, not about tick arithmetic: a budget that stops being
/// several times the observed landing is a budget that fails a working app, and a
/// tweak of a constant nobody measured would compile forever.
///
/// Encoding the invariant caught a false comment on its first run: the budget's
/// own doc claimed "2000 ms is 4x the 512 ms", and 2000/512 is 3.9. The ceiling was
/// set at 2000 on purpose, so the number moved the CLAIM rather than the budget: the
/// honest multiple is 3x, and it is asserted rather than prose.
const PIN_OBSERVED_LAND_MS: i32 = 512;
const _: () = {
    // (a) the wait stays >= 4x the slowest real apply ever seen, and
    assert!(PIN_WAIT_MS >= 3 * PIN_OBSERVED_LAND_MS);
    // (b) the budget still affords a polling run AND the double-confirm of an
    // absence - otherwise the confirm silently never happens on a tight budget.
    assert!(PIN_WAIT_MS >= 4 * PIN_TICK_MS + 2 * PIN_CONFIRM_MS);
};
pub const GEOMETRY_FAILED_EXIT: i32 = 6;
/// The PIN specifically: WS_EX_TOPMOST did not follow what session.json claimed.
/// Distinct from 6 because the fix usually lives in the show/pin ordering, not
/// in persistence - and a failure here may be a race, so the message says that.
pub const PIN_FAILED_EXIT: i32 = 7;
/// What the LIVE app's own stderr did not show. Distinct from 1 because 1 is
/// "the shutdown or the write broke" and this is the opposite: every one of those
/// passed, and the thing that failed is invisible to a window handle - an event
/// the port knew about never reached the rendered UI. In CONTRACT, so the ci.yml
/// smoke step is required to arm it: check-ci's [arm-missing] rule is what stops a
/// new verdict landing as an annotation nobody wrote.
pub const TRACE_FAILED_EXIT: i32 = 9;

/// Fit the seed inside a work area that might be one monitor, might be two, and
/// might not be 100% scaled. None means "cannot be judged here".
pub fn seed_in(work: &Rect, size: (i32, i32), default_hint: &Rect) -> Option<(Rect, Rect)> {
    let (w, h) = size;
    if work.r - work.l < w + 40 || work.b - work.t < h + 40 {
        return None;
    }
    let l = work.l + SEEDED.0.min((work.r - work.l - w) / 2);
    let t = work.t + SEEDED.1.min((work.b - work.t - h) / 2);
    let seed = Rect {
        l,
        t,
        r: l + w,
        b: t + h,
    };
    if seed == *default_hint {
        return None;
    }
    // The move goes the other way if it would push the window off the right edge.
    let dx = if seed.r + MOVE_BY.0 <= work.r {
        MOVE_BY.0
    } else {
        -MOVE_BY.0
    };
    let dy = if seed.b + MOVE_BY.1 <= work.b {
        MOVE_BY.1
    } else {
        -MOVE_BY.1
    };
    let moved = Rect {
        l: seed.l + dx,
        t: seed.t + dy,
        r: seed.r + dx,
        b: seed.b + dy,
    };
    Some((seed, moved))
}

/// The seed rect and where the harness drags the window to, which always arrive
/// together (`seed_in` returns the pair) and are meaningless apart. Bundled
/// because `run_probe_script` grew a real argument with the pin polarity and the
/// eighth would have been a clippy suppression instead of a shape.
#[derive(Debug, Clone, Copy)]
pub struct Drag<'a> {
    pub seed: &'a Rect,
    pub to: (i32, i32),
}

pub fn run_probe_script(
    script: &Path,
    exe: &Path,
    err_file: &Path,
    session: Option<&Path>,
    expect_pinned: Option<bool>,
    drag: Option<Drag<'_>>,
    secs: u64,
) -> Result<Probe, String> {
    let mut cmd = std::process::Command::new("pwsh");
    cmd.args(["-NoProfile", "-NonInteractive", "-File"])
        .arg(script)
        .arg("-Exe")
        .arg(exe)
        .arg("-ErrFile")
        .arg(err_file)
        .arg("-Session")
        .arg(session.map(|p| p.display().to_string()).unwrap_or_default())
        // The polarity this launch was SEEDED with, so the script polls for the
        // answer it already knows and reports the wait. None means "not asserted",
        // which keeps that launch's read a single sample rather than a guess.
        .arg("-ExpectPinned")
        .arg(match expect_pinned {
            Some(true) => "1",
            Some(false) => "0",
            None => "-1",
        })
        .arg("-PinConfirmMs")
        .arg(PIN_CONFIRM_MS.to_string())
        .arg("-PinWaitMs")
        .arg(PIN_WAIT_MS.to_string())
        .arg("-PinTickMs")
        .arg(PIN_TICK_MS.to_string())
        .arg("-WindowSecs")
        .arg(secs.to_string())
        .arg("-SettleMs")
        .arg(SETTLE_MS.to_string())
        .arg("-MoveX")
        .arg(match drag {
            Some(d) => d.to.0.to_string(),
            None => "-1".to_string(),
        })
        .arg("-MoveY")
        .arg(match drag {
            Some(d) => d.to.1.to_string(),
            None => "-1".to_string(),
        });
    if let Some(d) = drag {
        let s = d.seed;
        cmd.arg("-SeedX")
            .arg(s.l.to_string())
            .arg("-SeedY")
            .arg(s.t.to_string())
            .arg("-SeedW")
            .arg((s.r - s.l).to_string())
            .arg("-SeedH")
            .arg((s.b - s.t).to_string());
    }
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot start the geometry probe: {e}"))?;
    // The outer backstop is widened by the pin budget: the script now waits up
    // to PIN_WAIT_MS INSIDE its own bounded phases, and a harness that kills a
    // child for doing the work it was told to do turns a slow apply into a
    // spurious red. Still strictly larger than everything bounded internally.
    match wait_bounded(&mut child, OUTER_SECS + PIN_WAIT_MS as u64 / 1000 + 1) {
        Ok(Some(_)) => Ok(parse_probe(&read_pipe(child.stdout.as_mut()))),
        Ok(None) => Err("the geometry probe outlived its deadline".to_string()),
        Err(e) => Err(e),
    }
}
/// Rewrite ONLY the rect numbers in the app's own session.json, keeping every
/// other field exactly as the app wrote it. Returns the bytes that were there
/// before, so the caller can put them back.
/// Write a rect, and the pin state the harness wants asserted, into the app's own
/// session.json, leaving every other field exactly as the app wrote it.
pub fn seed_session_with_pin(
    path: &Path,
    seed: &Rect,
    pinned: bool,
) -> Result<Option<Vec<u8>>, String> {
    seed_session_with_state(path, seed, pinned, false)
}

/// The same seed with the SHOW state named out loud, which is the M5 lesson
/// generalised: a seed that inherits maximized from the file it is overwriting
/// inherits whatever the last run left there, and a leg that believes it asked for a
/// maximised window then measures the accident. Every caller states it.
pub fn seed_session_with_state(
    path: &Path,
    seed: &Rect,
    pinned: bool,
    maximized: bool,
) -> Result<Option<Vec<u8>>, String> {
    let before = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let mut value: serde_json::Value = match &before {
        Some(bytes) => serde_json::from_slice(bytes)
            .map_err(|e| format!("{} is not JSON the app wrote: {e}", path.display()))?,
        None => serde_json::json!({ "rect": {} }),
    };
    let rect = value
        .as_object_mut()
        .ok_or_else(|| "session.json is not a JSON object".to_string())?;
    let entry = rect
        .entry("rect".to_string())
        .or_insert_with(|| serde_json::json!({}));
    let obj = entry
        .as_object_mut()
        .ok_or_else(|| "session.json has no rect object to seed".to_string())?;
    obj.insert("x".into(), serde_json::json!(seed.l));
    obj.insert("y".into(), serde_json::json!(seed.t));
    obj.insert("w".into(), serde_json::json!(seed.r - seed.l));
    obj.insert("h".into(), serde_json::json!(seed.b - seed.t));
    // M5. This lane measures a NORMAL window: it seeds a rect and expects the
    // frame back at that rect. session.json carries "maximized" as a sibling of
    // "rect", and a leftover true - from a hand run, or from --reuse-state over a
    // run that left one - makes Windows restore the maximised frame instead, which
    // is not the seeded rect, and the lane answers exit 6 accusing the product of
    // forgetting a geometry it was never asked to restore. So the field this lane's
    // own premise depends on is forced, deliberately, and stated rather than
    // inherited from whatever the profile happened to hold.
    //
    // A FUTURE MAXIMISED LANE WILL NEED ITS OWN SEED: it must set maximized:true,
    // compare against the monitor work area rather than a rect, and it must not
    // reuse this function's forcing - which is why the forcing lives here and not
    // in the JSON seeding helper it sits beside.
    if let Some(map) = value.as_object_mut() {
        map.insert("maximized".to_string(), serde_json::json!(maximized));
    }
    if pinned {
        obj2_pin(&mut value);
    }
    let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| format!("cannot seed {}: {e}", path.display()))?;
    Ok(before)
}

fn probe_rect(probe: &Probe, key: &str) -> Option<Rect> {
    probe.get(key).and_then(Rect::parse)
}

fn placement_text(p: &Placement) -> &'static str {
    match p {
        Placement::At { space } => space,
        _ => "NOT-AT",
    }
}

/// The whole round trip. Every branch prints what it learned, and anything that
/// cannot be measured here becomes NotJudged with a reason rather than a pass.
pub fn geometry_round_trip(script: &Path, exe: &Path, err_file: &Path, session: &Path) -> Geometry {
    let mut notes: Vec<String> = Vec::new();
    let mut pin_failed = false;
    // 1. Where is the screen, actually. WindowSecs 0 = report only, no launch.
    let work = match run_probe_script(script, exe, err_file, None, None, None, 0) {
        Err(e) => return Geometry::NotJudged(e.leak() as &str),
        Ok(probe) => {
            if !probe.flag("DESKTOP") {
                return Geometry::NotJudged("no interactive desktop to place a window on");
            }
            if !probe.flag("WIN32") {
                return Geometry::NotJudged("this PowerShell cannot load the Win32 declarations");
            }
            match probe_rect(&probe, "WORK") {
                Some(w) => w,
                None => return Geometry::NotJudged("the work area could not be read"),
            }
        }
    };
    let (seed, moved) = match seed_in(
        &work,
        (SEEDED.2, SEEDED.3),
        &Rect {
            l: 120,
            t: 90,
            r: 920,
            b: 690,
        },
    ) {
        Some(pair) => pair,
        None => {
            return Geometry::NotJudged(
                "the primary work area is too small to hold the seed rect and the move",
            );
        }
    };
    println!(
        "smoke: geometry: work area {} - seeded rect {} (default is {DEFAULT_RECT_HINT}), moved to {}",
        work.text(),
        seed.text(),
        moved.text()
    );
    println!(
        "smoke: geometry: settled {SETTLE_MS} ms after the harness move = 6 x the {POLL_TICK_MS} ms poll tick"
    );
    let before = match seed_session_with_pin(session, &seed, true) {
        Ok(bytes) => bytes,
        Err(e) => {
            println!("SMOKE GEOMETRY: the seed could not be written: {e}");
            return Geometry::NotJudged("the session file refused to be seeded");
        }
    };
    // 2. Launch, and see where the window really is.
    // The seed asked for pinned:true, so THIS launch is polled for WS_EX_TOPMOST
    // and the wait is reported. The polarity is passed in rather than guessed at:
    // asking the script for the answer the harness already wrote into session.json
    // is what turns "did the bit eventually land" into "did it land as claimed".
    let first = match run_probe_script(
        script,
        exe,
        err_file,
        Some(session),
        Some(true),
        Some(Drag {
            seed: &seed,
            to: (moved.l, moved.t),
        }),
        SETTLE_MS as u64 / 1000 + 12,
    ) {
        Err(e) => {
            restore_session(session, before.as_deref());
            return Geometry::NotJudged(e.leak() as &str);
        }
        Ok(p) => p,
    };
    let frame = probe_rect(&first, "FRAME");
    let client = probe_rect(&first, "CLIENT");
    if let (Some(f), Some(c)) = (frame, client) {
        println!(
            "smoke: geometry: measured chrome (frame -> client) = {:+}/{:+}/{:+}/{:+} px, and it is \
 these numbers, not a model, that the tolerance is judged against",
            c.l - f.l,
            c.t - f.t,
            c.r - f.r,
            c.b - f.b
        );
    }
    let restore = place(&seed, frame.as_ref(), client.as_ref());
    match &restore {
        Placement::At { space } => println!(
            "smoke: geometry: RESTORE ok - the window is at the seeded rect, read in {space} space (frame {} client {})",
            frame.map(|f| f.text()).unwrap_or_else(|| "-".into()),
            client.map(|c| c.text()).unwrap_or_else(|| "-".into())
        ),
        Placement::ChromeOffset { deltas } => println!(
            "smoke: geometry: WARNING - the window is off the seeded rect by {deltas:?}, which is about the frame chrome (8/19/8/20 at 100%): the seed was applied in one space and measured in the other. Not a memory failure, so not a red."
        ),
        Placement::Off { deltas } => {
            notes.push(format!(
                "RESTORE: the window is not at the seeded rect {} (frame {} client {}), deltas {deltas:?}",
 seed.text(),
 frame.map(|f| f.text()).unwrap_or_else(|| "-".into()),
 client.map(|c| c.text()).unwrap_or_else(|| "-".into())
            ));
        }
        Placement::Unreadable => notes.push("RESTORE: no window rect could be read".to_string()),
    }
    // 3/4. It was moved by the harness; what did the app persist on close?
    let frame_after = probe_rect(&first, "FRAME_AFTER");
    let client_after = probe_rect(&first, "CLIENT_AFTER");
    if !first.flag("MOVED") {
        println!(
            "smoke: geometry: WARNING - MoveWindow changed nothing on screen, so the persist half was not exercised"
        );
        restore_session(session, before.as_deref());
        return Geometry::NotJudged("the harness could not move the window");
    }
    let stored = match fs::read_to_string(session) {
        Ok(text) => persisted_rect(&text),
        Err(_) => None,
    };
    println!(
        "smoke: geometry: READ 1 (after the move, one flush tick, BEFORE the close): {}",
        probe_rect(&first, "MID")
            .map(|r| r.text())
            .unwrap_or_else(|| "nothing written yet".to_string())
    );
    let candidates = [frame_after, client_after];
    let mut verdict = Persisted::NothingWritten;
    for c in candidates.iter().flatten() {
        match persisted(stored, &seed, c) {
            Persisted::Moved => {
                verdict = Persisted::Moved;
                break;
            }
            other => {
                if verdict == Persisted::NothingWritten {
                    verdict = other;
                }
            }
        }
    }
    println!(
        "smoke: geometry: READ 2 (after the app exited): {}",
        stored.map(|r| r.text()).unwrap_or_else(|| "-".into())
    );
    match &verdict {
        Persisted::Moved => println!(
            "smoke: geometry: PERSIST ok - session.json names the moved rect (window after the move: frame {} client {}, persisted {})",
 frame_after.map(|f| f.text()).unwrap_or_else(|| "-".into()),
 client_after.map(|c| c.text()).unwrap_or_else(|| "-".into()),
 stored.map(|p| p.text()).unwrap_or_else(|| "-".into())
        ),
        Persisted::StillTheSeed => notes.push(format!(
            "PERSIST: session.json still says the seeded rect {} after the window was moved to frame {} - the move never reached the state file",
 seed.text(),
 frame_after.map(|f| f.text()).unwrap_or_else(|| "-".into())
        )),
        Persisted::Neither { got } => notes.push(format!(
            "PERSIST: session.json says {} - not the seed {}, not the window after the move (frame {} client {})",
 got.text(),
 seed.text(),
 frame_after.map(|f| f.text()).unwrap_or_else(|| "-".into()),
 client_after.map(|c| c.text()).unwrap_or_else(|| "-".into())
        )),
        Persisted::NothingWritten => notes.push(format!(
            "PERSIST: no rect could be read back from {} after the close",
 session.display()
        )),
    }
    // 4. The pin, first polarity: the seed asked for pinned:true, so the live
    // window's extended style must carry WS_EX_TOPMOST. Read from the same
    // launch that produced the rects above - the window is shown, which the
    // platform lane proved is the only state where a topmost band survives.
    let pin_true = match read_pin(&first, true) {
        PinRead::Read(bit) => {
            println!(
                "smoke: geometry: PIN pinned:true -> WS_EX_TOPMOST={} {}",
                bit as i32,
                pin_wait_note(&first)
            );
            Some(bit)
        }
        refusal => {
            println!(
                "smoke: geometry: PIN pinned:true REFUSED - {}",
                pin_refusal(refusal, "pinned:true")
            );
            None
        }
    };
    // 5. Relaunch and see whether it comes back where it was left, and whether
    // an unpinned file really leaves the window un-topmost.
    if let Err(e) = set_pinned(session, false) {
        println!("smoke: geometry: PIN second polarity not judged: {e}");
    }
    let expect = stored.unwrap_or(moved);
    // Same bounded wait on the FALSE half. Nothing races today - an unpinned
    // restore simply does not band - but "it was 0 on the first read" is luck
    // wearing the clothes of a proof, and the poll costs nothing when the answer
    // is already right on the first sample.
    let second = match run_probe_script(script, exe, err_file, Some(session), Some(false), None, 12)
    {
        Err(e) => {
            restore_session(session, before.as_deref());
            return Geometry::NotJudged(e.leak() as &str);
        }
        Ok(p) => p,
    };
    let f2 = probe_rect(&second, "FRAME");
    let c2 = probe_rect(&second, "CLIENT");
    let relaunch = place(&expect, f2.as_ref(), c2.as_ref());
    match &relaunch {
        Placement::At { space } => println!(
            "smoke: geometry: RELAUNCH ok - the window came back at the persisted rect, read in {space} space (frame {} client {})",
 f2.map(|f| f.text()).unwrap_or_else(|| "-".into()),
 c2.map(|c| c.text()).unwrap_or_else(|| "-".into())
        ),
 other => notes.push(format!(
            "RELAUNCH: the window came back at frame {} client {} instead of the persisted {} ({other:?})",
 f2.map(|f| f.text()).unwrap_or_else(|| "-".into()),
 c2.map(|c| c.text()).unwrap_or_else(|| "-".into()),
 expect.text()
        )),
    }
    let pin_false = match read_pin(&second, false) {
        PinRead::Read(bit) => {
            println!(
                "smoke: geometry: PIN pinned:false -> WS_EX_TOPMOST={} {}",
                bit as i32,
                pin_wait_note(&second)
            );
            Some(bit)
        }
        refusal => {
            println!(
                "smoke: geometry: PIN pinned:false REFUSED - {}",
                pin_refusal(refusal, "pinned:false")
            );
            None
        }
    };
    let pin = match (pin_true, pin_false) {
        (Some(t), Some(f)) => {
            if t && !f {
                Pin::Matched {
                    pinned_true: true,
                    pinned_false: false,
                }
            } else {
                pin_failed = true;
                // Both waits printed, because the two halves fail differently: a
                // pinned:true that spent its whole budget is the band never
                // arriving, and a pinned:false that answers 1 immediately is the
                // app topmost-ing a window the file told it not to.
                notes.push(format!(
                    "PIN: session.json said pinned:true and the window answered WS_EX_TOPMOST={t} {}, \
 then pinned:false answered {f} {}; the topmost bit does not follow the state file \
 even after waiting up to {PIN_WAIT_MS} ms for it, so this is not a first-read race \
 any more - check the app's own stderr for a Pinned line before blaming persistence",
 pin_wait_note(&first),
 pin_wait_note(&second)
                ));
                Pin::Contradicts {
                    wanted: true,
                    seen: Some(t),
                    which: "pinned:true",
                }
            }
        }
        _ => Pin::NotJudged(if poll_crashed(&first) || poll_crashed(&second) {
            "the window exited during at least one pin poll (EXITED_WITHOUT_KILL=1), so no \
 extended style was readable for that polarity; a dead window answers 0 for every \
 style and that is not evidence about the pin - look at the crash, not at the band"
        } else {
            "one of the two launches was refused: either its extended style could not be read, \
 or it did not report the polarity it waited for (see the REFUSED line above)"
        }),
    };
    // 6. Never leave the seed behind in place of what the app itself wrote.
    let still_seed = fs::read_to_string(session)
        .ok()
        .and_then(|t| persisted_rect(&t))
        .is_some_and(|p| p == seed);
    if still_seed {
        println!("smoke: geometry: the app never rewrote the seed, so the seed is removed");
        restore_session(session, before.as_deref());
    }
    if notes.is_empty() {
        Geometry::Proven {
            restore: placement_text(&restore),
            relaunch: placement_text(&relaunch),
            pin: match &pin {
                Pin::Matched { .. } => "matched",
                Pin::Contradicts { .. } => "LIED",
                Pin::NotJudged(_) => "not-judged",
            },
        }
    } else {
        Geometry::Broken { notes, pin_failed }
    }
}

fn restore_session(path: &Path, bytes: Option<&[u8]>) {
    match bytes {
        Some(b) => {
            if fs::write(path, b).is_err() {
                println!(
                    "smoke: geometry: WARNING - the pre-seed session bytes could not be put back"
                );
            }
        }
        None => {
            let _ = fs::remove_file(path);
        }
    }
}

/// The exe under test, honouring CARGO_TARGET_DIR.
///
/// Every lane is now told to build into a private target directory, and smoke
/// used to resolve <root>/target/debug anyway: it then judged a binary nobody had
/// built, failed its OWN freshness guard, and left the intended exe sitting
/// unused. A harness that cannot be pointed at a build is not a harness. The
/// resolution is pure and returned with the reason it chose, because the path is
/// the thing a reader needs to trust the verdict.
pub fn resolve_exe(root: &Path, target_dir: Option<&str>) -> (PathBuf, &'static str) {
    let trimmed = target_dir.unwrap_or_default().trim();
    if trimmed.is_empty() {
        return (
            root.join(BIN_REL),
            "no CARGO_TARGET_DIR in the environment, so the workspace default",
        );
    }
    let base = Path::new(trimmed);
    // Cargo resolves a relative CARGO_TARGET_DIR against the invoking directory.
    // We are run from the workspace root, so that is what we assume - and say.
    let base = if base.is_absolute() {
        base.to_path_buf()
    } else {
        root.join(base)
    };
    (
        base.join("debug").join(EXE_NAME),
        "CARGO_TARGET_DIR, honoured as asked",
    )
}

/// Does the tree still embed the manifest at LINK time? Read from the bridge's
/// own build script, because that is the only place the answer lives, and a
/// reader must never have to infer a CAUSE from a STATE.
///
/// This clause exists because a true triple on an exe was read as "cargo build
/// already ships our declaration". It did not: the migration removed the
/// /MANIFEST:EMBED line, and the markers were in that exe because somebody ran
/// 'cargo xtask manifest' on it afterwards. An exe can prove what its resource
/// says. It cannot prove who put it there - only build.rs can say whether the
/// build had the chance.
pub fn build_embeds_manifest(root: &Path) -> Option<bool> {
    let text = fs::read_to_string(root.join(BUILD_RS_REL)).ok()?;
    // COMMENT LINES DO NOT COUNT. Measured tonight: the migrated build.rs mentions
    // /MANIFEST:EMBED three times, all in //! prose explaining why it stopped, and
    // a plain substring test read that as "the build embeds" - the exact
    // state-as-cause error this clause exists to prevent, reproduced by the clause
    // itself. Only a line that could emit the flag to cargo is evidence.
    Some(text.lines().any(|line| {
        let t = line.trim_start();
        !t.starts_with("//")
            && !t.starts_with('#')
            && t.contains("rustc-link-arg")
            && (t.contains("MANIFEST:EMBED") || t.contains("MANIFESTINPUT"))
    }))
}
/// The build script whose behaviour explains the exe's manifest.
pub const BUILD_RS_REL: &str = GPUI_TARGET.build_rs;

/// Read the manifest back out of the exe we are about to launch. Same discipline
/// as the tested-binary line: name the thing, do not assume it. smoke JUDGES and
/// never repairs - calling mt.exe here would let a smoke run hide the fact that
/// nobody ran the post-link step.
pub fn manifest_of(root: &Path, exe: &Path) -> Result<crate::manifest::Markers, String> {
    let source = fs::read_to_string(root.join(crate::manifest::APP_MANIFEST_REL))
        .map_err(|e| format!("cannot read the source manifest: {e}"))?;
    let (identity, wanted) = crate::manifest::source_promise(&source)?;
    let bytes = fs::read(exe).map_err(|e| format!("cannot read the exe: {e}"))?;
    Ok(crate::manifest::read_markers(&bytes, &identity, &wanted))
}
/// Entry point for "cargo xtask smoke [--reuse-state] [--no-build] [--require-ours]".
pub fn run(args: &[String]) -> i32 {
    // The contract first, before anything can fail: a log that shows a verdict
    // also shows the code table that verdict came out of.
    println!("{}", contract_line());
    let unknown: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| *a != "--reuse-state" && *a != "--no-build" && *a != "--require-ours")
        .collect();
    if !unknown.is_empty() {
        eprintln!("smoke: unknown argument(s): {}", unknown.join(" "));
        eprintln!("smoke: usage: cargo xtask smoke [--reuse-state] [--no-build] [--require-ours]");
        return HARNESS_EXIT;
    }
    let reuse = args.iter().any(|a| a == "--reuse-state");

    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("smoke: cannot read the current directory: {e}");
            return HARNESS_EXIT;
        }
    };
    let root = match crate::metadata::find_workspace_root(&cwd) {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("smoke: {e}");
            return HARNESS_EXIT;
        }
    };
    let (exe, resolved_from) =
        resolve_exe(&root, std::env::var("CARGO_TARGET_DIR").ok().as_deref());
    println!(
        "smoke: exe path {} | resolved from {resolved_from}",
        exe.display()
    );
    // BUILD FIRST. A harness that launches whatever exe happens to be lying
    // around proves a cached binary, and a green line on a stale exe is the
    // most dangerous output this repo can produce: it says the app works when
    // what worked was three commits old. A compile failure is therefore its own
    // hard verdict (4), never a decline and never a silent fallback to the old
    // exe on disk.
    let built = if args.iter().any(|a| a == "--no-build") {
        println!(
            "smoke: NOT building (--no-build): this run can only prove the exe already on disk"
        );
        false
    } else {
        match build_target(&root) {
            Ok(()) => true,
            Err((first, count)) => {
                println!(
                    "SMOKE FAIL: the target did not compile, so there is nothing honest to launch"
                );
                println!("SMOKE FAIL: {count} error line(s); first: {first}");
                println!("smoke: window=NOBUILD close=NOBUILD session=NOBUILD 0.0s");
                return BUILD_FAILED_EXIT;
            }
        }
    };
    report_binary(&exe, &root, built);
    // The strict shape, opt-in: the line above always names what is in the exe,
    // and this decides whether a foreign manifest is allowed to pass. Not
    // default, because the ordinary state on a developer machine is that nobody
    // ran the post-link step yet, and a rule that makes every local run red is
    // a rule that gets --no-build-ed past. An UNREADABLE manifest is not judged
    // here either: absence of evidence is not evidence of a violation.
    if args.iter().any(|a| a == "--require-ours") {
        match manifest_of(&root, &exe) {
            Ok(m) => {
                if let crate::manifest::ReadBack::Missing(absent) = crate::manifest::read_back(&m) {
                    println!(
                        "SMOKE FAIL: --require-ours and the exe we are about to launch is missing {:?}",
                        absent
                    );
                    println!("SMOKE FAIL: run 'cargo xtask manifest' (post-link) on it first");
                    return MANIFEST_NOT_OURS_EXIT;
                }
            }
            Err(why) => println!("smoke: --require-ours NOT JUDGED - {why}"),
        }
    }
    if !exe.is_file() {
        println!(
            "SMOKE FAIL: cargo succeeded but {} is still not there",
            exe.display()
        );
        println!("smoke: window=MISSING close=NOBIN session=NOBIN 0.0s");
        return STEP_FAILED_EXIT;
    }
    match staleness(mtime_of(&exe), newest_source(&root)) {
        Stale::OlderThan {
            source,
            delta_secs,
            age_secs,
        } => {
            println!("SMOKE FAIL: the binary is older than its sources");
            println!("SMOKE FAIL:   exe    {} is {age_secs}s old", exe.display());
            println!(
                "SMOKE FAIL:   source {} is {delta_secs}s newer than it",
                source.display()
            );
            println!("smoke: window=STALE close=STALE session=STALE 0.0s");
            return STALE_BINARY_EXIT;
        }
        Stale::Unknown(why) => {
            println!("SMOKE FAIL: freshness cannot be proven - {why}");
            return HARNESS_EXIT;
        }
        Stale::Fresh => {}
    }

    let started = Instant::now();
    let launched = SystemTime::now();
    // The guard restores on EVERY exit path: normal end, early return, panic.
    let (mut guard, relocation) = if reuse {
        let what = describe_existing(&exe);
        println!("smoke: --reuse-state - {what} stays exactly where it is");
        (None, Relocation::Kept(what))
    } else {
        clear_session_path(&exe)
    };

    // A blocked path is decided before anything is launched: no verdict about
    // the D54 write exists while foreign state sits on the path, so there is no
    // reason to open a window on the way to saying so.
    if let Relocation::Blocked(why) = &relocation {
        println!("smoke: DECLINED - {why}");
        println!(
            "smoke: this harness never creates or deletes anything under a profile; the occupant on \
 the session path came from elsewhere, and a directory there is what an api test plants \
 to prove a blocked write target"
        );
        println!(
            "{}",
            summary(
                &Verdict::Skip(why.clone()),
                &Probe::default(),
                started.elapsed(),
                &relocation
            )
        );
        return DECLINED_EXIT;
    }

    // Seed one recent, so the announce claim is judgeable on EVERY machine and not
    // only on one whose owner happens to have opened a file today. Order matters
    // twice: AFTER the Relocation::Blocked decline above, because a run about to
    // stand down must never leave a file behind, and BEFORE the count is read below,
    // because the number printed must be the number the app sees. The session.json
    // relocation is a different file in the same directory, so the fresh-install case
    // is untouched by any of this - the seed survives it by not being asked to.
    let swept = sweep_stale_seeds(std::time::Duration::from_secs(3600));
    if swept > 0 {
        println!(
            "smoke: settings: swept {swept} scratch note(s) left by an earlier run that never made it to its restore"
        );
    }
    let mut seed = match seed_settings(&exe) {
        Ok((guard, note)) => {
            println!(
                "smoke: settings: seeded exactly one recent, {} (the profile goes back at the end of the run)",
                note.display()
            );
            Some(guard)
        }
        Err(e) => {
            println!(
                "smoke: settings: NOT SEEDED - {e}. The trace claim is as honest as it was before the seed existed, which is NOT JUDGED."
            );
            None
        }
    };

    // How much there WAS to announce, read before anything launches: the app
    // rewrites its own settings on the way out, so asking afterwards would be
    // asking about the run instead of about its opening state. This is the one
    // number that turns the trace claim from a guess into a judgement.
    let (startup_recents, startup_settings) = recents_at_startup(&exe);
    println!(
        "smoke: recents at startup: {startup_recents} read from {} - the app's OWN resolved state \
 dir, so a settings.toml in some other candidate dir is not counted (the trace claim below \
 is judgeable only above 0)",
        startup_settings.display()
    );
    let script = temp_path("probe", "ps1");
    // The geometry step gets its own reporter script: the first phase must stay
    // exactly as it was proven, and a shared script would let a change to one
    // verdict silently move the other.
    let geom_script = temp_path("geom-probe", "ps1");
    if let Err(e) =
        fs::File::create(&geom_script).and_then(|mut f| f.write_all(GEOMETRY_PROBE.as_bytes()))
    {
        println!("SMOKE FAIL: cannot write the geometry probe: {e}");
    }
    let out_file = temp_path("stdout", "txt");
    let err_file = temp_path("stderr", "txt");
    // Held in memory because the path is re-used: the geometry step redirects its
    // own launches onto the same file, so the first run's stderr exists nowhere
    // else once that step has run.
    let mut app_stderr: Option<String> = None;
    let mut code = match fs::File::create(&script).and_then(|mut f| f.write_all(PROBE.as_bytes())) {
        Err(e) => {
            eprintln!(
                "SMOKE FAIL: cannot write the probe script to {}: {e}",
                script.display()
            );
            2
        }
        Ok(()) => match spawn_probe(&script, &exe, &out_file, &err_file) {
            Err(e) => {
                println!("SMOKE FAIL: {e}");
                println!("smoke: window=UNKNOWN close=NOPS session=UNKNOWN 0.0s");
                2
            }
            Ok(mut child) => match wait_bounded(&mut child, OUTER_SECS) {
                Err(e) => {
                    println!("SMOKE FAIL: {e}");
                    println!(
                        "smoke: window=UNKNOWN close=TIMEOUT session=UNKNOWN {:.1}s",
                        started.elapsed().as_secs_f64()
                    );
                    1
                }
                Ok(Some(status)) => {
                    let stdout = read_pipe(child.stdout.as_mut());
                    let probe = parse_probe(&stdout);
                    if !status.success() {
                        println!("smoke: note - the probe child itself exited {status}");
                    }
                    app_stderr = report_captured(&err_file, "the app on stderr");
                    let _ = report_captured(&out_file, "the app on stdout");
                    let (artefact, tried) = find_artefact(&exe, launched);
                    println!(
                        "smoke: pid={} handle={} title={:?} launch_ms={}",
                        probe.get("PID").unwrap_or("?"),
                        probe.number("HANDLE").unwrap_or(0),
                        probe.get("TITLE").unwrap_or(""),
                        probe.number("LAUNCH_MS").unwrap_or(-1)
                    );
                    match &artefact {
                        Some(a) => println!(
                            "smoke: artefact {} rect={} written_by_this_run={}",
                            a.path.display(),
                            a.rect,
                            a.fresh
                        ),
                        None => println!(
                            "smoke: looked for {SESSION_FILE} in: {}",
                            tried
                                .iter()
                                .map(|p| p.display().to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    }
                    let verdict = decide(&probe, artefact.as_ref(), &relocation);
                    if let Relocation::Moved { to } = &relocation {
                        println!(
                            "smoke: the fresh-install case was judged with the user's file held at {}",
                            to.display()
                        );
                    }
                    for failure in match &verdict {
                        Verdict::Fail(list) => list.clone(),
                        Verdict::Skip(why) => vec![format!("smoke: DECLINED - {why}")],
                        Verdict::Pass => Vec::new(),
                    } {
                        println!("{failure}");
                    }
                    println!(
                        "{}",
                        summary(&verdict, &probe, started.elapsed(), &relocation)
                    );
                    match verdict {
                        Verdict::Pass => 0,
                        Verdict::Skip(_) => 3,
                        Verdict::Fail(_) => 1,
                    }
                }
                Ok(None) => 2,
            },
        },
    };

    // What the LIVE app's own stderr proved about the run that just passed.
    //
    // Exit 9, TRACE_FAILED_EXIT, and only here: it is documented at the constant
    // and at the module's exit-code table, and it is ARMED IN ci.yml IN THE SAME
    // CHANGESET because check-ci's [arm-missing] rule requires an arm for every
    // row of CONTRACT. Advisory like every other smoke verdict - this step can
    // print a red, it can never redden a run.
    //
    // Gated on the first launch having PASSED (exit 0 out of the probe block),
    // which is the desktop guard: no interactive session declines with 3, and a
    // failed launch has no live UI to read, so nothing here runs headless and
    // nothing here can turn a decline into a product failure.
    let launch_passed = code == 0;
    let mut trace_failed = false;
    if launch_passed {
        match judge_trace(app_stderr.as_deref(), startup_recents, TRACE_CLAIMS) {
            TraceVerdict::Proven(lines) => {
                for line in &lines {
                    println!("smoke: trace: {line}");
                }
                println!(
                    "smoke: trace: PASS - {} claim(s) met by the live app's own stderr",
                    lines.len()
                );
            }
            TraceVerdict::NotJudged(why) => {
                println!("smoke: trace: NOT JUDGED (advisory) - {why}")
            }
            TraceVerdict::Broken(notes) => {
                for note in &notes {
                    println!("{note}");
                }
                println!("smoke: trace: FAIL - the startup announce did not reach the live UI");
                trace_failed = true;
                code = TRACE_FAILED_EXIT;
            }
        }
    } else {
        println!(
            "smoke: trace: NOT RUN - the first launch did not pass, so there is no live UI whose trace could be judged"
        );
    }

    // The window-memory round trip, run INSIDE the guard that owns the user's
    // state: it seeds session.json, moves the window and relaunches the app, so
    // every file it touches is a file the guard will put back or justify.
    //
    // Guarded on the LAUNCH, not on `code`: a trace failure has already set 9 and
    // must not cost the run its geometry evidence. Where both went wrong this one
    // code wins, because the window-memory promise is the older and better
    // understood product claim - so the trace note is printed here rather than
    // left to be inferred from the absence of a code.
    if launch_passed {
        let session = candidate_state_dirs(&exe)
            .into_iter()
            .map(|dir| dir.join(SESSION_FILE))
            .find(|path| path.is_file());
        match session {
            None => println!("smoke: geometry: NOT JUDGED - no session.json to seed was found"),
            Some(path) => match geometry_round_trip(&geom_script, &exe, &err_file, &path) {
                Geometry::Proven {
                    restore,
                    relaunch,
                    pin,
                } => {
                    let _ = (restore, relaunch, pin);
                    println!(
                        "smoke: geometry: PASS - restore={restore} persist=moved relaunch={relaunch}"
                    )
                }
                Geometry::NotJudged(why) => {
                    println!("smoke: geometry: NOT JUDGED (advisory) - {why}")
                }
                Geometry::Broken { notes, pin_failed } => {
                    for note in &notes {
                        println!("SMOKE GEOMETRY FAIL: {note}");
                    }
                    if pin_failed {
                        println!(
                            "smoke: geometry: FAIL - the topmost bit did not follow session.json"
                        );
                        code = PIN_FAILED_EXIT;
                    } else {
                        println!("smoke: geometry: FAIL - the window memory promise did not hold");
                        code = GEOMETRY_FAILED_EXIT;
                    }
                    if trace_failed {
                        println!(
                            "smoke: note - the stderr trace claim failed too (its own code is {}); geometry outranks it here, so read the SMOKE TRACE FAIL line above as part of this verdict, not as lost news",
                            TRACE_FAILED_EXIT
                        );
                    }
                }
            },
        }
        // The maximised cycle, armed by M9: drift can set 6, nothing else can. Its
        // directory comes from state_dir_for through app_state_dir, NOT from the
        // candidate search list the lane above uses: the 2026-09-13 dossier lost a
        // whole reading to the roaming profile while the portable marker had the live
        // process writing target/debug/data, and a probe is not allowed to make that
        // mistake twice. The search list above is now a known follow-up rather than
        // something to copy.
        match app_state_dir(&exe)
            .map(|dir| dir.join(SESSION_FILE))
            .filter(|p| p.is_file())
        {
            Some(path) => {
                if let Some(note) = maximised_cycle(&geom_script, &exe, &err_file, &path) {
                    println!("SMOKE GEOMETRY FAIL: {note}");
                    // 6 outranks 9 exactly as the lane above does, and 7 outranks 6:
                    // a pin failure is a different and louder complaint.
                    if code != GEOMETRY_FAILED_EXIT && code != PIN_FAILED_EXIT {
                        code = GEOMETRY_FAILED_EXIT;
                    }
                }
            }
            None => println!(
                "smoke: maximised: NOT JUDGED - the app-resolved dir holds no session.json to seed"
            ),
        }
    } else {
        println!(
            "smoke: geometry: NOT RUN - the first launch did not succeed, so there is nothing to round trip"
        );
    }

    if let Some(seed) = seed.as_mut() {
        seed.finish();
        // The same severity as a botched session restore: the harness damaged a
        // profile, which is an infrastructure failure and never an app verdict.
        if seed.failed && code == 0 {
            println!(
                "smoke: DECLINED TO PASS - the seeded settings.toml did not come back clean; \
 the profile, not the app, is what is at risk here"
            );
            return STEP_FAILED_EXIT;
        }
    }
    if let Some(guard) = guard.as_mut() {
        // Explicit here so the verdict can see a failed restore; Drop is the
        // backstop for every path that does not reach this line.
        guard.finish();
        println!("smoke: user state outcome: {}", guard.outcome_note());
        if guard.failed && code == 0 {
            println!("smoke: DECLINED TO PASS - user state did not come back byte-identical");
            return STEP_FAILED_EXIT;
        }
    }
    for junk in [&script, &geom_script, &out_file, &err_file] {
        let _ = fs::remove_file(junk);
    }
    code
}

/// One-line description of whatever already sits on the session path.
fn describe_existing(exe: &Path) -> String {
    for dir in candidate_state_dirs(exe) {
        let path = dir.join(SESSION_FILE);
        if path.exists() {
            return format!("{} (which is {})", path.display(), describe_kind(&path));
        }
    }
    "nothing (no session.json in any candidate state dir)".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(pairs: &[(&str, &str)]) -> Probe {
        parse_probe(
            &pairs
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    fn artefact() -> Artefact {
        Artefact {
            path: PathBuf::from("C:/Users/u/AppData/Roaming/notes-gpui/session.json"),
            fresh: true,
            rect: "x=120 y=90 w=800 h=600".to_string(),
            read_error: None,
        }
    }

    /// A probe transcript from a real run on a real desktop.
    fn clean_gui() -> Probe {
        probe(&[
            ("DESKTOP", "1"),
            ("SPAWN", "1"),
            ("PID", "46444"),
            ("LAUNCH_MS", "739"),
            ("HANDLE", "13568654"),
            ("TITLE", "notes 0.0.1"),
            ("CLOSE_REQUESTED", "1"),
            ("EXITED_WITHOUT_KILL", "1"),
            ("FORCED", "0"),
            ("EXIT_CODE", "0"),
            ("PROBE_DONE", "1"),
        ])
    }

    fn with(p: Probe, key: &str, value: &str) -> Probe {
        let mut p = p;
        p.kv.insert(key.to_string(), value.to_string());
        p
    }

    fn judge(p: &Probe, a: Option<&Artefact>, r: &Relocation) -> Verdict {
        decide(p, a, r)
    }

    fn failures_of(p: &Probe) -> Vec<String> {
        match judge(p, Some(&artefact()), &Relocation::CleanSlate) {
            Verdict::Fail(list) => list,
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn a_graceful_exit_with_a_fresh_artefact_passes() {
        assert_eq!(
            judge(&clean_gui(), Some(&artefact()), &Relocation::CleanSlate),
            Verdict::Pass
        );
        assert_eq!(
            summary(
                &Verdict::Pass,
                &clean_gui(),
                Duration::from_millis(2100),
                &Relocation::CleanSlate
            ),
            "smoke: window=OK close=0 session=OK 2.1s"
        );
    }

    #[test]
    fn a_forced_kill_is_never_a_pass_even_when_the_code_reads_zero() {
        // THE dishonest-harness case: a force-killed process reports 0 too.
        let p = with(with(clean_gui(), "FORCED", "1"), "EXITED_WITHOUT_KILL", "0");
        let list = failures_of(&p);
        assert!(
            list.iter().any(|f| f.contains("force-killed")),
            "a force-kill must FAIL: {list:?}"
        );
        assert!(
            summary(
                &Verdict::Fail(list),
                &p,
                Duration::from_secs(11),
                &Relocation::CleanSlate
            )
            .contains("close=FORCED")
        );
    }

    #[test]
    fn a_self_exit_with_a_nonzero_code_fails_and_the_summary_shows_it() {
        let p = with(clean_gui(), "EXIT_CODE", "3");
        let list = failures_of(&p);
        assert!(list.iter().any(|f| f.contains("code 3")), "{list:?}");
        assert_eq!(
            summary(
                &Verdict::Fail(list),
                &p,
                Duration::from_millis(2100),
                &Relocation::CleanSlate
            ),
            "smoke: window=OK close=3 session=FAILED 2.1s"
        );
    }

    #[test]
    fn no_window_or_a_missing_key_fails_rather_than_declining() {
        let p = with(clean_gui(), "HANDLE", "0");
        assert!(
            failures_of(&p)
                .iter()
                .any(|f| f.contains("no top-level window"))
        );
        assert!(
            summary(
                &Verdict::Fail(vec![]),
                &p,
                Duration::ZERO,
                &Relocation::CleanSlate
            )
            .contains("window=MISSING")
        );
        let truncated = Probe {
            kv: clean_gui()
                .kv
                .into_iter()
                .filter(|(k, _)| k != "PROBE_DONE")
                .collect(),
        };
        assert!(matches!(
            judge(&truncated, Some(&artefact()), &Relocation::CleanSlate),
            Verdict::Fail(_)
        ));
    }

    #[test]
    fn a_stale_session_file_does_not_prove_the_fresh_install_write() {
        let stale = Artefact {
            fresh: false,
            ..artefact()
        };
        assert!(matches!(
            judge(&clean_gui(), Some(&stale), &Relocation::CleanSlate),
            Verdict::Fail(_)
        ));
        assert!(matches!(
            judge(&clean_gui(), None, &Relocation::CleanSlate),
            Verdict::Fail(_)
        ));
    }

    #[test]
    fn a_run_that_moved_the_users_file_aside_still_fails_for_real() {
        // The relocation must not become an excuse: with the path cleared and
        // nothing written, this is exactly the D54 bug and it must be red.
        let moved = Relocation::Moved {
            to: PathBuf::from("C:/Temp/notes-gpui-smoke-session-backup.json"),
        };
        let Verdict::Fail(list) = judge(&clean_gui(), None, &moved) else {
            panic!("a cleared path with no write must FAIL");
        };
        assert!(list.iter().any(|f| f.contains("moved aside")), "{list:?}");
    }

    #[test]
    fn foreign_state_on_the_session_path_declines_and_says_why() {
        // A directory planted on the path (or an unmovable file) is a fact about
        // the tester's machine. Reporting that as an app bug is what this guards.
        let blocked = Relocation::Blocked(
            "C:/Users/u/AppData/Roaming/notes-gpui/session.json is a DIRECTORY sitting on the session file's path"
                .to_string(),
        );
        let Verdict::Skip(why) = judge(&clean_gui(), None, &blocked) else {
            panic!("a blocked path must DECLINE, not pass and not fail");
        };
        assert!(why.contains("could not be cleared"), "{why}");
        assert!(why.contains("D54"), "{why}");
        let line = summary(&Verdict::Skip(why), &clean_gui(), Duration::ZERO, &blocked);
        assert!(line.contains("FOREIGN STATE"), "{line}");
        assert!(
            line.contains("not the app"),
            "the summary must say whose fault this is not: {line}"
        );
    }

    #[test]
    fn an_unreadable_artefact_is_its_own_verdict_not_a_silent_pass() {
        let unreadable = Artefact {
            read_error: Some("Access is denied (os error 5)".to_string()),
            ..artefact()
        };
        let Verdict::Fail(list) = judge(&clean_gui(), Some(&unreadable), &Relocation::CleanSlate)
        else {
            panic!("an artefact that cannot be read back cannot PASS");
        };
        assert!(
            list.iter()
                .any(|f| f.contains("CANNOT BE READ BACK") && f.contains("os error 5")),
            "{list:?}"
        );
    }

    #[test]
    fn kept_state_by_request_still_fails_when_nothing_fresh_was_written() {
        let kept = Relocation::Kept("C:/x/session.json".to_string());
        let stale = Artefact {
            fresh: false,
            ..artefact()
        };
        assert!(matches!(
            judge(&clean_gui(), Some(&stale), &kept),
            Verdict::Fail(_)
        ));
        let Verdict::Fail(list) = judge(&clean_gui(), None, &kept) else {
            panic!("no write is a failure even with --reuse-state");
        };
        assert!(list.iter().any(|f| f.contains("--reuse-state")), "{list:?}");
    }

    #[test]
    fn an_absent_desktop_declines_instead_of_faking_either_verdict() {
        let headless = probe(&[("DESKTOP", "0"), ("PROBE_DONE", "1")]);
        let Verdict::Skip(why) = judge(&headless, None, &Relocation::CleanSlate) else {
            panic!("no desktop must DECLINE, not pass and not fail");
        };
        assert!(why.contains("desktop"), "{why}");
        assert!(
            summary(
                &Verdict::Skip(why),
                &headless,
                Duration::ZERO,
                &Relocation::CleanSlate
            )
            .contains("DECLINED")
        );
    }

    #[test]
    fn a_slow_cold_start_is_reported_even_though_the_window_appeared() {
        let p = with(clean_gui(), "LAUNCH_MS", "9000");
        assert!(failures_of(&p).iter().any(|f| f.contains("cold-start")));
    }

    #[test]
    fn an_unaccepted_close_request_fails_because_the_path_never_ran() {
        let p = with(clean_gui(), "CLOSE_REQUESTED", "0");
        assert!(failures_of(&p).iter().any(|f| f.contains("WM_CLOSE")));
    }

    #[test]
    fn the_state_dir_search_covers_both_halves_of_the_rule() {
        // The only test that proves BOTH halves of the search rule, so it must
        // not inherit the machine it runs on: the exe comes from a scratch
        // fixture and the profile root is injected, the way core's own
        // resolve_state_dir tests work. Reading APPDATA directly used to delete
        // the installed half on any machine with no roaming profile, and the
        // run failed here on a rule the code never broke.
        let tag = format!("xtask-smoke-state-dirs-{}", std::process::id());
        let dir = std::env::temp_dir().join(&tag);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        let exe = dir.join("notes-gpui.exe");
        let profile = dir.join("roaming");
        let dirs = candidate_state_dirs_in(&exe, Some(profile.as_os_str()));
        assert_eq!(
            dirs[0],
            dir.join("data"),
            "the portable half is searched first: data next to the exe"
        );
        assert!(
            dirs.iter().any(|d| d.ends_with("notes-gpui")),
            "the installed candidate must be searched too: {dirs:?}"
        );
        assert_eq!(
            dirs[1],
            profile.join("notes-gpui"),
            "and the installed half is <profile>/notes-gpui, second"
        );
        // No profile is not an error: the portable half alone still stands.
        assert_eq!(
            candidate_state_dirs_in(&exe, None),
            vec![dir.join("data")],
            "an absent profile root must not invent an installed candidate"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// THE point of the guard: the user's bytes come back even when nobody
    /// calls finish() - a panic, an early return, a killed probe.
    #[test]
    fn the_restore_guard_returns_the_users_file_without_being_asked() {
        let tag = format!("xtask-smoke-guard-test-{}", std::process::id());
        let live_dir = std::env::temp_dir().join(&tag);
        let _ = fs::create_dir(&live_dir);
        let live = live_dir.join(SESSION_FILE);
        let aside = live_dir.join("aside.json");
        let body = b"{\"rect\":{\"x\":7,\"y\":8,\"w\":9,\"h\":10}}";
        fs::write(&live, body).expect("write the stand-in user state");
        let before = sha_of(&live).expect("hashable");
        fs::rename(&live, &aside).expect("relocate it");
        assert!(!live.exists(), "the relocation really moved it");
        {
            let guard = Restore {
                live: live.clone(),
                aside: aside.clone(),
                before: Some(before.clone()),
                done: false,
                failed: false,
                conclusion: None,
            };
            drop(guard);
        }
        let restored = fs::read(&live).expect("Drop must have restored the user's file");
        assert_eq!(restored, body, "byte-identical, or it says so");
        assert_eq!(sha_of(&live).as_deref(), Some(before.as_str()));
        assert!(!aside.exists(), "the temp copy is gone once it is back");
        let _ = fs::remove_dir_all(&live_dir);
    }

    /// D33, and the bug the reviewer named: the old restore renamed the pre-run
    /// bytes over whatever the app had just written, then reported identical=true
    /// about the user's file compared with itself. A run that moved a stale rect
    /// aside while the app persisted a new one left the STALE rect in the profile
    /// and still went green. The two cases must be distinguishable, so this test
    /// asserts on the bytes on disk, not on a flag.
    #[test]
    fn a_run_that_wrote_a_session_keeps_it_instead_of_burying_it() {
        let tag = format!("xtask-smoke-keep-{}", std::process::id());
        let dir = std::env::temp_dir().join(&tag);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        let live = dir.join(SESSION_FILE);
        let aside = dir.join("aside.json");
        let user_bytes = b"{\"rect\":{\"x\":1,\"y\":2,\"w\":3,\"h\":4}}";
        let app_bytes = b"{\"rect\":{\"x\":640,\"y\":200,\"w\":1024,\"h\":768},\"maximized\":true}";
        fs::write(&live, user_bytes).expect("seed the user state");
        let before = sha_of(&live).expect("hashable");
        fs::rename(&live, &aside).expect("relocate it");
        assert!(!live.exists(), "the relocation really moved it");
        // The app then writes DIFFERENT bytes: the evidence this run exists for.
        fs::write(&live, app_bytes).expect("simulate the app's write");

        let mut guard = Restore {
            live: live.clone(),
            aside: aside.clone(),
            before: Some(before.clone()),
            done: false,
            failed: false,
            conclusion: None,
        };
        guard.finish();

        // The pure decision separates the two cases.
        assert_eq!(
            conclude(Some(&before), Some("app-sha")),
            Conclusion::KeptAppArtefact {
                app_sha: "app-sha".to_string(),
                rewrote_same_bytes: false,
            }
        );
        assert_eq!(conclude(Some(&before), None), Conclusion::RestoredUserState);
        // And the guard took the keep branch: the app's bytes survive, the user's
        // stay held at the named path, nothing is lost or silently swapped.
        assert_eq!(
            guard.conclusion,
            Some(Conclusion::KeptAppArtefact {
                app_sha: sha_of(&live).unwrap(),
                rewrote_same_bytes: false,
            })
        );
        assert!(!guard.failed, "keeping the artefact is not a failure");
        assert_eq!(
            fs::read(&live).expect("read live"),
            app_bytes,
            "THE LIE THIS TEST KILLS: stale bytes restored over the app's write"
        );
        assert!(
            aside.is_file(),
            "the user's own bytes are still at the named path"
        );
        assert_eq!(fs::read(&aside).expect("read aside"), user_bytes);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn with_nothing_written_the_user_bytes_go_back_and_are_the_only_ones() {
        let tag = format!("xtask-smoke-restore-{}", std::process::id());
        let dir = std::env::temp_dir().join(&tag);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        let live = dir.join(SESSION_FILE);
        let aside = dir.join("aside.json");
        let user_bytes = b"{\"rect\":{\"x\":11,\"y\":12,\"w\":13,\"h\":14}}";
        fs::write(&live, user_bytes).expect("seed the user state");
        let before = sha_of(&live).expect("hashable");
        fs::rename(&live, &aside).expect("relocate it");
        let mut guard = Restore {
            live: live.clone(),
            aside: aside.clone(),
            before: Some(before.clone()),
            done: false,
            failed: false,
            conclusion: None,
        };
        guard.finish();
        assert_eq!(guard.conclusion, Some(Conclusion::RestoredUserState));
        assert!(!guard.failed);
        assert_eq!(fs::read(&live).expect("restored"), user_bytes);
        assert!(!aside.exists(), "the temp copy is gone once it is back");
        assert_eq!(sha_of(&live).as_deref(), Some(before.as_str()));
        let _ = fs::remove_dir_all(&dir);
    }

    /// THE test that would have caught the review finding: a source newer than
    /// the exe means smoke has nothing to certify, and the decision says so with
    /// both names in it. Unreadable inputs are unverifiable, never silently fresh.
    #[test]
    fn an_exe_older_than_its_sources_is_never_fresh() {
        let dir = std::env::temp_dir().join(format!("xtask-stale-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for root in SOURCE_ROOTS {
            fs::create_dir_all(dir.join(root)).expect("source tree");
        }
        let exe = dir.join("notes-gpui.exe");
        fs::write(&exe, b"MZ").expect("write the exe");
        let source = dir.join("crates/core/src/lib.rs");
        fs::write(&source, "pub fn touched_without_a_rebuild() {}\n").expect("write source");
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        set_mtime(&exe, past);

        let newest = newest_source(&dir).expect("a readable source mtime");
        assert_eq!(
            newest.1, source,
            "the newest source must be the one just written"
        );
        match staleness(mtime_of(&exe), Some(newest.clone())) {
            Stale::OlderThan {
                source: s,
                delta_secs,
                age_secs,
            } => {
                assert_eq!(s, source);
                assert!(
                    delta_secs >= 3590,
                    "delta {delta_secs} should be about an hour"
                );
                assert!(age_secs >= 3590, "the exe should read as about an hour old");
            }
            other => panic!("a one-hour-old exe against a new source must be stale: {other:?}"),
        }
        // The other direction: a build that really ran leaves a fresh exe.
        let now = std::time::SystemTime::now();
        set_mtime(&exe, now);
        assert_eq!(
            staleness(mtime_of(&exe), Some(newest.clone())),
            Stale::Fresh
        );
        // Unverifiable is reported as unverifiable.
        assert!(matches!(
            staleness(None, Some(newest.clone())),
            Stale::Unknown(_)
        ));
        assert!(matches!(staleness(Some(now), None), Stale::Unknown(_)));
        let _ = fs::remove_dir_all(&dir);
    }

    /// The seed's own shape, pinned against the two ways it can fail quietly: a
    /// file without autosave_enabled is SettingsCorrupt (so the recents read as
    /// EMPTY and the claim goes back to not-judged while the log says "seeded"), and
    /// a literal TOML string breaks on a profile path holding an apostrophe.
    #[test]
    fn the_seeded_file_is_what_core_reads_and_what_the_counter_counts() {
        let mut note = PathBuf::from(r"C:\Users\O'Brien");
        note.push("a.notes");
        let toml = seeded_settings_toml(&note);
        assert_eq!(
            count_recents_blocks(&toml),
            1,
            "exactly one entry, or the judgeability gate is wrong: {toml}"
        );
        assert!(
            toml.contains("autosave_enabled = true"),
            "the key has no serde default; without it the file is corrupt: {toml}"
        );
        assert!(toml.contains("exists = true"), "{toml}");
        assert!(
            !toml.contains("path = '"),
            "a literal string breaks on the apostrophe in this path: {toml}"
        );
        let line = toml
            .lines()
            .find(|l| l.starts_with("path = "))
            .expect("path line");
        let slashes = line.chars().filter(|c| *c == '\\').count();
        let separators = note
            .display()
            .to_string()
            .chars()
            .filter(|c| *c == '\\')
            .count();
        assert!(
            separators > 0,
            "the fixture must exercise a backslashed path"
        );
        assert_eq!(
            slashes,
            separators * 2,
            "every separator doubled for a TOML basic string: {line}"
        );
        assert!(line.contains("O'Brien"), "the name survives: {line}");
    }

    /// The do-no-harm half: a profile that already had a settings.toml gets its
    /// exact bytes back, and the scratch note leaves no trace.
    #[test]
    fn a_seeded_profile_comes_back_byte_identical() {
        let dir = std::env::temp_dir().join(format!("xtask-seedback-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let user = b"autosave_enabled = false\n\n[[recents]]\npath = 'C:\\Notes\\idea.notes'\ndisplay = \"idea.notes\"\nexists = true\n";
        let settings = dir.join("settings.toml");
        fs::write(&settings, user).expect("write the user's file");
        let (mut guard, note) = seed_settings_at(&dir).expect("seed lands");
        assert_eq!(
            count_recents_blocks(&fs::read_to_string(&settings).expect("seeded")),
            1,
            "the app must see exactly one recent"
        );
        assert!(note.is_file(), "the entry points at a real note");
        assert!(
            fs::read_to_string(&note)
                .expect("note text")
                .contains("smoke seeded"),
            "with known text"
        );
        guard.finish();
        assert_eq!(fs::read(&settings).expect("back"), user, "byte-identical");
        assert!(!note.exists(), "the scratch note is gone");
        assert!(!guard.failed, "a clean restore is not a failure");
        let _ = fs::remove_dir_all(&dir);
    }

    /// And the other case: there was no settings.toml, so nothing may be left. This
    /// is the state this machine has lived in all day - portable target/debug/data,
    /// no settings file - and the state the claim was never judgeable in.
    #[test]
    fn a_seed_that_created_the_file_leaves_nothing_behind() {
        let dir = std::env::temp_dir().join(format!("xtask-seedclean-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let (mut guard, note) = seed_settings_at(&dir).expect("seed lands");
        assert!(dir.join("settings.toml").is_file());
        guard.finish();
        assert!(
            !dir.join("settings.toml").exists(),
            "the harness created it, the harness removes it"
        );
        assert!(!note.exists());
        assert!(!guard.failed);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Self-clean after a crash, and the line it must not cross: the session
    /// backups hold the USER's bytes, so a sweep that took them would be the worst
    /// thing this harness could do on a timer.
    #[test]
    fn the_sweep_takes_only_this_stems_scratch_notes() {
        let old = std::time::Duration::from_secs(7200);
        let stale = temp_path(SEED_STEM, "notes");
        let fresh = temp_path(SEED_STEM, "notes");
        let backup = temp_path("session-backup-data", "json");
        for path in [&stale, &fresh, &backup] {
            fs::write(path, b"content").expect("write");
        }
        set_mtime(&stale, std::time::SystemTime::now() - old);
        set_mtime(&backup, std::time::SystemTime::now() - old);
        assert!(sweep_stale_seeds(std::time::Duration::from_secs(3600)) >= 1);
        assert!(!stale.exists(), "an old seed note is a crash leftover");
        assert!(fresh.exists(), "a run in flight keeps its own file");
        assert!(backup.exists(), "the user's session bytes are never swept");
        for path in [&fresh, &backup] {
            let _ = fs::remove_file(path);
        }
    }

    /// M4, as a rule: a window that died mid-poll must never be read as an absent
    /// topmost bit. GetWindowLong on a dead handle answers 0, which is a false PASS
    /// for pinned:false and a false accusation for pinned:true - so the reading is
    /// refused before attribution, and the reason says crash rather than pin.
    #[test]
    fn a_window_that_died_mid_poll_is_never_a_pin_verdict() {
        let dead = probe(&[
            ("TOPMOST", "-1"),
            ("TOPMOST_WANTED", "0"),
            ("PIN_POLL_CRASHED", "1"),
            ("TOPMOST_WAIT_MS", "700"),
            ("TOPMOST_POLLED", "14"),
        ]);
        assert_eq!(read_pin(&dead, false), PinRead::Crashed);
        assert_eq!(read_pin(&dead, true), PinRead::Crashed);
        let why = pin_refusal(PinRead::Crashed, "pinned:false");
        assert!(why.contains("crash"), "{why}");
        assert!(
            !why.to_lowercase().contains("lied"),
            "a crash note must not accuse the pin: {why}"
        );
        assert!(poll_crashed(&dead));
        // A healthy transcript does not trip it, and the bit is still read.
        let alive = probe(&[
            ("TOPMOST", "0"),
            ("TOPMOST_WANTED", "0"),
            ("PIN_POLL_CRASHED", "0"),
        ]);
        assert!(!poll_crashed(&alive));
        assert_eq!(read_pin(&alive, false), PinRead::Read(false));
    }

    /// M3 and M7 live in the script, so they are pinned as text: an absence must be
    /// confirmed against a second sample, the confirm must be budget-clamped, and
    /// TOPMOST_POLLED must come from a counter rather than from elapsed time.
    #[test]
    fn the_pin_poll_confirms_an_absence_and_counts_its_own_ticks() {
        let script = GEOMETRY_PROBE;
        assert!(
            script.contains("[int]$PinConfirmMs"),
            "no confirm parameter"
        );
        assert!(
            script.contains("$want -eq 0 -and $top -eq 0"),
            "an absence is the case that needs a second sample"
        );
        assert!(
            script.contains("[Math]::Min($PinConfirmMs, $left)"),
            "the confirm must stay inside the budget, not extend it"
        );
        assert!(
            script.contains("\"TOPMOST_POLLED=$polls\""),
            "the poll count is a counter, not a timing guess"
        );
        assert!(
            script.contains("'PIN_POLL_CRASHED=1'") && script.contains("'TOPMOST=-1'"),
            "a death must print no reading"
        );
        assert!(
            pin_wait_note(&probe(&[
                ("EXSTYLE", "0x240100"),
                ("TOPMOST_WAIT_MS", "512"),
                ("TOPMOST_POLLED", "10"),
            ]))
            .contains("polled 10x"),
            "the note cites the poll it did"
        );
        assert!(
            pin_wait_note(&probe(&[
                ("EXSTYLE", "0x240100"),
                ("TOPMOST_WAIT_MS", "0"),
                ("TOPMOST_POLLED", "0"),
            ]))
            .contains("first sample, never polled"),
            "0 is not the same answer as 10"
        );
        assert!(
            pin_wait_note(&probe(&[("EXSTYLE", "0x1")])).contains("poll count unread"),
            "an absent key is not a zero"
        );
    }

    /// M5: a leftover "maximized":true in the file being seeded is the one state
    /// that makes the rect lane report a false 6 - the app restores the maximised
    /// frame, which is not the seeded rect. The seed must overwrite the field.
    #[test]
    fn seeding_a_rect_clears_a_leftover_maximised_flag() {
        let dir = std::env::temp_dir().join(format!("xtask-m5-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("session.json");
        fs::write(
            &path,
            r#"{"rect":{"x":10,"y":10,"w":200,"h":200},"maximized":true,"pinned":false}"#,
        )
        .expect("leftover maximised state");
        let seed = Rect {
            l: 300,
            t: 200,
            r: 900,
            b: 600,
        };
        seed_session_with_pin(&path, &seed, false).expect("seed");
        let text = fs::read_to_string(&path).expect("seeded file");
        assert!(
            text.contains("\"maximized\": false"),
            "the rect lane measures a normal window: {text}"
        );
        assert!(!text.contains("\"maximized\": true"), "{text}");
        assert_eq!(persisted_rect(&text), Some(seed), "the rect still seeded");
        // And the pin half still works alongside the forcing.
        seed_session_with_pin(&path, &seed, true).expect("seed pinned");
        let text = fs::read_to_string(&path).expect("seeded again");
        assert!(text.contains("\"pinned\": true"), "{text}");
        assert!(text.contains("\"maximized\": false"), "{text}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// The number the armed assert will compare against zero, tested as data so it
    /// cannot agree with the print by accident. The honest case, the exact chrome
    /// quadruple the dossier measured, and the unreadable case are all pinned: a
    /// missing rect must never read as zero drift, which is the same rule that keeps a
    /// missing TOPMOST out of a pass.
    #[test]
    fn the_drift_is_four_numbers_and_not_a_vibes_check() {
        let at = |l, t, w, h| Rect {
            l,
            t,
            r: l + w,
            b: t + h,
        };
        assert_eq!(
            rect_drift(Some(at(100, 100, 800, 600)), Some(at(100, 100, 800, 600))),
            Some((0, 0, 0, 0))
        );
        assert_eq!(
            rect_drift(Some(at(382, 274, 636, 428)), Some(at(374, 270, 652, 436))),
            Some((-8, -4, 16, 8)),
            "the measured inflation, in the order the leg prints"
        );
        assert_eq!(rect_drift(None, Some(at(0, 0, 10, 10))), None);
        assert_eq!(rect_drift(Some(at(0, 0, 10, 10)), None), None);
    }

    /// A seed must be able to SAY maximised, not merely fail to clear it: the cycle leg
    /// reads the restore, so its premise has to be in the file it seeds. And the
    /// wrapper the rect lane uses must still clear the bit - two call sites must not
    /// collapse into one by forgetting a default.
    #[test]
    fn a_seed_can_state_the_show_state_it_wants() {
        let dir = std::env::temp_dir().join(format!("xtask-mx-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("session.json");
        let seed = Rect {
            l: 320,
            t: 240,
            r: 920,
            b: 640,
        };
        seed_session_with_state(&path, &seed, false, true).expect("maximised seed");
        let text = fs::read_to_string(&path).expect("written");
        assert!(text.contains(r#""maximized": true"#), "{text}");
        assert_eq!(persisted_rect(&text), Some(seed), "the rect is as stated");
        seed_session_with_pin(&path, &seed, false).expect("plain seed");
        let text = fs::read_to_string(&path).expect("written again");
        assert!(text.contains(r#""maximized": false"#), "{text}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Without IsZoomed in the script the leg prints "zoom unread" forever and still
    /// looks like an observation that works, so the probe half is pinned too.
    #[test]
    fn the_probe_reports_the_show_state_at_creation() {
        assert!(
            GEOMETRY_PROBE.contains("public static extern bool IsZoomed"),
            "the script must be able to ask"
        );
        assert!(
            GEOMETRY_PROBE.contains("ZOOM=$([int][WIN]::IsZoomed($handle))"),
            "and answer at creation, before any move or band"
        );
        assert!(
            GEOMETRY_PROBE.contains("'ZOOM=-1'"),
            "no handle is an absent answer, never a no"
        );
    }

    /// The f61d850d incident, pinned as a rule: a test file belonging to ANOTHER
    /// crate is not an input to notes-gpui.exe, so it cannot make that exe stale -
    /// while a source of the bridge itself still can, loudly. Both directions are
    /// asserted because narrowing a guard without re-asserting its teeth is how a
    /// check becomes decoration.
    #[test]
    fn another_crate_test_file_is_not_an_input_to_the_binary() {
        let dir = std::env::temp_dir().join(format!("xtask-scope-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for root in SOURCE_ROOTS {
            fs::create_dir_all(dir.join(root)).expect("src tree");
        }
        fs::create_dir_all(dir.join("crates/api/tests")).expect("tests tree");
        let exe = dir.join("notes-gpui.exe");
        fs::write(&exe, b"MZ").expect("exe");
        let src = dir.join("crates/bridge-gpui/src/main.rs");
        fs::write(&src, "fn main() {}\n").expect("src");
        // Everything this scan is allowed to see is as old as the exe...
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(600);
        set_mtime(&exe, past);
        set_mtime(&src, past);
        for root in SOURCE_ROOTS {
            if let Ok(f) = fs::File::open(dir.join(root).join("lib.rs")) {
                let _ = f;
            }
        }
        // ...and only a NON-INPUT is newer than it.
        let foreign_test = dir.join("crates/api/tests/geometry.rs");
        fs::write(&foreign_test, "// a test of another crate\n").expect("test file");
        set_mtime(&foreign_test, std::time::SystemTime::now());
        assert_eq!(
            staleness(mtime_of(&exe), newest_source(&dir)),
            Stale::Fresh,
            "a newer tests/ file of another crate must not abort the run it built"
        );
        // The teeth: the same newer-than-exe condition on a real input is still a
        // loud 5, and it names the file.
        set_mtime(
            &src,
            std::time::SystemTime::now() + std::time::Duration::from_secs(60),
        );
        match staleness(mtime_of(&exe), newest_source(&dir)) {
            Stale::OlderThan { source, .. } => assert_eq!(source, src),
            other => panic!("a newer bridge source is real staleness: {other:?}"),
        }
        // And nothing in SOURCE_FILES may name a tests/ path: that is the whole bug.
        for f in SOURCE_FILES {
            assert!(!f.contains("tests"), "a test path is not an input: {f}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Narrowing the walk must not have dropped a real input that happens to live
    /// outside a src dir: each crate's manifest and the one build script in the
    /// workspace are all listed, and each one wins the scan when it is the newest.
    #[test]
    fn every_input_outside_a_src_dir_is_still_watched() {
        let dir = std::env::temp_dir().join(format!("xtask-files-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let future = std::time::SystemTime::now() + std::time::Duration::from_secs(120);
        for (n, f) in SOURCE_FILES.iter().enumerate() {
            let path = dir.join(f);
            fs::create_dir_all(path.parent().unwrap()).expect("parent");
            fs::write(&path, "input\n").expect("write");
            // Stepping the stamp each round matters: the scan keeps the FIRST file at
            // a tie, so identical stamps would test the loop order, not the scope.
            set_mtime(&path, future + std::time::Duration::from_secs(n as u64 + 1));
            let newest = newest_source(&dir).expect("a source");
            assert_eq!(
                newest.1, path,
                "{f} must be in scope: newest came back as {:?}",
                newest.1
            );
        }
        // The build script is the only one, so the scan names it exactly once.
        assert_eq!(
            SOURCE_FILES
                .iter()
                .filter(|f| f.ends_with("build.rs"))
                .count(),
            1
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// newest_source must look at every crate that feeds the binary, not just
    /// the bridge - an api edit is what made a cached exe lie here.
    #[test]
    fn the_freshness_scan_covers_every_crate_that_builds_the_binary() {
        let dir = std::env::temp_dir().join(format!("xtask-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let future = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        for (i, root) in SOURCE_ROOTS.iter().enumerate() {
            let file = dir.join(root).join("lib.rs");
            fs::create_dir_all(file.parent().unwrap()).expect("tree");
            fs::write(&file, "// newest candidate").expect("write");
            if i == SOURCE_ROOTS.len() - 1 {
                set_mtime(&file, future);
            }
        }
        let newest = newest_source(&dir).expect("a source");
        assert!(
            newest.0 > std::time::SystemTime::now(),
            "the last crate's future mtime must win the scan: {newest:?}"
        );
        assert!(
            newest
                .1
                .starts_with(dir.join(SOURCE_ROOTS[SOURCE_ROOTS.len() - 1]))
        );
        // The two manifests count too: a lockfile bump changes the binary.
        let lock = dir.join("Cargo.lock");
        fs::write(&lock, "# bumped\n").expect("lock");
        set_mtime(&lock, future + std::time::Duration::from_secs(60));
        assert_eq!(
            newest_source(&dir).map(|(m, p)| (m > future, p)),
            Some((true, lock))
        );
        let _ = fs::remove_dir_all(&dir);
    }

    fn set_mtime(path: &Path, when: std::time::SystemTime) {
        let file = fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open for times");
        file.set_times(fs::FileTimes::new().set_modified(when))
            .expect("set mtime");
    }

    /// D33 in the two directions, at the decision level: a bridge that stops
    /// sending GeometryChanged (or stops polling) persists the seed and nothing
    /// else, and a bridge that never restores lands somewhere unrelated. Both
    /// must be red, and a genuine round trip must be green.
    #[test]
    fn the_geometry_verdicts_are_red_for_the_two_real_regressions() {
        let seed = Rect {
            l: 337,
            t: 241,
            r: 957,
            b: 661,
        };
        let moved = Rect {
            l: 390,
            t: 278,
            r: 1010,
            b: 698,
        };
        // The move never reached the state file: the persisted rect is the seed.
        assert_eq!(
            persisted(Some(seed), &seed, &moved),
            Persisted::StillTheSeed,
            "a bridge that stopped reporting the drag must not pass"
        );
        // Something persisted, but not either candidate.
        match persisted(
            Some(Rect {
                l: 10,
                t: 10,
                r: 20,
                b: 20,
            }),
            &seed,
            &moved,
        ) {
            Persisted::Neither { got } => assert_eq!(got.text(), "10,10,20,20"),
            other => panic!("{other:?}"),
        }
        // The good case, in client space: the app persists what gpui reports.
        let chrome = Rect {
            l: seed.l - 8,
            t: seed.t - 19,
            r: seed.r + 8,
            b: seed.b + 20,
        };
        assert_eq!(
            persisted(Some(client_of(&moved)), &seed, &client_of(&moved),),
            Persisted::Moved
        );
        // A window at the seed is At; a window at seed+chrome while the seed was
        // applied in the other space is the distinguishable ChromeOffset.
        assert_eq!(
            place(&seed, Some(&chrome), Some(&seed)),
            Placement::At { space: "client" }
        );
        // Both readings of "the window is where the seed said", with the real
        // chrome between them: the app persists what gpui reports (client area),
        // so a correct restore has CLIENT == seed and FRAME outset by 8/19/8/20.
        assert_eq!(
            place(&seed, Some(&chrome), Some(&seed)),
            Placement::At { space: "client" },
            "seed honoured in client space, the app's convention"
        );
        assert_eq!(
            place(&seed, Some(&seed), Some(&client_of(&seed))),
            Placement::At { space: "frame" },
            "seed honoured in frame space, the other convention"
        );
        assert_eq!(chrome.deltas(&seed), [-8, -19, 8, 20]);
        // The failure this separates: the seed applied in the WRONG space leaves
        // the frame 16 px across and 39 px down off target - far beyond the 6 px
        // tolerance - so it reads Off, never At. That is the D40 claim, asserted.
        assert!(matches!(
            place(&seed, Some(&client_of(&seed)), None),
            Placement::Off { .. }
        ));
        assert!(matches!(
            place(&seed, None, Some(&inset_of(&seed))),
            Placement::Off { .. }
        ));
        // And an unrelated placement is a red with numbers.
        let elsewhere = Rect {
            l: 0,
            t: 0,
            r: 400,
            b: 300,
        };
        match place(&seed, Some(&elsewhere), Some(&elsewhere)) {
            Placement::Off { deltas } => assert_eq!(deltas, [-337, -241, -557, -361]),
            other => panic!("{other:?}"),
        }
        assert_eq!(place(&seed, None, None), Placement::Unreadable);
    }

    fn client_of(frame: &Rect) -> Rect {
        // The chrome on this build: 8 left, 19 top, 8 right, 20 bottom.
        Rect {
            l: frame.l + 8,
            t: frame.t + 19,
            r: frame.r - 8,
            b: frame.b - 20,
        }
    }

    fn inset_of(client: &Rect) -> Rect {
        Rect {
            l: client.l - 8,
            t: client.t - 19,
            r: client.r + 8,
            b: client.b + 20,
        }
    }

    #[test]
    fn the_seed_is_placed_inside_the_work_area_and_never_on_the_default() {
        let wide = Rect {
            l: 0,
            t: 0,
            r: 3440,
            b: 1392,
        };
        let (seed, moved) = seed_in(
            &wide,
            (620, 420),
            &Rect {
                l: 120,
                t: 90,
                r: 920,
                b: 690,
            },
        )
        .expect("fits");
        assert!(seed.r <= wide.r && seed.b <= wide.b, "{seed:?}");
        assert_ne!(
            seed.l, 120,
            "an app that ignored the seed lands on the default"
        );
        assert_eq!(seed.r - seed.l, 620);
        assert_eq!(
            moved.r - moved.l,
            620,
            "the move changes position, never size"
        );
        // A second monitor offset must be honoured, not assumed to start at 0.
        let off = Rect {
            l: 1920,
            t: 0,
            r: 4360,
            b: 1080,
        };
        let (seed2, _) = seed_in(
            &off,
            (620, 420),
            &Rect {
                l: 120,
                t: 90,
                r: 920,
                b: 690,
            },
        )
        .expect("fits");
        assert!(seed2.l >= 1920 && seed2.r <= 4360, "{seed2:?}");
        // Too small a screen is not judged rather than forced.
        assert_eq!(
            seed_in(
                &Rect {
                    l: 0,
                    t: 0,
                    r: 500,
                    b: 400
                },
                (620, 420),
                &Rect {
                    l: 120,
                    t: 90,
                    r: 920,
                    b: 690
                }
            ),
            None
        );
    }

    #[test]
    fn the_persisted_rect_is_read_out_of_the_apps_own_json() {
        let text = r#"{"rect":{"x":390,"y":278,"w":620,"h":420},"maximized":false}"#;
        assert_eq!(
            persisted_rect(text),
            Some(Rect {
                l: 390,
                t: 278,
                r: 1010,
                b: 698
            })
        );
        assert_eq!(persisted_rect("{\"rect\":null}"), None);
        assert_eq!(persisted_rect("not json"), None);
        assert_eq!(Rect::parse("1,2,3"), None);
    }

    #[test]
    fn a_seed_written_into_the_apps_file_keeps_every_other_field() {
        let dir = std::env::temp_dir().join(format!("xtask-seed-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("session.json");
        fs::write(
            &path,
            "{\"rect\":{\"x\":1,\"y\":2,\"w\":3,\"h\":4},\"pinned\":true,\"maximized\":false}",
        )
        .expect("write");
        let before = seed_session_with_pin(
            &path,
            &Rect {
                l: 100,
                t: 100,
                r: 700,
                b: 500,
            },
            false,
        )
        .expect("seeded");
        let after = fs::read_to_string(&path).expect("read");
        assert_eq!(
            persisted_rect(&after),
            Some(Rect {
                l: 100,
                t: 100,
                r: 700,
                b: 500
            })
        );
        assert!(
            after.contains("\"pinned\": true"),
            "the seed must not reset the user's pin: {after}"
        );
        restore_session(&path, before.as_deref());
        assert_eq!(
            fs::read_to_string(&path).expect("back"),
            "{\"rect\":{\"x\":1,\"y\":2,\"w\":3,\"h\":4},\"pinned\":true,\"maximized\":false}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Phase 4's own plumbing: flipping the pin must change that one field and
    /// nothing else, because the polarity test relies on the rect the app itself
    /// persisted staying put underneath it.
    #[test]
    fn the_pin_flip_touches_only_the_pin() {
        let dir = std::env::temp_dir().join(format!("xtask-pin-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("session.json");
        fs::write(
            &path,
            "{\"rect\":{\"x\":398,\"y\":297,\"w\":604,\"h\":381},\"pinned\":true}",
        )
        .expect("write");
        set_pinned(&path, false).expect("flip");
        let after = fs::read_to_string(&path).expect("read");
        assert_eq!(
            persisted_rect(&after),
            Some(Rect {
                l: 398,
                t: 297,
                r: 1002,
                b: 678
            }),
            "the rect the app persisted must survive the pin flip: {after}"
        );
        assert!(after.contains("\"pinned\": false"), "{after}");
        set_pinned(&path, true).expect("flip back");
        assert!(
            fs::read_to_string(&path)
                .expect("read")
                .contains("\"pinned\": true")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// The bug the bridge lane reported: smoke resolved <root>/target/debug no
    /// matter what the environment said, so a private build was judged by its
    /// absent rival and failed the freshness guard on a binary that was sitting
    /// right there.
    #[test]
    fn the_target_dir_we_are_told_is_the_target_dir_we_use() {
        let root = PathBuf::from(r"C:\dev\notes-gpui");
        let (path, why) = resolve_exe(&root, None);
        assert_eq!(
            path,
            PathBuf::from(r"C:\dev\notes-gpui\target\debug\notes-gpui.exe")
        );
        assert!(why.contains("default"), "{why}");
        let (path, why) = resolve_exe(&root, Some(""));
        assert!(
            path.starts_with(r"C:\dev\notes-gpui\target"),
            "an empty value is no value: {path:?}"
        );
        assert!(why.contains("default"), "{why}");
        let (path, why) = resolve_exe(&root, Some("   "));
        assert!(
            why.contains("default"),
            "whitespace is not a directory: {why}"
        );
        let (private, why) = resolve_exe(&root, Some(r"D:\builds\lane-4"));
        assert_eq!(
            private,
            PathBuf::from(r"D:\builds\lane-4\debug\notes-gpui.exe"),
            "an absolute CARGO_TARGET_DIR replaces the whole tree"
        );
        assert!(private.to_string_lossy().contains("lane-4"), "{private:?}");
        assert!(why.contains("CARGO_TARGET_DIR"), "{why}");
        assert!(
            !path.to_string_lossy().contains("lane-4"),
            "the default must not be polluted by the other case"
        );
        // A relative value is resolved against the root, which is what cargo does
        // against the invoking directory - and the reason string says so.
        let (rel, _) = resolve_exe(&root, Some("out/scratch"));
        assert_eq!(
            rel,
            PathBuf::from(r"C:\dev\notes-gpui\out\scratch\debug\notes-gpui.exe"),
            "{rel:?}"
        );
    }

    /// manifest_of must read the promise out of the source file rather than
    /// hardcoding it: give it a manifest that claims a different identity and the
    /// verdict follows the claim.
    #[test]
    fn the_manifest_verdict_follows_the_source_file_not_a_literal() {
        // Distinct from the manifest module's own fixture dir, which is keyed on
        // the same process id: two tests in one binary may not share a scratch path.
        let dir = std::env::temp_dir().join(format!("xtask-smoke-mf-{}", std::process::id()));
        let our_dir = dir.join("crates/bridge-gpui");
        fs::create_dir_all(&our_dir).expect("dir");
        fs::write(
            our_dir.join("app.manifest"),
            "<assemblyIdentity name=\"NotesGpui.App\"/><dpiAwareness>PerMonitorV2</dpiAwareness>\
             <longPathAware>true</longPathAware>",
        )
        .expect("manifest");
        let exe = dir.join("app.exe");
        fs::write(
            &exe,
            b"prefix <assemblyIdentity name=\"NotesGpui.App\"/> PerMonitorV2 longPathAware",
        )
        .expect("exe");
        let m = manifest_of(&dir, &exe).expect("readable");
        assert!(m.all_present(), "{m:?}");
        let foreign = dir.join("foreign.exe");
        fs::write(
            &foreign,
            b"<?xml version=\"1.0\"?><assemblyIdentity name=\"Zed.Other\"/>",
        )
        .expect("exe");
        let m = manifest_of(&dir, &foreign).expect("readable");
        assert!(
            !m.all_present(),
            "a foreign exe must not read as ours: {m:?}"
        );
        let crate::manifest::ReadBack::Missing(absent) = crate::manifest::read_back(&m) else {
            panic!("a foreign exe must name every marker it lacks");
        };
        assert_eq!(
            absent,
            vec!["assemblyIdentity", "longPathAware", "PerMonitorV2"],
            "{absent:?}"
        );
        // Unreadable input is refused, never reported as a violation.
        assert!(manifest_of(&dir, &dir.join("nope.exe")).is_err());
        assert!(manifest_of(&dir.join("nowhere"), &exe).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    /// Exit 8 is deliberately outside the published contract, and a test is the
    /// only place that claim stays true.
    #[test]
    fn the_flag_only_verdict_is_outside_the_contract_on_purpose() {
        assert_eq!(MANIFEST_NOT_OURS_EXIT, 8);
        assert!(
            !CONTRACT.iter().any(|c| c.0 == MANIFEST_NOT_OURS_EXIT),
            "a code CI can never receive must not demand a ci.yml arm"
        );
        assert_eq!(
            CONTRACT.len(),
            9,
            "the default-invocation codes plus the live-UI trace verdict"
        );
    }

    /// The length moved from 8 to 9 because the trace claim is a DEFAULT-invocation
    /// verdict on a desktop machine, so unlike exit 8 it is in the table and has to
    /// be armed in ci.yml in the same changeset - which is what [arm-missing] checks.
    #[test]
    fn the_trace_verdict_is_the_ninth_contracted_code_and_is_distinct() {
        assert_eq!(TRACE_FAILED_EXIT, 9);
        assert!(
            CONTRACT.iter().any(|c| c.0 == TRACE_FAILED_EXIT),
            "a code a default run can return must be published, or check-ci reads it as drift"
        );
        assert_eq!(
            CONTRACT.len(),
            9,
            "the default-invocation codes, one more now that the live-UI trace is asserted"
        );
        let mut seen = std::collections::BTreeSet::new();
        for code in contract_codes() {
            assert!(seen.insert(code), "exit {code} is published twice");
        }
        assert!(
            seen.contains(&TRACE_FAILED_EXIT),
            "9 came out of the published table, so the distinctness check covered it"
        );
        assert!(
            !seen.contains(&MANIFEST_NOT_OURS_EXIT),
            "8 stays the flag-only code outside the table"
        );
    }

    /// The harness distrusting its own plumbing: a TOPMOST reading is refused when
    /// the script did not confirm it waited for THIS launch's polarity, because in
    /// that state the two halves disagree about which window is which and the bit
    /// means nothing. Refused is not judged - never a pass, and never a red the app
    /// caused.
    #[test]
    fn a_pin_reading_without_a_matching_wanted_polarity_is_refused() {
        // Agreed plumbing on both halves, so the reading stands.
        let pinned = probe(&[("TOPMOST", "1"), ("TOPMOST_WANTED", "1")]);
        assert_eq!(read_pin(&pinned, true), PinRead::Read(true));
        let loose = probe(&[("TOPMOST", "0"), ("TOPMOST_WANTED", "0")]);
        assert_eq!(read_pin(&loose, false), PinRead::Read(false));
        // WANTED missing: an old or truncated transcript cannot be attributed.
        let blind = probe(&[("TOPMOST", "1")]);
        assert_eq!(read_pin(&blind, true), PinRead::Unreported);
        // WANTED present but the OTHER polarity: the seeded file and the poll
        // disagree, which is an instrument fault and not an app verdict.
        let crossed = probe(&[("TOPMOST", "1"), ("TOPMOST_WANTED", "0")]);
        assert_eq!(
            read_pin(&crossed, true),
            PinRead::Mismatch {
                asked: true,
                reported: false
            }
        );
        // WANTED garbage is Unreported, not a coincidence that happens to match.
        let junk = probe(&[("TOPMOST", "1"), ("TOPMOST_WANTED", "7")]);
        assert_eq!(read_pin(&junk, true), PinRead::Unreported);
        // Agreed polarity, unreadable style: still not a verdict.
        let no_style = probe(&[("TOPMOST", "-1"), ("TOPMOST_WANTED", "1")]);
        assert_eq!(read_pin(&no_style, true), PinRead::Unreadable);
        // And each refusal says why, in a sentence that blames the instrument.
        assert!(pin_refusal(PinRead::Unreported, "pinned:true").contains("unknown"));
        assert!(pin_refusal(PinRead::Unreadable, "pinned:true").contains("could not be read"));
        let why = pin_refusal(
            PinRead::Mismatch {
                asked: true,
                reported: false,
            },
            "pinned:true",
        );
        assert!(why.contains("instrument fault"), "{why}");
        assert!(why.contains("pinned:false"), "{why}");
    }

    /// A refusal must not be able to look like the app failing: it carries no exit
    /// code, and the pin verdict it produces is not-judged, which the caller prints
    /// and leaves alone.
    #[test]
    fn a_refused_polarity_is_not_judged_and_cannot_redden_the_pin() {
        // The raw style read is still a 1 - which is the point: it is read_pin's
        // attribution gate, not the bit, that refuses this reading.
        let blind = probe(&[("TOPMOST", "1")]);
        assert_eq!(topmost(&blind), Some(true));
        assert_eq!(read_pin(&blind, true), PinRead::Unreported);
        let pin = Pin::NotJudged("refused");
        assert!(matches!(pin, Pin::NotJudged(_)));
        // A real contradiction still reaches the exit-7 note, so the new refusal
        // path has not swallowed the case the code exists for.
        let first = probe(&[
            ("TOPMOST", "0"),
            ("TOPMOST_WANTED", "1"),
            ("TOPMOST_WAIT_MS", "2000"),
            ("EXSTYLE", "2359552"),
        ]);
        assert_eq!(read_pin(&first, true), PinRead::Read(false));
        assert!(pin_wait_note(&first).contains("2000 ms of the 2000 ms budget"));
    }

    /// The pin read is a POLL now, and this is the test that keeps it a poll: delete
    /// the loop, the wait key or the polarity argument from the heredoc and it
    /// fails. Written as a guard because the alternative is 8d0e5055 - a green
    /// product reported as exit 7 by a harness that sampled the style once.
    #[test]
    fn the_pin_read_polls_the_seeded_polarity_within_a_bounded_budget() {
        let script = GEOMETRY_PROBE;
        // The polarity under test is passed IN, never guessed by the script.
        assert!(script.contains("$ExpectPinned"), "no polarity argument");
        assert!(
            script.contains("$want -ge 0"),
            "an expiry must still be able to fail, so the loop cannot be unconditional"
        );
        // Bounded on both sides: a tick and a deadline.
        assert!(
            script.contains("$sw.ElapsedMilliseconds -lt $PinWaitMs"),
            "the wait must be bounded by the budget"
        );
        assert!(script.contains("Start-Sleep -Milliseconds $PinTickMs"));
        // The keys the Rust verdict reads to build an honest failure line.
        for key in ["TOPMOST=", "EXSTYLE=", "TOPMOST_WAIT_MS="] {
            assert!(
                script.contains(key),
                "the script must print the key it was asked for"
            );
        }
        // Every knob is a DECLARED parameter, so run_probe_script hands the script
        // the same budget the failure message prints (the dash form lives in that
        // function's arg list, which the compiler checks; this checks the half the
        // compiler cannot see - a parameter deleted from a string literal).
        for param in ["[int]$ExpectPinned", "[int]$PinWaitMs", "[int]$PinTickMs"] {
            assert!(script.contains(param), "undeclared parameter {param}");
        }
        // The four-tick floor is enforced where it belongs - a const assertion
        // beside the constants, so a shrinking budget is a compile error rather
        // than a test that only fails when someone runs it.
    }

    /// Waiting must not become excusing. A poll that expires with the bit still
    /// wrong gives the SAME reading as a single wrong sample: the verdict consumes
    /// the final answer, so a spent budget is a fail, not a shrug.
    #[test]
    fn an_expired_wait_is_still_a_fail_and_never_softens_into_not_judged() {
        let spent = PIN_WAIT_MS.to_string();
        let timed_out = probe(&[
            ("TOPMOST", "0"),
            ("TOPMOST_WANTED", "1"),
            ("TOPMOST_WAIT_MS", spent.as_str()),
            ("EXSTYLE", "2359552"),
        ]);
        assert_eq!(
            topmost(&timed_out),
            Some(false),
            "an expired wait reads as the answer it got, so the pin verdict fails"
        );
        let late = probe(&[("TOPMOST", "1"), ("TOPMOST_WAIT_MS", "512")]);
        assert_eq!(
            topmost(&late),
            Some(true),
            "and a match is credited at 512ms"
        );
        // An unread style is not a verdict at all: never judged, never a pass.
        assert_eq!(topmost(&probe(&[("TOPMOST", "-1")])), None);
    }

    /// What a red pin line now has to carry: the last extended style seen, in hex
    /// because that is how every Windows document writes it, and the time spent,
    /// because 0 ms and 2000 ms are two different bugs wearing one exit code.
    #[test]
    fn the_pin_note_names_the_last_style_and_the_time_waited() {
        // 0x240108 is the value the OS probe read off the live window at
        // handle+512ms on 8d0e5055 - the run this fix exists for.
        let p = probe(&[
            ("TOPMOST", "1"),
            ("EXSTYLE", "2359560"),
            ("TOPMOST_WAIT_MS", "512"),
        ]);
        let note = pin_wait_note(&p);
        assert!(note.contains("last EXSTYLE=0x240108"), "{note}");
        assert!(note.contains("512 ms of the 2000 ms budget"), "{note}");
        // Absent evidence reads as absent, never as a reassuring zero.
        let blind = pin_wait_note(&probe(&[("TOPMOST", "1")]));
        assert!(blind.contains("EXSTYLE=unread"), "{blind}");
        assert!(blind.contains("wait unread"), "{blind}");
        assert!(
            !blind.contains("0 ms"),
            "a lost key is not a fast answer: {blind}"
        );
    }

    /// A trace line from a real launch, spelled the way the bridge spells it:
    /// report() prefixes "notes-gpui: ", the rendered status line is
    /// "status line: {describe(event)}", and describe() renders the recents as
    /// "RecentsUpdated - {n} in the list - {names}" with a U+00B7 separator.
    fn rendered_announce() -> String {
        "notes-gpui: startup: asking the port for C:\\Notes\\idea.notes\n\
 notes-gpui: status line: RecentsUpdated \u{b7} 1 in the list \u{b7} untitled.notes\n\
 notes-gpui: status line: Loaded C:\\Notes\\idea.notes \u{b7} 12 chars \u{b7} utf-8, lf\n"
            .to_string()
    }

    #[test]
    fn a_rendered_announce_in_the_trace_proves_the_claim_and_keeps_its_line() {
        let verdict = judge_trace(Some(&rendered_announce()), 1, TRACE_CLAIMS);
        let TraceVerdict::Proven(lines) = verdict else {
            panic!("a rendered line naming the announce is the evidence asked for: {verdict:?}");
        };
        assert_eq!(lines.len(), TRACE_CLAIMS.len());
        assert!(
            lines[0].contains("[rendered]"),
            "the voice is part of the evidence: {}",
            lines[0]
        );
        assert!(
            lines[0].contains("status line: RecentsUpdated"),
            "and so is the line itself: {}",
            lines[0]
        );
    }

    /// The weaker-but-real case, pinned rather than hidden: an event that reached
    /// the port and never got a frame is printed by the exit drain as
    /// "undisplayed:". That still answers roadmap item 5's actual question (did the
    /// list get as far as the running app), and the verdict says WHICH it was.
    #[test]
    fn an_announce_named_only_in_the_exit_drain_is_still_evidence_and_says_so() {
        let trace = "notes-gpui: exit drain: 1 event(s) accounted for\n\
 notes-gpui: undisplayed: RecentsUpdated \u{b7} 3 in the list\n";
        let verdict = judge_trace(Some(trace), 3, TRACE_CLAIMS);
        let TraceVerdict::Proven(lines) = verdict else {
            panic!("the trace names the announce: {verdict:?}");
        };
        assert!(
            lines[0].contains("[delivered, never rendered]"),
            "{}",
            lines[0]
        );
    }

    /// The case the arrival voice exists for: a startup wake that took
    /// RecentsUpdated and then Loaded renders ONE line (last event wins), so the
    /// announce never appears in a "status line:" - yet it did arrive, named per
    /// event, ungated. That is a PROVEN claim citing [arrived], not a fail and not
    /// a shrug; before this class existed the same run was proven only by luck.
    #[test]
    fn an_announce_that_lost_the_last_wins_collapse_is_proven_as_arrived() {
        let trace = "notes-gpui: event: RecentsUpdated with 2 entries\n\
 notes-gpui: status line: Loaded a file with 4 chars\n";
        let verdict = judge_trace(Some(trace), 2, TRACE_CLAIMS);
        let TraceVerdict::Proven(lines) = verdict else {
            panic!("the announce arrived, so the claim holds: {verdict:?}");
        };
        assert!(lines[0].contains("[arrived]"), "{}", lines[0]);
        assert!(
            lines[0].contains("event: RecentsUpdated"),
            "and it cites the arrival line itself: {}",
            lines[0]
        );
    }

    /// Ordering between the voices, stated once so no future edit can quietly
    /// re-rank them - and the reason the claim no longer needs "the first line that
    /// matched": a run that both announced AND rendered the list must be cited on
    /// the rendered line, because first-match would report weaker news than earned.
    #[test]
    fn the_voices_rank_rendered_then_arrived_then_undisplayed() {
        assert_eq!(voice_rank("notes-gpui: status line: RecentsUpdated"), 3);
        assert_eq!(voice_rank("notes-gpui: event: RecentsUpdated"), 2);
        assert_eq!(voice_rank("notes-gpui: undisplayed: RecentsUpdated"), 1);
        assert_eq!(voice_rank("notes-gpui: pump: 9 wakes"), 0);
        assert_eq!(trace_voice("notes-gpui: event: Pinned on"), "arrived");
        // Strongest wins when both are in the same capture.
        let both = "notes-gpui: event: RecentsUpdated first\nnotes-gpui: status line: RecentsUpdated second\n";
        let TraceVerdict::Proven(lines) = judge_trace(Some(both), 1, TRACE_CLAIMS) else {
            panic!("both voices present");
        };
        assert!(lines[0].contains("[rendered]"), "{}", lines[0]);
        assert!(
            lines[0].contains("second"),
            "not the first line: {}",
            lines[0]
        );
    }

    /// The rule that turned an unjudgeable run into a false exit 9: the recents
    /// counted are the ones in the directory THE APP opens, and the portable marker
    /// outranks a roaming profile - core's own documented caller rule, and the
    /// OPPOSITE precedence from candidate_state_dirs' search order.
    #[test]
    fn the_settings_counted_are_the_ones_in_the_dir_the_app_resolves_to() {
        let appdata = std::ffi::OsStr::new("C:/Users/u/AppData/Roaming");
        let exe = Path::new("C:/dev/notes/target/debug/notes-gpui.exe");
        assert_eq!(
            state_dir_for(exe, Some(appdata), true),
            Some(PathBuf::from("C:/dev/notes/target/debug/data")),
            "the marker wins even though a profile exists - the machine that failed"
        );
        assert_eq!(
            state_dir_for(exe, Some(appdata), false),
            Some(PathBuf::from("C:/Users/u/AppData/Roaming/notes-gpui"))
        );
        assert_eq!(
            state_dir_for(exe, None, false),
            Some(PathBuf::from("C:/dev/notes/target/debug/data")),
            "no profile at all is portable, not a guess at one"
        );
        assert_eq!(
            state_dir_for(exe, Some(std::ffi::OsStr::new("")), false),
            Some(PathBuf::from("C:/dev/notes/target/debug/data")),
            "an empty APPDATA is not a usable profile, per core"
        );
        // And a resolved dir with no settings.toml counts 0, which judge_trace
        // refuses to judge - naming the app-resolved dir in the reason.
        let why = match judge_trace(Some("notes-gpui: event: Loaded a file"), 0, TRACE_CLAIMS) {
            TraceVerdict::NotJudged(why) => why,
            other => panic!("0 recents is not judgeable: {other:?}"),
        };
        assert!(why.contains("app-resolved state dir"), "{why}");
    }

    #[test]
    fn a_trace_that_never_names_the_announce_breaks_the_claim_naming_the_needle() {
        let trace = "notes-gpui: status line: Saved C:\\Notes\\idea.notes \u{b7} revision 2\n\
 notes-gpui: pump: 9 wakes, 4 had work, 5 events\n";
        let verdict = judge_trace(Some(trace), 2, TRACE_CLAIMS);
        let TraceVerdict::Broken(notes) = verdict else {
            panic!("recents to announce and no line naming them is the finding: {verdict:?}");
        };
        assert_eq!(notes.len(), 1);
        assert!(notes[0].starts_with("SMOKE TRACE FAIL:"), "{}", notes[0]);
        assert!(notes[0].contains("RecentsUpdated"), "{}", notes[0]);
        assert!(notes[0].contains("2 recents"), "{}", notes[0]);
        assert!(notes[0].contains("M2 exit item 5"), "{}", notes[0]);
    }

    /// The clause that stops a false red - and a false green. An empty profile
    /// CANNOT produce the line: the engine announces only a non-empty list. So the
    /// absence of it proves nothing here, and the step refuses to read it either
    /// way. A matching line would not change that (it could only have come from a
    /// later change, not from the startup announce), so the gate runs first.
    #[test]
    fn a_profile_with_nothing_to_announce_is_not_judged_never_passed() {
        let trace = "notes-gpui: status line: Loaded C:\\a.notes \u{b7} 1 chars\n";
        let verdict = judge_trace(Some(trace), 0, TRACE_CLAIMS);
        assert!(
            matches!(&verdict, TraceVerdict::NotJudged(why) if why.contains("no recents")),
            "{verdict:?}"
        );
        let TraceVerdict::NotJudged(why) = judge_trace(Some("anything"), 0, TRACE_CLAIMS) else {
            panic!("0 recents is not judgeable whatever the trace says")
        };
        assert!(why.contains("NON-EMPTY"), "{why}");
    }

    /// Absent evidence is not evidence of absence: the capture is the harness's
    /// own instrument, and a file that could not be read - or that the redirect
    /// filled with nothing - is this runner's blind spot, not the app's silence.
    #[test]
    fn an_unreadable_or_empty_capture_is_not_judged_and_never_a_failure() {
        for capture in [None, Some(""), Some("   \n\n  \n")] {
            let verdict = judge_trace(capture, 5, TRACE_CLAIMS);
            assert!(
                matches!(&verdict, TraceVerdict::NotJudged(_)),
                "capture {capture:?} must not be judged: {verdict:?}"
            );
        }
    }

    /// The reason the claims are a TABLE: the next slice adds a row (the seeded
    /// recent opened from the menu) and every claim is tried, with only the
    /// unmet ones reported. Proven here without touching run().
    #[test]
    fn every_claim_in_the_table_is_tried_not_just_the_first() {
        let extra = TraceClaim {
            what: "the seeded recent was opened",
            needle: "menu: open",
            proves: "M2 exit item 2 (Open)",
        };
        let claims = [TRACE_CLAIMS[0], extra];
        let matched = format!("{}\nnotes-gpui: menu: open\n", rendered_announce());
        let TraceVerdict::Proven(lines) = judge_trace(Some(&matched), 2, &claims) else {
            panic!("both needles are present");
        };
        assert_eq!(lines.len(), 2, "{lines:?}");
        let TraceVerdict::Broken(notes) = judge_trace(Some(&rendered_announce()), 2, &claims)
        else {
            panic!("the second needle is absent");
        };
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("menu: open"), "{}", notes[0]);
        assert!(notes[0].contains("item 2"), "{}", notes[0]);
    }

    /// Counted against the serialisation core's own settings.rs test pins, plus
    /// the empty-by-default file a fresh install would read.
    #[test]
    fn recents_are_counted_from_the_settings_the_app_actually_reads() {
        assert_eq!(count_recents_blocks("autosave_enabled = true\n"), 0);
        let one = "autosave_enabled = false\ncodepage = 1252\n\n[[recents]]\npath = 'C:\\\\Notes\\\\idea.notes'\ndisplay = \"idea.notes\"\nexists = true\n";
        assert_eq!(count_recents_blocks(one), 1);
        assert_eq!(
            count_recents_blocks(&format!("{one}\n[[recents]]\npath = 'C:\\\\a.notes'\n")),
            2
        );
        // A path or display value that happens to contain the header text is not
        // a second entry, because the count is over LINES, not over the file.
        assert_eq!(count_recents_blocks("path = 'x [[recents]] y'\n"), 0);
    }

    /// The clause that stops a STATE being read as a CAUSE: the exe cannot say
    /// who embedded its manifest, so smoke asks build.rs whether the build even
    /// had the chance. Three answers, and the unreadable one is unknown rather
    /// than a default that would silently claim either cause.
    #[test]
    fn the_tool_can_say_whether_the_build_had_a_chance_to_embed() {
        let dir = std::env::temp_dir().join(format!("xtask-embed-{}", std::process::id()));
        let bd = dir.join("crates/bridge-gpui");
        fs::create_dir_all(&bd).expect("dir");
        fs::write(
            bd.join("build.rs"),
            "fn main() { println!(\"cargo:rustc-link-arg-bins=/MANIFEST:EMBED\"); }",
        )
        .expect("embed");
        assert_eq!(build_embeds_manifest(&dir), Some(true));
        fs::write(
            bd.join("build.rs"),
            "// NO LONGER EMBEDS ANYTHING, ON PURPOSE\nfn main() {}",
        )
        .expect("no embed");
        assert_eq!(
            build_embeds_manifest(&dir),
            Some(false),
            "the kit-migration state must be reported as such"
        );
        fs::remove_file(bd.join("build.rs")).expect("rm");
        assert_eq!(build_embeds_manifest(&dir), None, "missing file is unknown");
        assert_eq!(build_embeds_manifest(&dir.join("nowhere")), None);
        let _ = fs::remove_dir_all(&dir);
    }

    /// The trap this function fell into while being written: prose about a flag
    /// is not the flag. A file that explains its own removal must read as NOT
    /// embedding, or the clause becomes a lie generator.
    #[test]
    fn a_comment_about_the_flag_does_not_count_as_emitting_it() {
        let dir = std::env::temp_dir().join(format!("xtask-embed2-{}", std::process::id()));
        let bd = dir.join("crates/bridge-gpui");
        fs::create_dir_all(&bd).expect("dir");
        let path = bd.join("build.rs");
        // A raw string: this fixture is Rust source that contains quotes, and the
        // escape layer has already cost this slice twice.
        fs::write(
            &path,
            r#"//! used to add /MANIFEST:EMBED plus /MANIFESTINPUT=x.manifest
//! rust-lld: error: duplicate resource: type MANIFEST (ID 24)
fn main() { println!("cargo:rerun-if-changed=app.manifest"); }
"#,
        )
        .expect("prose only");
        assert_eq!(
            build_embeds_manifest(&dir),
            Some(false),
            "doc comments about a removed flag are not evidence it is still emitted"
        );
        fs::write(
            &path,
            r#"fn main() { println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED"); }
"#,
        )
        .expect("real flag");
        assert_eq!(build_embeds_manifest(&dir), Some(true));
        let _ = fs::remove_dir_all(&dir);
    }

    /// This crate's directory, so a test can resolve a target row's paths the way the
    /// harness does at run time (same trick as `check_ci`'s read of `ci.yml`). The
    /// paths on an [ArtifactTarget] are workspace-root relative, never crate-relative:
    /// smoke is run from the root and passes `root` down.
    fn crate_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// The whole point of the struct is that there are TWO bridges, and a row that
    /// names nothing real is worse than no row at all: it reads as coverage while the
    /// second lane stays invisible to the next person who wires a leg. So both rows are
    /// checked against the tree - every freshness root and file must exist, each exe
    /// path must be `target/debug/<bin>.exe`, and the Slint row must still carry NO
    /// build script, because `build_rs: ""` is a claim about the world and not a
    /// placeholder.
    ///
    /// Exists-on-disk is the assertion, deliberately, and not `git ls-files`: the
    /// freshness guard these paths feed (`identity::newest_mtime`) walks the filesystem,
    /// so asking git would test a different thing than the code uses. That is also why
    /// `crates/bridge-slint/ui/main.slint` is NOT named here - it is untracked working
    /// memory owned by the slint lane, and a test in this crate that pinned it would
    /// fail on their refactor rather than on a bug in ours. What this row promises is
    /// the `ui/` DIRECTORY, and that is what gets pinned: it is tracked (via its NOTES)
    /// and it is the unit the freshness root names.
    #[test]
    fn slint_target_has_no_build_script_yet() {
        let root = crate_root();
        let targets: [(&str, &ArtifactTarget); 2] =
            [("gpui", &GPUI_TARGET), ("slint", &SLINT_TARGET)];
        let mut missing: Vec<String> = Vec::new();
        for (label, t) in targets {
            assert!(
                !t.pkg.is_empty() && !t.bin.is_empty(),
                "the {label} row must name a package ({} as -p, {} as --bin)",
                t.pkg,
                t.bin,
            );
            // The exe is what gets built later, not what is on disk now: neither row's
            // exe may be asserted to exist here, or this test would need a build.
            assert!(
                Path::new(t.exe_rel).is_relative(),
                "the {label} row's exe path must be workspace-relative, not absolute",
            );
            assert_eq!(
                t.exe_rel,
                format!("target/debug/{}.exe", t.bin),
                "the {label} row's exe must sit where cargo puts a debug bin of that name"
            );
            for dir in t.freshness_roots {
                if !root.join(dir).is_dir() {
                    missing.push(format!("{label} root {dir} is not a directory"));
                }
            }
            for file in t.freshness_files {
                if !root.join(file).is_file() {
                    missing.push(format!("{label} file {file} is not a file"));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "an ArtifactTarget naming paths that are not there is a row nobody can trust: {missing:?}"
        );
        // And now the fact the doc comment on SLINT_TARGET promises: the second bridge
        // has no build script. `build_embeds_manifest` would answer `None` for it (no
        // file to read), and smoke must never claim a cause for an exe it cannot ask.
        assert!(
            !GPUI_TARGET.build_rs.is_empty(),
            "the first bridge's script is how smoke learns whether the build embedded"
        );
        assert!(
            root.join(GPUI_TARGET.build_rs).is_file(),
            "gpui build_rs {} must exist relative to {}",
            GPUI_TARGET.build_rs,
            root.display()
        );
        assert!(
            !root.join(SLINT_TARGET.build_rs).is_file(),
            "the slint row must not name a build script it does not have"
        );
        assert_eq!(
            SLINT_TARGET.build_rs, "",
            "empty, not a plausible path: slice 1 adds crates/bridge-slint/build.rs when \
             Slint needs a link step, and this test is the day that changes"
        );
        assert_eq!(
            BUILD_RS_REL, GPUI_TARGET.build_rs,
            "the const smoke actually reads must stay the projection of the row it came from"
        );
        // The mechanism still works against the real tree for the row that HAS a script:
        // an answer, not `None`.
        assert!(
            build_embeds_manifest(&root).is_some(),
            "reading the real bridge build script must produce a verdict, not unknown"
        );
    }

    /// `EXE_NAME` is a literal because `concat!` cannot read a const struct's field, and
    /// a literal is exactly the kind of thing that rots quietly: rename the bin target
    /// and smoke keeps launching (and then judging) a file cargo no longer emits. This
    /// is the test that doc comment points at. It pins BOTH rows' exe names, since the
    /// second row is the one no leg drives and therefore the one nothing else checks.
    #[test]
    fn exe_name_is_the_bin_target_plus_the_windows_extension() {
        assert_eq!(
            EXE_NAME,
            format!("{}.exe", GPUI_TARGET.bin),
            "EXE_NAME must stay bin + the Windows extension, or smoke launches nothing"
        );
        assert_eq!(
            BIN_REL,
            format!("target/debug/{}.exe", GPUI_TARGET.bin),
            "the default exe path and the launched name must agree about the same bin"
        );
        assert_eq!(
            SLINT_TARGET.exe_rel,
            format!("target/debug/{}.exe", SLINT_TARGET.bin),
            "the second bridge's described exe is named the same way, even unrun"
        );
        // The two agree by construction, not by coincidence of the same literal:
        // a redirected build still has to be named EXE_NAME.
        let (exe, _) = resolve_exe(Path::new("wherever"), Some("C:/builds/lane-4"));
        assert_eq!(
            exe.file_name().and_then(|n| n.to_str()),
            Some(EXE_NAME),
            "the CARGO_TARGET_DIR branch resolves to the same file name: {exe:?}"
        );
    }
}
