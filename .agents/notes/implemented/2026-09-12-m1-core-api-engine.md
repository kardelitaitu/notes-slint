---
title: M1 — the headless core and api engine
status: implemented
id: 2026-09-12-m1-core-api-engine
created: 2026-09-12
updated: 2026-09-12
relates: [§4.5, §5.4]
decision: null
---

## Question

Can the product's whole brain — document model, save engine, session and settings, the
Command/Event gateway — exist and be *proven* with no window, no toolkit and no OS types?

## Options

### A. Prove it through a mock engine

Fast and hermetic, but it proves the mock. A behaviour only visible from inside the crate
is not a behaviour a bridge can rely on.

### B. Real engine, real files, everything through the port

Slower to set up; every assertion is an `Event` or bytes on disk — exactly what a bridge
actually receives.

## Recommendation

**B — and it is built.** `crates/api/tests/session.rs` plays whole user sessions with no
window, no toolkit and no mock engine: real files on disk, real events out of the port,
nothing but `Gateway::send` and the `EventRx` the caller was handed.

## What was built

- `crates/core` — pure Rust, no gpui, no windows crate, no platform, no unsafe: the
  document model (`document.rs`), encoding and line-ending detection and preservation
  (`encoding.rs`, `format.rs`), the atomic save engine (`save.rs`), session and settings
  serialisation (`session.rs`, `settings.rs`), state-dir resolution (`paths.rs`), path
  hazard policy (`path_policy.rs`), the recent-files MRU (`recent.rs`), geometry types
  (`geometry.rs`).
- `crates/api` — the port (§5.4): `command.rs`, `event.rs`, the engine loop (`engine.rs`),
  `gateway.rs` (`Gateway::send` / `EventRx`), `dto.rs`. Commands in, events out; no UI
  types; it knows no bridge exists.
- `crates/xtask` — generates and hash-checks the round-trip fixture corpus the do-no-harm
  gate consumes.

## Tested

- `crates/api/tests/session.rs` — the headless session proof roadmap §9 asks of M1:
  open/save/save-as with path rebinding, the ADR-0001 arming scenarios, a session restored
  across a restart, and arming never leaking from one document to the next. Every byte
  comparison is a byte slice, because a String comparison would pass on a file whose line
  endings had been rewritten.
- `crates/core/tests/` — `roundtrip.rs` (the §4.5 gate: 30 manifest fixtures round-trip
  byte-exactly), `crash_safety.rs`, `save_hazards.rs`, `path_hazards.rs`,
  `recents_identity.rs`, `purity.rs`.
- `crates/api/tests/` — `public_surface.rs`, `read_policy_parity.rs`, `reentrancy.rs`
  (the engine loop traps reentrancy), `hang.rs` (a slow consumer never blocks the
  producer).

## Consequences

- A bridge can be written against `Command`/`Event` alone; the engine's behaviour was
  pinned before any UI existed, and M2 is building on exactly that.
- The fixture corpus is manifest-driven (`crates/core/tests/fixtures/manifest.json`): a
  fixture added there becomes a checked case without editing `roundtrip.rs`.
- Cost accepted: going through the port makes the tests integration-shaped, and the
  engine's 750 ms cadence bounds how fast some assertions can run.

## Reopening conditions

If a bridge ever needs a synchronous, `Result`-returning save path, that is not an M1
change — it reopens the async-autosave invariant in AGENTS.md and deserves its own note.
