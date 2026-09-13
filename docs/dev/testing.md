# Testing

What the gate runs, and what each command proves. The same commands appear as a
pre-flight checklist in [`AGENTS.md`](../../AGENTS.md) ("Before reporting work
done"); this file is the prose. `.cargo/config.toml` defines local aliases for
the long forms: `cargo gate`, `cargo arch`, and `cargo xtask`.

## The commands

```sh
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
cargo run -p xtask -- check-arch
cargo run -p xtask -- fixtures verify
```

### `cargo test --workspace`

Unit and integration tests for all six workspace members. The port's tests are
headless by design: `crates/api/tests/support/host_mock.rs` stands in for the host,
so session, debounce, reentrancy and geometry behaviour are tested with no window at
all. The bridge's two test binaries (`ime_seam.rs`, `paint_cost.rs`) are also
windowless — the crate has no lib target, so they compile `editor.rs` by path and
measure its logic with no window and no GPU. They run on any machine.

The Slint bridge is **two bins over one package**, so `cargo test -p
notes-bridge-slint` runs **two test roots** and prints two totals: the probe root
(`src/probe.rs`) reported `34 passed`, and the product root (`src/product.rs`) reported
`14 passed`. The arithmetic that matters: those are not 48 facts about the crate.
`surface.rs` (4 tests) and `title_contract.rs` (8) are shared modules compiled into both
bins, so 12 of the 34 run twice; the 22 `chords::*` guards exist only in the probe root,
and only 2 (`the_drag…`, `the_panic_note…`) are the product's own. 34 + 14 executions =
36 distinct tests. A green probe root therefore says nothing about the product root, and a
contributor reading one number has read half the crate.

CI wraps a bounded variant, the `cargo gate` alias:
`cargo test --locked --workspace --exclude notes-bridge-gpui`, 15-minute timeout.
`--locked` makes a stale `Cargo.lock` an error before compilation instead of a
silent mid-run rewrite; the bound exists because a hang must cost 15 minutes with a
named step, not the job's whole timeout with no suspect.

### `cargo clippy --all-targets -- -D warnings`

The lint gate. CI runs it in two named steps — the workspace excluding
`notes-bridge-gpui`, then the bridge alone, same `-D warnings` — so an upstream
toolkit deprecation cannot blur which crate went red. Locally, the single command
above is the same standard.

### `cargo fmt --all -- --check`

Rustfmt defaults. `--all` covers every workspace member, bridge included. It
resolves no dependency graph, so it cannot touch `Cargo.lock`.

### `cargo run -p xtask -- check-arch`

The per-crate extern-rule gate: it reads `cargo metadata` and enforces the layering
rules in `crates/xtask/src/arch.rs` (`RULES`). Exit 0 is clean; exit 1 is at least
one violation, each named with the edge that broke it; exit 2 means the check itself
could not run (e.g. `cargo metadata` failed) — a red step is never a silent skip.

It exists because the six `cargo tree -p X -i Y` probes in `AGENTS.md` are wrong
as a gate: `-i` is transitive, so a *correct* bridge → api → core layering prints a
tree for `cargo tree -p notes-bridge-gpui -i notes-core` and "fails", and `-i
windows` is ambiguous while the graph carries two `windows` versions at once.
check-arch evaluates the raw dependency graph instead: direct checks read
`packages[].dependencies` (normal, dev, and build edges), transitive rules follow
normal and build edges only, repo-crate rules are structural against
`workspace_members` at runtime (a crate added tomorrow is already forbidden where
the rules say "nothing else in this repo"), families match by root-and-prefix so an
upstream rename cannot switch a rule off, and metadata runs with `--all-features`
so a feature-gated edge counts. Its stated limit: it sees edges and names only — a
hand-written `extern` block is invisible; the backstop for that is
`cargo xtask check-unsafe`.

### `cargo run -p xtask -- fixtures verify`

Checks the byte-exact round-trip fixture corpus against its sha256 manifest (exits 1
on any difference; the generator comparison is always on).

## The smoke harness: one flag, three artifacts

Everything above is headless. `cargo xtask smoke` is the one end-to-end GUI check: it
launches a real exe, waits for a real window, closes it with `WM_CLOSE` the way the
title-bar X does, and proves the process exits by itself. It is **advisory** — a
headless runner may have no desktop at all — so `cargo xtask check` reports its verdict
and `--quick` skips it; it is never a blocking gate step.

`--binary` names which artifact the harness resolves, builds, staleness-checks and, where
a leg is wired, judges. Both shapes parse (`--binary=slint`, `--binary slint`); the
default is `gpui`, which is what CI and `cargo xtask check` run (`crates/xtask/src/smoke.rs`,
`BINARY_NAMES`). One package, two bins, two contracts: which is why the flag picks an
**artifact**, not a package.

| `--binary` | exe | what the harness does with it |
|---|---|---|
| `gpui` | `target\debug\notes-gpui.exe` | judged — the needle schedule (`Leg::GpuiSchedule`) |
| `slint` | `target\debug\notes-slint.exe` | judged — the product contract (`Leg::Product`) |
| `slint-probe` | `target\debug\notes-slint-probe.exe` | path named, then declined — **not built, not launched, not judged** (`Leg::NotWired`) |

An unlisted value is refused, naming the legal list, before anything is built.

### The Product leg, and what its green does not buy

`--binary=slint` runs a **different contract**, not a shorter version of the gpui one. It
reads the startup lines off the product's own piped stderr, asserts the window is still
there at 45 s (nothing in the product self-hides, so "alive" is assertable rather than
inferred), and sends a `WM_CLOSE` the app must answer by exiting 0 on its own. The pass
line is the disclaimer, in the same breath:

> `smoke:   what this proves: notes-slint said its startup lines, was still on screen at
> 45s, took a WM_CLOSE, said the close and the joined shutdown, and left with 0 by
> itself. What it does NOT prove: the rect, the pin, the recents trace, or the maximised
> cycle - those are the needle schedule's claims, and this leg does not run it.`

So a green `--binary=slint` is proof of **launch, presence and an honest shutdown** for the
shipping exe — and it says so rather than leaving the reader to assume the M3/M4 promises
came with it. The leg deliberately does not move the user's `session.json`, seed a recent,
read a rect or poll a pin.

### `slint-probe` declines, out loud

The instrumented build is never judged, and the harness prints why instead of leaving a
blank to be read as a pass. The decline is asked **before** the build step — after the exe
path is named, before anything is compiled, launched or measured — so the run costs nothing
and claims nothing. The reason lives in one function, `not_wired_reason`, and is printed
verbatim:

> `the only schedule this harness speaks is the gpui one, and pointing it at the
> instrumented probe bytes is a port, not a flag: its needles are the same lines
> the probe leg would have to re-decide, so that leg is its own slice of work`

(The runs of spaces are the string's own — it is a continued literal, pasted as printed.)
Alongside it the run prints `legs wired today: [gpui, slint]` and states that it launched
nothing, so there is **no verdict about `slint-probe` in either direction**.

### 2 is the harness; 3 is the machine

Both of those exits stop short of a verdict, and the difference is what a reader acts on.

* **2 — the harness could not run.** No PowerShell to post `WM_CLOSE`, no exe, the probe
  script could not be written, freshness could not be read, the outer deadline fired, a bad
  invocation of the flag — and the `slint-probe` decline above. The missing leg is this
  repo's, not the desktop's; a box with a perfect window station answers the same 2. That is
  exactly why an unwired leg is **not** a 3: 3 claims something about the machine that 2 is
  not entitled to claim. Nothing was launched either way, so no verdict about the app exists.
* **3 — this machine cannot host the run.** No interactive desktop, no window handle even
  though the app kept running, or foreign state sitting on the session path. It is not the
  app's fault and never becomes a red against it; `cargo xtask check` reports a 3 as
  declined, never as passed.

The verdict roster is 0, 1, 2, 3, 4, 5, 6, 7 and 9 — each run prints it as its first line,
`smoke: contract 0 1 2 3 4 5 6 7 9`, built from the same `CONTRACT` table `check-ci`
asserts `ci.yml` branches on. 4 (the target did not compile) and 5 (the exe is older than
its sources) are the two that mean "nothing about the app was measured today". One code is
deliberately **not** in that table: 8, the foreign-win32-manifest verdict, which exists only
behind the opt-in `--require-ours` and stays a WARN line without the flag.

## The six architecture invariants, as enforced checks

Each invariant from `AGENTS.md` ("Architecture invariants") is enforced by a rule in
`crates/xtask/src/arch.rs`, run by `check-arch`:

| Invariant (`AGENTS.md`) | Enforced by |
|---|---|
| `core` no `gpui` | `core-is-pure` — UI-toolkit family, transitive |
| `core` no `windows` | `core-no-os` — Win32/browser FFI family, transitive |
| `core` no `platform` | `core-orthogonal-to-platform` — structural: no member but itself anywhere in its closure |
| `api` no `gpui` | `port-is-ui-agnostic` — toolkit family, transitive; the OS-FFI family is policed direct-only, since `windows` reaches `api` through `notes-platform` by design |
| `bridge-gpui` no `core` | `bridge-sees-only-api` — structural, direct: every member except `notes-api` is forbidden (its transitive payload contains core *by design*) |
| `bridge-gpui` no `platform` | `bridge-sees-only-api` — the same rule |

check-arch enforces more than the six: `notes-platform` may not reach any other
member, `notes-api` joins core and platform and nothing else
(`no-new-backdoor`), the platform crate may not import a toolkit, and `xtask`
must not depend on what it checks (`checker-is-independent`).

## Do no harm: the round-trip fixtures

The product promise (`AGENTS.md`, "Do no harm"): loading then saving a foreign file
must be **byte-identical** — encoding, BOM, line endings, and trailing newline all
preserved. Never "normalise" a file to be tidy; that is the exact bug this gate
exists for.

The gate is `crates/core/tests/roundtrip.rs`, driven by
`crates/core/tests/fixtures/manifest.json` — a corpus of byte-exact fixtures
covering UTF-8 with and without BOM, UTF-16LE and UTF-16BE, ANSI-1252, each crossed
with LF/CRLF and with/without a trailing newline, plus edge files (empty, BOM-only,
lone CR, mixed EOLs, a frontmatter case). A fixture added to the manifest becomes a
checked case without editing the test. The manifest must exist and be non-empty: a
missing manifest fails the tests, because a skipped gate looks identical to a green
one. Verify the corpus (and its hashes) with
`cargo run -p xtask -- fixtures verify`.

## One line more

`cargo xtask check` runs the whole gate with per-step verdicts, and
`cargo xtask check-ci` proves the CI workflow's step roster has not drifted from
this list. Use them to check the checks.
