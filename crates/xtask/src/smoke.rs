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

const BIN_REL: &str = "target/debug/notes-gpui.exe";
/// Just the file name, for a CARGO_TARGET_DIR that replaces the whole tree.
pub const EXE_NAME: &str = "notes-gpui.exe";
const BUILD_HINT: &str = "cargo build -p notes-bridge-gpui --bin notes-gpui";
const SESSION_FILE: &str = "session.json";

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
    let exe_dir = exe.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut out = vec![exe_dir.join("data")];
    if let Some(appdata) = std::env::var_os("APPDATA") {
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

fn report_captured(path: &Path, label: &str) {
    if let Ok(text) = fs::read_to_string(path) {
        let text = text.trim();
        if !text.is_empty() {
            println!("smoke: {label} said ({} bytes): {text}", text.len());
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
const BUILD_ARGS: &[&str] = &["build", "-p", "notes-bridge-gpui", "--bin", "notes-gpui"];
/// The crates whose sources end up inside that binary, plus the two manifests
/// that decide the graph. Anything newer than the exe means the exe is not this
/// tree.
const SOURCE_ROOTS: &[&str] = &[
    "crates/bridge-gpui",
    "crates/api",
    "crates/core",
    "crates/platform",
];
const SOURCE_FILES: &[&str] = &["Cargo.toml", "Cargo.lock"];
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
       [int]$WindowSecs = 10, [int]$SettleMs = 4500, [int]$CloseSecs = 10)
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
if ($handle -ne 0) {
    $style = [WIN]::GetWindowLong($handle, -20)
    "TOPMOST=$([int](($style -band 8) -ne 0))"
    "EXSTYLE=$style"
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
pub const GEOMETRY_FAILED_EXIT: i32 = 6;
/// The PIN specifically: WS_EX_TOPMOST did not follow what session.json claimed.
/// Distinct from 6 because the fix usually lives in the show/pin ordering, not
/// in persistence - and a failure here may be a race, so the message says that.
pub const PIN_FAILED_EXIT: i32 = 7;

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

pub fn run_probe_script(
    script: &Path,
    exe: &Path,
    err_file: &Path,
    session: Option<&Path>,
    seed: Option<&Rect>,
    move_to: Option<(i32, i32)>,
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
        .arg("-WindowSecs")
        .arg(secs.to_string())
        .arg("-SettleMs")
        .arg(SETTLE_MS.to_string())
        .arg("-MoveX")
        .arg(match move_to {
            Some((x, _)) => x.to_string(),
            None => "-1".to_string(),
        })
        .arg("-MoveY")
        .arg(match move_to {
            Some((_, y)) => y.to_string(),
            None => "-1".to_string(),
        });
    if let Some(s) = seed {
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
    match wait_bounded(&mut child, OUTER_SECS) {
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
    let first = match run_probe_script(
        script,
        exe,
        err_file,
        Some(session),
        Some(&seed),
        Some((moved.l, moved.t)),
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
    let pin_true = topmost(&first);
    if pin_true.is_none() {
        println!("smoke: geometry: PIN not judged - the extended style could not be read");
    } else {
        println!(
            "smoke: geometry: PIN pinned:true -> WS_EX_TOPMOST={}",
            pin_true.unwrap_or(false) as i32
        );
    }
    // 5. Relaunch and see whether it comes back where it was left, and whether
    // an unpinned file really leaves the window un-topmost.
    if let Err(e) = set_pinned(session, false) {
        println!("smoke: geometry: PIN second polarity not judged: {e}");
    }
    let expect = stored.unwrap_or(moved);
    let second = match run_probe_script(script, exe, err_file, Some(session), None, None, 12) {
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
    let pin_false = topmost(&second);
    println!(
        "smoke: geometry: PIN pinned:false -> WS_EX_TOPMOST={}",
        match pin_false {
            Some(v) => v as i32,
            None => -1,
        }
    );
    let pin = match (pin_true, pin_false) {
        (Some(t), Some(f)) => {
            if t && !f {
                Pin::Matched {
                    pinned_true: true,
                    pinned_false: false,
                }
            } else {
                pin_failed = true;
                notes.push(format!(
                    "PIN: session.json said pinned:true and the window answered WS_EX_TOPMOST={t}, \
                     then pinned:false answered {f}; the topmost bit does not follow the state file \
                     (a show/pin race here is possible, so check the app log before blaming persistence)"
                ));
                Pin::Contradicts {
                    wanted: true,
                    seen: Some(t),
                    which: "pinned:true",
                }
            }
        }
        _ => Pin::NotJudged("the extended style could not be read on one of the two launches"),
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
pub const BUILD_RS_REL: &str = "crates/bridge-gpui/build.rs";

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
                    report_captured(&err_file, "the app on stderr");
                    report_captured(&out_file, "the app on stdout");
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

    // The window-memory round trip, run INSIDE the guard that owns the user's
    // state: it seeds session.json, moves the window and relaunches the app, so
    // every file it touches is a file the guard will put back or justify.
    if code == 0 {
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
                }
            },
        }
    } else {
        println!(
            "smoke: geometry: NOT RUN - the first launch did not succeed, so there is nothing to round trip"
        );
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
        let dirs = candidate_state_dirs(Path::new("C:/dev/notes/target/debug/notes-gpui.exe"));
        assert_eq!(dirs[0], PathBuf::from("C:/dev/notes/target/debug/data"));
        assert!(
            dirs.iter().any(|d| d.ends_with("notes-gpui")),
            "the installed candidate must be searched too: {dirs:?}"
        );
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
            fs::create_dir_all(dir.join(root).join("src")).expect("source tree");
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

    /// newest_source must look at every crate that feeds the binary, not just
    /// the bridge - an api edit is what made a cached exe lie here.
    #[test]
    fn the_freshness_scan_covers_every_crate_that_builds_the_binary() {
        let dir = std::env::temp_dir().join(format!("xtask-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let future = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        for (i, root) in SOURCE_ROOTS.iter().enumerate() {
            let file = dir.join(root).join("src").join("lib.rs");
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
            8,
            "the default-invocation codes, unchanged by this slice"
        );
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
//!     rust-lld: error: duplicate resource: type MANIFEST (ID 24)
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
}
