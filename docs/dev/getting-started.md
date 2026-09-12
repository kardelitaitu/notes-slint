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
among them, so expect a few minutes. Warm rebuilds are seconds. The build produces
`target\debug\notes-gpui.exe`: the *bin* is named `notes-gpui`, the *package* is
`notes-bridge-gpui`, because the package names the adapter, not the product.

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

For a plain `cargo run` that means `%APPDATA%\notes-gpui\session.json`.
