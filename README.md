# notes-gpui

A small, fast desktop note app that never asks you to save, always comes back the size and
in the place you left it, and has one button to pin itself above every other window.

**Not a knowledge base.** No sync, no accounts, no plugin ecosystem, no graph view. It is
the window you throw your thoughts into — the bet is that the *window behaviour* is the
product, and the editing surface can stay deliberately minimal.

> **Version 0.0.1** · planning phase, no code yet
>
> This repository currently contains design documents only. Before any implementation work,
> **M0 (below) must run** — it exists to find out whether GPUI on Windows is viable at all
> for a standalone app, and its answer may change the UI layer entirely.

## What it does

| | |
|---|---|
| **Remembers the window** | Exact size and position on every launch — including when a monitor has been unplugged, scaling has changed, or the window was maximised. |
| **Autosaves** | Continuously, debounced, atomically. No save dialog and no "save changes?" prompt. Toggleable. |
| **Pins** | One click (plus a shortcut) puts it above every other window. State persists. |
| **Plays nicely with files** | Native `.notes` format, but opens ordinary text files too — and will not silently change their encoding or line endings. |
| **Menu** | Open, Save, Save As, auto-save toggle, recent files (up to 10). |

## Stack

- **Engine:** Rust — pure, headless, unit-testable, no UI or OS types in the core.
- **UI:** [GPUI](https://github.com/zed-industries/gpui), Zed's GPU-accelerated toolkit,
  isolated behind a bridge so the toolkit itself is a swappable detail.
- **Platform:** Windows first. macOS and Linux planned; the plan and its honest limits
  (Wayland restricts both window positioning and always-on-top) are in the whitepaper §6.

## Where things are

Two documentation trees, split by tense: **intent** lives in `docs/` and `whitepaper.md`;
**outcomes** live in `.agents/notes/`.

```
whitepaper.md      the founding sketch: product, architecture, risks, roadmap  <-- start here
AGENTS.md          conventions and invariants for anyone working on this repo

docs/              planning & developer docs  (intent: what we mean to build)
  dev/             how to build/test/package - written once code exists
  decisions/       settled architecture decisions (ADR) - append-only

.agents/           decision & implementation records  (outcomes: what happened)
  notes/           proposed / implemented / rejected / archived
  skills/          agent capabilities (doc-management, + its placement validator)
```

Everything about *why* the project is shaped the way it is lives in
[`whitepaper.md`](whitepaper.md). It is a single document on purpose — the placement
validator fails the build on a second copy.

## Roadmap

| | |
|---|---|
| **M0** | **Spike.** Does a standalone GPUI app build and run on Windows, and can we set topmost, set position, and open a native file dialog? Gates everything below. |
| M1 | Core engine, headless. Tested with no window at all. |
| M2 | First usable UI — *you can use it as a notepad.* |
| M3 | Window persistence. |
| M4 | Autosave and pin. |
| M5 | Polish, and the Windows portable + installer builds. |
| M6–M8 | Cross-platform seams, then mac and Linux builds. |

Detail: whitepaper §9. Ship targets: `win-install`, `win-portable`, `linux-install`,
`linux-portable`, `mac-install` (whitepaper §7).

## Questions we have not answered

Tracked in whitepaper §10, and worked through one at a time in
[`.agents/notes/proposed/`](.agents/notes/proposed/). The one that currently blocks M1
scaffolding: [autosave behaviour on files the app did not
create](.agents/notes/proposed/2026-09-10-autosave-foreign-files.md).

## Contributing

Nothing to build yet. When code lands, this section will hold the build and test commands;
until then, read [`AGENTS.md`](AGENTS.md) first — it covers the invariants that are already
decided and the things deliberately ruled out, which is the fastest way to avoid proposing
something the project has already considered.

## License

Not yet chosen. Until it is, all rights reserved.
