# AGENTS.md

Conventions and invariants for working on this repository — for AI agents and humans alike.
This file is about **how** to work here. The *what* and *why* are in
[`whitepaper.md`](whitepaper.md).

## Read in this order

1. [`whitepaper.md`](whitepaper.md) — the index; the plan itself lives in `docs/`
   (§4 features, §5 architecture, §8 risks, §10 open decisions).
2. This file.
3. [`.agents/skills/doc-management/SKILL.md`](.agents/skills/doc-management/SKILL.md) — before
   creating or moving **any** markdown file.
4. [`.agents/notes/`](.agents/notes/) — especially `rejected/`, before proposing a feature.

## Current phase: M2 in flight

M0 ran on 2026-09-10 and came back **viable** — the results are recorded in
[`docs/roadmap.md`](docs/roadmap.md) §12, which owns §9 and §12 of the plan. Code exists:
M1 (the headless `core` + `api` engine), M3 (window persistence) and M4 (autosave, pin,
recents) are built and tested; what is demonstrably working, and where it diverged from the
plan, is recorded in [`.agents/notes/implemented/`](.agents/notes/implemented/). M2 — the
first usable `bridge-gpui` UI — is being built slice by slice on top of that engine.

The architecture invariants below are live, not aspirations: CI already runs the layering
gate (`cargo xtask check-arch` in `.github/workflows/ci.yml`), and it fails the build when
a boundary is crossed.

## Architecture invariants

These are not preferences. Each is enforced by CI, and each exists because violating it
breaks something the product depends on.

```
crates/core       pure Rust. No gpui, no windows crate, no platform, no unsafe.
crates/platform   OS primitives that take a window handle and decide nothing.
crates/api        the port. Commands in, Events out. No UI types, knows no bridge exists.
crates/bridge-*   one adapter per UI toolkit. bridge-gpui is the only one built.
```

```sh
cargo xtask check-arch   # the layering gate — exit 0 clean, 1 violation, 2 the check itself could not run
```

One command, six probes retired: `cargo tree -i` is transitive, so correct layering made
`bridge-gpui -i core` print a tree, and `-i windows` was ambiguous between the versions
platform and gpui each pull. `check-arch` reads `cargo metadata --all-features` and enforces
the diagram above per crate — by name *family* (`gpui*`, `windows*`) and, for repo crates,
structurally, so a new member or a reach-around through a non-member wrapper is caught with
no list to update. Rules and their stated limits: `crates/xtask/src/arch.rs`.

Rules that are easy to break politely:

- **A bridge may import `api` and its own toolkit. Nothing else in this repo.** Reaching
  around the port to fix a missing event is how the seam disappears. Add the event instead.
- **`api` contains no business logic.** It routes and translates. A rule living in
  `api/engine.rs` instead of `core/` is a silent architecture change.
- **The bridge owns the text editor and the window.** `api` never sees a keystroke, only
  `Flush { text }`. Text editing is the one thing that cannot be abstracted across
  toolkits (whitepaper §5.5).
- **Startup order is fixed** (whitepaper §5.5): query session → bridge creates window *at
  the saved rect* → register handle → apply topmost. Geometry *restore* is bridge work;
  geometry *storage* is core work.
- **Autosave is asynchronous.** A save failure has no caller to return `Err` to — it must
  arrive as `Event::SaveFailed`. Do not add a synchronous `Result`-returning save path
  just because it is convenient.
- **Do not design a `trait Bridge`** while GPUI is the only implementation. The seam is
  "api has no UI types", already enforced. An interface guessed from one implementation
  encodes that implementation's shape into what you call generic (whitepaper §8, R12).
- **Do no harm.** Loading then saving a foreign file must be byte-identical: preserve
  encoding, BOM, line endings, trailing newline (whitepaper §4.5). Round-trip fixtures in
  `crates/core/tests/fixtures/` gate this; never "normalise" a file to be tidy.

## Deliberate non-goals

