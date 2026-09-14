# Getting started

Build and run the app from source. This is Windows-first: the thing you build is the
Windows app, `notes-platform` compiles only for `cfg(windows)`, and CI's gate job
is defined to run on `windows-latest`. (`notes-core` itself is pure Rust; the workflow
proves the dependency graph resolves on Linux, but does not build the app there. One
honest caveat while you are reading this: the repo has no remote, so nothing in
`.github/workflows/` has ever executed — every "CI" row below is a defined row, not a
green tick.)

## Prerequisites

- **Windows** (10 or later).
- **Rust, via rustup** — the stable toolchain. [`rust-toolchain.toml`](../../rust-toolchain.toml)
  pins `channel = "stable"` and nothing more, on purpose: pinning a patch version
  would make rustup download a toolchain during someone else's build. The workspace
  MSRV (`rust-version = "1.85"` in the root `Cargo.toml`) is enforced by cargo
  itself, not by the toolchain file.
- **The MSVC toolchain** (rustup's default host on Windows) — the build links with
  `link.exe`.

## Get the code

```sh
git clone <repository-url>
cd notes-gpui
```

## Build

```sh
cargo build
```

The first build compiles the whole dependency graph — ~440 crates, the Slint and GPUI
toolkits among them, so expect a few minutes. Warm rebuilds are seconds. A default build links
three **app** exes into `target\debug\`: `notes-slint.exe` (the product) and
`notes-slint-probe.exe` (its instrument) from `crates/bridge-slint`, and `notes-gpui.exe`
(the frozen bridge — the other UI path, no new work) from `crates/bridge-gpui` — beside the
`xtask.exe` tool itself.
In every app case the *bin* is the product
name and the *package* names the adapter — both Slint bins from `notes-bridge-slint`,
`notes-gpui` from `notes-bridge-gpui` — because one package can ship more than one thing.
They are not three copies of the same app; [Run](#run) says which to start.

The toolkit dependency is pinned exactly: `gpui-kit = "=0.6.1"` in the root
`Cargo.toml` (gpui-pre 0.3.4 beneath it, re-exported as `gpui_kit::*`). It stays an
inherited workspace dependency — reaching for `gpui` directly is how this repo hit
the duplicate-manifest link error described there.

## Run

The app is `notes-slint.exe`:

```sh
cargo run -p notes-bridge-slint --bin notes-slint
```

This opens the note window at the rect the session saved, autosaves as you type, and stays
open until you close it. From a script, the repo's own launch-and-close check is
`cargo xtask smoke`: it builds, launches the exe, closes the window with `WM_CLOSE`,
and proves the process exits by itself. `smoke` is being retargeted to default to that
product binary in the same wave as [ADR-0006](../decisions/0006-gpui-is-frozen-not-deleted.md);
until the flip is in your tree, name it — `cargo xtask smoke --binary=slint` — and see
[`testing.md`](testing.md) for what each leg judges.

### The two Slint exes are different things

`crates/bridge-slint` is one package with two bins, and they hold different contracts — so
`cargo run -p notes-bridge-slint` is refused outright ("could not determine which binary to
run") until you name one:

```sh
cargo run -p notes-bridge-slint --bin notes-slint         # the product
cargo run -p notes-bridge-slint --bin notes-slint-probe   # the instrument
```

**Run `notes-slint` to use the app.** It is the shipping artifact — the one
`cargo xtask check` links on its own `slint-build` row, the one `cargo xtask smoke` is
retargeted to judge by default ([ADR-0006](../decisions/0006-gpui-is-frozen-not-deleted.md);
name it with `--binary=slint` until that lands), and the name every downstream owner fences
against — which is why it is declared first in `crates/bridge-slint/Cargo.toml`. It runs no
scheduled acts and hides itself for no harness's benefit, so it stays
open until you close it; the stderr it does write is its own startup report — which is
exactly what the product leg reads.

**`notes-slint-probe.exe` is the same crate's instrument**, and you run it when you want the
printed evidence rather than a notepad: the `notes-gpui: startup: …` / `event: …` /
`status line: …` needles the harness reads, the timed acts, the do-no-harm hash checks. It
is a measuring device, not a smaller product — and `cargo xtask smoke --binary=slint-probe`
**refuses to judge it**, exiting 2 before it builds or launches anything, because the only
needle schedule wired speaks gpui's contract. What each leg proves is in [`testing.md`](testing.md).

The native file picker is the one act neither Slint bin can be asked to show on a scripted
run. `SLINT_NO_DIALOG=1` stands it down: no modal appears, and the request is answered
through the same channel with the path that document already has
(`crates/bridge-slint/src/plumbing.rs`, `dialog_allowed`). The gate is **presence, not
value** — `SLINT_NO_DIALOG=0` skips the dialog too — and the run prints
`dialog[skipped]: SLINT_NO_DIALOG …` so the log says the modal never happened. Absent the
variable, both bins show a real dialog, because that is what a shipped bridge does.

### notes-gpui.exe — the frozen bridge

```sh
cargo run -p notes-bridge-gpui     # (frozen bridge — the other UI path, no new work)
```

This exe still builds and still opens a working note window, and its tests remain part of the
evidence M2's 3-of-7 ledger cites. It is **frozen**, not deleted:
[ADR-0006](../decisions/0006-gpui-is-frozen-not-deleted.md) rules out new features and new
needles on this side, forbids loosening a §9 row in either direction, and gates the terminal
delete behind six equivalent proofs earned on the Slint side. Its smoke leg stays legal
(`cargo xtask smoke --binary=gpui` — see [`testing.md`](testing.md)), because the needle
schedule is still what proves the window contract while no Slint leg covers those claims. Run it
to see the other UI path, not to take notes.

> A run is not read-only. The app autosaves on close, so it writes to the state
> directory described below. `smoke` moves a pre-existing `session.json` aside for
> the duration and puts it back afterwards.

## Where your state lives

Everything the app persists sits in one directory, resolved in
`crates/core/src/paths.rs` (`resolve_state_dir`):

| Deployment | State directory |
|---|---|
| Installed — `%APPDATA%` resolves and no `data` directory sits next to the exe | `%APPDATA%\notes-gpui` |
| Portable — a `data` directory exists next to the exe, or there is no `%APPDATA%` | `<exe_dir>\data` |

Inside it:

- `session.json` — window geometry, read synchronously before the window is created
  (the fixed startup order: query session, create window at the saved rect, register
  the handle, apply topmost).
- `settings.toml` — user preferences and the recent-files list.
- `notes\untitled.notes` — the scratch note (a fixed name on purpose: no pid, no
  timestamp, no counter).

For a plain `cargo run` that means `%APPDATA%\notes-gpui\session.json` — and the rule is
the *port's*, not the exe's, so `notes-gpui.exe` and `notes-slint.exe` resolve to the same
directory. They are two toolkits over one identity: run both and they share one
`session.json`, which is a thing to know before you compare them side by side.

`notes-slint-probe.exe` is the exception, deliberately. After running the rule above it
overrides the answer with a directory beside its own exe
(`<exe_dir>\slint-probe\data` — for a debug build, `target\debug\slint-probe\data`) so a
thousand measured runs cannot dirty a person's real session, and it prints both halves:
`state-dir: isolated … (port rule alone would give …)`
(`crates/bridge-slint/src/probe.rs`, `state_dir`). The product has no such override — that
single function is the whole difference between the two bins at startup.
