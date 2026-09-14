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
| **Remembers the window** | Exact size and position on every launch — including when a monitor has been unplugged or scaling has changed. A maximised window comes back maximised. |
| **Autosaves** | Continuously, debounced, atomically. No save dialog and no "save changes?" prompt. Toggleable. |
| **Pins** | One click (plus a shortcut) puts it above every other window. State persists. |
| **Plays nicely with files** | Native `.notes` format, but opens ordinary text files too — and will not silently change their encoding or line endings. |
| **Menu** | Open, Save, Save As, auto-save toggle, recent files (up to 10) with Clear. Because Windows stores the menu bar without drawing it, the commands reach the user as key chords too, and the startup legend prints exactly these rows in exactly this order: `Ctrl+O` Open · `Ctrl+S` Save · `Ctrl+Shift+S` Save As · `Ctrl+T` toggle auto-save · `Ctrl+Shift+R` clear recent files · `Alt+1`…`Alt+0` recent 1…10 (`Alt+0` is the tenth). One table generates all of it — `SHORTCUTS` in `crates/bridge-slint/src/surface.rs` — and docs/features.md §4.4 owns the command list it implements. |

## Stack

- **Engine:** Rust — pure, headless, unit-testable, no UI or OS types in the core.
- **UI:** [Slint](https://slint.dev) — it **ships**, and it is the product: `crates/bridge-slint`
  builds `notes-slint.exe`, beside the instrumented probe that proved the window contract
  (`notes-slint-probe.exe`). GPUI is becoming the past tense of this one: `crates/bridge-gpui` is
  **frozen** — it still builds, still runs, and still earns citation as evidence, but it takes no new
  features, and its terminal delete is governed by the six re-earnings in
  [ADR-0006](docs/decisions/0006-gpui-is-frozen-not-deleted.md). Same rule on both sides: one adapter
  per toolkit, so the toolkit stays a swappable detail ([record](.agents/notes/implemented/2026-09-14-strip-program.md),
  [how it got here](.agents/notes/implemented/2026-09-13-bridge-slint-spike.md)).
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
| M2 | **In flight on the Slint bridge.** First usable UI — *you can use it as a notepad*, and the app to run is `notes-slint.exe`. Its machine evidence is **one chord and no pointer**: `cargo xtask smoke --binary=slint` proves launch, presence at 45 s, an honest `WM_CLOSE` that ends in exit 0, the M9 restore-rect fixed point across a real maximise — close — relaunch — close, and the auto-save toggle round-tripped through a real injected `Ctrl+T` on the shipping exe — judged as bytes appearing and disappearing in the product's own draft file, not as a line the app chose to log ([docs/dev/testing.md](docs/dev/testing.md); when a precondition is missing the toggle prints NOT JUDGED rather than pretending). Everything about the menu answering a **mouse** is unproven: the four press legs (smoke modes 4–7) carry no recorded green run, and no instrument here has ever watched a pixel reach a handler ([`2026-09-15-no-instrument-tests-a-pointer`](.agents/notes/proposed/2026-09-15-no-instrument-tests-a-pointer.md)). **M2 pointer interaction is untested**, and that is a sentence about the instruments, not a report that the menu is broken or that the app is keyboard-only; a person with a mouse settles it in a minute. `bridge-gpui` is **frozen**, not deleted — still built, still cited by the 3-of-7 machine-proven ledger in [docs/roadmap.md](docs/roadmap.md) §9, no new needles and no loosening of rows, and its removal waits on [ADR-0006](docs/decisions/0006-gpui-is-frozen-not-deleted.md). What is still owed is the CI evidence trail (this repo has no remote, so no workflow has ever run) and the human eye-pass list — [detail](docs/roadmap.md) §9. |
| M3 | **Done.** Restore, monitor validation, DPI, and coming back maximised — the last one machine-proven, the smoke harness asserting the restore rect is unchanged across a real maximise-close-relaunch. |
| M4 | **Done.** Autosave and pin. |
| M5 | Polish, and the Windows portable + installer builds. |
| M6–M8 | Cross-platform seams, then mac and Linux builds. |

Detail: [docs/roadmap.md](docs/roadmap.md) §9. Ship targets: `win-install`, `win-portable`, `linux-install`,
`linux-portable`, `mac-install` (whitepaper §7).

## Questions we have not answered

Tracked in §10 ([docs/open-questions.md](docs/open-questions.md)), and worked through one
at a time in [`.agents/notes/proposed/`](.agents/notes/proposed/), which holds **eleven open
notes** as of 2026-09-15. The count and the one-line summaries live in that folder's own
[index](.agents/notes/proposed/README.md), so this sentence is never patched again; the short
version is four groups — what the save path still owes (five notes), what the menu and its
keyboards and pointers still owe (three), what the window's own surface still owes (two), and
one gap in the record itself: an `Event::ExternalChange` every bridge can render and no code
emits.

Two questions left this list by being decided, not by being dropped. Autosave on a file the app
did not create is settled: [ADR-0001](docs/decisions/0001-autosave-arms-on-explicit-save.md)
arms it on one explicit save and M4 implements exactly that. Which act counts as that save is
settled too, by [ADR-0007](docs/decisions/0007-save-is-a-conformance.md): `Command::Save` on
`Ctrl+S` *is* the explicit save ADR-0001 was waiting for, so the menu no longer offers Save As
and nothing else. The note that recorded the gap has graduated —
[`2026-09-14-explicit-save-act`](.agents/notes/archived/2026-09-14-explicit-save-act.md) is now
`archived/` with its `decision:` set, and its body stands as written, including the drafting
recommendation the ADR overruled. One earlier note on the same subject,
[`2026-09-12-arm-on-explicit-save-path`](.agents/notes/proposed/2026-09-12-arm-on-explicit-save-path.md),
is overtaken by the same decision and still sits in `proposed/`: it records what was argued
before it shipped, and moving it is not this edit's call. The Slint swap is settled and built
([`2026-09-14-strip-program`](.agents/notes/implemented/2026-09-14-strip-program.md)), and coming
back maximised is settled and shipped
([`2026-09-12-maximized-persistence`](.agents/notes/implemented/2026-09-12-maximized-persistence.md)).

## Contributing

A Rust workspace builds here: `cargo build` from the repo root. The full pre-report gate —
fmt, clippy, tests, the layering gate (`cargo xtask check-arch`), the docs validator — is
in [`AGENTS.md`](AGENTS.md); read it first anyway. It covers the invariants that are
enforced and the things deliberately ruled out, which is the fastest way to avoid proposing
something the project has already considered.

## License

Not yet chosen. Until it is, all rights reserved.
