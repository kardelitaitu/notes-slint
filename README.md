# notes-gpui

A small, fast desktop note app that never asks you to save, always comes back the size and
in the place you left it, and has one button to pin itself above every other window.

**Not a knowledge base.** No sync, no accounts, no plugin ecosystem, no graph view. It is
the window you throw your thoughts into — the bet is that the *window behaviour* is the
product, and the editing surface can stay deliberately minimal.

> **Version 0.0.1** · engine built, first UI in flight
>
> The **M0 spike ran on 2026-09-10** and answered yes — GPUI on Windows is viable for a
> standalone app (results: [docs/roadmap.md](docs/roadmap.md) §12). M1 (headless engine),
> M3 (window persistence) and M4 (autosave, pin, recents) are built and tested — what
> works, and how, is recorded in [`.agents/notes/implemented/`](.agents/notes/implemented/).
> M2, the first usable UI, is in flight.

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
whitepaper.md      the index: what the plan set covers, and where each part lives  <-- start here
AGENTS.md          conventions and invariants for anyone working on this repo

docs/              planning & developer docs  (intent: what we mean to build)
  dev/             how to build/test/package - written once code exists
  decisions/       settled architecture decisions (ADR) - append-only

.agents/           decision & implementation records  (outcomes: what happened)
  notes/           proposed / implemented / rejected / archived
  skills/          agent capabilities (doc-management, + its placement validator)
```

Everything about *why* the project is shaped the way it is lives in the plan set indexed
by [`whitepaper.md`](whitepaper.md) — one owner file per numbered section, on purpose; the
placement validator fails the build when two files claim the same section.

## Roadmap

| | |
|---|---|
| **M0** | **Spike — done 2026-09-10.** Does a standalone GPUI app build and run on Windows, and can we set topmost, set position, and open a native file dialog? Verdict: viable, every blocking question passed ([docs/roadmap.md](docs/roadmap.md) §12). |
| M1 | **Done.** Core engine, headless. Tested with no window at all. |
| M2 | **In flight.** First usable UI — *you can use it as a notepad.* |
| M3 | **Done.** Window persistence. |
| M4 | **Done.** Autosave and pin. |
| M5 | Polish, and the Windows portable + installer builds. |
| M6–M8 | Cross-platform seams, then mac and Linux builds. |

Detail: [docs/roadmap.md](docs/roadmap.md) §9. Ship targets: `win-install`, `win-portable`, `linux-install`,
`linux-portable`, `mac-install` (whitepaper §7).

## Questions we have not answered

Tracked in §10 ([docs/open-questions.md](docs/open-questions.md)), and worked through one
at a time in [`.agents/notes/proposed/`](.agents/notes/proposed/) — which is empty right
now. The question that used to block M1, autosave on files the app did not create, is
settled: [ADR-0001](docs/decisions/0001-autosave-arms-on-explicit-save.md) arms autosave
only after an explicit save on a foreign file, and M4 implements exactly that.

## Contributing

A Rust workspace builds here: `cargo build` from the repo root. The full pre-report gate —
fmt, clippy, tests, the layering gate (`cargo xtask check-arch`), the docs validator — is
in [`AGENTS.md`](AGENTS.md); read it first anyway. It covers the invariants that are
enforced and the things deliberately ruled out, which is the fastest way to avoid proposing
something the project has already considered.

## License

Not yet chosen. Until it is, all rights reserved.
