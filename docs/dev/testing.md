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

Unit and integration tests for all five workspace members. The port's tests are
headless by design: `crates/api/tests/support/host_mock.rs` stands in for the host,
so session, debounce, reentrancy and geometry behaviour are tested with no window at
all. The bridge's two test binaries (`ime_seam.rs`, `paint_cost.rs`) are also
windowless — the crate has no lib target, so they compile `editor.rs` by path and
measure its logic with no window and no GPU. They run on any machine.

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
