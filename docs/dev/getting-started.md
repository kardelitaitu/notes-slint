# Getting started

Build and run the app from source. This is Windows-first: the thing you build is the
Windows app, `notes-platform` compiles only for `cfg(windows)`, and CI's gate job
runs on `windows-latest`. (`notes-core` itself is pure Rust; CI proves the
dependency graph resolves on Linux, but does not build the app there.)

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

The first build compiles the whole dependency graph — ~440 crates, the GPUI toolkit
among them, so expect a few minutes. Warm rebuilds are seconds. A default build links
three exes into `target\debug\`: `notes-gpui.exe`, and `notes-slint.exe` plus
`notes-slint-probe.exe` from `crates/bridge-slint`. In every case the *bin* is the product
name and the *package* names the adapter — `notes-gpui` from `notes-bridge-gpui`, both
Slint bins from `notes-bridge-slint` — because one package can ship more than one thing.
They are not three copies of the same app; [Run](#run) says which to start.

The toolkit dependency is pinned exactly: `gpui-kit = "=0.6.1"` in the root
`Cargo.toml` (gpui-pre 0.3.4 beneath it, re-exported as `gpui_kit::*`). It stays an
inherited workspace dependency — reaching for `gpui` directly is how this repo hit
the duplicate-manifest link error described there.

## Run

```sh
cargo run -p notes-bridge-gpui
```

This opens the note window. From a script, the repo's own launch-and-close check is
`cargo xtask smoke`: it builds, launches the exe, closes the window with `WM_CLOSE`,
and proves the process exits by itself.

### The two Slint exes are different things

`crates/bridge-slint` is one package with two bins, and they hold different contracts — so
`cargo run -p notes-bridge-slint` is refused outright ("could not determine which binary to
run") until you name one:

```sh
cargo run -p notes-bridge-slint --bin notes-slint         # the product
cargo run -p notes-bridge-slint --bin notes-slint-probe   # the instrument
```

**Run `notes-slint` to use the app.** It is the shipping artifact — the one CI links (the
`slint-build` row of `cargo xtask check`), the one `cargo xtask smoke --binary=slint`
judges, and the name the icon, the application manifest and the `%APPDATA%` identity fence
against. It runs no scheduled acts and hides itself for no harness's benefit, so it stays
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