Do not add these without a note in `.agents/notes/proposed/` first — several have been
considered and set aside (check `rejected/`):

Tabs or multiple documents per window · a notes database, index, or sidebar · sync or
accounts · collaboration · plugins · mobile · tags and backlinks · search across a corpus ·
a theming system · **rich text** (the single largest scope risk in the project).

An MSIX package is ruled out for a concrete technical reason (whitepaper §7), not taste.

## Documentation

There are **two trees, split by tense.** Never create a markdown file without consulting
the placement map in the `doc-management` skill. The short version:

`docs/` + `whitepaper.md` = **intent** (planning and developer docs).
`.agents/notes/` = **outcomes** (decision and implementation records).
When they disagree, the code wins — edit the doc, leave the note standing.

| It is… | Goes in |
|---|---|
| Product/architecture thinking | `whitepaper.md` — exactly one, living |
| How to build, test, package, or debug | `docs/dev/<topic>.md` — **only once the code exists** |
| A settled decision | `docs/decisions/NNNN-slug.md` — append-only, never edited |
| An idea under discussion | `.agents/notes/proposed/` |
| Decided against | `.agents/notes/rejected/` |
| Actually built | `.agents/notes/implemented/` |
| Superseded | `.agents/notes/archived/` |
| Repo conventions | `AGENTS.md` |

- A **note** is disposable working memory; an **ADR** is an immutable record. Never let one
  turn into the other in place.
- **A plan is not a dev doc.** Do not write `docs/dev/` about code that does not exist yet —
  promote a plan into `docs/dev/` when it is implemented, not when it is agreed. Today that
  means `docs/dev/` holds its README plus `getting-started.md` and `testing.md` — the two
  files whose code now exists.
- `implemented` means the code exists and works — **not** "we agreed to it".
- `status:` in frontmatter must match the folder.
- **Never fork the plan.** `whitepaper.md` is the index; the plan lives in the area docs
  under `docs/` that it indexes (`docs/roadmap.md` owns §9 and §12). Section numbers are
  global across the set with exactly one owner file each; the validator fails when two
  files claim the same §, or on a `whitepaper` filename anywhere but the root.
- `whitepaper.md` `§` numbering carries ~60 cross-references. If you insert a section,
  renumber **and** fix every inbound reference in the same edit.

## Code conventions

- `rustfmt` defaults; `cargo clippy --all-targets -- -D warnings` stays clean.
- `core`: no `unwrap()`/`expect()` on paths reachable from I/O — return typed errors. The
  `SaveError` enum is part of the public contract because the UI must render the reason.
- `unsafe` only in `platform`, always with a comment naming the API and the invariant.
- Never block on a channel inside a GPUI frame; never call into `api` from the save worker
  thread. Both deadlock, and only under load.
- Windows paths are case-insensitive but not case-preserving: canonicalise for identity
  (recent-files dedupe), keep the original for display.
- No network calls in `core`. Cold start is a budget (whitepaper §2), so no work-stealing
  initialisation, no eager font or index loading.

## Before reporting work done

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo xtask check-arch   # the layering gate — exit 0 clean, 1 violation, 2 the check itself could not run
pwsh .agents/skills/doc-management/scripts/check-docs.ps1
```

All of these apply now that code exists. A documentation-only change still owes the last
line (the validator) before it reports done.

## Working style

- **Raise problems, not just solutions.** Several of this project's most important
  constraints were found by asking what a nice-sounding requirement implied. If a decision
  looks wrong, say so with the reason — silent compliance on a bad plan is a cost, not a
  virtue.
- Prefer the boring implementation. Novelty belongs in the window behaviour, nowhere else.
- When a decision is genuinely 50/50, pick one, write it down as a note with reopening
  conditions, and move on. Unmade decisions are more expensive than wrong ones.
- Do not expand scope quietly. A "small" feature that needs a new `Command` variant and a
  settings key is not small; it is a design change, and it deserves a paragraph.
