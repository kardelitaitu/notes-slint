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
//! Scope of that rule, stated because it generalises: a built exe is the ONLY
//! long-lived artefact this crate launches, so it is the only thing that can be
//! stale - and WHICH exe that is follows the selection: `notes-slint.exe` for a bare
//! run, because the product is the default (ADR-0006), `notes-gpui.exe` for the
//! frozen leg that `--binary=gpui` still selects. fixtures verify reads fixture files
//! and compares bytes - no binary, no staleness exposure - and every other gate row
//! is a cargo invocation, which rebuilds from the current source by definition. The
//! second launched artefact the old wording imagined did arrive, and it got the same
//! build-then-prove-freshness treatment.
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
//!   move never reached session.json, or the relaunch came back elsewhere, or - on the
//!   PRODUCT leg, since S8 - a real maximise closed out of a window that came back at a
//!   different restore rect. TWO producers now, one per lane: the needle schedule's seeded
//!   round trip, and a default `--binary=slint` run whose cycle needs no flag, no seed and
//!   no gpui. That second one is why the code has a producer again: with only the needle
//!   lane able to reach 6, and ADR-0006 freezing that lane, the ci.yml arm for 6 was
//!   guarding a code no default run could ever return. Geometry
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

/// Which cargo profile an artifact was built by.
///
/// NOT a CLI input yet - `--binary` is the slice that lets a caller name one, and
/// today every leg asks for [Profile::Debug]. It exists now because the profile was
/// baked into two string literals: `target/debug/...` in [GPUI_TARGET] and
/// `.join("debug")` in [resolve_exe]. That is the same bug shape this file just
/// fixed for the target - a literal quietly encoding one choice stops being one choice
/// the moment a second one appears. With the type here, the profile is an argument with
/// two legal values and [ArtifactTarget::exe_rel] is the only place the string is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Debug,
    /// Described, not driven: no leg builds or resolves a release artifact yet, so
    /// this variant is constructed only by the tests that prove [Profile::dir] and
    /// [ArtifactTarget::exe_rel] actually vary with it. Same shape as the marker on
    /// [SLINT_TARGET], same reason for the marker rather than a deletion: the seam is
    /// worth nothing if it has one legal value, and the day `--binary` grows a
    /// profile the consumer is the thing that removes this allow.
    #[allow(dead_code)]
    Release,
}

impl Profile {
    /// The directory cargo writes this profile's bins into.
    pub const fn dir(self) -> &'static str {
        match self {
            Profile::Debug => "debug",
            Profile::Release => "release",
        }
    }
}

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
    /// Where the exe sits IN THE DEBUG PROFILE when nobody set CARGO_TARGET_DIR.
    ///
    /// A const FIELD, and named `_debug` so nobody mistakes it for the truth
    /// about the row, for exactly the reason the launched name used to be a literal:
    /// `const &str`, and a const context cannot call the formatting function
    /// below. The guard against the two drifting is
    /// `exe_name_is_the_bin_target_plus_the_windows_extension`, which asserts
    /// this field EQUALS `exe_rel(Profile::Debug)` for BOTH rows - so the
    /// projection is checked against the seam rather than restated beside it.
    pub exe_rel_debug: &'static str,
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
    exe_rel_debug: "target/debug/notes-gpui.exe",
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
/// NO LONGER `#[allow(dead_code)]`: `--binary slint` selects this row, so it has a
/// non-test consumer from the commit it landed in - which is the condition its old
/// marker named ("described, not driven: no smoke leg runs this exe"). The DRIVE is
/// still missing, and [Leg::NotWired] is where that says so at runtime rather than
/// letting the row look judged.
const SLINT_TARGET: ArtifactTarget = ArtifactTarget {
    pkg: "notes-bridge-slint",
    bin: "notes-slint",
    exe_rel_debug: "target/debug/notes-slint.exe",
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

/// THE INSTRUMENTED SLINT BUILD, and the reason this row is a row and not a field on
/// the row above: as of STRIP-2b the crate ships TWO bins from one package -
/// `notes-slint` (the product: no self-hide, the real state dir, nothing a harness
/// can peek at) and `notes-slint-probe` (the same bytes with the needle plumbing
/// still in). Same package, same sources, same freshness roots, DIFFERENT contract -
/// which is exactly why a target is not a package name. The `-probe` row is what a
/// needle-schedule leg could be pointed at; the product is not, and never will be,
/// because its whole point is that it stops saying those lines.
const SLINT_PROBE_TARGET: ArtifactTarget = ArtifactTarget {
    bin: "notes-slint-probe",
    exe_rel_debug: "target/debug/notes-slint-probe.exe",
    ..SLINT_TARGET
};

/// THE THREE THINGS `--binary` ACCEPTS, in the order the refusal prints them.
pub const BINARY_NAMES: &[&str] = &["gpui", "slint", "slint-probe"];

/// THE ONE ARTIFACT A BARE RUN JUDGES: the product.
///
/// ADR-0006 froze `bridge-gpui` and named Slint the product, so a harness whose default
/// still picked the frozen bridge was printing the verdict about the wrong binary every
/// time nobody typed a flag - and "the default is the instrument, not the app" is not a
/// decision anyone made on purpose. `gpui` stays a legal value of `--binary` and keeps
/// its own leg (frozen is not deleted); it is no longer what you get for typing nothing.
///
/// Spelled once, as a literal, and PINNED to its slot in [BINARY_NAMES] rather than
/// derived from it: `the_binary_flag_parses_both_shapes_and_defaults_to_the_product`
/// asserts this equals `BINARY_NAMES[1]` and that the row it names resolves to
/// `notes-slint.exe`. Reordering [BINARY_NAMES] or renaming the product therefore goes
/// red here, in the same test that reads the flag - not quietly, in whichever string
/// happened to be indexed by hand.
pub const DEFAULT_BINARY: &str = "slint";

/// The row `--binary` names, or None for a name that is not one of [BINARY_NAMES].
/// Total and pure: the flag's whole job is picking which artifact the harness
/// resolves, builds and staleness-checks, so the mapping is a lookup this crate can
/// test without touching a disk.
/// The `'static spelling of a legal `--binary` value, or None.
///
/// The parser stores THIS and not the caller's bytes, for two reasons that are one
/// reason: a value that is not in [BINARY_NAMES] must be refused (an unlisted string
/// reaching the selection would be a name nobody validated), and a stored reference
/// into the argument vector is a lifetime the invocation struct does not own. So
/// `Invocation.binary` is always a `&'static str` drawn from one list, and
/// [select_target] is the lookup that list indexes.
fn legal_name(v: &str) -> Option<&'static str> {
    BINARY_NAMES.iter().copied().find(|n| *n == v)
}

pub fn select_target(name: &str) -> Option<&'static ArtifactTarget> {
    match name {
        "gpui" => Some(&GPUI_TARGET),
        "slint" => Some(&SLINT_TARGET),
        "slint-probe" => Some(&SLINT_PROBE_TARGET),
        _ => None,
    }
}

/// Which CONTRACT a run judges, given the artifact it selected. This is the honest
/// half of the wiring: resolving and building an exe is not the same as being able to
/// judge it, and a harness that runs gpui's needle schedule against a binary that
/// never emits those needles produces a verdict about the wrong product.
#[derive(Debug, PartialEq, Eq)]
pub enum Leg {
    /// The full needle schedule: self-hide, the window/close/session triple, the
    /// geometry and recents probes. Only the gpui artifact speaks it today.
    GpuiSchedule,
    /// THE PRODUCT CONTRACT, and it is a different contract: the startup lines the
    /// product says on its OWN stderr, a window that is still THERE at 45s (nothing
    /// in the product self-hides, so "alive" is assertable rather than inferred), and
    /// a WM_CLOSE the app answers by exiting 0 by itself. Deliberately NOT
    /// [Leg::GpuiSchedule]: the product stopped saying the needles that schedule
    /// reads, so pointing it at that schedule would print a verdict about a binary
    /// that was never meant to answer them.
    Product,
    /// Resolved, buildable, and NOT judged - with the reason, so `--binary slint`
    /// cannot read as a pass, nor as a product failure.
    NotWired(&'static str),
}

pub fn leg_for(target: &ArtifactTarget) -> Leg {
    // Compared by bin, not by pointer: two rows sharing a bin name could only be an
    // accident, and an accident should surface as a wrong verdict loudly.
    if target.bin == GPUI_TARGET.bin {
        Leg::GpuiSchedule
    } else if target.bin == SLINT_TARGET.bin {
        // THE PRODUCT OF THE SECOND BRIDGE. Its sibling `notes-slint-probe` is the
        // same package and a different contract again - the instrumented bytes still
        // answer the needle schedule - so that row falls through to [Leg::NotWired]
        // and stays there until somebody wires a schedule leg to it. One package,
        // two bins, two legs: which is why a target is not a package name.
        Leg::Product
    } else {
        Leg::NotWired(target.bin)
    }
}

/// The ONE honest reason a [Leg::NotWired] row is not judged, so the decline at
/// runtime names a cause instead of shrugging.
fn not_wired_reason(target: &ArtifactTarget) -> &'static str {
    if target.bin == SLINT_PROBE_TARGET.bin {
        "the only schedule this harness speaks is the gpui one, and pointing it at the          instrumented probe bytes is a port, not a flag: its needles are the same lines          the probe leg would have to re-decide, so that leg is its own slice of work"
    } else {
        "no leg was ever written for this artifact"
    }
}

/// One parsed invocation: everything `run()` decides before it touches a disk.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Invocation {
    pub reuse: bool,
    pub no_build: bool,
    pub require_ours: bool,
    /// The selected artifact, spelled as [BINARY_NAMES]. Defaults to [DEFAULT_BINARY] -
    /// the product - because a bare run is a verdict about the app that ships, not about
    /// the bridge [ADR-0006](../../../docs/decisions/0006-gpui-is-frozen-not-deleted.md)
    /// froze. Every one of the three names is still selectable.
    pub binary: &'static str,
}

/// Parse smoke's arguments. `--binary` takes both shapes (`--binary=slint` and
/// `--binary slint`): the two-token form is what a person types, the value form is
/// what a CI line can quote without ambiguity. A bad value refuses WITH the legal
/// names - a usage line that omits them is a refusal that does not help.
pub fn parse_args(args: &[String]) -> Result<Invocation, String> {
    let mut inv = Invocation {
        binary: DEFAULT_BINARY,
        ..Invocation::default()
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some(v) = a.strip_prefix("--binary=") {
            // legal_name, not select_target's argument: the parser stores the
            // 'static spelling from BINARY_NAMES, never a borrow of argv.
            inv.binary = legal_name(v).ok_or_else(|| unknown_binary(v))?;
            continue;
        }
        match a.as_str() {
            "--reuse-state" => inv.reuse = true,
            "--no-build" => inv.no_build = true,
            "--require-ours" => inv.require_ours = true,
            "--binary" => {
                let v = it.next().ok_or_else(|| {
                    format!(
                        "--binary needs a value: one of {} (--binary alone is not a default)",
                        BINARY_NAMES.join(", ")
                    )
                })?;
                inv.binary = legal_name(v).ok_or_else(|| unknown_binary(v))?;
            }
            other => {
                return Err(format!(
                    "unknown argument '{other}'; usage: cargo xtask smoke [--reuse-state] [--no-build] [--require-ours] [--binary={}] (no flag at all judges {DEFAULT_BINARY})",
                    BINARY_NAMES.join("|")
                ));
            }
        }
    }
    Ok(inv)
}

fn unknown_binary(v: &str) -> String {
    format!(
        "unknown --binary value '{v}'; this harness can point at {} - {} is the product AND the default a bare run judges, {} is the same bytes with the needle plumbing, and gpui is the frozen bridge (frozen, not deleted: naming it still runs the needle schedule)",
        BINARY_NAMES.join(", "),
        BINARY_NAMES[1],
        BINARY_NAMES[2],
    )
}

/// THE FROZEN LEG'S exe path, and the name says what it used to be: this was THE
/// default's path, and it still is one - [DEFAULT_BINARY] is `slint`, so a bare run
/// never resolves this string. Its only runtime reader is [decide], the needle
/// schedule, which is the `gpui` leg; retargeting it to `notes-slint.exe` would have a
/// red `--binary=gpui` run blame an antivirus block on a file it never launched. The
/// DEFAULT's own path is derived from the selected row, never from here: `slint` resolves
/// to `target/debug/notes-slint.exe`, and
/// `the_binary_flag_parses_both_shapes_and_defaults_to_the_product` pins that.
const BIN_REL: &str = GPUI_TARGET.exe_rel_debug;
// EXE_NAME is gone entirely, with no wrapper left behind. It was a third copy of the
// bin name, guarded only by a test comparing it to GPUI_TARGET.bin; once
// ArtifactTarget::bin_file existed the copy had no job, because every path through
// resolve_exe takes the derived name from the SELECTED row. What still earns its keep
// is the literal "notes-gpui.exe" in the test that pins what CI artifact paths and
// xtask manifest assume by hand. Deleting a guarded duplicate beats guarding one;
// deleting a duplicate's alias beats both.
/// The build command the FROZEN `gpui` leg suggests when its exe will not start - the
/// same leg as [BIN_REL], and the same reason not to widen it to the product. Every
/// OTHER build text the harness prints takes the selected row's own [ArtifactTarget::
/// build_args], so a bare run says "notes-slint" because that is the row it picked, not
/// because a literal here was moved.
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

impl ArtifactTarget {
    /// THE PATH SEAM: `target/{profile}/{bin}.exe`. Every place that used to
    /// concatenate `"debug"` with an exe name goes through here, so a profile is
    /// one argument instead of a fourth literal for somebody to forget.
    pub fn exe_rel(&self, profile: Profile) -> String {
        format!("target/{}/{}.exe", profile.dir(), self.bin)
    }

    /// Just the file name, for a CARGO_TARGET_DIR that replaces the whole tree - the
    /// The test `exe_name_is_the_bin_target_plus_the_windows_extension` pins what
    /// this returns against the spelled-out name CI, [crate::manifest] and the smoke
    /// steps all use by hand - so the derivation is checked, not assumed.
    pub fn bin_file(&self) -> String {
        format!("{}.exe", self.bin)
    }

