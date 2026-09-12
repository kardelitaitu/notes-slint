---
title: M4 — autosave, pin, recents, and the byte-exact gate
status: implemented
id: 2026-09-12-m4-autosave-pin-recents
created: 2026-09-12
updated: 2026-09-12
relates: [§4.2, §4.3, §4.4, §4.5]
decision: null
---

## Question

How does the app never ask to save, never lose a save failure in silence, never rewrite a
foreign file the user did not explicitly save — and never tidy a file it opened?

## Options

### A. One synchronous save path and one global toggle

Simple — but a save failure has no caller to return `Err` to, and one toggle cannot mean
two things for the user's own notes and someone else's `package.json`.

### B. Async autosave, per-document arming (ADR-0001), typed skip events

Two modes where A had one, an event vocabulary so silence always has a reason, and the
failure path arriving as an event.

## Recommendation

**B — and it is built**, per
[ADR-0001](../../../docs/decisions/0001-autosave-arms-on-explicit-save.md). The armed flag
is per document; a skipped autosave always says why; a failed one arrives as
`Event::SaveFailed`.

## What was built

- **Debounce.** The engine's autosave cadence is 750 ms of quiet (`AUTOSAVE_IDLE`,
  `crates/api/src/engine.rs`). The bridge declares the same 750 ms itself rather than
  importing it — a deliberate duplication, documented where it is declared
  (`crates/bridge-gpui/src/main.rs`).
- **Arming (ADR-0001).** A `.notes` file autosaves immediately; a foreign file starts
  disarmed, reports `armed: false` in its `FileMeta`, and one explicit save or Save As
  arms it. Skips are typed and evaluated in one fixed order —
  `SkipReason::ForeignFileNotArmed` and friends — and `Event::AutosaveSkipped` carries
  the reason so the UI can render it. A brand-new scratch note is armed from birth,
  because the app created it.
- **Pin.** `Command::SetPinned` writes the pin bit to exactly one home, `session.json`;
  on window registration the port applies topmost from the stored bit, so a window never
  pops up unpinned; `set_topmost` reports a verdict read back from `WS_EX_TOPMOST`, not
  assumed.
- **Recents.** `crates/core/src/recent.rs` — an MRU capped at `MAX_RECENTS = 10` whose
  push/mark_missing/clear are pure over a `Vec`; identity is canonicalise (Windows:
  case-insensitive, full-Unicode lowercase, the verbatim prefix stripped) with the
  original path kept for display; the list lives in `settings.toml`.
- **Byte-exact gate.** `crates/core/tests/roundtrip.rs` — loading then saving a foreign
  file must be byte-identical: encoding, BOM, line endings, trailing newline (§4.5). It
  is driven by the 30-fixture `manifest.json` that `xtask` generates and hash-checks; a
  missing or empty manifest fails the gate rather than silently skipping it.

## Tested

- `crates/api/tests/debounce.rs` — the 750 ms cadence: fires once when quiet, re-arms
  after firing.
- `crates/api/tests/session.rs` — the ADR-0001 scenarios end to end, including that
  arming never leaks between documents.
- `crates/api/tests/first_run.rs` and `scratch_restart.rs` — what survives an unclean
  exit, and what the 750 ms tick bounds.
- `crates/core/tests/roundtrip.rs` and `recents_identity.rs`.

## Where it diverged

- The menu's global auto-save toggle and per-document arming are deliberately different
  questions (`crates/api/src/command.rs` carries the warning not to conflate them) — the
  consequence ADR-0001 accepted when it foreclosed one toggle meaning one thing for every
  file.

## Consequences

- Two behaviours where one was simpler, accepted by ADR-0001; `AutosaveSkipped` is
  load-bearing, because an unexplained silent no-save is worse than either mode.
- The fixture corpus is the contract: adding a manifest fixture extends the gate without
  code changes, and normalising a file is a test failure, not a tidy-up.

## Reopening conditions

Per ADR-0001: move to always-autosave if the arming step causes lost work or a misread
indicator; revisit shadow copies only if sync ever lands.
