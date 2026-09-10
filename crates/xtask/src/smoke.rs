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
//! Exit codes: 0 every step passed, 1 a step failed, 2 the harness could not
//! run (no PowerShell, cannot write the probe, its own deadline), 3 declined -
//! there was no interactive desktop to test on.

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
}

/// The outcome of a probe run. Skip is a deliberate third state: "no desktop"
/// is neither a pass nor a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Skip(String),
    Fail(Vec<String>),
}

/// Every claim this harness makes, decided here, over parsed keys only.
pub fn decide(p: &Probe, artefact: Option<&Artefact>) -> Verdict {
    let mut failures: Vec<String> = Vec::new();
    if p.get("DESKTOP") == Some("0") {
        return Verdict::Skip(
            "no interactive desktop in this session (no windowed process, or the session is not              interactive) - a GUI smoke run cannot be had here"
                .to_string(),
        );
    }
    if !p.flag("PROBE_DONE") {
        failures.push(format!(
            "SMOKE FAIL: the probe did not finish (keys seen: [{}]) - it hit its own deadline or              died; a missing key is never a pass",
            p.keys()
        ));
        return Verdict::Fail(failures);
    }
    if !p.flag("SPAWN") {
        failures.push(format!(
            "SMOKE FAIL: {BIN_REL} exists but could not be started - run {BUILD_HINT} and look              for an antivirus block"
        ));
        return Verdict::Fail(failures);
    }

    let handle = p.number("HANDLE").unwrap_or(0);
    if handle == 0 {
        failures.push(format!(
            "SMOKE FAIL: no top-level window handle within {WINDOW_SECS}s (MainWindowHandle stayed              0; pid={}, title={:?}) - the window was never created, so nothing downstream can be              credited",
            p.get("PID").unwrap_or("?"),
            p.get("TITLE").unwrap_or("")
        ));
    } else if let Some(ms) = p.number("LAUNCH_MS") {
        if ms > COLD_START_BUDGET_MS {
            failures.push(format!(
                "SMOKE FAIL: launch to window took {ms}ms, over the {COLD_START_BUDGET_MS}ms                  cold-start budget (whitepaper §2)"
            ));
        }
    }

    if !p.flag("CLOSE_REQUESTED") {
        failures.push(format!(
            "SMOKE FAIL: CloseMainWindow (WM_CLOSE) was not accepted (handle={handle}) - the              graceful-shutdown path was never asked to run, so its silence proves nothing"
        ));
    }

    let forced = p.flag("FORCED");
    if forced || !p.flag("EXITED_WITHOUT_KILL") {
        failures.push(format!(
            "SMOKE FAIL: the app did NOT exit by itself within {CLOSE_SECS}s of WM_CLOSE and was              force-killed (code after the kill: {:?}) - a force-kill is not a graceful shutdown and              cannot PASS whatever code it ends with",
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
        None => failures.push(format!(
            "SMOKE FAIL: this run wrote no {SESSION_FILE}"
        )),
        Some(a) if !a.fresh => failures.push(format!(
            "SMOKE FAIL: {} exists but is OLDER than this launch, so this run never wrote it and              the fresh-install behaviour is not proven (rect={})",
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

fn read_artefact(path: &Path, launched: SystemTime) -> Option<Artefact> {
    let mtime = fs::metadata(path).ok()?.modified().ok()?;
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
    Some(Artefact {
        path: path.to_path_buf(),
        fresh: mtime >= launched,
        rect,
    })
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

/// Move a pre-existing session.json aside, so this run really is a fresh
/// install. Returns (live path, temp backup). Never deleted, never quiet.
fn stage_fresh_install(exe: &Path) -> Option<(PathBuf, PathBuf)> {
    for dir in candidate_state_dirs(exe) {
        let path = dir.join(SESSION_FILE);
        if !path.is_file() {
            continue;
        }
        let tag = dir
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "state".to_string());
        let aside = temp_path(&format!("session-backup-{tag}"), "json");
        if let Err(e) = fs::rename(&path, &aside) {
            println!(
                "smoke: could not move {} aside ({e}); freshness is judged by mtime instead",
                path.display()
            );
            return None;
        }
        println!(
            "smoke: fresh-install run - moved the existing {} aside to {}",
            path.display(),
            aside.display()
        );
        println!(
            "smoke: it is restored at the end of this run; if this process is killed, copy it              back from that temp path"
        );
        return Some((path, aside));
    }
    None
}

fn restore(path: &Path, aside: &Path) {
    match fs::rename(aside, path) {
        Ok(()) => println!("smoke: restored the user's {} unchanged", path.display()),
        Err(e) => eprintln!(
            "smoke: COULD NOT RESTORE {} from {} - copy it back by hand: {e}",
            path.display(),
            aside.display()
        ),
    }
}

/// One summary line, readable on its own in a CI log.
fn summary(verdict: &Verdict, p: &Probe, elapsed: Duration) -> String {
    let secs = format!("{:.1}s", elapsed.as_secs_f64());
    let window = if p.number("HANDLE").unwrap_or(0) != 0 {
        "OK"
    } else {
        "MISSING"
    };
    match verdict {
        Verdict::Pass => format!("smoke: window=OK close=0 session=OK {secs}"),
        Verdict::Skip(_) => "smoke: window=SKIP close=SKIP session=SKIP (no desktop)".to_string(),
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

/// Entry point for "cargo xtask smoke [--reuse-state]".
pub fn run(args: &[String]) -> i32 {
    let unknown: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| *a != "--reuse-state")
        .collect();
    if !unknown.is_empty() {
        eprintln!("smoke: unknown argument(s): {}", unknown.join(" "));
        eprintln!("smoke: usage: cargo xtask smoke [--reuse-state]");
        return 2;
    }
    let reuse = args.iter().any(|a| a == "--reuse-state");

    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("smoke: cannot read the current directory: {e}");
            return 2;
        }
    };
    let root = match crate::metadata::find_workspace_root(&cwd) {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("smoke: {e}");
            return 2;
        }
    };
    let exe = root.join(BIN_REL);
    println!("smoke: binary {}", exe.display());
    if !exe.is_file() {
        // Deliberately not built here: a harness that rebuilds what it tests
        // hides a build break inside a smoke verdict.
        println!("SMOKE FAIL: {BIN_REL} is missing - build it first with: {BUILD_HINT}");
        println!("smoke: window=MISSING close=NOBIN session=NOBIN 0.0s");
        return 1;
    }

    let started = Instant::now();
    let launched = SystemTime::now();
    let backup = if reuse {
        println!("smoke: --reuse-state - a pre-existing session.json stays where it is");
        None
    } else {
        stage_fresh_install(&exe)
    };

    let script = temp_path("probe", "ps1");
    let out_file = temp_path("stdout", "txt");
    let err_file = temp_path("stderr", "txt");
    let code = match fs::File::create(&script).and_then(|mut f| f.write_all(PROBE.as_bytes())) {
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
                    let verdict = decide(&probe, artefact.as_ref());
                    for failure in match &verdict {
                        Verdict::Fail(list) => list.clone(),
                        Verdict::Skip(why) => vec![format!("smoke: SKIPPED - {why}")],
                        Verdict::Pass => Vec::new(),
                    } {
                        println!("{failure}");
                    }
                    println!("{}", summary(&verdict, &probe, started.elapsed()));
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

    if let Some((path, aside)) = &backup {
        restore(path, aside);
    }
    for junk in [&script, &out_file, &err_file] {
        let _ = fs::remove_file(junk);
    }
    code
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

    fn failures_of(p: &Probe) -> Vec<String> {
        match decide(p, Some(&artefact())) {
            Verdict::Fail(list) => list,
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn a_graceful_exit_with_a_fresh_artefact_passes() {
        assert_eq!(decide(&clean_gui(), Some(&artefact())), Verdict::Pass);
        assert_eq!(
            summary(&Verdict::Pass, &clean_gui(), Duration::from_millis(2100)),
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
            summary(&Verdict::Fail(list), &p, Duration::from_secs(11)).contains("close=FORCED")
        );
    }

    #[test]
    fn a_self_exit_with_a_nonzero_code_fails_and_the_summary_shows_it() {
        let p = with(clean_gui(), "EXIT_CODE", "3");
        let list = failures_of(&p);
        assert!(list.iter().any(|f| f.contains("code 3")), "{list:?}");
        assert_eq!(
            summary(&Verdict::Fail(list), &p, Duration::from_millis(2100)),
            "smoke: window=OK close=3 session=FAILED 2.1s"
        );
    }

    #[test]
    fn no_window_or_a_missing_key_fails_rather_than_skipping() {
        let p = with(clean_gui(), "HANDLE", "0");
        assert!(
            failures_of(&p)
                .iter()
                .any(|f| f.contains("no top-level window"))
        );
        assert!(summary(&Verdict::Fail(vec![]), &p, Duration::ZERO).contains("window=MISSING"));
        // A probe that died half-way is not a pass and not a decline either.
        let truncated = Probe {
            kv: clean_gui()
                .kv
                .into_iter()
                .filter(|(k, _)| k != "PROBE_DONE")
                .collect(),
        };
        assert!(matches!(
            decide(&truncated, Some(&artefact())),
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
            decide(&clean_gui(), Some(&stale)),
            Verdict::Fail(_)
        ));
        assert!(matches!(decide(&clean_gui(), None), Verdict::Fail(_)));
    }

    #[test]
    fn an_absent_desktop_declines_instead_of_faking_either_verdict() {
        let headless = probe(&[("DESKTOP", "0"), ("PROBE_DONE", "1")]);
        let Verdict::Skip(why) = decide(&headless, None) else {
            panic!("no desktop must SKIP, not pass and not fail");
        };
        assert!(why.contains("desktop"), "{why}");
        assert!(summary(&Verdict::Skip(why), &headless, Duration::ZERO).contains("SKIP"));
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
}