    /// The five tokens that build THIS artifact. Derived rather than stored because
    /// the pair (package, bin) has to agree with [ArtifactTarget::exe_rel]: if
    /// `cargo build -p A --bin B`$ does not emit `target/debug/B.exe`$, one of the two is
    /// wrong, and a single source cannot be half wrong.
    pub const fn build_args(&self) -> [&'static str; 5] {
        ["build", "-p", self.pkg, "--bin", self.bin]
    }
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
/// BUILD_ARGS is gone, and it died the way [EXE_NAME] did: it was already derived
/// from [GPUI_TARGET], but as a `const` it could only ever name ONE row, so the moment
/// `--binary` existed the build step and the launch step would disagree about which
/// crate they meant - a compile failure in crate A judged on crate B's stale exe.
/// [ArtifactTarget::build_args] is the same five tokens, per row.
///
/// No `--locked` here, deliberately, unlike CI's bridge steps: smoke runs on a dirty
/// working tree by design, and --locked would refuse the run for a manifest the
/// developer is mid-edit on. Locking the graph is CI's gate's business (see check.rs).
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
/// The gpui row's roots and files, as consts.
///
/// TEST-ONLY, and that is the point: the runtime path takes the roots from the
/// SELECTED row (see [newest_source]), so a const here that names one bridge would be
/// a second opinion nobody asked for - exactly the drift the long story below
/// documents. What these two are for is the tests that walk the list, and the story
/// that explains why the list is what it is.
#[cfg(test)]
const SOURCE_ROOTS: &[&str] = GPUI_TARGET.freshness_roots;
/// Inputs that live OUTSIDE a src dir, named one by one because a scan that walked
/// whole crate dirs would walk their tests with them: the workspace manifest and the
/// lock (the lock decides the entire external graph), one manifest per crate above (a
/// feature or dependency line there changes the binary), and the bridge's build.rs (it
/// decides the embedded manifest, so a stale build script is a stale exe).
#[cfg(test)]
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
pub fn newest_source(
    root: &Path,
    target: &ArtifactTarget,
) -> Option<(std::time::SystemTime, PathBuf)> {
    // THE ROOTS FOLLOW THE SELECTION. This is the false-5 half of the wiring: a
    // slint exe judged against gpui's source list is "stale" the moment anybody
    // touches crates/bridge-gpui, and "fresh" when crates/bridge-slint changes -
    // both wrong, one of them silently.
    crate::identity::newest_mtime(root, target.freshness_roots, target.freshness_files)
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
pub fn build_target(root: &Path, target: &ArtifactTarget) -> Result<(), (String, usize)> {
    let out = std::process::Command::new("cargo")
        .args(target.build_args())
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
///
/// The CAUSE of a manifest state is named from the SELECTED row's own build script,
/// and never from gpui's. This used to print bridge-gpui's build.rs sentence over
/// whatever exe was on display, which for the slint product read as "bridge-slint's
/// build script embeds nothing" - a claim about a file that row does not have. A
/// reader must not be handed another artifact's explanation for these bytes.
fn report_binary(exe: &Path, root: &Path, built: bool, target: &ArtifactTarget) {
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
    let embed = if target.build_rs.is_empty() {
        format!(
            "{} declares NO build script of its own, so NOTHING of ours is embedded into \
 these bytes at link time - the manifest's only possible authors are the toolkit itself or a \
 post-link 'cargo xtask manifest' run (and {} build.rs explains {} bytes, not these)",
            target.pkg, GPUI_TARGET.pkg, GPUI_TARGET.bin
        )
    } else {
        match build_embeds_manifest(root) {
            Some(true) => format!("{} EMBEDS app.manifest at link time", target.build_rs),
            Some(false) => format!(
                "{} does NOT embed anything, so these bytes did not come from cargo build \
 - they came from a post-link 'cargo xtask manifest' run, or from the toolkit's own manifest",
                target.build_rs
            ),
            None => format!(
                "{} could not be read, so the cause is unknown",
                target.build_rs
            ),
        }
    };
    match manifest_of(root, exe) {
        Ok(m) => {
            println!(
                "smoke: manifest in that exe: identity={} longPathAware={} PerMonitorV2={} | {}",
                m.identity, m.long_path, m.per_monitor, embed
            );
            if let crate::manifest::ReadBack::Missing(absent) = crate::manifest::read_back(&m) {
                // The CAUSE is named from the row again: "the kit migration build.rs"
                // is a bridge-gpui sentence, and printing it over another artifact's
                // bytes accuses a file that exe was never linked with.
                let why = if target.build_rs.is_empty() {
                    format!(
                        "{} has NO build script at all, so nothing could have embedded our \
 declaration into it at link time: the post-link 'cargo xtask manifest' step is the only door \
 to a compliant exe here.",
                        target.pkg
                    )
                } else {
                    format!(
                        "{} embeds NOTHING (the kit migration stopped it), so a plain cargo \
 build is NOT compliant: the post-link 'cargo xtask manifest' step is required and CI gates on \
 it.",
                        target.build_rs
                    )
                };
                println!(
                    "SMOKE WARN: this exe does NOT carry our manifest - no {:?}. {} Locally \
 this stays a warning, because a bare cargo run still gets PerMonitorV2 from the \
 toolkit - byte-identical DPI semantics - and a red nobody can clear without \
 learning a new command trains people to ignore red. --require-ours makes it 8.",
                    absent, why
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
  // M9: window memory is about the RESTORED position - rcNormalPosition - which
  // GetWindowRect can never produce while the window is maximised (that answer is
  // the zoomed frame with its overhang). So the probe asks the OS that actually
  // stores it. Struct layout is the documented WINDOWPLACEMENT: 4+4+4, two POINTs,
  // two RECTs.
  [StructLayout(LayoutKind.Sequential)] public struct WINDOWPLACEMENT { public int length; public int flags; public int showCmd; public POINT ptMinPosition; public POINT ptMaxPosition; public RECT rcNormalPosition; public RECT rcReserved; }
  [DllImport("user32.dll")] public static extern bool GetWindowPlacement(IntPtr h, ref WINDOWPLACEMENT wp);
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
function Get-Placement($h) {
    $wp = New-Object WIN+WINDOWPLACEMENT
    $wp.length = [Runtime.InteropServices.Marshal]::SizeOf($wp)
    if (-not [WIN]::GetWindowPlacement($h, [ref]$wp)) { return '' }
    $r = $wp.rcNormalPosition
    # showCmd rides along in the same string so the stability test below covers the
    # show state too - a rect that has stopped moving while the state is still
    # flipping is not settled.
    return "$($r.Left),$($r.Top),$($r.Right),$($r.Bottom)|$($wp.showCmd)"
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
# M9, THE NORMAL POSITION, SETTLED. Two things this replaces: a FRAME-vs-FRAME
# comparison taken on the handle-sighting tick with zero settle (which compared the
# zoomed overhang against itself and called 0,0 VACUOUSLY green, and read a cold
# exe's pre-zoom 382,274 as RED), and any single sample at all. The rule is now:
# poll GetWindowPlacement every 100 ms until the SAME value appears in two
# consecutive samples, with $SettleMs as the ceiling, print every sighting that
# changed, and report NORMAL_STABLE so a value that never stopped moving becomes a
# finding rather than the quietest number on the page.
$norm = ''
$stable = 0
$pcmd = -1
if ($handle -ne 0) {
    $prev = Get-Placement $handle
    $sw2 = [Diagnostics.Stopwatch]::StartNew()
    while ($sw2.ElapsedMilliseconds -lt $SettleMs) {
        Start-Sleep -Milliseconds 100
        $next = Get-Placement $handle
        if ($next -eq '') { break }
        if ($next -ne $prev) {
            "NORMAL_SEEN=$($next.Split('|')[0]) SHOWCMD=$($next.Split('|')[1])"
            $prev = $next
        } else { $stable = 1; break }
    }
    $norm = $prev
}
if ($norm -ne '') {
    $parts = $norm.Split('|')
    "NORMAL=$($parts[0])"
    "NORMAL_SHOWCMD=$($parts[1])"
} else {
    'NORMAL='
    'NORMAL_SHOWCMD=-1'
}
"NORMAL_STABLE=$stable"
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

/// The maximised cycle: seed the file to say maximized:true, launch it, close it
/// clean, relaunch, and measure what the restore did to the STORED position.
///
/// WHAT IT COMPARES, and this is the honesty fix. The promise is about
/// `rcNormalPosition` - where the window comes back - so both sides of the drift are
/// NORMAL readings from `GetWindowPlacement`, and the baseline is specifically the
/// rect launch 1 wrote into session.json. It used to compare two live `GetWindowRect`
/// FRAME samples taken on the handle-sighting tick with NO settle at all, which was
/// wrong twice over: a maximised window's frame is the same zoomed overhang in both
/// samples, so the drift was 0,0 VACUOUSLY green, and the same leg went RED when a
/// cold exe simply had not applied the zoom yet on launch 1 (a pre-zoom 382,274 read
/// against a post-zoom relaunch). Neither of those was a fact about the product.
///
/// It is also cold-cache safe now: the reading is taken after the probe has polled
/// `GetWindowPlacement` until two consecutive 100 ms samples agree, so a slow first
/// link does not move the baseline; a launch whose position NEVER agrees is reported
/// as a failure of the promise, not judged on a moving number.
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
    // the promise, and the SETTLED normal position is the rect that gets persisted on
    // the way out. That is why the comparison below never starts from a live FRAME
    // sample (see the leg's doc comment).
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
    let rect_before = probe_rect(&first, "NORMAL");
    if !first.flag("NORMAL_STABLE") {
        restore_session(session, before.as_deref());
        return Some(
            "MAXIMISED: launch 1's normal position never settled inside the settle ceiling \
             (NORMAL_STABLE=0) - the window the promise has to remember was still moving \
             when the run ended"
                .to_string(),
        );
    }
    // THE BASELINE IS WHAT LAUNCH 1 STORED, not a second live sample of the same
    // window. A ratchet walks the persisted normal position, so the pair to diff is
    // (the rect launch 1 wrote to session.json) -> (the normal position launch 2
    // comes back at). Two live FRAME samples of a maximised window agree with each
    // other for the wrong reason - the zoomed overhang is the same overhang twice -
    // which is how a 0,0,0,0 drift could be reported VACUOUSLY while a cold exe read
    // of the pre-zoom 382,274 made the same leg go red for an unrelated reason.
    let persisted_first = fs::read_to_string(session)
        .ok()
        .and_then(|t| persisted_rect(&t));
    let base = match (persisted_first, rect_before) {
        (Some(r), _) => Some(r),
        (None, q) => q,
    };
    println!(
        "smoke: maximised: INFO - launch 1 stored {} (NORMAL at sighting {}, stable={}, showCmd {})",
        base.map(|r| r.text()).unwrap_or_else(|| "-".into()),
        rect_before.map(|r| r.text()).unwrap_or_else(|| "-".into()),
        first.flag("NORMAL_STABLE"),
        first.number("NORMAL_SHOWCMD").unwrap_or(-1)
    );
    // Launch two: the same file, relaunched, is where a frame/client double-count
    // becomes visible, because the persisted rect is applied as bounds and measured
    // back as a normal position again.
    let second = match run_probe_script(script, exe, err_file, Some(session), None, None, 12) {
        Ok(p) => p,
        Err(e) => {
            println!("smoke: maximised: NOT JUDGED - relaunch did not report: {e}");
            restore_session(session, before.as_deref());
            return None;
        }
    };
    let rect_after = probe_rect(&second, "NORMAL");
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
    // A value that never stopped moving is not a reading. Both launches must settle
    // inside the ceiling the probe was handed; the probe reports that itself, and a
    // NO here is the promise failing rather than a measurement problem - the restored
    // position is supposed to BE stable.
    if !second.flag("NORMAL_STABLE") {
        restore_session(session, before.as_deref());
        return Some(
            "MAXIMISED: the relaunch's normal position never settled inside the settle \
             ceiling (NORMAL_STABLE=0) - a window still moving at the end of the run has \
             no fixed point to assert"
                .to_string(),
        );
    }
    // Fallback, stated where it is used: if this launch's GetWindowPlacement failed,
    // the rect the app STORED while maximised is by definition the normal position it
    // would restore to, so comparing against that loses only the certainty that the
    // OS said it and not the app.
    let after = match (rect_after, persisted) {
        (Some(r), _) => Some(r),
        (None, q) => q,
    };
    if rect_after.is_none() {
        println!(
            "smoke: maximised: INFO - launch 2's GetWindowPlacement gave nothing; comparing the persisted rect"
        );
    }
    let drift = rect_drift(base, after);
    println!(
        "smoke: maximised: INFO - drift across one cycle (dx, dy, dw, dh) = {drift:?}; the rect launch 1 persisted was {}, and the chrome measured this run is (+8, +0, -8, -8) - a non-zero quadruple matching that border is the frame/client double-count, not a race",
        base.map(|r| r.text()).unwrap_or_else(|| "-".into())
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
    // the product did. The settle the DRIFT now relies on is a GetWindowPlacement
    // poll; ZOOM itself is still one sample taken at the sighting tick, so arming it
    // stays a part-3 question. The drift is honest now; the zoom line still reports
    // when the harness looked, not what the product did.
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
            r1 = base.map(|r| r.text()).unwrap_or_else(|| "-".into()),
            r2 = after.map(|r| r.text()).unwrap_or_else(|| "-".into())
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

/// THE PROFILE EVERY LEG RUNS AT TODAY, expressed as a guard rather than a habit.
///
/// gpui's leg is Debug for a concrete reason, not a default: [ArtifactTarget::build_args]
/// builds a debug bin, [crate::manifest] rewrites and reads back
/// `target/debug/notes-gpui.exe`, and CI's artifact chain names that same path -
/// so a Release exe would have smoke judge a file nothing in this repo produces.
/// When `--binary` grows a profile argument, THIS is the function that stops
/// being constant, and the assertion below is what fails the day somebody widens one
/// side and not the other. Until then it takes the target and ignores it, which is
/// honest: the seam exists so the profile has one home, not so it can vary.
const fn driven_profile(target: &ArtifactTarget) -> Profile {
    let _ = target;
    Profile::Debug
}

/// The guard, evaluated at compile time: if the driven profile ever stops being
/// Debug for the target smoke actually runs, this does not build.
const _: () = assert!(matches!(driven_profile(&GPUI_TARGET), Profile::Debug));

/// The exe under test, honouring CARGO_TARGET_DIR.
///
/// Every lane is now told to build into a private target directory, and smoke
/// used to resolve <root>/target/debug anyway: it then judged a binary nobody had
/// built, failed its OWN freshness guard, and left the intended exe sitting
/// unused. A harness that cannot be pointed at a build is not a harness. The
/// resolution is pure and returned with the reason it chose, because the path is
/// the thing a reader needs to trust the verdict.
pub fn resolve_exe(
    root: &Path,
    target_dir: Option<&str>,
    target: &ArtifactTarget,
) -> (PathBuf, &'static str) {
    let profile = driven_profile(target);
    let trimmed = target_dir.unwrap_or_default().trim();
    if trimmed.is_empty() {
        return (
            root.join(target.exe_rel(profile)),
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
        // Same helper the non-redirected branch's path is built from, so a
        // redirected lane's exe is named by exactly one expression in this crate.
        base.join(profile.dir()).join(target.bin_file()),
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
/// Entry point for `cargo xtask smoke [--reuse-state] [--no-build] [--require-ours] [--binary=slint|gpui|slint-probe]`; with no `--binary` the row is [DEFAULT_BINARY], the product.
pub fn run(args: &[String]) -> i32 {
    // The contract first, before anything can fail: a log that shows a verdict
    // also shows the code table that verdict came out of.
    println!("{}", contract_line());
    let inv = match parse_args(args) {
        Ok(inv) => inv,
        Err(why) => {
            eprintln!("smoke: {why}");
            return HARNESS_EXIT;
        }
    };
    // parse_args only ever stores a name select_target accepts, so the unwrap is the
    // parser's own invariant, restated rather than trusted: if the two ever disagree
    // this says which one lied.
    let target = select_target(inv.binary).unwrap_or_else(|| {
        eprintln!("smoke: INTERNAL - parse_args accepted '{0}' and select_target rejects it; the flag's two lists disagree", inv.binary);
        std::process::exit(HARNESS_EXIT);
    });
    let reuse = inv.reuse;

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
    let (exe, resolved_from) = resolve_exe(
        &root,
        std::env::var("CARGO_TARGET_DIR").ok().as_deref(),
        target,
    );
    println!(
        "smoke: exe path {} | resolved from {resolved_from}",
        exe.display()
    );
    // THE LEG QUESTION, asked here - after the path is named, before anything is
    // built, launched or judged.
    //
    // Selecting an artifact and being able to JUDGE it are different abilities. The
    // needle schedule this harness IS (self-hide, the window/close/session triple, the
    // geometry and recents probes) is gpui's contract; running it against
    // notes-slint.exe - which as of STRIP-2b deliberately stops hiding itself, stops
    // using the probe state dir and stops saying the lines the schedule reads - would
    // print a confident verdict about a binary that was never meant to answer them.
    // Green there is a lie, red there is a false accusation against the product, so
    // neither is allowed to happen, and the only honest answer left is "not judged".
    //
    // Exit 2 (HARNESS_EXIT), deliberately NOT 3: 3 claims "this MACHINE cannot host the
    // check", which is a statement about the runner this decline is not entitled to
    // make - a box with a perfect window station answers the same 2. 2 says the harness
    // itself stopped before judging anything, which is exactly what happened here: the
    // missing leg is ours, not the desktop's.
    let leg = leg_for(target);
    if let Leg::NotWired(bin) = &leg {
        println!(
            "smoke: NOT JUDGED - '{bin}' resolves to {} ({}) but no leg speaks its contract yet.",
            exe.display(),
            if exe.is_file() {
                "an exe that EXISTS on disk"
            } else {
                "no exe on disk"
            }
        );
        println!(
            "smoke:   legs wired today: [{}]",
            BINARY_NAMES
                .iter()
                .filter(|n| {
                    !matches!(
                        leg_for(select_target(n).unwrap_or(&GPUI_TARGET)),
                        Leg::NotWired(_)
                    )
                })
                .copied()
                .collect::<Vec<_>>()
                .join(", ")
        );
        // WHY this one is not judged, named at the row rather than guessed by the
        // reader: the product leg shipped, so a decline after it is a specific
        // absence and not the general "nobody got here yet".
        println!("smoke:   reason: {}", not_wired_reason(target));
        println!(
            "smoke:   judged legs today: the gpui needle schedule, and the product contract \
             (the startup lines on piped stderr, alive at 45s, a WM_CLOSE answered by an exit 0 \
             with the session write joined, and the M9 maximise fixed point on the restore rect) \
             against {}.exe. This run launched nothing: there \
             is NO verdict about {bin} in either direction.",
            SLINT_TARGET.bin
        );
        return HARNESS_EXIT;
    }
    // BUILD FIRST. A harness that launches whatever exe happens to be lying
    // around proves a cached binary, and a green line on a stale exe is the
    // most dangerous output this repo can produce: it says the app works when
    // what worked was three commits old. A compile failure is therefore its own
    // hard verdict (4), never a decline and never a silent fallback to the old
    // exe on disk.
    let built = if inv.no_build {
        println!(
            "smoke: NOT building (--no-build): this run can only prove the exe already on disk"
        );
        false
    } else {
        match build_target(&root, target) {
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
    report_binary(&exe, &root, built, target);
    // The strict shape, opt-in: the line above always names what is in the exe,
    // and this decides whether a foreign manifest is allowed to pass. Not
    // default, because the ordinary state on a developer machine is that nobody
    // ran the post-link step yet, and a rule that makes every local run red is
    // a rule that gets --no-build-ed past. An UNREADABLE manifest is not judged
    // here either: absence of evidence is not evidence of a violation.
    if inv.require_ours {
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
    // THE ROOTS THE SELECTED ROW NAMES. This call used to pass &GPUI_TARGET by
    // literal, and that was the false-5 half of the wiring: judging notes-slint.exe
    // for freshness against bridge-gpui's sources cries "stale" the moment anybody
    // touches a gpui file, and stays silent when the slint sources move. Same shape
    // as the --binary bug the row exists to fix, one call site further down: a
    // literal quietly encoding one choice after a second choice has appeared.
    match staleness(mtime_of(&exe), newest_source(&root, target)) {
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

    // THE PRODUCT LEG BRANCHES HERE, and the place matters. Everything above is
    // row-driven and belongs to BOTH legs: the build comes from the selected row's
    // build_args(), the freshness roots from that row, the manifest's CAUSE from that
    // row's own build script. Everything below is the gpui NEEDLE SCHEDULE - the
    // self-hide, the probe state dir, the session/geometry/recents triple - and the
    // product stopped saying those lines on purpose. So the product is judged by its
    // own contract from here, and the schedule never sees these bytes.
    if matches!(leg, Leg::Product) {
        return run_product_leg(target, &exe);
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

/// How long the PRODUCT must still be there, on screen and running, before the
/// harness is allowed to ask it to close. This is the assertion the needle schedule
/// can never make, because that schedule's own artifact hides itself: the product has
/// no self-hide, so "alive at 45s" is a fact about the shipped app rather than a
/// favour done to the probe.
const PRODUCT_ALIVE_SECS: u64 = 45;
/// How long each leg of the M9 cycle sits on screen before it is asked to close. NOT
/// [PRODUCT_ALIVE_SECS]: "alive at 45s" is a claim about the shipped app and the plain
/// launch makes it ONCE. A cycle launch only has to be up long enough to take a SETTLED
/// placement reading, so it pays seconds rather than another 45s each.
const PRODUCT_CYCLE_ALIVE_SECS: u64 = 4;
/// [SETTLE_MS] spelled in seconds, rounded UP: the cycle's two settle polls are bounded
/// internally by the millisecond number and externally by this one, and an outer bound
/// that truncates the ceiling it is guarding kills a healthy child for no reason.
const PRODUCT_CYCLE_SETTLE_SECS: u64 = SETTLE_MS as u64 / 1000 + 1;
/// The outer bound on ONE cycle launch: the sighting wait, the short sit, the two settle
/// polls, the bounded wait for the show state to apply, the close, and the same 20s
/// headroom. Deliberately NOT folded into [PRODUCT_OUTER_SECS]: the plain launch keeps
/// its own deadline, so a slow maximise can never turn a geometry that could not be
/// measured into a false TIMEOUT on the promise the plain leg already proved.
const PRODUCT_CYCLE_OUTER_SECS: u64 = WINDOW_SECS
    + PRODUCT_CYCLE_ALIVE_SECS
    + 2 * PRODUCT_CYCLE_SETTLE_SECS
    + (PIN_WAIT_MS as u64 / 1000 + 1)
    + CLOSE_SECS
    + 20;
/// The product probe's outer bound: every window the script itself bounds, plus the
/// same 20s headroom [OUTER_SECS] keeps for the same reason.
const PRODUCT_OUTER_SECS: u64 = WINDOW_SECS + PRODUCT_ALIVE_SECS + CLOSE_SECS + 20;

/// `SW_SHOWMAXIMIZED`, the value `GetWindowPlacement` answers in `showCmd` for a window
/// that is maximised NOW. The test is EQUALITY against it, which is the rule
/// crates/platform/src/windows/monitors.rs:69-77 applies to the same iconic field and
/// the reason it is not the `WPF_RESTORETOMAXIMIZED` flag: that bit names an INTENT to
/// restore maximised, not a resting place, and a checker that reads the flag would pass
/// a window the user never left zoomed.
const SW_SHOWMAXIMIZED: i64 = 3;

/// THE PREFIX, asserted rather than assumed: [crate] = plumbing.rs's `report()` is the
/// one voice the product has with no console, and a line in somebody else's voice is
/// not the product saying it. Slint's own warnings share this stderr.
pub const PRODUCT_VOICE: &str = "notes-gpui: ";

/// What the product owes BEFORE its window is judged alive, each needle paired with
/// the file that emits it, so a red line says which promise broke instead of leaving
/// a reader to grep for the string. Every needle is spelled WITHOUT the voice prefix
/// and matched only on a line that carries it.
pub const PRODUCT_STARTUP_NEEDLES: &[(&str, &str)] = &[
    (
        "startup: state dir ",
        "product.rs - where it decided to keep its state",
    ),
    (
        "chord: legend",
        "product.rs - the chords a user has to reach the menu by",
    ),
    (
        "startup: entering the loop",
        "product.rs - that it got as far as running",
    ),
    (
        "drop: armed hwnd=",
        "plumbing.rs - a real handle, armed for drops",
    ),
];

/// What the product owes AFTER the harness asks it to close, in order. These two are
/// the whole shutdown-honesty claim: the close was granted the FIRST time, and the
/// session write ran on a thread that was joined rather than abandoned.
pub const PRODUCT_CLOSE_NEEDLES: &[(&str, &str)] = &[
    (
        "close: requested #1, granted",
        "product.rs - the first WM_CLOSE was accepted, not swallowed",
    ),
    (
        "shutdown: joined cleanly",
        "product.rs - the save thread was joined, so the session write ran",
    ),
];

/// Which owed lines a capture is missing - and which arrived in the wrong voice.
/// Pure over its inputs, because the assert is the product of this leg and a verdict
/// nobody can test is a verdict nobody can trust.
pub fn product_gaps(trace: &str, needles: &[(&str, &str)], phase: &str) -> Vec<String> {
    let mut gaps = Vec::new();
    for (needle, source) in needles {
        let hits: Vec<&str> = trace
            .lines()
            .filter(|l| l.contains(needle))
            .collect::<Vec<_>>();
        match hits.first() {
            None => gaps.push(format!(
                "SMOKE FAIL: {phase} - the product never said {needle:?}; that line comes from \
 {source}, and its absence means the step it reports did not happen"
            )),
            Some(line) => {
                if !line.trim_start().starts_with(PRODUCT_VOICE) {
                    gaps.push(format!(
                        "SMOKE FAIL: {phase} - {needle:?} appeared on stderr WITHOUT the \
 {PRODUCT_VOICE:?} voice (line: {line:?}), so this harness cannot credit it to the product's \
 own report()"
                    ));
                }
            }
        }
    }
    gaps
}

/// The three reasons this MACHINE cannot host a product run, in the order the two
/// existing scripts (PROBE and GEOMETRY_PROBE) already distinguish them: no Win32
/// declarations to compile, a non-interactive session, and an interactive session with
/// no windowed process on it. Named apart because they are fixed apart, and because a
/// decline that says "no desktop" has been read as an app bug often enough to matter.
pub fn product_station_gaps(p: &Probe) -> Vec<String> {
    let mut gaps = Vec::new();
    if !p.flag("WIN32") {
        gaps.push("this PowerShell could not compile the Win32 declarations (WIN32=0)".to_string());
    }
    if !p.flag("INTERACTIVE") {
        gaps.push("[Environment]::UserInteractive is false, so the session has no window station a GUI can be shown in".to_string());
    }
    if !p.flag("WINDOWED") {
        gaps.push("no process on this session owns a top-level window (WINDOWED=0), so there is no desktop to be seen on".to_string());
    }
    gaps
}

/// The product's own probe. It launches the exe with its stderr captured, waits for a
/// REAL window, sits on it for ALIVE_SECS, then closes it through the one sanctioned
/// door - [System.Diagnostics.Process]::CloseMainWindow, the WM_CLOSE the existing
/// PROBE already drives - and reports whether the app left by itself.
///
/// It decides nothing, in the file's existing discipline: KEY=VALUE out, verdict in
/// Rust. And the kill at the end is a TEARDOWN ON FAILURE only: a force-kill is
/// reported as FORCED=1, which the verdict turns into a failure rather than a pass.
const PRODUCT_PROBE: &str = r#"
param([Parameter(Mandatory)][string]$Exe, [string]$OutFile, [string]$ErrFile,
       [string]$Session = '', [int]$WindowSecs = 10, [int]$AliveSecs = 45,
       [int]$CloseSecs = 10, [int]$Maximise = 1, [int]$SettleMs = 4500)
$ErrorActionPreference = 'SilentlyContinue'
$code = @'
using System;
using System.Runtime.InteropServices;
public static class PROD {
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  // THE MAXIMISE DOOR, and the RESTORE RECT door. ShowWindow(SW_MAXIMIZE) is what the
  // caption button and the title-band double-click both end up doing - the OS zooms the
  // window and winit reports the resulting state change, which is the same route a user
  // act takes; it is NOT SendMessage of a private message and it is not a call into the
  // app. GetWindowPlacement is read because rcNormalPosition, not GetWindowRect, IS the
  // thing the promise is about: a maximised window's frame is the zoomed overhang and can
  // never show the place it will restore to.
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [StructLayout(LayoutKind.Sequential)] public struct WINDOWPLACEMENT { public int length; public int flags; public int showCmd; public POINT ptMinPosition; public POINT ptMaxPosition; public RECT rcNormalPosition; public RECT rcReserved; }
  [DllImport("user32.dll")] public static extern bool GetWindowPlacement(IntPtr h, ref WINDOWPLACEMENT wp);
  // S9b ITEM 5: THE KEY AND POINTER DOORS, and the thread doors that decide whether using
  // them is honest. keybd_event and mouse_event inject at the OS level, so a press walks
  // the same road a user's takes - Slint's capture-key-pressed scope, the route table, the
  // menu handler, the Command, the engine's autosave bit - and nothing is SendMessage'd
  // into the app. MapVirtualKeyW supplies the REAL scancode: a key injected with scan 0
  // was measured reaching the chord scope and never the caret.
  // AttachThreadInput is the FOREGROUND LOCK, broken the way the chord-wiring E2E broke
  // it: attach this thread to the target's AND the current foreground thread to the
  // target's, and only then ask for the foreground. Bare SetForegroundWindow can be
  // refused, and the keys then land nowhere near the app.
  // GetForegroundWindow is the SAFETY gate, not decoration: a keystroke goes to whatever
  // is focused, so no key is sent unless the tested window IS the foreground window.
  [DllImport("user32.dll")] public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
  [DllImport("user32.dll")] public static extern uint MapVirtualKeyW(uint uCode, uint uMapType);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool mix);
  [DllImport("user32.dll")] public static extern bool DetachThreadInput(uint a, uint b);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  // GetCursorPos is the LIVENESS read, added the day this lane's mouse stopped arriving and
  // nothing could say WHY. SetCursorPos answers a bool and nothing else, and a locked
  // session - or a desktop this process does not belong to - can answer TRUE and move
  // nothing at all. Reading the pointer back is the only way a leg can tell "this window is
  // deaf" from "the OS never moved my cursor", and it buys an explanation, never a verdict:
  // the reading is advisory in BOTH directions.
  [DllImport("user32.dll")] public static extern bool GetCursorPos(ref POINT p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  // THE MENU LEGS' DOORS. A click is placed by the CLIENT area, not the frame: Chrome is
  // drawn inside the window (main.slint's WindowLayout::None), so the bar's own (0,0) is
  // the client's (0,0) and GetWindowRect's top-left is the shadow edge. GetDpiForWindow
  // is the logical-to-physical step, because every coordinate in chrome.slint is a LOGICAL
  // px and the cursor is moved in physical ones. WindowFromPoint is asked BEFORE the press
  // is made - the OS delivers a click to whatever is under the cursor, so "the menu did not
  // answer" and "we pressed on somebody else's window" must be tellable apart, and they are
  // only tellable apart if the press says who was under it.
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
  // THE DIALOG's tree. A modal file picker is a second top-level of OUR pid, and the only
  // way to see one is to walk the desktop: EnumWindows + GetClassNameW + the owner pid.
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, System.Text.StringBuilder s, int max);
  [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
}
'@
$win32 = [bool](Add-Type -TypeDefinition $code -PassThru)
"WIN32=$([int]$win32)"
$interactive = [bool][Environment]::UserInteractive
"INTERACTIVE=$([int]$interactive)"
$windowed = (Get-Process | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1)
"WINDOWED=$([int][bool]($null -ne $windowed))"
if (-not ($win32 -and $interactive -and ($null -ne $windowed))) { 'DESKTOP=0'; 'PROBE_DONE=1'; exit 0 }
'DESKTOP=1'
$sw = [Diagnostics.Stopwatch]::StartNew()
$p = Start-Process -FilePath $Exe -PassThru -RedirectStandardOutput $OutFile -RedirectStandardError $ErrFile
if ($null -eq $p) { 'SPAWN=0'; 'PROBE_DONE=1'; exit 0 }
'SPAWN=1'
"PID=$($p.Id)"
$deadline = (Get-Date).AddSeconds($WindowSecs)
$handle = [IntPtr]::zero
while ((Get-Date) -lt $deadline) {
    $p.Refresh()
    if ($p.MainWindowHandle -ne 0) { $handle = $p.MainWindowHandle; break }
    if ($p.HasExited) { break }
    Start-Sleep -Milliseconds 100
}
"LAUNCH_MS=$($sw.ElapsedMilliseconds)"
"HANDLE=$([int64]$handle)"
"TITLE=$($p.MainWindowTitle)"
# THE ALIVE WINDOW: sit on the process until AliveSecs have passed since the launch,
# polling for the one thing that would end it early. A product that quits in the middle
# of this has failed the claim the schedule can't reach, and it is not re-launched.
while ((-not $p.HasExited) -and ($sw.Elapsed.TotalSeconds -lt $AliveSecs)) {
    Start-Sleep -Milliseconds 250
    $p.Refresh()
}
"ALIVE_MS=$($sw.ElapsedMilliseconds)"
"ALIVE=$([int][bool]((-not $p.HasExited) -and ([int64]$p.MainWindowHandle -ne 0)))"
if ($p.HasExited) { "EXIT_CODE_EARLY=$($p.ExitCode)" } else { 'EXIT_CODE_EARLY=' }
# ---- S8: THE GEOMETRY FIXED POINT, RUN ON PRODUCT BYTES -----------------------------
# Two rules this phase is built out of, both earned elsewhere in this file: read
# rcNormalPosition and NOT GetWindowRect (a maximised window's frame is the zoomed
# overhang and can never show the place it restores to), and read a SETTLED value and
# not one sample (a rect that is still moving is not a fact about anything).
function Get-Handle($proc) {
    # Re-sighted on EVERY sample, and never the handle cached at sighting: the app may
    # destroy the transient it first showed and keep a different top-level as its main, so
    # a cached handle answers GetWindowPlacement about a window that is gone. The first
    # live run of this leg was taught that the expensive way - it read 0,0,16,16 with
    # showCmd 1, and called it a product that forgot its maximise, about a window the
    # product itself reported as zoomed. GEOMETRY_PROBE's comment states the same rule for
    # the needle lane ("sighting the title before the state, and not the handle it was
    # sighted on"); this is that rule, ported, earned again in the worst fashion.
    $proc.Refresh()
    $h = $proc.MainWindowHandle
    if ([int64]$h -eq 0) { return [IntPtr]::zero }
    return $h
}
function Get-Placement($h) {
    $wp = New-Object PROD+WINDOWPLACEMENT
    $wp.length = [Runtime.InteropServices.Marshal]::SizeOf($wp)
    if (-not [PROD]::GetWindowPlacement($h, [ref]$wp)) { return '' }
    $r = $wp.rcNormalPosition
    # The show state rides along in the same string, so the stability test below covers
    # it too: a rect that has stopped moving while the show state is still flipping has
    # not settled either.
    return "$($r.Left),$($r.Top),$($r.Right),$($r.Bottom)|$($wp.showCmd)"
}
function Get-Settled($h, $ceiling) {
    $prev = Get-Placement $h
    if ($prev -eq '') { return '|-1|0' }
    $sw2 = [Diagnostics.Stopwatch]::StartNew()
    $stable = 0
    while ($sw2.ElapsedMilliseconds -lt $ceiling) {
        Start-Sleep -Milliseconds 100
        $next = Get-Placement $h
        if ($next -eq '') { break }
        if ($next -ne $prev) { $prev = $next } else { $stable = 1; break }
    }
    return "$prev|$stable"
}
# ---- THE POINTER arithmetic every menu leg shares --------------------------------
# Every number below is chrome.slint's, in chrome.slint's units (LOGICAL px), named at
# its source: Theme.bar-height 28, slot-width 60, icon-size 24, menu-width 190,
# menu-inset 4, menu-pad 4, menu-gap 2, menu-row-height 20; Chrome's popup-x rule
# max(8, min(menu-inset, host-width - menu-width - 8)) at :172, the popup hung at
# y = parent.height (:736) and the rows grid inset by one pad on each axis (:756), with
# row i starting at i*(row-height + gap). A stale constant here clicks on the wrong pixel,
# ONE ASSUMPTION IS LOADED HERE AND SAID OUT LOUD: every row coordinate below is measured from
# the popup hung UNDER the bar, which is what Chrome does while the popup fits. chrome.slint
# has since grown a popup-top() rule that lifts it above the bar when it does not, and these
# legs do not follow it - at 800x600 the 6-row menu fits comfortably, and a window small
# enough to flip the popup would show up as a MISS WITH A PIXEL BESIDE IT rather than a
# pass, which is the only failure shape a coordinate like this is allowed to have.
# and the leg PRINTS THE PIXEL IT CLICKED and WHO WAS UNDER IT, so the failure mode is a
# number in the log and not a silent "the product ignored the menu".
$BAR = 28.0; $SLOT = 60.0; $ICON = 24.0; $MENUW = 190.0; $MINSET = 4.0; $PAD = 4.0
$GAP = 2.0; $ROWH = 20.0
# The hamburger's centre: the left slot is a centred HorizontalLayout of two icon-size
# cells, so the FIRST cell runs from (slot - 2*icon)/2 = 6 to 30 and its centre is 18 -
# NOT slot/2 = 30, which is the boundary between the hamburger and the pin. Said because
# the boundary is exactly the kind of pixel that "looks like it works" on one DPI and
# presses the pin on another.
$HAMBURGER_X = ($SLOT - 2 * $ICON) / 2 + $ICON / 2
$BAR_MID_Y = $BAR / 2
function Get-RowY([int]$i) { return $script:BAR + $script:PAD + $i * ($script:ROWH + $script:GAP) + $script:ROWH / 2 }
function Get-Client($h) {
    $rc = New-Object PROD+RECT
    if (-not [PROD]::GetClientRect($h, [ref]$rc)) { return $null }
    $pt = New-Object PROD+POINT
    if (-not [PROD]::ClientToScreen($h, [ref]$pt)) { return $null }
    $dpi = [PROD]::GetDpiForWindow($h)
    $s = 1.0
    if ($dpi -gt 0) { $s = $dpi / 96.0 }
    return @{ ox = [int]$pt.X; oy = [int]$pt.Y; lw = [double](($rc.Right - $rc.Left) / $s); s = $s }
}
function Get-PopupX($h) {
    $c = Get-Client $h
    if ($null -eq $c) { return -1.0 }
    return [Math]::Max(8.0, [Math]::Min($MINSET, $c.lw - $MENUW - 8.0))
}
# One press, from the LOGICAL point the markup says the row occupies, delivered through
# the OS the way a hand delivers it: move the cursor, press, release. Returns ONE string of
# six fields: the pixel it asked for, whether our window was the thing under that pixel, the
# handle that was, and where GetCursorPos says the pointer ACTUALLY sat when the button went
# down. That last pair is the sharpening - an asked-for pixel proves only what the harness
# wanted, and on a machine where injected input never arrives it is the LANDED pixel that
# says whether the press was ever a press at all.
function Click-Logical($h, $lx, $ly) {
    $c = Get-Client $h
    if ($null -eq $c) { return 'none,0,0,0,-1,-1' }
    $px = [int]($c.ox + $lx * $c.s); $py = [int]($c.oy + $ly * $c.s)
    $probe = New-Object PROD+POINT; $probe.X = $px; $probe.Y = $py
    $under = [PROD]::WindowFromPoint($probe)
    $ours = ([int64]$under -eq [int64]$h)
    # -1,-1 is not a position and never will be: this branch never asked the OS to move the
    # cursor, so it has no landing to report. A missing landing reads as "nothing was said",
    # never as the damning "the cursor did not go where it was told".
    if (-not $ours) { return "$px,$py,0,$([int64]$under),-1,-1" }
    [void][PROD]::SetCursorPos($px, $py)
    Start-Sleep -Milliseconds 180
    # WHERE THE POINTER ACTUALLY IS - read after the settle, before the button. A press
    # delivered while the cursor sits somewhere else is not the press this leg claims.
    $land = New-Object PROD+POINT
    if ([PROD]::GetCursorPos([ref]$land)) { $ax = [int]$land.X; $ay = [int]$land.Y } else { $ax = -1; $ay = -1 }
    [PROD]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 70
    [PROD]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 420
    return "$px,$py,1,$([int64]$under),$ax,$ay"
}
# A title-band drag, the same door a hand uses: press, MOVE IN STEPS, release. The steps
# matter - Slint's TouchArea has no 'dragged' callback, so a drag is its 'moved' handler
# guarded by the pressed bit (chrome.slint:556), and a single jump from A to B delivers one
# 'moved' at most. Eight steps of 5 logical px is what the band's own delta arithmetic
# wants, and the window really does travel: every later click re-reads the client origin,
# because a coordinate cached before the drag is a coordinate about a window that moved.
function Drag-Band($h, $lx, $ly, $dx) {
    $c = Get-Client $h
    if ($null -eq $c) { return 'none,0' }
    $px = [int]($c.ox + $lx * $c.s); $py = [int]($c.oy + $ly * $c.s)
    $probe = New-Object PROD+POINT; $probe.X = $px; $probe.Y = $py
    $under = [PROD]::WindowFromPoint($probe)
    if ([int64]$under -ne [int64]$h) { return "$px,$py,0" }
    [void][PROD]::SetCursorPos($px, $py)
    Start-Sleep -Milliseconds 180
    [PROD]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 90
    for ($i = 1; $i -le 8; $i++) {
        [void][PROD]::SetCursorPos([int]($px + $i * $dx * $c.s / 8), $py)
        Start-Sleep -Milliseconds 30
    }
    Start-Sleep -Milliseconds 90
    [PROD]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 420
    return "$px,$py,1"
}
# The foreground lock, lifted from the mode-3 recipe VERBATIM (attach self->target and
# foreground->target with the ids from GetWindowThreadProcessId's RETURN value, then ask).
# Left in mode 3 as it shipped rather than refactored under a proven leg: mode 3 is green
# and paid for; these legs are new, and a shared helper is the right call for the NEXT
# slice, not for one running out of clock.
function Set-Active($h) {
    $cur = [PROD]::GetCurrentThreadId()
    $fgH = [PROD]::GetForegroundWindow()
    $a = [uint32]0; $b = [uint32]0
    $fgTid = [PROD]::GetWindowThreadProcessId($fgH, [ref]$a)
    $tTid = [PROD]::GetWindowThreadProcessId($h, [ref]$b)
    $att1 = [PROD]::AttachThreadInput($cur, $tTid, $true)
    $att2 = $false
    if ($fgTid -ne 0 -and $fgTid -ne $tTid) { $att2 = [PROD]::AttachThreadInput($fgTid, $tTid, $true) }
    [void][PROD]::SetForegroundWindow($h)
    Start-Sleep -Milliseconds 500
    $ok = ([int64][PROD]::GetForegroundWindow() -eq [int64]$h)
    if ($att1) { [void][PROD]::DetachThreadInput($cur, $tTid) }
    if ($att2) { [void][PROD]::DetachThreadInput($fgTid, $tTid) }
    return [int][bool]$ok
}
# Any visible #32770 belonging to a pid - THE DIALOG, if there is one. Walked rather than
# waited-for, because the only honest form of "the modal appeared" is a handle and a class
# name read off the desktop after the ask.
function Get-Dialog([int64]$wantPid) {
    # The delegate is a compiled action of its own: it sees SCRIPT scope, not this
    # function's locals, so the pid travels in a $script: variable. And the pid is checked
    # at all because a #32770 belonging to somebody else's explorer is not our dialog - a
    # proof that counted THAT would be a proof of nothing.
    $script:wantPid = $wantPid
    $script:dlg = [IntPtr]::Zero
    $cb = [PROD+EnumProc] {
        param($h, $l)
        $sb = New-Object System.Text.StringBuilder 256
        [void][PROD]::GetClassNameW($h, $sb, 256)
        if ($sb.ToString() -eq '#32770' -and [PROD]::IsWindowVisible($h)) {
            $pid2 = [uint32]0
            [void][PROD]::GetWindowThreadProcessId($h, [ref]$pid2)
            if ([int64]$pid2 -eq $script:wantPid) { $script:dlg = $h; return $false }
        }
        return $true
    }
    [void][PROD]::EnumWindows($cb, [IntPtr]::Zero)
    return [int64]$script:dlg
}
# From here every read goes through Get-Handle, so the cached $handle is only ever the
# "did a window appear at all" answer the ALIVE line above is about.
$handle = Get-Handle $p
if ($Maximise -eq 3) {
  # ---- S9b ITEM 5: THE AUTOSAVE TOGGLE, DRIVEN BY THE REAL CHORD ---------------------
  # Three acts, in the order a live run proved. (1) THE SETTLED RECT, then a click inside
  # it: Slint installs the caret's focus item on a POINTER PRESS, while a chord rides the
  # outermost scope the app already focuses itself - so without this click a chord lands
  # and a letter does not. Measured, not theorised. The rect is POLLED because the first
  # sample can answer 0,0,16,16 for a window that is really there. (2) THE FOREGROUND
  # LOCK, broken by AttachThreadInput TWICE with the ids read from
  # GetWindowThreadProcessId's RETURN value, and only then SetForegroundWindow. (3)
  # keybd_event with a REAL scancode from MapVirtualKeyW. The instrument is the product's
  # own scratch draft, resolved by the portable rule and never by a guessed %APPDATA%.
  # The state dir, by the app's OWN rule (core paths.rs, mirrored by app_state_dir): a
  # data/ directory beside the exe wins, whatever %APPDATA% says. Watching the roaming
  # profile while the portable marker sent the process next to the exe is the 2026-09-13
  # dossier's mistake, and this leg refuses to make it twice.
  $exeDir = Split-Path -Parent $Exe
  if (Test-Path (Join-Path $exeDir 'data')) { $stateDir = Join-Path $exeDir 'data' }
  else { $stateDir = Join-Path $env:APPDATA 'notes-gpui' }
  $draft = Join-Path (Join-Path $stateDir 'notes') 'untitled.notes'
  "DRAFTPATH=$draft"
  function Get-DraftText { if (-not (Test-Path $draft)) { return '' }; try { return [IO.File]::ReadAllText($draft) } catch { return '' } }
  function Report($tag) {
    $t = Get-DraftText
    # One key per line: parse_probe splits at the first '=' and keeps the rest as the
    # value, so a four-pair line is ONE unreadable value, not four readings.
    Write-Output ($tag + '_BYTES=' + $t.Length)
    Write-Output ($tag + '_HAS_ALPHA=' + [int][bool]$t.Contains('alpha1'))
    Write-Output ($tag + '_HAS_BETA=' + [int][bool]$t.Contains('beta2'))
    Write-Output ($tag + '_HAS_CHARLIE=' + [int][bool]$t.Contains('charlie3'))
  }
  function Send-Key([int]$vk, [bool]$up) {
    $scan = [PROD]::MapVirtualKeyW([uint32]$vk, 0)
    $flags = 0
    if ($up) { $flags = 2 }
    [PROD]::keybd_event([byte]$vk, [byte]$scan, [uint32]$flags, [UIntPtr]::Zero)
  }
  function Send-Chord {
    Send-Key 17 $false; Start-Sleep -Milliseconds 40
    Send-Key 84 $false; Start-Sleep -Milliseconds 40
    Send-Key 84 $true; Start-Sleep -Milliseconds 40
    Send-Key 17 $true; Start-Sleep -Milliseconds 150
  }
  function Send-Text($s) {
    foreach ($c in $s.ToCharArray()) {
      $vk = 0
      if ($c -match '[A-Za-z]') { $vk = [int][char]($c.ToString().ToUpper()) }
      elseif ($c -match '[0-9]') { $vk = [int][char]$c }
      elseif ($c -eq ' ') { $vk = 32 }
      if ($vk -eq 0) { continue }
      Send-Key $vk $false; Start-Sleep -Milliseconds 35
      Send-Key $vk $true; Start-Sleep -Milliseconds 35
    }
  }
  # ACT 1 - the rect, re-sighted on every sample and only accepted once it is a window.
  $rc = New-Object PROD+RECT
  $cx = 0; $cy = 0; $tries = 0
  while ($tries -lt 20) {
    $p.Refresh()
    $hh = $p.MainWindowHandle
    [void][PROD]::GetWindowRect($hh, [ref]$rc)
    $w = $rc.Right - $rc.Left; $ht = $rc.Bottom - $rc.Top
    if ($w -gt 200 -and $ht -gt 200) { $cx = [int]($rc.Left + $w/2); $cy = [int]($rc.Top + ($ht*7/10)); break }
    $tries++; Start-Sleep -Milliseconds 250
  }
  Write-Output "RECT_TRIES=$tries"
  Write-Output "RECT=$($rc.Left),$($rc.Top),$($rc.Right),$($rc.Bottom)"
  "CLICK_AT=$cx,$cy"
  # ACT 2 - the lock. The ids come from the RETURN value; the out parameter is the PROCESS
  # id, and attaching to a pid is a silent no-op that costs the whole leg its keys.
  # Re-sighted, never the handle cached at sighting - the rule this file already states
  # for every other read, and the one place a mode-3 leg can silently attach to nothing.
  $h3 = Get-Handle $p
  Write-Output "TOGGLE_HANDLE=$([int64]$h3)"
  $curTid = [PROD]::GetCurrentThreadId()
  $fgH = [PROD]::GetForegroundWindow()
  $pidOut = [uint32]0
  $fgTid = [PROD]::GetWindowThreadProcessId($fgH, [ref]$pidOut)
  $pidOut2 = [uint32]0
  $tTid = [PROD]::GetWindowThreadProcessId($h3, [ref]$pidOut2)
  $att1 = [PROD]::AttachThreadInput($curTid, $tTid, $true)
  $att2 = $false
  if ($fgTid -ne 0 -and $fgTid -ne $tTid) { $att2 = [PROD]::AttachThreadInput($fgTid, $tTid, $true) }
  $sfw = [PROD]::SetForegroundWindow($h3)
  Start-Sleep -Milliseconds 600
  $fgOk = ([int64][PROD]::GetForegroundWindow() -eq [int64]$h3)
  Write-Output "ATTACH_cur=$([int][bool]$att1)"
  Write-Output "ATTACH_fg=$([int][bool]$att2)"
  Write-Output "SFW=$([int][bool]$sfw)"
  Write-Output "FOREGROUND=$([int]$fgOk)"
  if ($fgOk) {
    [void][PROD]::SetCursorPos($cx, $cy)
    Start-Sleep -Milliseconds 300
    [PROD]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 80
    [PROD]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 500
    'CLICKED=1'
    Send-Text 'alpha1'
    Start-Sleep -Milliseconds 2500
    Report 'A1'
    # THE BASELINE, read BEFORE anything is claimed about the toggle: alpha1 with autosave
    # ON must reach the disk by itself. If it does not, letters never fed the caret and the
    # toggle was never tested - which is a fact about this instrument, not a verdict on the
    # app, and it is reported as NOT JUDGED.
    $baseline = [int][bool]((Get-DraftText).Contains('alpha1'))
    "BASELINE_LANDED=$baseline"
    if ($baseline -eq 1) {
      # ACT 3 - the chord OFF (settings default arms autosave, so THIS is the disarm) and a
      # NEW edit. beta2 must not reach the draft.
      Send-Chord
      Start-Sleep -Milliseconds 700
      Send-Text 'beta2'
      Start-Sleep -Milliseconds 2500
      Report 'A2'
      # ACT 4 - the chord back ON and another new edit: a re-armed autosave needs a fresh
      # change to have anything to write, and the buffer then carries both pending edits.
      Send-Chord
      Start-Sleep -Milliseconds 700
      Send-Text 'charlie3'
      Start-Sleep -Milliseconds 2500
      Report 'A3'
    }
    if ($att1) { [void][PROD]::DetachThreadInput($curTid, $tTid) }
    if ($att2) { [void][PROD]::DetachThreadInput($fgTid, $tTid) }
  } else {
    'CLICKED=0'; 'BASELINE_LANDED=0'
    Report 'A2'; Report 'A3'
  }
  'NORMAL='; 'NORMAL_STABLE=0'; 'NORMAL_SHOWCMD=-1'; 'MAX_ASKED=0'; 'SHOWCMD_AFTER_MAX=-1'
} elseif ([int64]$handle -ne 0 -and ($Maximise -eq 1 -or $Maximise -eq 2)) {
    # Mode 0 - the plain launch - takes NONE of this path at all, so the geometry phase
    // cannot add a single second to the runtime the 45s claim is timed against.
    $a = (Get-Settled (Get-Handle $p) $SettleMs).Split('|')
    "NORMAL=$($a[0])"
    "NORMAL_SHOWCMD=$($a[1])"
    "NORMAL_STABLE=$($a[2])"
    if ($Maximise -eq 1) {
        # THE ACT, through the product's own door for the caption button and the
        # title-band double-click (surface.rs on_toggle_max -> window.set_maximized):
        # ShowWindow(SW_MAXIMIZE) asks the OS to do what that callback asks the toolkit
        # to do, and winit reports the result. It is a user act in every way that the
        # harness is allowed to make one: no SendKeys, no message into the app, no call
        # across the bridge. Maximise=0 is the RELAUNCH leg, where nothing is driven -
        # the claim there is that the app zooms ITSELF from what it persisted.
        $asked = [PROD]::ShowWindow((Get-Handle $p), 3)
        "MAX_ASKED=$([int][bool]$asked)"
        $sw3 = [Diagnostics.Stopwatch]::StartNew()
        $landed = -1
        while ($sw3.ElapsedMilliseconds -lt $SettleMs) {
            $q = Get-Placement (Get-Handle $p)
            if ($q -ne '') { $landed = [int](($q.Split('|'))[1]); if ($landed -eq 3) { break } }
            Start-Sleep -Milliseconds 100
        }
        # The wait is printed, not trusted: "showCmd was 3 at once" and "showCmd was 3
        # after 1.4s of the settle watch running" are the same verdict and different
        # stories, and only the number separates them.
        "MAX_LAND_MS=$($sw3.ElapsedMilliseconds)"
        "SHOWCMD_AFTER_MAX=$landed"
        $b = (Get-Settled (Get-Handle $p) $SettleMs).Split('|')
        "NORMAL_WHILE_MAX=$($b[0])"
        "NORMAL_WHILE_MAX_SHOWCMD=$($b[1])"
        "NORMAL_WHILE_MAX_STABLE=$($b[2])"
        "ZOOM_AFTER_MAX=$([int][bool]([PROD]::IsZoomed((Get-Handle $p))))"
        # One grace tick so the app's OWN settle watch (250 ms quiet / 1 s force in
        # product.rs) measures the zoomed window and tells the port BEFORE the close is
        # asked. Without it the maximised bit could only ever reach disk through the
        # terminal flush, and the run would be proving the shutdown write path while
        # claiming to prove the mid-session one.
        Start-Sleep -Milliseconds 1200
    } else {
        # MODE 2, THE RELAUNCH: nothing is driven. The only question is whether the app
        # came back zoomed by itself, and where its restore rect is now. The show state
        # is POLLED for up to one settle ceiling before it is read, because the bridge
        # applies maximised from the session on a wake rather than inside the call that
        # created the window - a single sample at the sighting tick measures when this
        # harness looked, not what the product did. That is the needle leg's own stated
        # weakness, and this leg refuses to inherit it.
        $z = [Diagnostics.Stopwatch]::StartNew()
        $seen = -1
        while ($z.ElapsedMilliseconds -lt $SettleMs) {
            $q = Get-Placement (Get-Handle $p)
            if ($q -ne '') { $seen = [int](($q.Split('|'))[1]); if ($seen -eq 3) { break } }
            Start-Sleep -Milliseconds 100
        }
        "SHOWCMD_AT_CREATE=$seen"
        "ZOOM_AT_CREATE=$([int][bool]([PROD]::IsZoomed((Get-Handle $p))))"
        "CREATE_LAND_MS=$($z.ElapsedMilliseconds)"
        $c = (Get-Settled (Get-Handle $p) $SettleMs).Split('|')
        "NORMAL_RELAUNCH=$($c[0])"
        "NORMAL_RELAUNCH_STABLE=$($c[2])"
    }
}
# MODE 4 - LEG A: THE MENU ANSWERS A REAL CLICK. Two presses, both placed by the
# markup's own arithmetic, and this script decides NOTHING: it reports who was under the
# cursor and where the pixel was, and the machine-visible truth of the act is the line the
# PRODUCT prints on its own stderr ("menu: autosave-row toggled true->false" for the row,
# "dialog[skipped]: SLINT_NO_DIALOG - ... (Open -> ...)" for the Open row). The toggle's
# other half - the autosave actually holding a letter back - is what the chord leg proved
# at 1bf46326; what is proved here is that the SAME code path is reached by a POINTER,
# which is the claim nobody has made until now.
elseif ($Maximise -eq 4) {
    $h4 = Get-Handle $p
    "ACTIVE=$([int](Set-Active $h4))"
    $r = Click-Logical (Get-Handle $p) $HAMBURGER_X $BAR_MID_Y 'HAMBURGER'
    "HAMBURGER_AT=$r"
    Start-Sleep -Milliseconds 350
    $r2 = Click-Logical (Get-Handle $p) ((Get-PopupX (Get-Handle $p)) + $PAD + 40.0) (Get-RowY 2) 'ROW'
    "ROW_AT=$r2"
    Start-Sleep -Milliseconds 600
    "ROW_HANDLE=$([int64](Get-Handle $p))"
}
# MODE 5 - LEG B, THE REFUSAL PROOF. Same two presses, the Open row instead of the
# Auto-save row, and SLINT_NO_DIALOG set by the harness so no modal is in the way. What
# the refusal buys is the ONLY cheap proof that the row's geometry is right: the app
# reached Rust, decided not to show a modal, and said so out loud. A wrong pixel cannot
# produce that line.
elseif ($Maximise -eq 5) {
    $h5 = Get-Handle $p
    "ACTIVE=$([int](Set-Active $h5))"
    "HAMBURGER_AT=$(Click-Logical (Get-Handle $p) $HAMBURGER_X $BAR_MID_Y 'HAMBURGER')"
    Start-Sleep -Milliseconds 350
    "ROW_AT=$(Click-Logical (Get-Handle $p) ((Get-PopupX (Get-Handle $p)) + $PAD + 40.0) (Get-RowY 0) 'ROW')"
    Start-Sleep -Milliseconds 800
}
# MODE 6 - ITEM 4, THE NATIVE DIALOG ON CAMERA. Identical press, and NOTHING standing it
# down: the harness removes SLINT_NO_DIALOG for this mode, so the ask reaches rfd and a
# real #32770 appears as a second top-level of our pid. It is looked for by walking the
# desktop for two seconds, then dismissed with WM_CLOSE - the same message the main window
# gets at the end of every launch, sent to the dialog instead - and both the dismissal and
# the survival of the MAIN window are printed. Until this mode ran, no machine had ever
# asserted that the native dialog flows of this product work; ADR-0006's item 4 is that
# absence, and this is the first attempt at filling it.
elseif ($Maximise -eq 6) {
    $h6 = Get-Handle $p
    "ACTIVE=$([int](Set-Active $h6))"
    "HAMBURGER_AT=$(Click-Logical (Get-Handle $p) $HAMBURGER_X $BAR_MID_Y 'HAMBURGER')"
    Start-Sleep -Milliseconds 350
    "ROW_AT=$(Click-Logical (Get-Handle $p) ((Get-PopupX (Get-Handle $p)) + $PAD + 40.0) (Get-RowY 0) 'ROW')"
    # TWO SECONDS, polled: the dialog is spawned on its own thread (surface.rs's
    # ask_dialog, because the loop may not block), so the ask and the window are separate
    # events in time and a single sample after the click measures the harness, not the app.
    $zd = [Diagnostics.Stopwatch]::StartNew()
    $dlg = [int64]0
    while ($zd.ElapsedMilliseconds -lt 2000) {
        $dlg = Get-Dialog ([int64]$p.Id)
        if ($dlg -ne 0) { break }
        Start-Sleep -Milliseconds 100
    }
    "DIALOG_HANDLE=$dlg"
    "DIALOG_SEEN=$([int][bool]($dlg -ne 0))"
    "DIALOG_MS=$($zd.ElapsedMilliseconds)"
    if ($dlg -ne 0) {
        [void][PROD]::SendMessageW([IntPtr]$dlg, 16, [IntPtr]::Zero, [IntPtr]::Zero)
        $zg = [Diagnostics.Stopwatch]::StartNew()
        $gone = $false
        while ($zg.ElapsedMilliseconds -lt 3000) {
            if ((Get-Dialog ([int64]$p.Id)) -eq 0) { $gone = $true; break }
            Start-Sleep -Milliseconds 100
        }
        "DIALOG_GONE=$([int][bool]$gone)"
        "DIALOG_GONE_MS=$($zg.ElapsedMilliseconds)"
    } else {
        'DIALOG_GONE=0'; 'DIALOG_GONE_MS=-1'
    }
    $p.Refresh()
    "MAIN_ALIVE=$([int][bool]((-not $p.HasExited) -and [PROD]::IsWindow((Get-Handle $p))))"
}
# MODE 7 - LEG C: THE DRAG CLOSES THE POPUP (b76c277c's promise, first machine proof).
# The shape, and why THIS one: the bit that closes is Chrome's and Rust is forbidden from
# touching it (surface.rs forbids set_menu_open, and the guard test says so), so there is
# NO line for "the popup closed" to count. What there IS, is the ask. So the gesture is
# driven, the Open-row pixel is pressed AGAIN without reopening anything, and a SECOND
# full cycle hamburger-then-row is driven after it. Three outcomes and all three mean
# something: exactly one ask = the drag closed it AND the click path works; two asks =
# the drag left the popup open, which is the promise broken; zero asks = the row pixel is
# wrong, which is an instrument finding and is reported as one.
elseif ($Maximise -eq 7) {
    $h7 = Get-Handle $p
    "ACTIVE=$([int](Set-Active $h7))"
    "HAMBURGER_AT=$(Click-Logical (Get-Handle $p) $HAMBURGER_X $BAR_MID_Y 'HAMBURGER')"
    Start-Sleep -Milliseconds 350
    # The band, one slot in and 40 px right: 021bf7d0 made the band END where the caption
    # begins, so the press has to start clear of both the left slot and the caption cells -
    # slot-width is the markup's own left edge for the band (chrome.slint:537).
    "BAND_DRAG=$(Drag-Band (Get-Handle $p) ($SLOT + 24.0) $BAR_MID_Y 40.0)"
    Start-Sleep -Milliseconds 500
    # PRESS 1 AT THE ROW, with no hamburger before it. If the drag closed the popup this
    # press lands on the editor and asks for nothing.
    "ROW_AT=$(Click-Logical (Get-Handle $p) ((Get-PopupX (Get-Handle $p)) + $PAD + 40.0) (Get-RowY 0) 'ROW')"
    Start-Sleep -Milliseconds 400
    # THEN THE SAME PIXELS AGAIN, OPENED PROPERLY: this one must ask. Without this second
    # half, "no ask" would not distinguish a closed popup from dead coordinates.
    "HAMBURGER2_AT=$(Click-Logical (Get-Handle $p) $HAMBURGER_X $BAR_MID_Y 'HAMBURGER2')"
    Start-Sleep -Milliseconds 350
    "ROW2_AT=$(Click-Logical (Get-Handle $p) ((Get-PopupX (Get-Handle $p)) + $PAD + 40.0) (Get-RowY 0) 'ROW2')"
    Start-Sleep -Milliseconds 600
}
else { 'NORMAL='; 'NORMAL_STABLE=0'; 'NORMAL_SHOWCMD=-1'; 'MAX_ASKED=0'; 'SHOWCMD_AFTER_MAX=-1' }
# THE CLOSE: WM_CLOSE through CloseMainWindow, the same door PROBE uses. Nothing here
# kills the app unless it refused to leave, and a kill is reported, not hidden.
$closed = $false
$visible = $false
if (-not $p.HasExited) {
    $p.Refresh()
    if ([int64]$p.MainWindowHandle -ne 0) { $visible = [PROD]::IsWindowVisible($p.MainWindowHandle) }
    $closed = $p.CloseMainWindow()
}
"VISIBLE_AT_CLOSE=$([int][bool]$visible)"
"CLOSE_REQUESTED=$([int][bool]$closed)"
$exited = $false
if (-not $p.HasExited) { $exited = $p.WaitForExit($CloseSecs * 1000) }
"EXITED_WITHOUT_KILL=$([int][bool]$exited)"
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

/// How long the toggle launch sits on screen before it is driven: long enough that the
/// app has claimed its focus item (a chord dies without one) and the caret click has
/// landed. NOT [PRODUCT_ALIVE_SECS] - the 45s claim is one claim about the shipped app,
/// and the plain launch makes it exactly once.
const PRODUCT_TOGGLE_ALIVE_SECS: u64 = 5;
/// How long a MENU leg sits on screen before its presses. Shorter than the toggle's five
/// and far shorter than the plain launch's forty-five: these legs wait on nothing the app
/// has to do on its own - the act is theirs, and they start it as soon as the window is
/// settled - so the only thing a longer sit buys is a slower bare smoke.
const PRODUCT_MENU_ALIVE_SECS: u64 = 3;

/// What the autosave round trip said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Toggle {
    /// Ctrl+T disarmed the autosave (the edit typed after it stayed out of the draft) and
    /// Ctrl+T armed it again (the edit after THAT landed). The switch did its job through
    /// the chord a user presses - not through a file the harness wrote.
    Held,
    /// A precondition went missing: no foreground window, no caret under the click, no
    /// probe answer. Nothing is claimed in either direction. Advisory, never a red.
    NotJudged(String),
    /// The chord was driven and the draft disagreed with it.
    Broken(String),
}

/// THE TOGGLE VERDICT, pure over its four readings, because a judge nobody can feed
/// cannot be tested without a desktop.
///
/// THE BASELINE IS WHAT MAKES THIS HONEST. A letter that never reached the disk with
/// AUTOSAVE ON says the keys never fed the caret - and then "nothing landed while it was
/// off" proves nothing about the switch. That line is the whole difference between NOT
/// JUDGED and a product bug, and it is the exact trap this leg fell into once: the chord
/// landed in both directions, the caret never did, and the run read as a broken toggle.
pub fn judge_toggle(
    foreground: bool,
    baseline_landed: bool,
    beta_while_off: bool,
    charlie_after_on: bool,
) -> Toggle {
    if !foreground {
        return Toggle::NotJudged(
            "the product window never became the foreground window, so NO key was sent: a keystroke goes to whatever is focused, and the harness will not press a chord into somebody elses editor".to_string(),
        );
    }
    if !baseline_landed {
        return Toggle::NotJudged(
            "a letter typed with autosave ON never reached the draft, so the caret was never fed and the switch was never actually tested - an instrument finding, not a verdict on the toggle".to_string(),
        );
    }
    if beta_while_off {
        return Toggle::Broken(
            "the chord did not disarm the autosave: the edit typed after it reached the draft anyway".to_string(),
        );
    }
    if !charlie_after_on {
        return Toggle::Broken(
            "the chord did not re-arm the autosave: the baseline landed, the edit typed while off was held, and the edit typed after the way back never landed".to_string(),
        );
    }
    Toggle::Held
}
/// THE AUTOSAVE TOGGLE, ROUND-TRIPPED THROUGH THE REAL CHORD (roadmap M2 item 5).
///
/// One launch in mode 3, the product's OWN scratch draft as the instrument, and no door
/// into the app that a user does not have. The recipe lives in the probe mode-3 block and
/// every move in it was paid for by a run that failed without it: the SETTLED rect (the
/// first sample can answer 0,0,16,16 about a window that is really there), the CLICK that
/// installs the caret, the DOUBLE AttachThreadInput with the thread ids read from
/// GetWindowThreadProcessId's RETURN value (its out parameter is a PROCESS id, and
/// attaching to a pid is a silent no-op that costs the leg every key), and the scancode
/// from MapVirtualKeyW. Bare SetForegroundWindow with scan-0 keys was measured landing a
/// chord on the outermost scope and never a letter on the caret.
///
/// TODO, for whichever slice decides this verdict deserves a CODE of its own:
/// smoke::CONTRACT has no row for a toggle, so a BROKEN one rides the existing step code
/// (a gap, exit 1) and an unmeasurable one prints NOT JUDGED and changes nothing. Minting
/// a code is deliberately NOT this slice's job, and the rule it would owe is the one
/// check-ci enforces: changing the contract means arming it in ci.yml IN THE SAME COMMIT,
/// because [arm-missing] turns an unarmed row into a red run, not a review comment.
fn product_autosave_toggle(script: &Path, exe: &Path, session: &Path) -> Toggle {
    // The draft, by the app's OWN rule - the same [app_state_dir] the geometry cycle reads
    // its session through, so a data/ directory beside the exe wins over %APPDATA%. That
    // is what keeps this leg from watching a roaming profile while the portable marker
    // sends the process next to the exe, which is the mistake the 2026-09-13 dossier
    // records. The candidate SEARCH list answers a different question and is not used here.
    let Some(state) = app_state_dir(exe) else {
        return Toggle::NotJudged("the app own state dir could not be resolved".to_string());
    };
    let draft = state.join("notes").join("untitled.notes");
    let before = fs::read(&draft).ok();
    println!(
        "smoke: autosave-toggle: armed (S9b) on {} - Ctrl+T, a typed edit, 2.5s, and the draft read between them",
        draft.display()
    );
    let probe = match run_product_probe(script, exe, session, PRODUCT_TOGGLE_ALIVE_SECS, 3) {
        Ok((probe, _)) => probe,
        Err(e) => {
            restore_draft(&draft, before.as_deref());
            return Toggle::NotJudged(format!("the toggle launch did not report: {e}"));
        }
    };
    let verdict = judge_toggle(
        probe.flag("FOREGROUND"),
        probe.number("BASELINE_LANDED") == Some(1),
        probe.number("A2_HAS_BETA") == Some(1),
        probe.number("A3_HAS_CHARLIE") == Some(1),
    );
    // The readings, printed whenever the launch answered at all. The byte counts ARE the
    // evidence, so they get their own line rather than being folded into a verdict that a
    // green could hide.
    println!(
        "smoke: autosave-toggle: draft bytes before={} after_alpha={} after_beta={} after_charlie={} | rect_tries={} click_at={} attach_cur={} attach_fg={} fg={}",
        before.as_ref().map(|b| b.len()).unwrap_or(usize::MAX),
        probe.number("A1_BYTES").unwrap_or(-1),
        probe.number("A2_BYTES").unwrap_or(-1),
        probe.number("A3_BYTES").unwrap_or(-1),
        probe.number("RECT_TRIES").unwrap_or(-1),
        probe
            .get("CLICK_AT")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string()),
        probe.flag("ATTACH_cur"),
        probe.flag("ATTACH_fg"),
        probe.flag("FOREGROUND")
    );
    // Do no harm, in both directions: the draft comes back byte-for-byte, or goes away if
    // it was not there. The typed marks were the test, never the product.
    restore_draft(&draft, before.as_deref());
    println!(
        "smoke: autosave-toggle: the draft is put back {}",
        match &before {
            Some(b) => format!("as the {} bytes it was found at", b.len()),
            None => "absent, which is how it was found".to_string(),
        }
    );
    verdict
}

/// Put the scratch draft back exactly as it was found, INCLUDING ABSENT - which is why a
/// file that was not there is removed rather than written as zero bytes.
fn restore_draft(path: &Path, bytes: Option<&[u8]>) {
    match bytes {
        Some(b) => {
            let _ = fs::write(path, b);
        }
        None => {
            let _ = fs::remove_file(path);
        }
    }
}
/// Same two-PowerShell fallback the first probe uses: a runner may ship either, and
/// neither is a dependency of this crate.
fn spawn_product_probe(
    script: &Path,
    exe: &Path,
    out: &Path,
    err: &Path,
    session: &Path,
    alive_secs: u64,
    // MODE: 0 = the plain launch, which reads nothing about geometry; 1 = the maximise
    // leg; 2 = the relaunch leg; 3 = the chord leg; 4, 5, 6 and 7 = the menu legs. An int
    // and not a bool, because one script answers EIGHT behaviours and a bool plus a second
    // flag is how the two start disagreeing.
    mode: u8,
) -> Result<Child, String> {
    // THE DIALOG GATE, decided by the MODE and nowhere else. plumbing.rs's rule is that the
    // PRESENCE of SLINT_NO_DIALOG stands the modal down and its value is irrelevant, so this
    // is env/env_remove and never "=0" - and it lives here, beside the spawn, because a leg
    // that relies on an environment set somewhere upstream of itself is proving whatever the
    // shell happened to hold.
    //
    // Modes 4, 5 and 7 press the Open row to say something about the ROW - its geometry, its
    // wiring, whether a drag closed the popup it is in - and cannot afford a modal in the
    // way. Mode 6 is the leg that WANTS the modal, so it removes the gate from the child's
    // environment whatever the parent's happened to be.
    let mut missing = Vec::new();
    for program in ["pwsh", "powershell"] {
        let mut cmd = Command::new(program);
        cmd.args([
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
        // The path the app persists to, handed to the script so the relaunch can be
        // asked about the SAME file the first launch wrote. Read-only: the script
        // never writes it, and the harness does not move it aside either - which is
        // the point of the leg, and is why it resolves the app's own directory
        // instead of guessing at one.
        .arg("-Session")
        .arg(session.as_os_str())
        .arg("-WindowSecs")
        .arg(WINDOW_SECS.to_string())
        .arg("-AliveSecs")
        .arg(alive_secs.to_string())
        .arg("-CloseSecs")
        .arg(CLOSE_SECS.to_string())
        .arg("-Maximise")
        .arg(mode.to_string())
        .arg("-SettleMs")
        .arg(SETTLE_MS.to_string());
        match mode {
            4 | 5 | 7 => {
                cmd.env("SLINT_NO_DIALOG", "1");
            }
            6 => {
                cmd.env_remove("SLINT_NO_DIALOG");
            }
            _ => {}
        }
        let spawned = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
        match spawned {
            Ok(child) => return Ok(child),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                missing.push(program.to_string());
            }
            Err(e) => return Err(format!("cannot run {program}: {e}")),
        }
    }
    Err(format!(
        "no PowerShell to run the product probe with (tried: {})",
        missing.join(", ")
    ))
}

/// One launch of the product probe, run to completion and parsed.
///
/// Split out of [run_product_leg] because the fixed point needs the SAME script twice
/// more - once to maximise and close, once to relaunch and close - and a second copy of
/// the spawn/wait/parse block is how two of them start disagreeing about the deadline
/// while looking identical in the log.
fn run_product_probe(
    script: &Path,
    exe: &Path,
    session: &Path,
    alive_secs: u64,
    mode: u8,
) -> Result<(Probe, String), String> {
    let out = temp_path("cycle-out", "txt");
    let err = temp_path("cycle-err", "txt");
    let junk = [out.clone(), err.clone()];
    let drop_junk = || {
        for p in junk.iter() {
            let _ = fs::remove_file(p);
        }
    };
    let mut child = spawn_product_probe(script, exe, &out, &err, session, alive_secs, mode)?;
    let probe = match wait_bounded(&mut child, PRODUCT_CYCLE_OUTER_SECS) {
        Err(e) => {
            drop_junk();
            return Err(e);
        }
        Ok(None) => {
            drop_junk();
            return Err("the product probe reported no exit status at all".to_string());
        }
        Ok(Some(_)) => parse_probe(&read_pipe(child.stdout.as_mut())),
    };
    // The launch's OWN stderr, handed back beside the readings. Every menu leg decides on
    // a line the product printed there, so a probe that threw the capture away would be an
    // instrument burning its own evidence.
    let trace = fs::read_to_string(&err).unwrap_or_default();
    // The relaunch writes its own stderr, and that capture is the one holding the
    // product's own line about what the port told it at startup. Printed, never judged
    // here: the startup-needle rule belongs to the plain leg, and this launch exists to
    // be looked at, not to re-decide a promise the first launch already made.
    if mode == 2 {
        let _ = report_captured(&err, "the relaunched product on its own stderr");
    }
    drop_junk();
    Ok((probe, trace))
}

/// What the app's own session.json says right now: its persisted rect and its maximised
/// bit, both absent-tolerant. A file that cannot be read is an ABSENT answer and never a
/// false - which is exactly the line between NOT JUDGED and exit 6.
fn session_state(path: &Path) -> (Option<Rect>, Option<bool>) {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return (None, None),
    };
    let maximised = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v.get("maximized").and_then(serde_json::Value::as_bool));
    (persisted_rect(&text), maximised)
}

/// The M9 fixed point on product bytes, told apart from every other answer here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductCycle {
    /// Maximise, close, relaunch, close: the restore rect never moved.
    Held { drift: (i32, i32, i32, i32) },
    /// A precondition went missing, so nothing is claimed in either direction.
    NotJudged(String),
    /// Measured, and the window-memory promise did not hold. Exit 6.
    Broken(String),
}

/// THE VERDICT, pure over its readings, because the arithmetic is the deliverable and a
/// judge nobody can feed cannot be tested. Four inputs are the whole story: pre, the
/// live SETTLED normal position before anything was maximised; persisted, what
/// session.json said after the maximised window closed (its rect and its maximized bit);
/// relaunch_maximised, whether the relaunched window answered SW_SHOWMAXIMIZED in
/// GetWindowPlacement.showCmd; restore, the rect persisted on close-AGAIN.
///
/// What is deliberately NOT asserted here is that the maximise landed. A window the
/// harness failed to zoom proves nothing about the product that would have had to
/// remember it, so the caller reports NOT JUDGED before this function is ever reached -
/// the same rule the needle leg states about its own ZOOM line, applied one step
/// earlier, to the precondition rather than to the conclusion.
pub fn product_cycle_verdict(
    pre: Option<Rect>,
    persisted: Option<bool>,
    relaunch_maximised: Option<bool>,
    restore: Option<Rect>,
) -> ProductCycle {
    let Some(stored_maximised) = persisted else {
        return ProductCycle::NotJudged(
            "session.json could not be read after the maximised close, so there is no recorded  state to assert either way".to_string(),
        );
    };
    if !stored_maximised {
        return ProductCycle::Broken(
            "THE MAXIMISE WAS FORGOTTEN: the window was closed while it answered  SW_SHOWMAXIMIZED, and session.json came back with maximized:false - the state the user left  is not the state the port recorded".to_string(),
        );
    }
    let relaunch = match relaunch_maximised {
        None => {
            return ProductCycle::NotJudged(
                "the relaunched window's show state could not be read, so a promise about coming  back maximised is not judged either way".to_string(),
            )
        }
        Some(false) => {
            return ProductCycle::Broken(
                "THE MAXIMISE DID NOT COME BACK: session.json recorded maximized:true and the  relaunch answered a showCmd that was not SW_SHOWMAXIMIZED - the state was stored and then  never applied to a window".to_string(),
            )
        }
        Some(true) => true,
    };
    let _ = relaunch;
    match rect_drift(pre, restore) {
        Some((0, 0, 0, 0)) => ProductCycle::Held {
            drift: (0, 0, 0, 0),
        },
        Some(d) => ProductCycle::Broken(format!(
            "THE RESTORE RECT MOVED: one maximise-close-relaunch-close cycle took the rect from  {} to {}, a drift of ({},{},{},{}) - the window the user left maximised came back  remembering a different place",
            pre.map(|r| r.text()).unwrap_or_else(|| "-".into()),
            restore.map(|r| r.text()).unwrap_or_else(|| "-".into()),
            d.0,
            d.1,
            d.2,
            d.3
        )),
        None => ProductCycle::NotJudged(
            "one of the two normal positions could not be read, so there is nothing to compare"
                .to_string(),
        ),
    }
}

/// THE M9 FIXED POINT, RUN ON PRODUCT BYTES - the reason exit 6 has a producer again.
///
/// Three sentences of choreography, and one rule: the harness never writes the state it
/// is about to read. The needle lane SEEDS a maximised session and asks whether the app
/// obeyed, which is the right question for a bridge; this leg asks whether the product
/// REMEMBERS, so it drives a real maximise into a running window, closes it, and reads
/// what the app itself chose to write. Nothing here moves the user's file aside before
/// the cycle, because a copy the harness made is a copy the harness could have fixed.
///
/// Returns [ProductCycle::NotJudged] whenever a precondition is missing - no desktop, no
/// handle, a zoom the OS never entered, a file that cannot be read - and
/// [ProductCycle::Broken] only for the three things that ARE the promise: the maximised
/// state was not recorded, the relaunch did not come back maximised, or the restore rect
/// moved. [run_product_leg] turns Broken into [GEOMETRY_FAILED_EXIT].
fn product_maximised_cycle(script: &Path, exe: &Path, session: &Path) -> ProductCycle {
    // The user's bytes, taken BEFORE anything runs, for the one rule at the bottom: the
    // cycle leaves the app maximised in the file it persists, and that is a real change
    // to somebody's working state made by a check that was only passing through.
    let before = fs::read(session).ok();
    println!(
        "smoke: geometry: the cycle is armed (S8) - a real maximise, a close, a relaunch, a   close again; drift beyond zero is exit {GEOMETRY_FAILED_EXIT}"
    );

    // ---- LEG 1: read where it sits, ask the OS to zoom it, close it -----------------
    let first = match run_product_probe(script, exe, session, PRODUCT_CYCLE_ALIVE_SECS, 1) {
        Ok((probe, _)) => probe,
        Err(e) => {
            restore_session(session, before.as_deref());
            return ProductCycle::NotJudged(format!("the maximise launch did not report: {e}"));
        }
    };
    if !first.flag("DESKTOP") {
        restore_session(session, before.as_deref());
        return ProductCycle::NotJudged(
            "the maximise launch found no interactive desktop, so there was no window to zoom"
                .to_string(),
        );
    }
    if !first.flag("EXITED_WITHOUT_KILL") {
        restore_session(session, before.as_deref());
        return ProductCycle::Broken(format!(
            "THE MAXIMISED WINDOW WOULD NOT CLOSE: launch 1 of the cycle had to be force-killed \
             (EXIT_CODE_AFTER_FORCE={}), and a run that ends in Stop-Process -Force is never a \
             pass - but it is this leg's finding, not the plain launch's, because the plain \
             launch closed politely WITHOUT the zoom",
            first
                .number("EXIT_CODE_AFTER_FORCE")
                .map(|n| n.to_string())
                .unwrap_or_else(|| "-".into())
        ));
    }
    let pre = probe_rect(&first, "NORMAL");
    let pre_stable = first.flag("NORMAL_STABLE");
    let zoomed = first.number("SHOWCMD_AFTER_MAX") == Some(SW_SHOWMAXIMIZED);
    println!(
        "smoke: geometry: INFO - launch 1 read {} (stable={}) before the act; after \
 ShowWindow(SW_MAXIMIZE) it answered showCmd {} after {}ms and IsZoomed said {}",
        pre.map(|r| r.text()).unwrap_or_else(|| "-".into()),
        pre_stable,
        first.number("SHOWCMD_AFTER_MAX").unwrap_or(-1),
        first.number("MAX_LAND_MS").unwrap_or(-1),
        match first.number("ZOOM_AFTER_MAX") {
            Some(1) => "1",
            Some(0) => "0",
            _ => "unknown",
        }
    );
    if !first.flag("MAX_ASKED") || !zoomed {
        // The precondition, declined rather than accused. That the act did not land is
        // still worth a loud line - a product that cannot be maximised by the OS is news
        // - but it is news about a door this leg did not come to test, and code 6 means
        // "the window memory broke", which cannot be decided about a state that never
        // existed.
        restore_session(session, before.as_deref());
        return ProductCycle::NotJudged(
            "ShowWindow(SW_MAXIMIZE) never took effect on the window (MAX_ASKED=\
 ShowWindow's own answer, SHOWCMD_AFTER_MAX the polled showCmd), so there is no maximised \
 state for the product to have forgotten - the PRECONDITION was not measurable, which is not \
 the same claim as a broken promise"
                .to_string(),
        );
    }
    if pre.is_none() || !pre_stable {
        restore_session(session, before.as_deref());
        return ProductCycle::NotJudged(
            "the normal position before the maximise never settled into one answer, so the \
             rect it is supposed to come back to is not a fact this harness could read"
                .to_string(),
        );
    }

    // What the app wrote while it was zoomed: its rect, and the bit this leg exists to
    // check. Read from the app's OWN resolved directory (portable marker first) - never
    // from a guessed %APPDATA%, which is how the 2026-09-13 dossier's probe watched an
    // empty file all day while the process wrote next to the exe.
    let (persisted_rect, persisted_maximised) = session_state(session);
    println!(
        "smoke: geometry: INFO - the app's own {} after that close says rect {} with   maximized:{}",
        SESSION_FILE,
        persisted_rect
            .map(|r| r.text())
            .unwrap_or_else(|| "-".into()),
        persisted_maximised.unwrap_or(false)
    );
    let leg1 = product_cycle_verdict(pre, persisted_maximised, None, None);
    if is_broken(&leg1) {
        // The state was never recorded, so the relaunch could not have come back maximised
        // either, and saying so twice would be one finding counted as two.
        restore_session(session, before.as_deref());
        return leg1;
    }

    // ---- LEG 2: relaunch, read what it did BY ITSELF, close again --------------------
    let second = match run_product_probe(script, exe, session, PRODUCT_CYCLE_ALIVE_SECS, 2) {
        Ok((probe, _)) => probe,
        Err(e) => {
            restore_session(session, before.as_deref());
            return ProductCycle::NotJudged(format!("the relaunch did not report: {e}"));
        }
    };
    if !second.flag("DESKTOP") {
        restore_session(session, before.as_deref());
        return ProductCycle::NotJudged("the relaunch found no interactive desktop".to_string());
    }
    let came_back = match second.number("SHOWCMD_AT_CREATE") {
        Some(SW_SHOWMAXIMIZED) => Some(true),
        Some(-1) | None => None,
        Some(_) => Some(false),
    };
    let after = probe_rect(&second, "NORMAL_RELAUNCH");
    let (relaunch_rect, relaunch_maximised_now) = session_state(session);
    println!(
        "smoke: geometry: INFO - launch 2 answered showCmd {} after {}ms of polling, IsZoomed \
 {}, its live restore rect {}; the file on disk now says rect {} maximized:{}",
        second.number("SHOWCMD_AT_CREATE").unwrap_or(-1),
        second.number("CREATE_LAND_MS").unwrap_or(-1),
        second.number("ZOOM_AT_CREATE").unwrap_or(-1),
        after.map(|r| r.text()).unwrap_or_else(|| "-".into()),
        relaunch_rect
            .map(|r| r.text())
            .unwrap_or_else(|| "-".into()),
        relaunch_maximised_now.unwrap_or(false),
    );
    // The rect the cycle LEAVES behind is the one that has to equal the pre-maximise
    // reading; a missing live answer falls back to what the app persisted, which is the
    // same substitution the needle lane documents and says out loud for the same reason.
    let restore = after.or(relaunch_rect);
    let verdict = product_cycle_verdict(pre, persisted_maximised, came_back, restore);
    println!("smoke: geometry: {verdict}");
    // DO NO HARM, last and unconditionally: whatever the verdict, the user's session file
    // goes back byte-exactly. The check drove a maximise through their window to answer a
    // question, and leaving their app maximised tomorrow morning is not an answer.
    restore_session(session, before.as_deref());
    verdict
}

/// A verdict that is specifically a broken promise (and not merely an unreadable one).
fn is_broken(c: &ProductCycle) -> bool {
    matches!(c, ProductCycle::Broken(_))
}

impl std::fmt::Display for ProductCycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProductCycle::Held { drift } => write!(
                f,
                "PASS - the restore rect is a fixed point across one real maximise on product \
 bytes (drift ({},{},{},{}))",
                drift.0, drift.1, drift.2, drift.3
            ),
            ProductCycle::NotJudged(why) => write!(f, "NOT JUDGED (advisory) - {why}"),
            ProductCycle::Broken(note) => write!(f, "{note}"),
        }
    }
}

/// The three lines the MENU legs read, each with the file that prints it. Quoted in one
/// place on purpose: a leg that searches for its own copy of a string is a leg that keeps
/// passing after the product starts saying something else.
pub const PRODUCT_MENU_NEEDLES: [(&str, &str); 3] = [
    (
        "menu: autosave-row toggled",
        "surface.rs on_autosave_asked - the Auto-save row's handler ran",
    ),
    (
        "dialog[skipped]: SLINT_NO_DIALOG",
        "surface.rs ask_dialog - an Open row's ask reached Rust and the gate stood the modal down",
    ),
    (
        "dialog: spawning the",
        "surface.rs ask_dialog - an Open row's ask reached Rust and a real modal was asked for",
    ),
];

/// How many PRODUCT-VOICE lines carry a needle. Not `str::matches`: the same words can
/// appear inside a capture this harness echoes under its own prefix, and the plain leg's
/// rule has always been that an unvoiced match is not the product speaking. Counting
/// voiced lines only is what lets a leg assert "exactly one ask" instead of "an ask
/// somewhere in the noise".
pub fn voice_count(trace: &str, needle: &str) -> usize {
    trace
        .lines()
        .filter(|l| l.trim_start().starts_with(PRODUCT_VOICE) && l.contains(needle))
        .count()
}

/// Did the toggled line actually report a TRANSITION? The line carries both words -
/// "toggled true->false" - so this reads them rather than trusting that a print means a
/// change. A handler that printed its own name without flipping the bit would otherwise
/// pass every assert in this file.
pub fn voice_toggled(trace: &str) -> bool {
    trace.lines().any(|l| {
        l.trim_start().starts_with(PRODUCT_VOICE)
            && l.contains("menu: autosave-row toggled")
            && (l.contains("true->false") || l.contains("false->true"))
    })
}

/// One press as the script answered it: the pixel ASKED for, who was under it, and where the
/// pointer actually landed.
pub type PressReading = (i32, i32, bool, i64, i32, i32);

/// "asked-x,asked-y,ours,hwnd-under,landed-x,landed-y" - the pixel the harness ASKED the OS
/// for, who the OS said was under it, and where GetCursorPos says the pointer ACTUALLY sat
/// when the button went down. Malformed or absent is None, and None is never a pass - the
/// shape of every reading rule in this file. The two new fields are why the arity is six:
/// an asked-for pixel describes the wish, the landed pixel is the act.
pub fn press_reading(text: Option<&str>) -> Option<PressReading> {
    let parts: Vec<&str> = text?.split(',').map(|s| s.trim()).collect();
    if parts.len() != 6 {
        return None;
    }
    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2] == "1",
        parts[3].parse().ok()?,
        parts[4].parse().ok()?,
        parts[5].parse().ok()?,
    ))
}

/// What the CURSOR itself said about a press, beside what the harness asked for. This is the
/// distinction the dead-input run could not make: a menu that hears nothing and a session
/// that never moved the pointer print the same silence, and only the landed pixel separates
/// them. Advisory in BOTH directions and by rule - it explains a NOT JUDGED, it never
/// creates one and it never cancels one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorSaid {
    /// SetCursorPos asked for one pixel and GetCursorPos found the pointer at another: no
    /// press this leg counts was ever delivered. The likeliest cause is a session nobody is
    /// sitting in front of - locked, or a desktop this process does not belong to.
    NeverMoved {
        asked: (i32, i32),
        landed: (i32, i32),
    },
    /// The pointer sat exactly where it was told. A leg that heard nothing AFTER this is
    /// about the window, not about the OS.
    Moved { at: (i32, i32) },
    /// Nothing was said: the branch never moved the cursor (the press was not over our
    /// window), GetCursorPos refused, or there was no reading at all. Silence about the
    /// cursor is evidence about nothing, and is written that way.
    Unknown,
}

impl CursorSaid {
    /// The one honest sentence this evidence buys, and the only thing it is allowed to say.
    /// Note which way the exculpatory one points: it excuses the MENU and indicts the
    /// session, which is why it may ride a NOT JUDGED and may not ride a FAIL.
    pub fn clause(self) -> Option<&'static str> {
        match self {
            CursorSaid::NeverMoved { .. } => Some(
                "THE OS NEVER MOVED THE CURSOR (session locked/background?) - GetCursorPos still   reported the pointer where it already sat, so the pixel this leg pressed was never a   press at all and nothing here can accuse the menu of ignoring one",
            ),
            CursorSaid::Moved { .. } => Some("cursor moved, window ignored the press"),
            CursorSaid::Unknown => None,
        }
    }

    /// The INFO field's VALUE: where the pointer ACTUALLY landed, as the OS answered it, so
    /// a reader can check the sentence against the number instead of trusting it. The label
    /// lives in the INFO line, which is always printed - an unread cursor shows up as
    /// `cursor moved -> not read` rather than as a missing field.
    pub fn shown(self) -> String {
        match self {
            CursorSaid::Moved { at } => format!("{},{}", at.0, at.1),
            CursorSaid::NeverMoved { landed, .. } => {
                format!("{},{} (NOT where it was asked)", landed.0, landed.1)
            }
            CursorSaid::Unknown => "not read".to_string(),
        }
    }
}

/// The cursor evidence of one press reading. `-1,-1` is the script's own "this branch never
/// asked the OS to move" answer and reads as [CursorSaid::Unknown], never as a mismatch -
/// an unread cursor must not become the exculpatory sentence either.
pub fn cursor_said(press: PressReading) -> CursorSaid {
    let (ask_x, ask_y, ours, _under, land_x, land_y) = press;
    if !ours || (land_x, land_y) == (-1, -1) {
        return CursorSaid::Unknown;
    }
    if (land_x, land_y) == (ask_x, ask_y) {
        CursorSaid::Moved {
            at: (land_x, land_y),
        }
    } else {
        CursorSaid::NeverMoved {
            asked: (ask_x, ask_y),
            landed: (land_x, land_y),
        }
    }
}

/// A leg that drove SEVERAL presses: the most exculpatory reading wins. One press whose
/// cursor never moved is enough to say the gesture was never delivered, while a leg with no
/// reading at all still says nothing.
pub fn cursor_said_all(presses: impl IntoIterator<Item = PressReading>) -> CursorSaid {
    let said: Vec<CursorSaid> = presses.into_iter().map(cursor_said).collect();
    if let Some(never) = said
        .iter()
        .copied()
        .find(|s| matches!(s, CursorSaid::NeverMoved { .. }))
    {
        return never;
    }
    if let Some(moved) = said
        .iter()
        .copied()
        .find(|s| matches!(s, CursorSaid::Moved { .. }))
    {
        return moved;
    }
    CursorSaid::Unknown
}

/// THE DOWNGRADE AND THE SHARPENING, one pure function so both of its sentences are testable
/// without a desktop. A leg written to go red accepts being told to go grey - see the gate
/// text - and the cursor then says WHICH silence this run measured. It never goes the other
/// way: a Broken with input live stays Broken however the cursor read, because "the OS never
/// moved my cursor" is a reason to judge nothing, not a licence to judge more.
pub fn soften(verdict: Menu, input_live: bool, cursor: CursorSaid) -> Menu {
    match verdict {
        Menu::Broken(why) if !input_live => {
            let gate = "no answer to the press, but the chord leg did not land a keystroke on   this window either - injected input is not arriving, so this is not a verdict about the   menu.";
            let head = match cursor.clause() {
                Some(clause) => format!("{clause} - "),
                None => String::new(),
            };
            Menu::NotJudged(format!("{head}{gate} Named gap: {why}"))
        }
        Menu::NotJudged(why) => match cursor.clause() {
            Some(clause) => Menu::NotJudged(format!("{why} - {clause}")),
            None => Menu::NotJudged(why),
        },
        other => other,
    }
}

/// The reading a leg has on its wrist: no press line, no cursor opinion.
pub fn cursor_of(press: Option<PressReading>) -> CursorSaid {
    press.map_or(CursorSaid::Unknown, cursor_said)
}

/// What a MENU leg said. Its own type and not [Toggle]: a toggle is about a draft file
/// landing or not landing, and these are about an ASK arriving.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Menu {
    Held { detail: String },
    NotJudged(String),
    Broken(String),
}

impl std::fmt::Display for Menu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Menu::Held { detail } => write!(f, "PASS - {detail}"),
            Menu::NotJudged(why) => write!(f, "NOT JUDGED (advisory) - {why}"),
            Menu::Broken(note) => write!(f, "{note}"),
        }
    }
}

/// LEG A'S VERDICT: a pointer press on the Auto-save row reached the same handler the
/// chord reaches. Pure, so the counting rule is testable without a desktop.
pub fn judge_click_toggle(asked: usize, flipped: bool, press_landed: bool) -> Menu {
    if !press_landed {
        return Menu::NotJudged(
            "WindowFromPoint did not answer our window under the press, so nothing was clicked   that the product could see - an instrument finding, not a verdict on the menu".to_string(),
        );
    }
    if asked == 0 {
        return Menu::Broken(
            "THE MENU IGNORED A REAL CLICK: the hamburger and the Auto-save row were both pressed at the pixels chrome.slint's own arithmetic gives them, on our own window, and the product printed no 'menu: autosave-row toggled' line at all. Either the popup was not open under the cursor or the row's TouchArea is not the window the click reaches".to_string(),
        );
    }
    if !flipped {
        return Menu::Broken(
            "the row answered a click with a line that names NO transition - 'toggled' printed   without true->false or false->true beside it".to_string(),
        );
    }
    Menu::Held {
        detail: format!(
            "{asked} toggled line(s) from one press on row 2, and one of them carried a real   transition"
        ),
    }
}

/// LEG B'S VERDICT (the cheap half of item 4): with the modal stood down, the Open row's
/// ask still has to arrive - and the refusal line is the only proof of that which costs
/// nobody a window.
pub fn judge_dialog_asked(skipped: usize, press_landed: bool) -> Menu {
    if !press_landed {
        return Menu::NotJudged(
            "the press was not over our window, so the row was never asked to answer".to_string(),
        );
    }
    if skipped == 0 {
        return Menu::Broken(
            "THE OPEN ROW NEVER REACHED RUST: it was pressed at the pixel chrome.slint gives row 0 (popup-x + pad + 40, bar-height + pad + row-height/2) on our own window, and no 'dialog[skipped]: SLINT_NO_DIALOG' line came back - so either the popup was not open there, or the row geometry the markup states is not the geometry the window shows".to_string(),
        );
    }
    Menu::Held {
        detail: format!(
            "the Open row's ask reached Rust {skipped} time(s); the gate, not a wrong pixel, is   why no modal appeared"
        ),
    }
}

/// LEG C'S VERDICT - ADR-0006 re-earning item 4, the native dialog, on camera. Four
/// readings and one order, because the earliest missing one is the one that explains the
/// rest: did Rust get the ask, did the desktop get a window, did that window go away when
/// asked, and was the MAIN window still alive afterwards.
pub fn judge_native_dialog(
    spawned: usize,
    seen: bool,
    gone: bool,
    main_alive: bool,
    ms: i64,
) -> Menu {
    if spawned == 0 {
        return Menu::Broken(
            "THE OPEN ROW NEVER ASKED FOR A DIALOG: with SLINT_NO_DIALOG removed from the   child's environment the ask has to reach surface.rs ask_dialog and print 'dialog: spawning   the ... picker'; a silent stderr means the click never became an ask, which is leg A's   finding, not this one".to_string(),
        );
    }
    if !seen {
        return Menu::Broken(
            "ITEM 4 STILL OWED: the ask reached Rust and NO '#32770' top-level of our own pid   appeared on the desktop within {ms}ms - the native dialog this product promises on Open was   not on the screen, and nothing here can say it works".to_string(),
        );
    }
    if !gone {
        return Menu::Broken(
            "THE MODAL WOULD NOT CLOSE: a '#32770' of our pid was up and WM_CLOSE did not take   it down - and the leg refuses to Stop-Process its way out of that, because a killed dialog   is not a closed one".to_string(),
        );
    }
    if !main_alive {
        return Menu::Broken(
            "THE DIALOG TOOK THE LOOP WITH IT: the modal came and went and the MAIN window was   gone when we looked - an unresponsive-loop finding, exactly the one ask_dialog's own comment   says the rule was chosen to avoid".to_string(),
        );
    }
    Menu::Held {
        detail: format!(
            "a real '#32770' of our pid answered within {ms}ms of the row press, closed on WM_CLOSE,   and the main window was still alive behind it - the first machine proof this product's native   dialog flow works at all"
        ),
    }
}

/// LEG D'S VERDICT: b76c277c's promise, told through the ONLY observable that exists for
/// it. Chrome owns the menu-open bit and Rust is forbidden from touching it (surface.rs
/// names `set_menu_open(` as a forbidden call, and the guard test holds the line), so
/// there is no "the popup closed" print to count - but every ask IS printed, and the
/// gesture leg drives TWO presses at the row: one blind after the drag, one after a fresh
/// hamburger. One ask therefore means the drag closed it AND the pixel works. Two means
/// the popup was still open. Zero means the second press found nothing either, which is
/// the shape that makes this leg's evidence depend on leg B's - said out loud below rather
/// than hidden in an assert.
pub fn judge_drag_closes(asks: usize, press_landed: bool) -> Menu {
    if !press_landed {
        return Menu::NotJudged(
            "no press in the gesture landed on our window, so the gesture never happened"
                .to_string(),
        );
    }
    match asks {
        1 => Menu::Held {
            detail: "the row pixel asked ONCE - the press after the drag asked for nothing (the   popup was closed) and the press after a fresh hamburger did (the pixel is alive)".to_string(),
        },
        0 => Menu::Broken(
            "NEITHER PRESS ASKED: the popup was opened by a press and the Open-row pixel was   pressed twice, once after a fresh hamburger, and Rust heard nothing - so the drag leg cannot   say anything about a popup it could not open. Leg B (the same pixel, no drag) is the leg that   says whether this is geometry or the product".to_string(),
        ),
        n => Menu::Broken(format!(
            "THE DRAG DID NOT CLOSE THE MENU: {n} asks where exactly one was promised - the press   that was supposed to land on a closed popup landed on an open one, which is the b76c277c   behaviour breaking in the direction the user notices"
        )),
    }
}

/// THE MENU LEGS, all four, in the order their costs rise: two presses on the Auto-save
/// row, two on the Open row with the modal stood down, the same pair with the modal ALLOWED
/// (item 4), and the drag gesture. Each one is its own bounded child, its own probe run,
/// and its own verdict - a leg that stops at the first decline would hide the interesting
/// one behind the boring one.
///
/// The user's session.json is put back after all four. These legs move the window (the drag
/// moves it 40 px), close windows, and can leave a recents entry behind, and none of that is
/// what a developer wants to find in their own app tomorrow morning.
fn product_menu_legs(
    script: &Path,
    exe: &Path,
    session: &Path,
    // Whether the CHORD leg, the one instrument in this file that has ever landed a real
    // key on this window, actually got through. See the downgrade below.
    input_live: bool,
) -> Vec<(&'static str, Menu)> {
    let before = fs::read(session).ok();
    // The order below is cost and nothing else - A presses two pixels and counts a line, B
    // the same, C waits up to two seconds for a window it may never find, D drags - and the
    // vec is a literal rather than four pushes because a leg that stops the run at the first
    // decline would hide the interesting answer behind the boring one. Each leg is still its
    // own child, its own probe, its own verdict.
    let legs = vec![
        ("menu-click", product_menu_click(script, exe, session)),
        (
            "dialog-asked",
            product_dialog_asked_leg(script, exe, session),
        ),
        (
            "native-dialog",
            product_native_dialog_leg(script, exe, session),
        ),
        (
            "drag-closes-menu",
            product_drag_closes_menu(script, exe, session),
        ),
    ];
    // Each leg brings its verdict AND what its own presses said about the cursor, because
    // the cursor is the one reading that separates the two silences these legs can produce.
    let legs: Vec<(&'static str, Menu)> = legs
        .into_iter()
        .map(|(name, (verdict, cursor))| (name, soften(verdict, input_live, cursor)))
        .collect();
    // THE DOWNGRADE now lives in [soften], called above, because its text and the cursor
    // sentence are one judgement and two copies of them start disagreeing. Why a leg written
    // to go red accepts being told to go grey at all: every one of these four legs decides
    // on a line the PRODUCT prints after a press, and "no line" has two causes that look
    // identical from here - the menu is deaf, or the press never arrived. The chord leg above
    // is the instrument that answers the second question in the general case (the same
    // SetCursorPos/mouse_event doors, the same window, proven green on real bytes), and each
    // leg's own GetCursorPos reading is what answers it for THIS press. Neither is allowed to
    // go the other way: when input did land, a leg that heard nothing stays red.
    restore_session(session, before.as_deref());
    legs
}

/// LEG A: hamburger, then the Auto-save row. Verdict in [judge_click_toggle].
fn product_menu_click(script: &Path, exe: &Path, session: &Path) -> (Menu, CursorSaid) {
    let (probe, trace) = match run_product_probe(script, exe, session, PRODUCT_MENU_ALIVE_SECS, 4) {
        Ok(v) => v,
        Err(e) => {
            return (
                Menu::NotJudged(format!("the click launch did not report: {e}")),
                CursorSaid::Unknown,
            );
        }
    };
    if !probe.flag("DESKTOP") {
        return (
            Menu::NotJudged(
                "the click launch found no interactive desktop, so there was no cursor to place"
                    .to_string(),
            ),
            CursorSaid::Unknown,
        );
    }
    let press = press_reading(probe.get("ROW_AT"));
    let landed = press.map(|p| p.2).unwrap_or(false);
    let cursor = cursor_of(press);
    let asked = voice_count(&trace, PRODUCT_MENU_NEEDLES[0].0);
    println!(
        "smoke: menu-click: INFO - hamburger {} | row {} | toggle lines={} active={} | cursor moved -> {}",
        probe.get("HAMBURGER_AT").unwrap_or("-"),
        probe.get("ROW_AT").unwrap_or("-"),
        asked,
        probe.number("ACTIVE").unwrap_or(-1),
        cursor.shown()
    );
    (
        judge_click_toggle(asked, voice_toggled(&trace), landed),
        cursor,
    )
}

/// LEG B: the Open row, modal stood down. Verdict in [judge_dialog_asked].
fn product_dialog_asked_leg(script: &Path, exe: &Path, session: &Path) -> (Menu, CursorSaid) {
    let (probe, trace) = match run_product_probe(script, exe, session, PRODUCT_MENU_ALIVE_SECS, 5) {
        Ok(v) => v,
        Err(e) => {
            return (
                Menu::NotJudged(format!("the refusal launch did not report: {e}")),
                CursorSaid::Unknown,
            );
        }
    };
    if !probe.flag("DESKTOP") {
        return (
            Menu::NotJudged("the refusal launch found no interactive desktop".to_string()),
            CursorSaid::Unknown,
        );
    }
    let press = press_reading(probe.get("ROW_AT"));
    let landed = press.map(|p| p.2).unwrap_or(false);
    let cursor = cursor_of(press);
    let skipped = voice_count(&trace, PRODUCT_MENU_NEEDLES[1].0);
    println!(
        "smoke: dialog-asked: INFO - row {} | refusal lines={} | cursor moved -> {}",
        probe.get("ROW_AT").unwrap_or("-"),
        skipped,
        cursor.shown()
    );
    (judge_dialog_asked(skipped, landed), cursor)
}

/// LEG C: item 4 proper. Verdict in [judge_native_dialog].
fn product_native_dialog_leg(script: &Path, exe: &Path, session: &Path) -> (Menu, CursorSaid) {
    let (probe, trace) = match run_product_probe(script, exe, session, PRODUCT_MENU_ALIVE_SECS, 6) {
        Ok(v) => v,
        Err(e) => {
            return (
                Menu::NotJudged(format!("the dialog launch did not report: {e}")),
                CursorSaid::Unknown,
            );
        }
    };
    if !probe.flag("DESKTOP") {
        return (
            Menu::NotJudged(
                "the dialog launch found no interactive desktop, so no modal could exist"
                    .to_string(),
            ),
            CursorSaid::Unknown,
        );
    }
    let cursor = cursor_of(press_reading(probe.get("ROW_AT")));
    let spawned = voice_count(&trace, PRODUCT_MENU_NEEDLES[2].0);
    let seen = probe.flag("DIALOG_SEEN");
    let gone = probe.flag("DIALOG_GONE");
    let main_alive = probe.flag("MAIN_ALIVE");
    println!(
        "smoke: native-dialog: INFO - spawn lines={} handle={} seen={} at {}ms gone={}   main_alive={} row={} | cursor moved -> {}",
        spawned,
        probe.number("DIALOG_HANDLE").unwrap_or(0),
        seen,
        probe.number("DIALOG_MS").unwrap_or(-1),
        gone,
        main_alive,
        probe.get("ROW_AT").unwrap_or("-"),
        cursor.shown(),
    );
    (
        judge_native_dialog(
            spawned,
            seen,
            gone,
            main_alive,
            probe.number("DIALOG_MS").unwrap_or(-1),
        ),
        cursor,
    )
}

/// LEG D: the drag. Verdict in [judge_drag_closes]. Its cursor evidence is the WORST of
/// the three presses it drove: a gesture whose hamburger never moved the pointer was never
/// a gesture, whichever row press happened to look normal.
fn product_drag_closes_menu(script: &Path, exe: &Path, session: &Path) -> (Menu, CursorSaid) {
    let (probe, trace) = match run_product_probe(script, exe, session, PRODUCT_MENU_ALIVE_SECS, 7) {
        Ok(v) => v,
        Err(e) => {
            return (
                Menu::NotJudged(format!("the drag launch did not report: {e}")),
                CursorSaid::Unknown,
            );
        }
    };
    if !probe.flag("DESKTOP") {
        return (
            Menu::NotJudged("the drag launch found no interactive desktop".to_string()),
            CursorSaid::Unknown,
        );
    }
    let asks = voice_count(&trace, PRODUCT_MENU_NEEDLES[1].0);
    let presses: Vec<PressReading> = ["HAMBURGER_AT", "ROW_AT", "ROW2_AT"]
        .iter()
        .filter_map(|k| press_reading(probe.get(k)))
        .collect();
    let landed = presses.iter().filter(|p| p.2).count();
    let cursor = cursor_said_all(presses.iter().copied());
    println!(
        "smoke: drag-closes-menu: INFO - band {} | presses landed on our window={}/3 |   refusal lines={} (want exactly 1) | cursor moved -> {}",
        probe.get("BAND_DRAG").unwrap_or("-"),
        landed,
        asks,
        cursor.shown()
    );
    (judge_drag_closes(asks, landed >= 2), cursor)
}

/// Kill anything the failed run left on screen. ONLY called on a failure path: on a
/// pass the app has already exited by itself, and that is the whole claim.
fn product_teardown(target: &ArtifactTarget) {
    println!(
        "smoke: teardown: the run did not pass, so a product window left open would be  misread by the next one - killing any lingering {}.exe",
        target.bin
    );
    let _ = Command::new("taskkill")
        .args(["/F", "/IM", &target.bin_file()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// THE PRODUCT LEG: build and freshness were already done from the selected row, so
/// what is left is the launch, the lines, the 45s, the close, and the exit code.
///
/// What it does, in two acts. Act one is the plain launch: the startup lines, the 45s,
/// the WM_CLOSE, the exit code - read with no rect, no seed and no move, because those
/// were the needle schedule's claims about gpui's contract and the product's own claims
/// are its own. Act two, added by S8 and run ONLY when act one passed, is the M9 rect
/// fixed point on these bytes: a real maximise through the OS door the caption button
/// reaches, a close, a read of what the app persisted, a relaunch, and a second close -
/// so "comes back the size and place you left it" is now measured on the artifact that
/// ships, and not only on the frozen instrument.
///
/// It still does NOT move the user's session.json aside, seed a recent, poll a pin, or
/// write the state file it is about to read: the cycle drives the window and lets the
/// product do the persisting, and it puts the user's own bytes back when it is done
/// (see [product_maximised_cycle]). Verdict codes are still only the existing ones -
/// 0, 1, 2, 3 and now 6 - and the table gains no row: 6 was already in CONTRACT and
/// already armed in ci.yml; what it lacked was any default run able to produce it.
fn run_product_leg(target: &ArtifactTarget, exe: &Path) -> i32 {
    println!(
        "smoke: LEG=PRODUCT - {} is judged by the product contract: the startup lines on \
 its own captured stderr, still alive at {PRODUCT_ALIVE_SECS}s, and a WM_CLOSE it answers by \
 exiting 0 BY ITSELF. The needle schedule is not run against these bytes.",
        exe.display()
    );
    let script = temp_path("product-probe", "ps1");
    let out_file = temp_path("product-out", "txt");
    let err_file = temp_path("product-err", "txt");
    let junk = [script.clone(), out_file.clone(), err_file.clone()];
    let cleanup = |tag: &str| {
        let _ = tag;
        for p in junk.iter() {
            let _ = fs::remove_file(p);
        }
    };
    if let Err(e) =
        fs::File::create(&script).and_then(|mut f| f.write_all(PRODUCT_PROBE.as_bytes()))
    {
        println!("SMOKE FAIL: cannot write the product probe: {e}");
        return HARNESS_EXIT;
    }
    let started = Instant::now();
    // The session path the cycle reads back. Resolved through the app's OWN rule
    // (state_dir_for, portable-marker-first) and not through the candidate SEARCH list:
    // for a debug exe that is target/debug/data, not %APPDATA%, and a probe that watched
    // the roaming profile all day while the portable marker sent the process next to the
    // exe is the mistake the 2026-09-13 dossier records. app_state_dir is the same
    // function the recents seed and the user-bytes relocation resolve through.
    let session = app_state_dir(exe)
        .map(|dir| dir.join(SESSION_FILE))
        .unwrap_or_else(|| PathBuf::from(SESSION_FILE));
    let mut child = match spawn_product_probe(
        &script,
        exe,
        &out_file,
        &err_file,
        &session,
        PRODUCT_ALIVE_SECS,
        // Mode 0: the plain launch reads NOTHING about geometry, so the 45s claim keeps the
        // runtime it had before this slice and the cycle's cost stays entirely its own.
        0,
    ) {
        Err(e) => {
            println!("SMOKE FAIL: {e}");
            println!("smoke: product=NOPOWERSHELL alive=NOBUILD close=NOBUILD 0.0s");
            cleanup("spawn");
            return HARNESS_EXIT;
        }
        Ok(child) => child,
    };
    let probe = match wait_bounded(&mut child, PRODUCT_OUTER_SECS) {
        Err(e) => {
            println!("SMOKE FAIL: {e}");
            println!("smoke: product=TIMEOUT alive=TIMEOUT close=TIMEOUT 0.0s");
            product_teardown(target);
            cleanup("timeout");
            return STEP_FAILED_EXIT;
        }
        Ok(None) => {
            println!("SMOKE FAIL: the product probe reported no exit status at all");
            product_teardown(target);
            cleanup("nostatus");
            return HARNESS_EXIT;
        }
        Ok(Some(status)) => {
            let stdout = read_pipe(child.stdout.as_mut());
            if !status.success() {
                println!("smoke: note - the product probe child itself exited {status}");
            }
            parse_probe(&stdout)
        }
    };
    let trace = report_captured(&err_file, "the product on its own stderr").unwrap_or_default();
    let _ = report_captured(&out_file, "the product on stdout");
    println!(
        "smoke: product pid={} handle={} title={:?} launch_ms={} alive_at={}ms ALIVE={} \
 visible_at_close={} exit_code_early={:?}",
        probe.get("PID").unwrap_or("?"),
        probe.number("HANDLE").unwrap_or(0),
        probe.get("TITLE").unwrap_or(""),
        probe.number("LAUNCH_MS").unwrap_or(-1),
        probe.number("ALIVE_MS").unwrap_or(-1),
        probe.flag("ALIVE"),
        probe.flag("VISIBLE_AT_CLOSE"),
        probe.get("EXIT_CODE_EARLY").filter(|v| !v.is_empty())
    );
    // THREE CLASSES OF DECLINE, before any claim: this machine cannot host the run,
    // which is not the app's fault and never gets a red against it.
    if !probe.flag("PROBE_DONE") || !probe.flag("SPAWN") || !probe.flag("DESKTOP") {
        let gaps = product_station_gaps(&probe);
        if !gaps.is_empty() {
            println!(
                "smoke: DECLINED - this window station cannot host a product run: {}",
                gaps.join("; ")
            );
            println!(
                "smoke: a decline is not a verdict about the app: nothing was launched that  could fail (DESKTOP={:?}, keys seen [{}])",
                probe.get("DESKTOP").unwrap_or("<absent>"),
                probe.keys()
            );
            cleanup("declined");
            return DECLINED_EXIT;
        }
        // The desktop was there and the probe still did not answer: that is the
        // harness's own failure, and it says so rather than borrowing the app's code.
        println!(
            "SMOKE FAIL: the product probe never completed (keys seen: [{}]) - a missing  key is never a pass, and here it is the harness, not the app",
            probe.keys()
        );
        product_teardown(target);
        cleanup("incomplete");
        return HARNESS_EXIT;
    }
    let mut gaps: Vec<String> = Vec::new();
    if probe.number("HANDLE").unwrap_or(0) == 0 {
        gaps.push(format!(
            "SMOKE FAIL: startup - no top-level window within {WINDOW_SECS}s \
 (pid={}, title={:?}): the product never showed, so nothing downstream is creditable",
            probe.get("PID").unwrap_or("?"),
            probe.get("TITLE").unwrap_or("")
        ));
    } else if let Some(ms) = probe.number("LAUNCH_MS") {
        if ms > COLD_START_BUDGET_MS {
            gaps.push(format!(
                "SMOKE FAIL: startup - launch to window took {ms}ms, over the \
 {COLD_START_BUDGET_MS}ms cold-start budget (whitepaper §2)"
            ));
        }
    }
    gaps.extend(product_gaps(
        &trace,
        PRODUCT_STARTUP_NEEDLES,
        "the startup lines",
    ));
    if !probe.flag("ALIVE") {
        gaps.push(format!(
            "SMOKE FAIL: alive - the product was NOT there at {PRODUCT_ALIVE_SECS}s \
 (gone by {}ms, exit code after the early end: {:?}). The needle schedule cannot make \
 this claim, because its artifact hides itself; the product has no self-hide, so a \
 process that quit early quit on its own",
            probe.number("ALIVE_MS").unwrap_or(-1),
            probe.get("EXIT_CODE_EARLY").unwrap_or("<absent>")
        ));
    }
    // The close phase: only judged if the app was actually alive to be asked.
    if probe.flag("ALIVE") {
        if !probe.flag("VISIBLE_AT_CLOSE") {
            gaps.push(
                "SMOKE FAIL: close - the window was not visible when the close was asked \
 (VISIBLE_AT_CLOSE=0), so the WM_CLOSE went to something a user could not have closed"
                    .to_string(),
            );
        }
        if !probe.flag("CLOSE_REQUESTED") {
            gaps.push(
                "SMOKE FAIL: close - CloseMainWindow (WM_CLOSE) was not accepted, so the \
 graceful-shutdown path was never asked to run and its silence would prove nothing"
                    .to_string(),
            );
        }
        gaps.extend(product_gaps(
            &trace,
            PRODUCT_CLOSE_NEEDLES,
            "the close lines",
        ));
        if probe.flag("FORCED") || !probe.flag("EXITED_WITHOUT_KILL") {
            gaps.push(format!(
                "SMOKE FAIL: close - the app did NOT exit by itself within {CLOSE_SECS}s of \
 WM_CLOSE and was force-killed (code after the kill: {:?}). A force-kill is not a graceful \
 shutdown and cannot PASS whatever code it ends with",
                probe.get("EXIT_CODE_AFTER_FORCE")
            ));
        } else {
            match probe.number("EXIT_CODE") {
                Some(0) => {}
                Some(code) => gaps.push(format!(
                    "SMOKE FAIL: close - the app exited on its own, with code {code}, not 0"
                )),
                None => gaps.push(
                    "SMOKE FAIL: close - EXIT_CODE is absent, so the exit status is unknown"
                        .to_string(),
                ),
            }
        }
    }
    // ---- S8: THE M9 FIXED POINT, RUN ON THESE BYTES -------------------------------
    // The plain launch has been judged and nothing about it is in question, so the cycle
    // runs now against the same exe, and it is allowed to speak about exactly one thing:
    // the window-memory promise. Two more bounded children. Leg 1 reads the window's
    // SETTLED normal rect, drives a REAL maximise through the same OS door the caption
    // button and the title-band double-click end up at (surface.rs on_toggle_max asks the
    // toolkit to zoom; ShowWindow(SW_MAXIMIZE) asks the OS to, and winit reports the
    // result to the app like any user act), and closes. Leg 2 asks whether the app came
    // back zoomed BY ITSELF and where its restore rect is now. Between the two there is
    // only one writer of the state file: the product.
    //
    // NOT RUN when the plain launch has gaps. A product that would not start or would not
    // close has already been accused of the louder thing; launching it twice more to
    // discover it also forgets its geometry adds a second finding about an app that never
    // got as far as having any, and spends 15s doing it.
    let cycle: Option<ProductCycle> = if gaps.is_empty() {
        Some(product_maximised_cycle(&script, exe, &session))
    } else {
        println!(
            "smoke: geometry: NOT RUN - the plain launch did not pass, so there is no window  whose memory could be judged"
        );
        None
    };
    // THE DRIFT LINE, printed in every case that reached the cycle. It is the whole
    // deliverable of this slice in one number, so it is not folded into a PASS sentence
    // where a green would hide it: a reader looking for "geometry: drift=" finds it
    // whether the answer was zero, non-zero, or unreadable.
    match &cycle {
        Some(ProductCycle::Held { drift }) => println!(
            "smoke: geometry: drift=({},{},{},{}) verdict=HELD - maximise, close, relaunch,  close: the rect a maximised window came back at is the rect it sat at BEFORE the  maximise, to the pixel",
            drift.0, drift.1, drift.2, drift.3
        ),
        Some(ProductCycle::NotJudged(why)) => {
            println!("smoke: geometry: drift=UNREADABLE verdict=NOT JUDGED (advisory) - {why}")
        }
        Some(ProductCycle::Broken(note)) => {
            println!("SMOKE GEOMETRY FAIL: {note}");
            println!(
                "smoke: geometry: drift=NONZERO-or-UNRECORDED verdict=BROKE - see the line  above; this is the code {GEOMETRY_FAILED_EXIT} was written for"
            );
        }
        None => {}
    }
    let cycle_broke = matches!(cycle, Some(ProductCycle::Broken(_)));
    // ---- S9b ITEM 5: THE AUTOSAVE TOGGLE, ROUND-TRIPPED THROUGH THE REAL CHORD -------
    // OFF has to mean nothing reaches the disk; ON has to mean the buffer does. Driven by
    // the recipe in the probe mode-3 block, judged in [judge_toggle], and put back by
    // [restore_draft]. Run only off a clean plain launch AND a cycle that did not break:
    // a window that will not close politely has already been accused of the louder thing.

    // ITEM 4 (the OPEN / SAVE-AS dialogs) IS NO LONGER AN ABSENT ASSERTION - see the menu
    // legs below, of which one exists to walk the desktop and find the modal. What is still
    // not proved here is the SAVE-AS half: the legs press the Open row, because Open is the
    // row the ADR names and the one whose refusal is printed with its label.
    let toggle: Option<Toggle> = if gaps.is_empty() && !cycle_broke {
        Some(product_autosave_toggle(&script, exe, &session))
    } else {
        println!(
            "smoke: autosave-toggle: NOT RUN - the launch before it did not pass, so there was no window left that could take a chord"
        );
        None
    };
    match &toggle {
        Some(Toggle::Held) => println!("smoke: autosave-toggle: off-held on-landed"),
        Some(Toggle::NotJudged(why)) => {
            println!("smoke: autosave-toggle: NOT JUDGED (advisory) - {why}")
        }
        Some(Toggle::Broken(why)) => {
            println!("SMOKE TOGGLE FAIL: {why}");
            println!("smoke: autosave-toggle: BROKE - see the line above");
            gaps.push(format!("SMOKE FAIL: autosave-toggle - {why}"));
        }
        None => {}
    }
    // ---- THE MENU, ANSWERING A REAL CLICK: legs A, B, C and D -----------------------
    // Same gate as the toggle above, for the same reason: every one of these drives a
    // window, and a launch that did not pass leaves no window that could take a press.
    // Their findings go into `gaps`, which is the exit-1 road - these are NOT the geometry
    // promise, and nothing here reaches for {GEOMETRY_FAILED_EXIT}, whose two producers are
    // the cycle and stay the cycle's.
    let menu_legs: Vec<(&'static str, Menu)> = if gaps.is_empty() && !cycle_broke {
        product_menu_legs(&script, exe, &session, matches!(toggle, Some(Toggle::Held)))
    } else {
        println!("smoke: menu-click: NOT RUN - as above, and the three legs after it with it");
        Vec::new()
    };
    for (name, verdict) in &menu_legs {
        match verdict {
            Menu::Held { detail } => println!("smoke: {name}: PASS - {detail}"),
            Menu::NotJudged(why) => {
                println!("smoke: {name}: NOT JUDGED (advisory) - {why}")
            }
            Menu::Broken(why) => {
                println!("SMOKE {name} FAIL: {why}");
                println!("smoke: {name}: BROKE - see the line above");
                gaps.push(format!("SMOKE FAIL: {name} - {why}"));
            }
        }
    }
    let elapsed = started.elapsed();
    // THE MENU CLAUSE IS EARNED, NOT SHIPPED. Four legs that all said NOT JUDGED cannot be
    // summarised two lines later as "a pointer pressed its own menu and a native dialog was
    // found on the desktop" - that sentence is the one a reader quotes into a roadmap, and
    // it has to be able to survive being quoted.
    let menu_held = menu_legs
        .iter()
        .filter(|(_, verdict)| matches!(verdict, Menu::Held { .. }))
        .count();
    let menu_clause = if !menu_legs.is_empty() && menu_held == menu_legs.len() {
        " A pointer then pressed its own menu, and it answered: the Auto-save row, the Open \
row, the title-band drag, and a real native dialog that was found on the desktop, closed and \
survived."
            .to_string()
    } else if menu_held > 0 {
        format!(
            " A pointer then pressed its own menu, and {menu_held} of {n} legs answered; the \
             others are named above.",
            n = menu_legs.len()
        )
    } else if menu_legs.is_empty() {
        String::new()
    } else {
        " What a pointer did NOT prove this run: every one of the four menu legs declined to \
judge, so nothing about the menu answering a click is claimed here - the lines above say why."
            .to_string()
    };
    if gaps.is_empty() && !cycle_broke {
        println!(
            "smoke: product=PASS alive=PASS close=PASS exit=0 {:.1}s",
            elapsed.as_secs_f64()
        );
        println!(
            "smoke:   what this proves: {} said its startup lines, was still on screen at \
 {PRODUCT_ALIVE_SECS}s, took a WM_CLOSE, said the close and the joined shutdown, left with 0 \
 by itself, and then did the thing this leg used to refuse to read: a real maximise, a close, \
 a relaunch and a close again, with the restore rect unchanged to the pixel.{}{}",
            target.bin,
            menu_clause,
            match &cycle {
                // The drift was already printed; the sentence says which claim it carries.
                Some(ProductCycle::Held { .. }) => " That is the M9 window-memory promise,  measured on PRODUCT bytes.".to_string(),
                Some(ProductCycle::NotJudged(_)) => " What it does NOT prove this run: the  rect - the cycle could not be measured here, and the drift line above says what was  missing.".to_string(),
                Some(ProductCycle::Broken(_)) => unreachable!("a broken cycle returns below"),
                None => " What it does NOT prove: the rect - the cycle never ran.".to_string(),
            }
        );
        println!(
            "smoke:   what it still does NOT prove: the pin, the dragged-rect round trip, the  recents trace, and SAVE AS by pointer - the menu legs above press the Open row, not the Save As  row, because Open is the row ADR-0006 names. Those stay the needle schedule's claims, and  this leg does not run it.",
        );
        cleanup("pass");
        return PASS_EXIT;
    }
    for note in &gaps {
        println!("{note}");
    }
    println!(
        "smoke: product=FAIL gaps={} alive={} close={} {:.1}s",
        gaps.len(),
        probe.flag("ALIVE"),
        probe.flag("EXITED_WITHOUT_KILL"),
        elapsed.as_secs_f64()
    );
    product_teardown(target);
    cleanup("fail");
    // Exit 6, and it outranks 1 on purpose. Code 1 means the shutdown or the write broke;
    // a cycle that BROKE is the opposite run - the app started, said its lines, closed
    // politely, WROTE the file - and what failed is what the file remembers. That is the
    // claim GEOMETRY_FAILED_EXIT was minted for, and after S8 the code has a producer on
    // the product leg again: a default --binary=slint run can reach 6 with no flag, no
    // needle schedule and no gpui anywhere near it, which is what ci.yml's 3e arm for 6
    // has been waiting for since the product became the default.
    if cycle_broke {
        return GEOMETRY_FAILED_EXIT;
    }
    STEP_FAILED_EXIT
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

        let newest = newest_source(&dir, &GPUI_TARGET).expect("a readable source mtime");
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

    /// THE SABOTAGE, as a unit fact: a cycle that really does walk the STORED normal
    /// position must not read as a fixed point. The live leg cannot be made to
    /// produce that honestly without breaking the product's window memory on
    /// purpose, so the mutated-normal case is proven here, over the same pure math
    /// the leg arms on - and the numbers are the chrome this machine measures.
    #[test]
    fn a_ratcheted_normal_position_is_not_a_fixed_point() {
        let stored = Rect {
            l: 320,
            t: 240,
            r: 920,
            b: 640,
        };
        // The chrome this machine measures: the stored position gained the
        // non-client border on every side it should not have - 8 left, 4 top,
        // 8 right, 4 bottom.
        let walked = Rect {
            l: 312,
            t: 236,
            r: 928,
            b: 644,
        };
        assert_eq!(
            rect_drift(Some(stored), Some(walked)),
            Some((-8, -4, 16, 8)),
            "the double-count walking the stored position must be visible as drift"
        );
        assert_ne!(rect_drift(Some(stored), Some(walked)), Some((0, 0, 0, 0)));
    }

    /// The leg's whole fix, pinned in the script text: the normal position is
    /// ASKED FOR (GetWindowPlacement), printed (NORMAL=), and WAITED FOR
    /// (NORMAL_STABLE), with an empty answer for a failed call rather than a zero.
    #[test]
    fn the_probe_asks_placement_not_just_the_frame() {
        assert!(
            GEOMETRY_PROBE.contains("public static extern bool GetWindowPlacement"),
            "the script must be able to ask the OS that stores rcNormalPosition"
        );
        assert!(
            GEOMETRY_PROBE.contains("\"NORMAL=$($parts[0])\""),
            "and answer with it"
        );
        assert!(
            GEOMETRY_PROBE.contains("\"NORMAL_STABLE=$stable\""),
            "and say whether it ever stopped moving"
        );
        assert!(
            GEOMETRY_PROBE.contains("'NORMAL='"),
            "a failed call is an absent answer, never a zero-sized rect"
        );
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
            staleness(mtime_of(&exe), newest_source(&dir, &GPUI_TARGET)),
            Stale::Fresh,
            "a newer tests/ file of another crate must not abort the run it built"
        );
        // The teeth: the same newer-than-exe condition on a real input is still a
        // loud 5, and it names the file.
        set_mtime(
            &src,
            std::time::SystemTime::now() + std::time::Duration::from_secs(60),
        );
        match staleness(mtime_of(&exe), newest_source(&dir, &GPUI_TARGET)) {
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
            let newest = newest_source(&dir, &GPUI_TARGET).expect("a source");
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
        let newest = newest_source(&dir, &GPUI_TARGET).expect("a source");
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
            newest_source(&dir, &GPUI_TARGET).map(|(m, p)| (m > future, p)),
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
        let (path, why) = resolve_exe(&root, None, &GPUI_TARGET);
        assert_eq!(
            path,
            PathBuf::from(r"C:\dev\notes-gpui\target\debug\notes-gpui.exe")
        );
        assert!(why.contains("default"), "{why}");
        let (path, why) = resolve_exe(&root, Some(""), &GPUI_TARGET);
        assert!(
            path.starts_with(r"C:\dev\notes-gpui\target"),
            "an empty value is no value: {path:?}"
        );
        assert!(why.contains("default"), "{why}");
        let (path, why) = resolve_exe(&root, Some("   "), &GPUI_TARGET);
        assert!(
            why.contains("default"),
            "whitespace is not a directory: {why}"
        );
        let (private, why) = resolve_exe(&root, Some(r"D:\builds\lane-4"), &GPUI_TARGET);
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
        let (rel, _) = resolve_exe(&root, Some("out/scratch"), &GPUI_TARGET);
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
                Path::new(t.exe_rel_debug).is_relative(),
                "the {label} row's exe path must be workspace-relative, not absolute",
            );
            // The const projection and the seam must be THE SAME STRING. Without
            // this the field could quietly keep saying "debug" after the seam grew a
            // real profile, which is the exact drift the field's doc claims a test
            // prevents - so the claim is checked here, per row.
            assert_eq!(
                t.exe_rel_debug,
                t.exe_rel(Profile::Debug),
                "the {label} row's Debug projection is no longer what the seam builds"
            );
            // And the seam is not a tautology: a Release row has to differ, or the
            // profile argument means nothing and the enum is decoration.
            assert_eq!(
                t.exe_rel(Profile::Release),
                format!("target/release/{}.exe", t.bin),
                "the {label} row's Release path is not the same seam with the other profile"
            );
            assert_ne!(
                t.exe_rel(Profile::Debug),
                t.exe_rel(Profile::Release),
                "the {label} row resolves both profiles to one path"
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

    /// the launched file name is derived, and `concat!` cannot read a const struct's field, and
    /// a literal is exactly the kind of thing that rots quietly: rename the bin target
    /// and smoke keeps launching (and then judging) a file cargo no longer emits. This
    /// is the test that doc comment points at. It pins BOTH rows' exe names, since the
    /// second row is the one no leg drives and therefore the one nothing else checks.
    #[test]
    fn exe_name_is_the_bin_target_plus_the_windows_extension() {
        assert_eq!(
            GPUI_TARGET.bin_file(),
            format!("{}.exe", GPUI_TARGET.bin),
            "the launched name must stay bin + the Windows extension, or smoke launches nothing"
        );
        // The one place the spelled-out string still earns its keep: several OTHER
        // surfaces name this file by hand - CI's artifact path, the manifest step's
        // default target, the smoke step's needles - and none of them can be reached
        // from here. So the derived name is pinned against the literal they assume.
        assert_eq!(
            GPUI_TARGET.bin_file(),
            "notes-gpui.exe".to_string(),
            "the launched exe changed: rename it in ci.yml's artifact paths and in              xtask manifest's default target in the SAME commit, or smoke judges a file              nothing builds"
        );
        assert_eq!(
            BIN_REL,
            format!("target/debug/{}.exe", GPUI_TARGET.bin),
            "the FROZEN gpui leg's own exe path and its launched name must agree about the \
             same bin (this is not the default's path: a bare run judges notes-slint.exe)"
        );
        assert_eq!(
            SLINT_TARGET.exe_rel_debug,
            format!("target/debug/{}.exe", SLINT_TARGET.bin),
            "the second bridge's described exe is named the same way, even unrun"
        );
        // And the derivation is not circular: the row's bin, the derived name and the
        // literal above are three spellings of one fact.
        assert_eq!(
            GPUI_TARGET.bin_file(),
            GPUI_TARGET.bin_file(),
            "the gpui row and the launched name disagree"
        );
        // The two agree by construction, not by coincidence of the same literal:
        // a redirected build still has to be named the same file.
        let (exe, _) = resolve_exe(
            Path::new("wherever"),
            Some("C:/builds/lane-4"),
            &GPUI_TARGET,
        );
        assert_eq!(
            exe.file_name().and_then(|n| n.to_str()),
            Some("notes-gpui.exe"),
            "the CARGO_TARGET_DIR branch resolves to the same file name: {exe:?}"
        );
    }

    /// `--binary` parses in both shapes, defaults to THE PRODUCT, and refuses a name
    /// that is not one of the three without losing the flag's value.
    ///
    /// This is the test that says what a bare run judges, and as of ADR-0006 it judges
    /// `notes-slint.exe`. The default is asserted AGAINST [DEFAULT_BINARY] rather than a
    /// literal, and the literal is asserted separately - so retargeting the const cannot
    /// pass this test by accident, and cannot pass it while `main`'s usage line and the
    /// resolved exe path still say gpui. `gpui` is still parsed explicitly, in both
    /// shapes: frozen means it takes no new work, not that it stopped being legal.
    #[test]
    fn the_binary_flag_parses_both_shapes_and_defaults_to_the_product() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<String>>();
        assert_eq!(parse_args(&s(&[])).unwrap().binary, DEFAULT_BINARY);
        // ...and the const itself says the product, in the three ways a reader checks.
        assert_eq!(
            DEFAULT_BINARY, "slint",
            "a bare run judges the product; if this moved, main.rs's usage line and every \
             hint that names the default moved with it - or this test is now the only thing \
             that knows the default"
        );
        assert_eq!(
            BINARY_NAMES.get(1).copied(),
            Some(DEFAULT_BINARY),
            "the refusal text indexes BINARY_NAMES[1] as the product; the default is that \
             same slot, so the order is load-bearing and a reorder is a retarget"
        );
        assert_eq!(
            select_target(DEFAULT_BINARY).map(|t| (t.bin, t.exe_rel_debug)),
            Some(("notes-slint", "target/debug/notes-slint.exe")),
            "the default resolves to the PRODUCT exe - not the frozen bridge's file"
        );
        // A frozen target stays selectable, in both flag shapes.
        assert_eq!(parse_args(&s(&["--binary=gpui"])).unwrap().binary, "gpui");
        assert_eq!(
            parse_args(&s(&["--binary", "gpui"])).unwrap().binary,
            "gpui"
        );
        assert_eq!(parse_args(&s(&["--binary=slint"])).unwrap().binary, "slint");
        assert_eq!(
            parse_args(&s(&["--binary", "slint-probe"])).unwrap().binary,
            "slint-probe"
        );
        // The other flags still work, and the value form does not eat its neighbour.
        // Named NON-default on purpose: asking for `slint` here would pass even if the
        // flag were ignored, because `slint` is what you get for free now.
        let mixed = parse_args(&s(&["--no-build", "--binary=gpui", "--reuse-state"])).unwrap();
        assert!(mixed.no_build && mixed.reuse && !mixed.require_ours);
        assert_eq!(mixed.binary, "gpui");
        assert!(!parse_args(&s(&["--require-ours"])).unwrap().no_build);
    }

    /// A refusal has to be USEFUL: name what was rejected, and name the three that
    /// were legal, and never accept a bare `--binary` as an implicit default - that is
    /// how a typo in a CI line turns into a green run of the wrong artifact.
    #[test]
    fn an_unusable_binary_value_refuses_naming_every_legal_one() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<String>>();
        for bad in [
            s(&["--binary=gpuii"]),
            s(&["--binary", "slint_probe"]),
            s(&["--binary", "notes-slint"]),
            s(&["--binary"]),
        ] {
            let why = parse_args(&bad).expect_err("this value must not parse");
            assert!(
                BINARY_NAMES.iter().all(|n| why.contains(n)),
                "the refusal does not list all three legal names: {why}"
            );
        }
        // A value that happens to be the PACKAGE name is not the artifact name.
        assert!(parse_args(&s(&["--binary=notes-bridge-slint"])).is_err());
        // And an unknown FLAG is still refused, with a usage line that now names --binary.
        let why = parse_args(&s(&["--frobnicate"])).expect_err("no such flag");
        assert!(
            why.contains("--frobnicate") && why.contains("--binary"),
            "{why}"
        );
    }

    /// Selection and leg are two different questions, and this pins that the second is
    /// answered honestly: all three names resolve to a row with its own exe path and
    /// its own build tokens, and exactly ONE of them has a leg. When a future commit
    /// wires the slint legs, the assert below is the one that goes red and demands the
    /// count be re-read - the discipline `every_row_that_resolves_a_graph_carries_the_lock`
    /// already enforces on the gate roster.
    #[test]
    fn each_selectable_artifact_resolves_its_own_paths_and_exactly_one_has_a_leg() {
        let rows: Vec<(&str, &'static ArtifactTarget)> = BINARY_NAMES
            .iter()
            .map(|n| (*n, select_target(n).expect("every listed name resolves")))
            .collect();
        assert_eq!(rows.len(), 3);
        let mut bins: Vec<&str> = rows.iter().map(|(_, t)| t.bin).collect();
        bins.sort();
        bins.dedup();
        assert_eq!(bins.len(), 3, "two rows share a bin name: {bins:?}");
        for (name, t) in &rows {
            assert_eq!(
                t.exe_rel(Profile::Debug),
                t.exe_rel_debug,
                "the {name} row's Debug projection drifted from its own seam"
            );
            assert_eq!(
                t.build_args().join(" "),
                format!("build -p {} --bin {}", t.pkg, t.bin),
                "the {name} row would build something other than the exe it launches"
            );
        }
        // The two slint rows are the same PACKAGE and different BINS - which is the
        // whole reason a target cannot just carry a package name.
        assert_eq!(SLINT_TARGET.pkg, SLINT_PROBE_TARGET.pkg);
        assert_ne!(SLINT_TARGET.bin, SLINT_PROBE_TARGET.bin);
        assert_eq!(
            leg_for(select_target("gpui").unwrap()),
            Leg::GpuiSchedule,
            "the gpui leg is the one thing that must stay wired"
        );
        // THE COUNT MOVED: exactly one of these rows is still unwired, and it is the
        // instrumented one. `notes-slint` now answers to the product contract, which
        // is the whole subject of this commit - so this assert is the day the
        // doc comment above it ("exactly ONE of them has a leg") got re-read.
        assert_eq!(
            leg_for(select_target("slint").unwrap()),
            Leg::Product,
            "the second bridge's PRODUCT bin is judged by the product leg now; if this \
             goes red, either the row or the leg moved and the decline text lies"
        );
        let probe = select_target("slint-probe").unwrap();
        assert_eq!(
            leg_for(probe),
            Leg::NotWired(probe.bin),
            "the probe leg was wired without the schedule being ported - the decline path\
             and its exit-2 CONTRACT text have to be retired in the same commit that does it"
        );
        // And the refusal that stands has a REASON that is not empty and not a shrug.
        assert!(
            not_wired_reason(probe).len() > 40 && not_wired_reason(probe).contains("schedule"),
            "a decline that names no cause is the shrug this print exists to avoid: {:?}",
            not_wired_reason(probe)
        );
    }

    /// THE SELECTION REGRESSION, in the direction the code once got wrong: every
    /// selectable artifact answers for its OWN roots and nothing else's. The freshness
    /// call in run() used to hand `newest_source` the GPUI_TARGET literal, so
    /// `--binary=slint` cried "stale" over a bridge-gpui edit and stayed silent about
    /// a bridge-slint one - a false 5 and a false pass from one wrong argument. The
    /// next test pins the rows; THIS one pins the walk, both directions.
    #[test]
    fn each_selectable_artifact_is_stale_only_by_its_own_sources() {
        let dir = std::env::temp_dir().join(format!("xtask-own-roots-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        // Every root and file of every row, all of them an hour old, so nothing is
        // stale by accident of being missing.
        let mut all: Vec<&str> = Vec::new();
        for t in [&GPUI_TARGET, &SLINT_TARGET, &SLINT_PROBE_TARGET].iter() {
            for root in t.freshness_roots.iter().chain(t.freshness_files.iter()) {
                if !all.contains(root) {
                    all.push(root);
                }
            }
        }
        for rel in &all {
            let path = dir.join(rel);
            // A root is a DIRECTORY, and set_mtime needs a write handle - opening a
            // directory that way is Access-denied on Windows. So the stamp goes on the
            // file inside it, which is also the file the freshness walk actually reads.
            let stamp = if rel.ends_with(".toml") || rel.ends_with(".lock") {
                fs::write(&path, "# fixture\n").expect("write a file input");
                path
            } else {
                fs::create_dir_all(&path).expect("source tree");
                let inner = path.join("lib.rs");
                fs::write(&inner, "// fixture\n").expect("write a root file");
                inner
            };
            set_mtime(&stamp, past);
        }
        let exe = dir.join("notes-slint.exe");
        fs::write(&exe, b"MZ").expect("write the exe");
        set_mtime(&exe, std::time::SystemTime::now());

        // A BRIDGE-SLINT edit: newer than the exe, and invisible to gpui's roots.
        let slint_src = dir.join("crates/bridge-slint/src/product.rs");
        fs::write(&slint_src, "// touched\n").expect("write the slint source");
        set_mtime(
            &slint_src,
            std::time::SystemTime::now() + std::time::Duration::from_secs(60),
        );
        assert!(
            !GPUI_TARGET
                .freshness_roots
                .iter()
                .any(|r| r.contains("bridge-slint")),
            "the gpui row must not claim the second bridge's sources - that is how the \
             two rows start answering for each other"
        );
        let gpui_newest = newest_source(&dir, &GPUI_TARGET).expect("gpui row answers");
        assert_eq!(
            staleness(mtime_of(&exe), Some(gpui_newest.clone())),
            Stale::Fresh,
            "a bridge-slint edit must NOT make the gpui artifact look stale"
        );
        let slint_newest = newest_source(&dir, &SLINT_TARGET).expect("slint row answers");
        assert_eq!(
            slint_newest.1, slint_src,
            "the slint row must find the file its own row names"
        );
        assert!(
            matches!(
                staleness(mtime_of(&exe), Some(slint_newest.clone())),
                Stale::OlderThan { .. }
            ),
            "and the SAME file must make the slint artifact stale - the one-row-fits-all \
             call site used to get both halves of this backwards"
        );
        // The probe row shares the slint sources (same package) and so sees the same
        // staleness, which is the point of it being a row and not a field.
        assert_eq!(
            newest_source(&dir, &SLINT_PROBE_TARGET).map(|(m, _)| m),
            Some(slint_newest.0)
        );

        // THE OTHER DIRECTION: a bridge-gpui edit, newer still.
        let gpui_src = dir.join("crates/bridge-gpui/src/lib.rs");
        fs::write(&gpui_src, "// touched\n").expect("write the gpui source");
        set_mtime(
            &gpui_src,
            std::time::SystemTime::now() + std::time::Duration::from_secs(120),
        );
        assert_eq!(
            newest_source(&dir, &GPUI_TARGET).map(|(_, p)| p),
            Some(gpui_src.clone()),
            "the gpui row sees its own bridge"
        );
        assert_eq!(
            newest_source(&dir, &SLINT_TARGET).map(|(_, p)| p),
            Some(slint_src),
            "the slint row keeps seeing the slint file: gpui is not an input to it, and a \
             freshness answer that changed here means a root list leaked between rows"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// The flag, the row and the leg, as one chain: what `--binary=X` parses to must be
    /// what selects a row and what picks the CONTRACT, so no surface can disagree with
    /// another about what the flag meant.
    #[test]
    fn the_binary_flag_the_row_and_the_leg_agree_for_every_legal_name() {
        for name in BINARY_NAMES {
            let args = vec![format!("--binary={name}")];
            let inv = parse_args(&args).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(&inv.binary, name, "the parser stored somebody else's name");
            let row = select_target(inv.binary).expect("a parsed name always selects");
            let leg = leg_for(row);
            let spelled = match &leg {
                Leg::GpuiSchedule => "GpuiSchedule",
                Leg::Product => "Product",
                Leg::NotWired(b) => *b,
            };
            match (*name, &leg) {
                ("gpui", Leg::GpuiSchedule)
                | ("slint", Leg::Product)
                | ("slint-probe", Leg::NotWired(_)) => {}
                _ => panic!(
                    "--binary={name} picked the {spelled} leg, which is not the contract \
                     that name selects: {leg:?}"
                ),
            }
            // And the row the leg was picked from is the row whose exe gets resolved:
            // no leg may be speaking a contract while pointed at another artifact's bin.
            let (exe, _) = resolve_exe(Path::new("wherever"), None, row);
            assert_eq!(
                exe.file_name().and_then(|n| n.to_str()),
                Some(format!("{}.exe", row.bin).as_str()),
                "the {name} leg resolves something other than its own exe"
            );
        }
        // Two names, one package: the rows are distinct BECAUSE the bins are.
        assert_ne!(
            select_target("slint").unwrap().bin,
            select_target("slint-probe").unwrap().bin
        );
    }

    /// The product's own line judge: a needle counts only when the product said it in
    /// its own voice, and a missing one is named with the file that owes it. Slint's
    /// warnings share the stderr, so "the words appeared" is not the claim.
    #[test]
    fn a_product_needle_counts_only_in_the_product_voice() {
        let said = concat!(
            "notes-gpui: startup: state dir C:\\Notes\\target\\debug\\data\n",
            "notes-gpui: chord: legend Ctrl+O open \u{b7} 1 in the list\n",
            "notes-gpui: startup: entering the loop; nothing in it fires on a schedule\n",
            "notes-gpui: drop: armed hwnd=0x1234 (this window)\n",
        );
        let mut whole = said.to_string();
        assert!(product_gaps(&whole, PRODUCT_STARTUP_NEEDLES, "startup").is_empty());
        // Missing one line is a named gap, not a near-miss.
        let partial = "notes-gpui: startup: state dir D:\\data\nnotes-gpui: chord: legend x\n";
        let gaps = product_gaps(partial, PRODUCT_STARTUP_NEEDLES, "startup");
        assert_eq!(gaps.len(), 2, "{gaps:?}");
        assert!(gaps.iter().any(|g| g.contains("entering the loop")));
        assert!(gaps.iter().any(|g| g.contains("armed hwnd")));
        // The WRONG VOICE is not the product talking: no prefix, no credit.
        let borrowed = "startup: entering the loop\nslint: drop: armed hwnd=0x1\n";
        let gaps = product_gaps(borrowed, PRODUCT_STARTUP_NEEDLES, "startup");
        assert_eq!(gaps.len(), 4, "{gaps:?}");
        let voiced = "startup: entering the loop\n".replace("startup", "notes-gpui: startup");
        let one = format!("{voiced}\n");
        assert_eq!(
            product_gaps(&one, &[("entering the loop", "product.rs")], "s").len(),
            0
        );
        // And a line that carries the words WITHOUT the prefix is reported as such.
        let quiet = "warning: startup: entering the loop\n";
        let gaps = product_gaps(quiet, &[("entering the loop", "product.rs")], "s");
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert!(gaps[0].contains("voice"), "{}", gaps[0]);
        whole.push_str("notes-gpui: close: requested #1, granted (a product closes the first time)\nnotes-gpui: shutdown: joined cleanly, the session write ran\n");
        assert_eq!(whole.matches("notes-gpui: ").count(), 6);
        assert!(product_gaps(&whole, PRODUCT_CLOSE_NEEDLES, "close").is_empty());
    }

    /// The three classes of "this machine cannot host the check", named apart, because
    /// a decline that says only "no desktop" is the sentence that got read as an app bug.
    #[test]
    fn the_station_decline_names_which_of_the_three_causes_it_is() {
        let all_three = product_station_gaps(&probe(&[
            ("WIN32", "0"),
            ("INTERACTIVE", "0"),
            ("WINDOWED", "0"),
        ]));
        assert_eq!(all_three.len(), 3, "{all_three:?}");
        assert!(all_three[0].contains("WIN32=0"));
        assert!(all_three[1].contains("UserInteractive"));
        assert!(all_three[2].contains("top-level window"));
        // One cause, one sentence: a box that merely has no windowed process must not be
        // told its PowerShell is broken.
        let one = product_station_gaps(&probe(&[
            ("WIN32", "1"),
            ("INTERACTIVE", "1"),
            ("WINDOWED", "0"),
        ]));
        assert_eq!(one.len(), 1, "{one:?}");
        assert!(one[0].contains("WINDOWED=0"));
        assert!(
            product_station_gaps(&probe(&[
                ("WIN32", "1"),
                ("INTERACTIVE", "1"),
                ("WINDOWED", "1")
            ]))
            .is_empty()
        );
        // A key ABSENT reads as 0, i.e. as a decline: absence is never a host.
        assert_eq!(product_station_gaps(&Probe::default()).len(), 3);
    }

    // ---- S8: the M9 fixed point, on the product leg --------------------------------
    //
    // Four kinds of test, in the order that costs least to run: the SCRIPT SHAPE (does
    // the probe even carry the doors and the keys the judge reads - a typo there is a leg
    // that silently stops asserting and keeps printing), the ARITHMETIC (pure over the
    // readings, so the drift rule is checked without a desktop), the BUDGET (a const
    // assertion, because "the cycle is bounded by more than it is bounded by" is the
    // failure mode a timeout hides), and the CONTRACT (exit 6 has a producer here).

    fn rect(s: &str) -> Option<Rect> {
        Rect::parse(s)
    }

    /// The judge's happy path, spelled as the number the brief asked for.
    #[test]
    fn a_zero_drift_cycle_is_held_and_names_its_four_zeros() {
        let v = product_cycle_verdict(
            rect("120,90,920,690"),
            Some(true),
            Some(true),
            rect("120,90,920,690"),
        );
        assert_eq!(
            v,
            ProductCycle::Held {
                drift: (0, 0, 0, 0)
            }
        );
        // The printed form is what a person reads in a CI log, so the format is asserted
        // too: exactly "geometry: drift=(0,0,0,0)" must be reachable from this verdict.
        let shown = format!("{v}");
        assert!(shown.contains("(0,0,0,0)"), "{shown}");
        assert!(shown.contains("PASS"), "{shown}");
    }

    /// ONE pixel of chrome, and the verdict is a finding with the arithmetic in it. The
    /// quadruple is asserted as a tuple, not as prose, because the whole point of the
    /// M9 fix was that dw/dh of +8/-8 is the frame/client double-count and dx/dy of 8/4
    /// is a moved window - two different bugs, and a message that printed only "not zero"
    /// could not tell them apart.
    #[test]
    fn any_movement_at_all_of_the_restore_rect_is_a_broken_promise() {
        for (after, want) in [
            // A moved LEFT edge is also a narrower window: rect_drift reports the size as
            // (right-left), which is why pushing only the left edge by +1 reads as
            // (+1, 0, -1, 0) and not as a pure move. Spelled here so the next reader does
            // not "fix" the arithmetic and lose the double-count signature.
            ("121,90,920,690", (1, 0, -1, 0)),
            ("120,94,920,694", (0, 4, 0, 0)),
            ("120,90,928,698", (0, 0, 8, 8)),
        ] {
            let v =
                product_cycle_verdict(rect("120,90,920,690"), Some(true), Some(true), rect(after));
            match v {
                ProductCycle::Broken(note) => {
                    let (dx, dy, dw, dh) =
                        rect_drift(rect("120,90,920,690"), rect(after)).expect("both rects parse");
                    assert_eq!((dx, dy, dw, dh), want, "{after} => {note}");
                    assert!(note.contains("THE RESTORE RECT MOVED"), "{note}");
                    assert!(note.contains(&format!("({dx},{dy},{dw},{dh})")), "{note}");
                }
                other => panic!("{after}: expected Broken, got {other:?}"),
            }
        }
    }

    /// The three ways this cycle can FAIL to be measurable, each of which must stay NOT
    /// JUDGED rather than turn into an accusation: no file to read, no show state on the
    /// relaunch, and a rect that cannot be read at all. All three print as advisory and
    /// keep the leg's exit code at whatever the plain launch earned.
    #[test]
    fn an_absent_reading_never_becomes_an_acusation() {
        let no_file =
            product_cycle_verdict(rect("0,0,800,600"), None, Some(true), rect("0,0,800,600"));
        assert!(
            matches!(&no_file, ProductCycle::NotJudged(why) if why.contains("session.json")),
            "{no_file:?}"
        );
        let no_show =
            product_cycle_verdict(rect("0,0,800,600"), Some(true), None, rect("0,0,800,600"));
        assert!(matches!(no_show, ProductCycle::NotJudged(_)), "{no_show:?}");
        let no_rect = product_cycle_verdict(None, Some(true), Some(true), None);
        assert!(
            matches!(&no_rect, ProductCycle::NotJudged(why) if why.contains("nothing to compare")),
            "{no_rect:?}"
        );
        // And an UNPARSEABLE rect is absent, not zero-sized: a wrong shape must not become
        // a 0,0,0,0 rect that then "drifts" by the whole window.
        assert_eq!(rect("not-a-rect"), None);
    }

    /// Two findings this leg exists to catch, kept separate because they point at
    /// different code: the state never reached the file (persistence), versus the file
    /// said it and the window ignored it (restore).
    #[test]
    fn a_forgotten_maximise_and_an_unapplied_one_are_different_failures() {
        let forgotten = product_cycle_verdict(
            rect("0,0,800,600"),
            Some(false),
            Some(false),
            rect("0,0,800,600"),
        );
        assert!(
            matches!(&forgotten, ProductCycle::Broken(n) if n.contains("THE MAXIMISE WAS FORGOTTEN")),
            "{forgotten:?}"
        );
        let unapplied = product_cycle_verdict(
            rect("0,0,800,600"),
            Some(true),
            Some(false),
            rect("0,0,800,600"),
        );
        assert!(
            matches!(&unapplied, ProductCycle::Broken(n) if n.contains("THE MAXIMISE DID NOT COME BACK")),
            "{unapplied:?}"
        );
        // And the order is the one that blames the earliest cause: a run that recorded
        // nothing is reported as a recording failure even though its window also did not
        // come back zoomed, because "it did not restore" about a value never stored would
        // send someone to the wrong file.
        assert_ne!(forgotten, unapplied);
    }

    /// The script has to carry the doors the judge reads keys from. Every key named in
    /// product_maximised_cycle appears here, because the day somebody edits one of the two
    /// and not the other, the cycle would print NOT JUDGED forever and look like a machine
    /// that simply has no geometry - which is exactly the silence this slice exists to end.
    #[test]
    fn the_product_probe_carries_every_door_and_every_key_the_judge_reads() {
        for needle in [
            // The OS doors, and the struct whose NORMAL position is the subject.
            "public static extern bool GetWindowPlacement",
            "public static extern bool ShowWindow",
            "public static extern bool IsZoomed",
            "rcNormalPosition",
            // The modes this branch serves, and the fact that mode 0 takes none of this
            // path. 1 and 2 are the geometry pair; 3 is the chord and 4 through 7 are the
            // menu legs, each with its own branch. The gate used to say "-ge 1", which was
            // true while those were the only two modes above 0 and became a lie the moment
            // a mode that owns no geometry at all was added - a maximise leg running
            // under a click leg would print a MAX_ASKED nobody asked for and read the
            // wrong window state. So the enumeration is explicit now, and asserted so.
            "[int]$Maximise = 1",
            "$Maximise -eq 1",
            "$Maximise -eq 2",
            // The keys, printed with the SAME spelling the Rust reads.
            "\"NORMAL=$($a[0])\"",
            "\"NORMAL_STABLE=$($a[2])\"",
            "\"MAX_ASKED=$([int][bool]$asked)\"",
            "\"SHOWCMD_AFTER_MAX=$landed\"",
            "\"NORMAL_RELAUNCH=$($c[0])\"",
            "\"SHOWCMD_AT_CREATE=$seen\"",
            "\"EXIT_CODE_AFTER_FORCE=$($p.ExitCode)\"",
        ] {
            assert!(
                PRODUCT_PROBE.contains(needle),
                "the product probe no longer contains {needle:?}, and the judge in  product_maximised_cycle reads that key"
            );
        }
        // A reading is only a fact once it stops moving; the settle helper is the only
        // way this leg gets one, so its double-sample rule is asserted, not assumed.
        assert!(PRODUCT_PROBE.contains("function Get-Settled"));
        assert!(
            PRODUCT_PROBE
                .contains("if ($next -ne $prev) { $prev = $next } else { $stable = 1; break }")
        );
    }

    /// The cycle's outer bound must be strictly larger than everything the script bounds
    /// inside itself, or a healthy child is killed mid-measure and the leg prints a
    /// TIMEOUT about a promise it never got to read. Encoded as arithmetic on the same
    /// consts the script is handed, in a const block, so it is checked at compile time and
    /// a "tidied" number cannot slip through a code path nobody ran.
    #[test]
    fn the_cycle_budget_affords_every_window_the_script_bounds() {
        const _: () = assert!(
            PRODUCT_CYCLE_OUTER_SECS
                > WINDOW_SECS
                    + PRODUCT_CYCLE_ALIVE_SECS
                    + 2 * PRODUCT_CYCLE_SETTLE_SECS
                    + CLOSE_SECS
        );
        const _: () = assert!(PRODUCT_CYCLE_ALIVE_SECS < PRODUCT_ALIVE_SECS);
        // The plain launch's deadline is UNCHANGED by this slice: folding the cycle into
        // PRODUCT_OUTER_SECS would have made a slow geometry eat the 45s claim's budget and
        // turn an unmeasurable rect into a false failure of a promise that held.
        const _: () =
            assert!(PRODUCT_OUTER_SECS == WINDOW_SECS + PRODUCT_ALIVE_SECS + CLOSE_SECS + 20);
        // And the growth this slice buys is bounded by the cycle's own two children, which
        // is the arithmetic the brief's "< 40s" was written against: two legs, each
        // sighting + a 4s sit + two settle polls + the close.
        assert!(
            2 * (PRODUCT_CYCLE_ALIVE_SECS as i64 + 2 * PRODUCT_CYCLE_SETTLE_SECS as i64) < 40,
            "the cycle's expected growth outgrew its brief"
        );
        // The show-state test is equality against the iconic value, not a flag bit.
        assert_eq!(SW_SHOWMAXIMIZED, 3);
    }

    /// S9b ITEM 5, the half that needs no desktop: the judge. The trap this leg fell into
    /// once was reading "nothing landed while off" as a verdict when the caret had never
    /// been fed at all - which is NOT JUDGED, not a broken toggle.
    #[test]
    fn the_autosave_toggle_judge_needs_a_landed_baseline_before_it_accuses_anything() {
        assert_eq!(judge_toggle(true, true, false, true), Toggle::Held);
        // Text on the disk while the switch was supposed to be off: it did not disarm.
        assert!(matches!(
            judge_toggle(true, true, true, true),
            Toggle::Broken(_)
        ));
        assert!(matches!(
            judge_toggle(true, true, true, false),
            Toggle::Broken(_)
        ));
        // Held while off and never landed after the way back: it did not re-arm.
        assert!(matches!(
            judge_toggle(true, true, false, false),
            Toggle::Broken(_)
        ));
        // The two declines. No foreground means no key was ever sent; no baseline means
        // the caret was never fed - either way the switch was not tested, so it is not
        // accused. A decline outranks every reading, including a loud-looking one.
        assert!(matches!(
            judge_toggle(false, false, false, false),
            Toggle::NotJudged(_)
        ));
        assert!(matches!(
            judge_toggle(true, false, false, false),
            Toggle::NotJudged(_)
        ));
        assert!(matches!(
            judge_toggle(false, true, true, true),
            Toggle::NotJudged(_)
        ));
    }

    /// The other half, and the reason the recipe is spelled out in the script: the probe
    /// carries the doors the leg presses and prints the keys the judge reads. Asserted
    /// against the script text for the same reason the geometry needles are - the
    /// alternative is a desktop run.
    #[test]
    fn the_product_probe_carries_the_chord_recipe_and_the_toggle_keys() {
        for needle in [
            "public static extern void keybd_event",
            "public static extern uint MapVirtualKeyW",
            "public static extern bool AttachThreadInput",
            "public static extern bool DetachThreadInput",
            "public static extern uint GetCurrentThreadId",
            "public static extern uint GetWindowThreadProcessId",
            "public static extern bool SetCursorPos",
            "public static extern void mouse_event",
            "public static extern bool GetWindowRect",
            "$Maximise -eq 3",
            "FOREGROUND=$([int]$fgOk)",
            "BASELINE_LANDED=$baseline",
            "_BYTES=",
            "_HAS_ALPHA=",
            "_HAS_BETA=",
            "_HAS_CHARLIE=",
            "Report 'A1'",
            "Report 'A2'",
            "Report 'A3'",
            "RECT_TRIES=",
            "untitled.notes",
            "_HAS_ALPHA=",
        ] {
            assert!(
                PRODUCT_PROBE.contains(needle),
                "the product probe no longer carries {needle:?}, and  product_autosave_toggle needs it"
            );
        }
        let leg = run_product_leg_text();
        assert!(
            leg.contains("product_autosave_toggle(&script, exe, &session)"),
            "{leg}"
        );
        assert!(leg.contains("gaps.is_empty() && !cycle_broke"));
        assert!(leg.contains("SMOKE FAIL: autosave-toggle"));
        assert!(leg.contains("autosave-toggle: off-held on-landed"));
        // And the draft is put back, in both shapes: a write path and a delete path.
        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/smoke.rs"))
            .expect("smoke.rs is readable from its tests");
        assert!(src.contains("fs::write(path, b)"));
        assert!(src.contains("fs::remove_file(path)"));
    }

    /// What this slice did NOT do, said in the suite so a green row is never read as item
    /// 4 having shipped: the native Open / Save-As dialog stays un-asserted.
    #[test]
    fn item_4_the_native_dialog_is_still_unasserted_and_not_stubbed() {
        assert!(!PRODUCT_PROBE.contains("UIAutomation"));
        assert!(!PRODUCT_PROBE.contains("System.Windows.Forms"));
        assert!(!PRODUCT_PROBE.contains("UIAutomation"));
        assert!(!PRODUCT_PROBE.contains("ValuePattern"));
    }
    /// The reason this slice exists as its own ticket: code 6 had no producer on the leg
    /// that runs by default. It is not enough that 6 is IN the table - ci.yml arms the
    /// table - the product leg has to be able to RETURN it, with no flag, no needle
    /// schedule and no gpui anywhere near the call. Asserted against the leg's own text
    /// because the alternative is a desktop run, and a test that needs a window cannot
    /// live in the suite that runs headless.
    #[test]
    fn the_product_leg_has_a_producer_for_exit_six_and_it_is_the_cycle() {
        let leg = run_product_leg_text();
        // 1. the leg runs the cycle, only off a clean plain launch, and only on these bytes
        assert!(
            leg.contains("product_maximised_cycle(&script, exe, &session)"),
            "{leg}"
        );
        assert!(
            leg.contains("if gaps.is_empty()"),
            "the cycle must not run on a launch that already failed - a second finding about an app that never started is noise, and 15s of it"
        );
        // 2. a broken cycle OUTRANKS the generic step-fail, which is the whole semantics of 6
        let at = leg
            .find("if cycle_broke")
            .expect("no cycle_broke branch in the product leg");
        let tail = &leg[at..];
        assert!(tail.contains("return GEOMETRY_FAILED_EXIT"), "{tail}");
        assert!(
            tail.find("return GEOMETRY_FAILED_EXIT") < tail.find("STEP_FAILED_EXIT"),
            "6 must be chosen before the fallback 1, or a geometry failure is reported as a shutdown failure: {tail}"
        );
        // 3. and 6 is still the same single row in the table: this leg gained a producer,
        // not a code - ci.yml arms exactly contract_codes(), and a new row would need a CI
        // edit that no one asked for.
        assert_eq!(
            contract_codes()
                .iter()
                .filter(|c| **c == GEOMETRY_FAILED_EXIT)
                .count(),
            1
        );
        assert_eq!(GEOMETRY_FAILED_EXIT, 6);
    }

    // ---- THE MENU LEGS ----------------------------------------------------------------
    // The counting rule first, because every one of the four verdicts below is a count, and
    // a count that credits an UNVOICED line is a leg passing on the harness's own echo.
    #[test]
    fn a_needle_only_counts_when_the_product_said_it() {
        let trace = "notes-gpui: menu: autosave-row toggled true->false\n\
menu: autosave-row toggled true->false (echoed, no voice)\n\
notes-gpui: dialog[skipped]: SLINT_NO_DIALOG - (Open -> x)\n";
        assert_eq!(
            voice_count(trace, "menu: autosave-row toggled"),
            1,
            "the unvoiced line is somebody else saying the words"
        );
        assert_eq!(voice_count(trace, "dialog[skipped]: SLINT_NO_DIALOG"), 1);
        assert_eq!(voice_count(trace, "dialog: spawning the"), 0);
        assert!(voice_toggled(trace));
        assert!(
            !voice_toggled("notes-gpui: menu: autosave-row toggled true->true"),
            "a handler that prints its own name without changing the bit did not toggle"
        );
    }

    #[test]
    fn a_press_reading_that_is_not_six_fields_is_no_answer() {
        assert_eq!(
            press_reading(Some("4299,228,1,197388,4299,228")),
            Some((4299, 228, true, 197388, 4299, 228))
        );
        assert_eq!(
            press_reading(Some("4299,228,0,7,-1,-1")),
            Some((4299, 228, false, 7, -1, -1))
        );
        // The landed pair is part of the SHAPE now: an answer that still carries only the
        // four fields the old script wrote is a press whose cursor was never read, and a
        // reading rule that guessed the missing half would be guessing at the one field
        // that tells a locked session from a deaf window.
        assert_eq!(press_reading(Some("4299,228,1,197388")), None);
        // The script's own "there was no client area to map" answer, and a missing key:
        // neither is a pass, and neither is the product's fault either.
        assert_eq!(press_reading(Some("none,0,0,0,-1,-1")), None);
        assert_eq!(press_reading(None), None);
    }

    /// THE SHARPENING, and it is the whole reason the landed pair exists: two runs that
    /// print the same silence have to be tellable apart from the reading alone. Both answers
    /// stay advisory - [soften] is allowed to explain a NOT JUDGED, never to create a FAIL,
    /// and a leg whose input DID land keeps its red however the cursor read.
    #[test]
    fn the_landed_pixel_says_which_silence_this_run_measured() {
        // Where this box sat: WindowFromPoint answered our HWND, the button went down, and
        // the pointer stayed where it was.
        let stuck = press_reading(Some("4299,228,1,197388,640,320"));
        let deaf = press_reading(Some("4299,228,1,197388,4299,228"));
        assert_eq!(
            cursor_of(stuck),
            CursorSaid::NeverMoved {
                asked: (4299, 228),
                landed: (640, 320)
            }
        );
        assert_eq!(cursor_of(deaf), CursorSaid::Moved { at: (4299, 228) });
        // Zero prints, and the cursor never moved: the session is the honest suspect.
        let stuck_said = soften(judge_click_toggle(0, false, true), false, cursor_of(stuck));
        let text = format!("{stuck_said}");
        assert!(matches!(stuck_said, Menu::NotJudged(_)));
        assert!(
            text.contains("THE OS NEVER MOVED THE CURSOR (session locked/background?)"),
            "the exculpatory half of the distinction: {text}"
        );
        // Zero prints, and the cursor went exactly where it was told: the window is the
        // suspect, and the sentence says so - while the verdict itself stays NOT JUDGED,
        // because the chord leg still has not landed a keystroke on this run.
        let moved_said = soften(judge_dialog_asked(0, true), false, cursor_of(deaf));
        let text = format!("{moved_said}");
        assert!(matches!(moved_said, Menu::NotJudged(_)));
        assert!(
            text.contains("cursor moved, window ignored the press"),
            "the indicting half of the distinction: {text}"
        );
        // NEVER a promotion: a red leg with input live stays red even when the cursor read
        // is the sympathetic one, and the sentence is not bolted onto it.
        let stays_red = soften(judge_click_toggle(0, false, true), true, cursor_of(stuck));
        assert!(matches!(stays_red, Menu::Broken(_)));
        assert!(!format!("{stays_red}").contains("NEVER MOVED THE CURSOR"));
        // A press whose cursor was never read says nothing about the cursor - the -1,-1 the
        // script writes when it declined to move is not a position, in either direction.
        let unread = press_reading(Some("4299,228,0,7,-1,-1"));
        assert_eq!(cursor_of(unread), CursorSaid::Unknown);
        let blind = soften(judge_click_toggle(3, true, false), true, cursor_of(unread));
        assert!(!format!("{blind}").contains("cursor"));
        assert_eq!(cursor_of(None).shown(), "not read");
        // And the gesture leg takes the worst of its three presses, not the last one.
        let worst = cursor_said_all([
            (4299, 228, true, 7, 4299, 228),
            (4299, 268, true, 7, 640, 320),
        ]);
        assert!(matches!(worst, CursorSaid::NeverMoved { .. }));
    }

    #[test]
    fn a_click_that_reached_the_row_is_held_and_one_that_did_not_is_red() {
        assert!(matches!(
            judge_click_toggle(1, true, true),
            Menu::Held { .. }
        ));
        let broke = judge_click_toggle(0, false, true);
        assert!(matches!(broke, Menu::Broken(_)));
        assert!(
            format!("{}", broke).contains("THE MENU IGNORED A REAL CLICK"),
            "the headline of the leg is the sentence a reader scans for"
        );
        assert!(matches!(
            judge_click_toggle(1, false, true),
            Menu::Broken(_)
        ));
        assert!(
            matches!(judge_click_toggle(3, true, false), Menu::NotJudged(_)),
            "a press that WindowFromPoint says was not ours proves nothing about the menu"
        );
    }

    #[test]
    fn the_refusal_line_is_what_proves_the_open_row_was_hit() {
        assert!(matches!(judge_dialog_asked(1, true), Menu::Held { .. }));
        let broke = judge_dialog_asked(0, true);
        assert!(matches!(broke, Menu::Broken(_)));
        assert!(
            format!("{}", broke).contains("row 0"),
            "the gap has to name the geometry it distrusts"
        );
        assert!(matches!(judge_dialog_asked(0, false), Menu::NotJudged(_)));
    }

    /// ADR-0006's item 4, in six lines: the ORDER of the four readings is the finding, so
    /// each failure mode has to name itself and not the one before it.
    #[test]
    fn item_four_reports_the_earliest_thing_that_went_wrong() {
        assert!(
            format!("{}", judge_native_dialog(0, false, false, false, 2000))
                .contains("NEVER ASKED FOR A DIALOG")
        );
        let no_window = judge_native_dialog(1, false, false, true, 2001);
        assert!(matches!(no_window, Menu::Broken(_)));
        assert!(
            format!("{}", no_window).contains("ITEM 4 STILL OWED"),
            "asked-but-no-modal is the gap the ADR is about, and it must be named"
        );
        assert!(
            format!("{}", judge_native_dialog(1, true, false, true, 120))
                .contains("WOULD NOT CLOSE")
        );
        assert!(
            format!("{}", judge_native_dialog(1, true, true, false, 120)).contains("TOOK THE LOOP")
        );
        assert!(matches!(
            judge_native_dialog(1, true, true, true, 120),
            Menu::Held { .. }
        ));
    }

    /// Leg D's three outcomes, all three meaningful - which is why this shape was chosen
    /// over screenshotting the popup: 1 = closed by the drag, 2 = the promise broken, 0 =
    /// the instrument blind. Only the middle one is the product failing.
    #[test]
    fn one_ask_is_the_drag_having_closed_the_menu() {
        assert!(matches!(judge_drag_closes(1, true), Menu::Held { .. }));
        assert!(
            format!("{}", judge_drag_closes(2, true)).contains("THE DRAG DID NOT CLOSE THE MENU")
        );
        let blind = judge_drag_closes(0, true);
        assert!(matches!(blind, Menu::Broken(_)));
        assert!(
            format!("{}", blind).contains("NEITHER PRESS ASKED"),
            "zero is an instrument finding and is printed as one, never as a pass"
        );
        assert!(matches!(judge_drag_closes(1, false), Menu::NotJudged(_)));
    }

    #[test]
    fn the_script_clicks_where_chrome_draws_and_walks_for_the_dialog() {
        // The coordinates are the markup's, the press is the OS's, and the dialog is looked
        // for by class name AND owner pid - all of it has to be in the shipped script.
        for shape in [
            "WindowFromPoint",
            // The press line carries where the pointer ACTUALLY landed, so the door that
            // reads it has to be in the shipped script and not just in the Rust.
            "GetCursorPos",
            "$HAMBURGER_X",
            "function Get-RowY",
            "function Drag-Band",
            "'#32770'",
            "EnumWindows",
            "DIALOG_GONE",
            "MAIN_ALIVE",
            "elseif ($Maximise -eq 6)",
        ] {
            assert!(
                PRODUCT_PROBE.contains(shape),
                "PRODUCT_PROBE lost a menu-leg shape: {shape}"
            );
        }
        // The dialog gate is a property of the MODE, not of whatever the shell happened to hold.
        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/smoke.rs"))
            .expect("smoke.rs must be readable from its own test");
        for shape in [
            "cmd.env(\"SLINT_NO_DIALOG\", \"1\")",
            "cmd.env_remove(\"SLINT_NO_DIALOG\")",
            "4 | 5 | 7 =>",
            "fn product_menu_legs",
        ] {
            assert!(
                src.contains(shape),
                "the spawn lost a menu-leg shape: {shape}"
            );
        }
    }

    #[test]
    fn the_product_leg_runs_the_menu_after_the_cycle_and_before_its_verdict() {
        let leg = run_product_leg_text();
        // The flag is the whole honesty of the four legs, so it is asserted at the call: a
        // menu leg that went red on a machine where NO injected input arrives would send a
        // reader to chrome.slint to fix a click that was never delivered.
        assert!(leg.contains(
            "product_menu_legs(&script, exe, &session, matches!(toggle, Some(Toggle::Held)))"
        ));
        assert!(leg.contains("gaps.is_empty() && !cycle_broke"));
        let at_menu = leg
            .find("SMOKE {name} FAIL")
            .expect("the menu legs must print their own headline");
        let at_elapsed = leg
            .find("let elapsed = started.elapsed()")
            .expect("the leg's verdict block moved");
        assert!(
            at_menu < at_elapsed,
            "a menu finding raised after the verdict is printed would never reach the exit code"
        );
    }

    /// The leg's own source, as text, for the shape asserts above. Read at test time from
    /// this file - the same trick geometry uses for its script, and it fails loudly (an
    /// expect with the path in it) rather than vacuously if the file ever moves.
    fn run_product_leg_text() -> String {
        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/smoke.rs"))
            .expect("smoke.rs must be readable from its own test");
        let start = src
            .find("fn run_product_leg(")
            .expect("run_product_leg vanished from smoke.rs");
        let rest = &src[start..];
        let end = rest
            .find(
                "
/// One-line description of whatever already sits on the session path.",
            )
            .expect("the end of run_product_leg is no longer where the product tests read it");
        rest[..end].to_string()
    }
}
