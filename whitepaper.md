# notes-gpui — Project Whitepaper

> **Version 0.0.1** — M0 spike complete and passed (see §12). M1 (headless engine), M3
> (window persistence) and M4 (autosave, pin, recents) are built and tested; M2, the first
> usable UI, is in flight.
> Working title: `notes-gpui`. Real name TBD. 0.0.1 is the target version; §7.4 explains
> what that number drives.

This file is the **index**. The plan itself is split by area into [`docs/`](docs/).

## 0. The map

Section numbers are **global across the set**, not per-file. A reference like `§4.5` means
the same thing from any file, and ~40 such references already exist — which is why the
numbering did not restart when the document was split. Each numbered section has exactly one
owner file; the validator fails if a number appears in two of them.

| § | Area | Owner file |
|---|---|---|
| §1 – §3 | What it is, why, v1 scope and non-goals | [`docs/product.md`](docs/product.md) |
| §4 | Core behaviour: persistence, autosave, pin, file model, `.notes` format | [`docs/features.md`](docs/features.md) |
| §5 | Architecture: crates, the `api` port, bridges, repo layout, docs model | [`docs/architecture.md`](docs/architecture.md) |
| §6 | Cross-platform: Windows first, then macOS + Linux, and Wayland's limits | [`docs/platforms.md`](docs/platforms.md) |
| §7 | Packaging: the five artifacts, the signing gates, CI build mechanics | [`docs/packaging.md`](docs/packaging.md) |
| §8 | Technical risks R1 – R14 | [`docs/risks.md`](docs/risks.md) |
| §9, §12 | Milestones M0 – M8, and the M0 spike results | [`docs/roadmap.md`](docs/roadmap.md) |
| §10 | Open decisions (stable numbering) | [`docs/open-questions.md`](docs/open-questions.md) |
| §11 | Next step | this file |

Alongside the plan set:

- [`docs/decisions/`](docs/decisions/) — ADRs. Settled decisions, append-only.
- [`docs/dev/`](docs/dev/) — developer docs. Empty by design until code exists.
- [`.agents/notes/`](.agents/notes/) — decision and implementation *records*, as opposed to
  this set's *intent*. See §5.6.
- [`AGENTS.md`](AGENTS.md) — conventions and the invariants that CI enforces.

## 0.1 In one paragraph

A small, fast desktop note app that never asks you to save, always returns to the size and
screen position you left it in, and pins itself above every other window at the press of a
button. The bet is that **the window behaviour is the product**; the editing surface stays
deliberately minimal. Rust engine, GPUI frontend behind a bridge, Windows first, shipping
five artifacts later. The single biggest risk — whether GPUI is viable for a standalone
Windows app at all — was retired on 2026-09-10; see §12.

## 11. Next step

**M0 is done and it passed** (§12): gpui 0.2.2 builds, opens a window, hands over its HWND,
and toggles `WS_EX_TOPMOST`. R1, R2, R3 and R9 are resolved. Proceed to **M1** — `core` and
`api`, headless, no UI.

Three things to carry into M1, in order of how much they cost to discover late:

1. **The coordinate-space trap** (§12.2). gpui bounds are client-space, Win32 frame rects
   are not, and the chrome is asymmetric. Decide the one persisted space in
   `platform/geometry.rs` *before* writing restore logic, and test across two launches, not
   one.
2. **ADR-0001 changes the autosave interface.** `.notes` autosaves immediately; a foreign
   file stays disarmed until one explicit save arms it, and `Event::AutosaveSkipped` must be
   rendered. The gate is `crates/core/src/document.rs` (`armed`, `should_autosave`, and
   `should_save_manual` for the hand-triggered act), the clock that fires it is
   `crates/api/src/engine.rs` (`AUTOSAVE_IDLE`, `on_tick`), and the events a bridge renders
   are declared in `crates/api/src/event.rs`; all of them were written knowing this. There is no
   `core/autosave.rs`, and the split is the rule rather than an accident — `core` decides,
   `api` routes and times.
3. **Two M0 checks are still open**: no dialog was actually presented, and no human looked
   at whether the painted frame is correct. Both are minutes. Do them before M2 is called
   done, not after.

Still parallel-path, because it is administrative and has a lead time we do not control: the
Apple Developer account (§10.8, §7.1).
