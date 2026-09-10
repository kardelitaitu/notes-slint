# notes-gpui — Project Whitepaper

> **Version 0.0.1** — planning phase, no code yet. Rough sketch, not a spec.
> Working title: `notes-gpui`. Real name TBD. 0.0.1 is the M0/target version; see §7.4 for what the number drives.

---

## 1. What this is

A small, fast desktop note app. It opens instantly, saves itself without being asked,
and comes back exactly where you left it — same window size, same screen position.
A single button pins it above every other window, so it behaves like a sticky note
when you need it to and like a normal window when you don't.

Not a knowledge base. Not a markdown editor with a plugin ecosystem. Not a replacement
for Obsidian or Logseq. It is the window you throw your thoughts into.

## 2. Why

Most note apps fail at the *small* moments: they reopen at the wrong window size, they
ask "save changes?" when you're trying to leave, they bury the one thing you wanted —
a scratchpad that stays on top of your IDE or browser. The friction is in the window
management, not the editing.

The bet: **the window behaviour *is* the product.** Get persistence, autosave, and
pinning right, and the editing surface can stay deliberately minimal.

Secondary bet: build it in Rust + GPUI and it should feel instant — cold start under
~200ms, no Electron-sized memory footprint, no renderer process.

## 3. Product definition

### v1 (must have)

| Feature | Summary |
|---|---|
| **Window persistence** | Size + position restored exactly on every launch, across restarts and reboots. |
| **Autosave** | Content saved continuously, debounced. No save dialog, no "save changes?" prompt, ever. Toggleable. |
| **Pin / always-on-top** | One click (plus a keyboard shortcut) toggles topmost. State persists across launches. |
| **File model** | One document at a time, backed by a real file on disk. Native `.notes`; opens ordinary text files too. |
| **Menu** | Hamburger menu: Open, Save, Save As, Auto-save toggle, Recent files (max 10). |
| **Crash safety** | Losing power mid-typing must not lose more than ~1s of text and must never corrupt the file. |
| **Do no harm** | Opening and autosaving a foreign file must not silently change its encoding or line endings. |

### Explicit non-goals for v1

Tabs or multiple open documents in one window; a notes database, index, or sidebar;
sync; accounts; collaboration; plugins; mobile; export/import pipelines; tags/backlinks;
search across a corpus; a theming system. If it shows up in a roadmap conversation, it is
a v2+ conversation.

## 4. Core behaviour, in detail

### 4.1 Window persistence

The naive version — save `x, y, w, h` — breaks in three predictable ways. All three are
in scope:

1. **Monitor topology changes.** A monitor is unplugged; the saved rect is now at
   `x=2560`, off-screen, and the window is unreachable. → On restore, validate the rect
   against currently-connected monitors; if it fails, clamp onto the primary monitor
   preserving size.
2. **DPI / scale factor changes.** A 1440×900 rect means different physical pixels at
   100% vs 150% scaling. → Persist in a single, documented unit (logical pixels, with
   the scale factor recorded alongside) and convert explicitly.
3. **Maximised state.** A maximised window's live bounds are the monitor bounds, not the
   user's preferred size. → Persist *restored* bounds and a separate `maximized` flag.

Persist on move/resize (debounced, ~300ms) and on close. Not on every pixel of a drag.

### 4.2 Autosave

- **Triggers:** idle debounce after the last keystroke (default ~750ms); a periodic flush
  (~15s) so long uninterrupted typing still lands; and immediately on focus loss, window
  close, and app quit.
- **Atomic writes.** Write to a sibling temp file, `fsync`, then rename over the target.
  A crash mid-write leaves the previous good file intact. This is non-negotiable — it is
  the whole reason autosave is trustworthy.
- **Dirty tracking.** Compare a content hash or revision counter before writing; don't
  rewrite unchanged content. Keeps disk churn and file-watcher noise near zero.
- **Off the UI thread.** The save runs in the core engine on a worker thread. The GPUI
  frame loop never blocks on I/O.
- **External edits.** If the file changed on disk while we held it, detect it (mtime +
  hash) before overwriting. v1 behaviour: warn and keep both, do not silently clobber.
- **Armed state (ADR-0001).** Autosave engages immediately for `.notes` files. For any
  file this app did not create it stays **disarmed** until the user performs one explicit
  save, after which it is armed for that document. Arming is per document — opening a
  foreign file must not inherit the previous one's state — and **Save As arms**,
  because the user just chose that path deliberately.
- **Never silently disarmed.** While disarmed the UI must say why nothing is being saved;
  `Event::AutosaveSkipped` exists for this and must be rendered, not merely defined.

### 4.3 Pin (always on top)

- A visible toggle in the window chrome, plus a keyboard shortcut.
- Windows implementation is `SetWindowPos` with `HWND_TOPMOST` / `HWND_NOTOPMOST` and
  `SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE` — the flags matter; without `NOACTIVATE`
  toggling pin steals focus, which is a bad experience.
- Pin state is persisted and restored on launch.
- **Open risk:** GPUI may not expose topmost or a raw window handle. See §6.

### 4.4 File model & the menu

The menu settles the product shape: this is a **single-document app**. One window, one
file open at a time, opening a file replaces the current one. Not a workspace with a
sidebar — that distinction is now explicit and tabs are a stated non-goal (§3).

| Menu item | Behaviour | Shortcut |
|---|---|---|
| **Open…** | Native file dialog, filtered to `.notes` first, all text types available. | `Ctrl+O` |
| **Save** | Write to the current path. With autosave on this is a manual flush, not a distinct action — keep it anyway for muscle memory. | `Ctrl+S` |
| **Save As…** | New path, and the document *rebinds* to it. Subsequent autosaves go to the new path. | `Ctrl+Shift+S` |
| **Auto-save** | Toggle, with a visible checked state. Persisted. | — |
| **Recent files** | Up to 10, most recent first. | `Ctrl+R` (optional) |

Consequences that need designing, not assuming:

- **Unsaved-changes guard.** With autosave on, Open-while-dirty is rare — but autosave
  *can* fail (read-only file, no permission, file deleted, disk full, OneDrive lock). So
  the guard is still required. It must not become a modal that interrupts typing.
- **Autosave failure must be visible and non-blocking.** A dirty indicator in the title
  bar that turns amber, plus the reason in the menu. Silent autosave failure is the worst
  possible outcome — the user believes they are saved and are not.
- **Recent files.** Dedupe by canonicalised path (`C:\DOCs\A.notes` and
  `c:\docs\a.notes` are one entry). Cap at 10. Grey out — don't silently delete — entries
  whose file has vanished, so the list doesn't quietly forget things. Needs a "Clear
  recent" action: this list discloses the user's folder structure and file names.
- **Session restores the file too.** "Always remembers" should mean the same *note* is
  there on relaunch, not just the same window size. Store the last path in `session.json`
  and reopen it. If it's gone, say so once, quietly.
- **Drag-and-drop a file onto the window to open it.** Nearly free to implement, and the
  fastest path for this kind of app.
- **Native file dialogs are probably not in GPUI.** Expect `rfd` or direct Win32
  `IFileOpenDialog`. See R9.

### 4.5 The `.notes` format

**Recommendation: `.notes` is UTF-8 plain text, Markdown-compatible, with an optional
frontmatter block.** Not JSON, not a binary or serialised container.

```
---
created: 2026-09-10T08:58:00+08:00
pinned: true
---

The note itself, as ordinary text.
```

Why plain text, specifically:

- **No lock-in, and that matters for a notes app.** If we are abandoned or the format
  changes, every file a user has must still open in any editor. A serialised format makes
  us a data hostage-taker, which is the wrong thing to be.
- **Corruption is survivable.** A truncated plain-text file is 95% recoverable by eye. A
  truncated JSON blob is total loss.
- **Diffable and greppable**, which users will do whether or not we support it.
- It keeps "opens all popular text extensions" honest — we are a text editor that also
  has a native extension, not a proprietary database.

**Worth challenging, though:** a distinct extension has a real cost. Users must learn it,
other apps won't open it by default, and it fragments their notes across two file types.
The benefit is file association (double-click → our app), a default for new files, and a
place for per-file metadata. If we end up storing no metadata, `.notes` is decoration and
we should ship `.md` instead. **Open decision §10.1.**

**Foreign files — the "do no harm" rule.** Opening `config.json` or someone's `.md` and
autosaving must not quietly rewrite it:

- **Preserve line endings.** Detect CRLF/LF on load, record it, write it back unchanged.
  Never normalise.
- **Preserve encoding.** UTF-8 with/without BOM, UTF-16 LE/BE, and legacy ANSI all exist
  in the wild on Windows. Record what we loaded and write the same thing back.
- **Preserve trailing-newline state.** Adding or stripping a final `\n` is a diff someone
  else has to explain.
- **Size guard.** Someone will drag a 500 MB log file in. Refuse or open read-only above a
  threshold, and state the threshold.

## 5. Architecture

The requirement is *"core engine in Rust, UI in GPUI."* Made concrete, that is a stack in
which the UI toolkit itself is a swappable detail, behind a bridge: the engine talks to
exactly one module, and `core` depends on nothing in this repo:

```
┌───────────────────────────────────────────────────┐
│  bridge-*       one adapter per UI toolkit        │
│  bridge-gpui    views, keybindings, render loop,  │
│                 and THE TEXT BUFFER               │
│                 may import ONLY api               │
│  bridge-tauri   ...only if a second one is        │
│                 ever actually built               │
├───────────────────────────────────────────────────┤
│                                                   │
│  api            the port. one surface, both       │
│                 directions. no logic of its own,  │
│                 no UI types, and it does not know │
│                 that any bridge exists.           │
├───────────────────────────────────────────────────┤
│  core           document model, save engine,      │
│                 session state, config             │
│                 pure Rust: no UI, no OS, no       │
│                 unsafe, no gpui                   │
│                                                   │
│  platform       window geometry, topmost,         │
│                 monitor/DPI, file dialogs         │
│                 trait + per-OS backends           │
└───────────────────────────────────────────────────┘
   bridge -> api -> { core, platform }      core is independent of platform
```

**`core` and `platform` do not know about each other**, and neither knows the UI exists.
`api` is the only place their outputs are joined. That asymmetry is what makes the
gateway a single one — see §5.4.

**`core` is headless and fully unit-testable.** No GPUI import, no `windows` crate, no
`unsafe`. Save engine, atomic write, dirty tracking, and state serialisation are all
testable without ever opening a window. This is the main structural reason to insist on
the split, and it is the reason cross-platform later is a refactor of `platform` rather
than a rewrite.

**`platform` is where the OS-specific ugliness is quarantined.** A trait
(`WindowBackend`?) with a Windows implementation first. macOS/Linux are additional
implementations of the same trait later — nothing above `platform` changes.

**The bridge is thin.** GPUI views and the event loop. Business logic living here is a bug.

### State & file layout (draft)

```
%APPDATA%\notes-gpui\
  settings.toml      # deliberate prefs: autosave on/off, interval, recent-files list (≤10)
  session.json       # restoration state: window rect, monitor id, scale, maximized,
                     #   pinned, last-open file path
  notes\             # default save location for new files, if the user lets us pick one
```

`session.json` is written by the core engine, not by the windowing layer, so it can be
tested without a display.

**Portable builds change this path.** A portable app must leave nothing behind in
`%APPDATA%` — that is the entire point of shipping one. So state resolution is a rule,
not a constant:

```
if  <exe dir>\data\ exists   →  use <exe dir>\data\      (portable mode)
else                         →  use %APPDATA%\notes-gpui  (installed mode)
```

Same rule on every OS. This belongs in `core` as a single `resolve_state_dir()` so the
behaviour is testable and identical everywhere. Cheap to add now; painful to retrofit
once users have notes in a location we chose for them. See §7.

### 5.1 Repository layout

One binary, five crates, mirroring the layering above. Everything else in the tree
exists to serve the five packaging artifacts or the do-no-harm rule.

```
notes-gpui/
├── README.md                  # human-facing intro
├── AGENTS.md                  # conventions + invariants for anyone working here
├── whitepaper.md              # this file — exactly one, living, never forked
│
├── .agents/                   # agent-facing project memory (§5.6)
│   ├── skills/
│   │   └── doc-management/
│   │       ├── SKILL.md       #   where every document goes, and why
│   │       ├── templates/note.md
│   │       └── scripts/check-docs.ps1   #   those rules, enforced
│   └── notes/
│       ├── proposed/          #   ideas on the table
│       ├── implemented/       #   built — code exists and works
│       ├── rejected/          #   decided against, with the reason
│       └── archived/          #   superseded or obsolete
│
├── Cargo.toml                 # workspace + [profile.release] lto/strip — cold-start budget
├── Cargo.lock
├── rust-toolchain.toml        # pin it; GPUI API churn is a known risk (R1)
├── .cargo/config.toml
├── .github/workflows/
│   ├── ci.yml                 # fmt + clippy + test + check-docs, matrix over 3 OSes
│   └── release.yml            # builds the 5 artifacts (§7) on tag
│
├── crates/
│   ├── core/                  # PURE RUST. No gpui, no windows, no unsafe.
│   │   ├── src/
│   │   │   ├── document.rs    #   buffer, revision counter, dirty tracking
│   │   │   ├── format.rs      #   .notes: frontmatter parse/serialise (§4.5)
│   │   │   ├── encoding.rs    #   UTF-8/16, BOM, EOL — detect once, write back same
│   │   │   ├── save.rs        #   atomic: temp → fsync → rename
│   │   │   ├── autosave.rs    #   debounce, periodic flush, blur/close triggers
│   │   │   ├── session.rs     #   rect, monitor id, scale, maximized, pinned, last file
│   │   │   ├── recent.rs      #   MRU ≤10, canonical-path dedupe, missing-file state
│   │   │   ├── settings.rs    #   deliberate prefs
│   │   │   └── paths.rs       #   resolve_state_dir() — portable vs installed
│   │   └── tests/
│   │       ├── roundtrip.rs   #   load→save byte-identical (R10)
│   │       └── fixtures/      #   CRLF / LF / BOM / UTF-16LE / empty / no-trailing-newline
│   │
│   ├── platform/              # OS seams. Trait + cfg-gated backend modules.
│   │   ├── src/
│   │   │   ├── lib.rs         #   WindowBackend trait, geometry types
│   │   │   ├── geometry.rs    #   rect/monitor/scale — ONE documented unit (§4.1)
│   │   │   └── windows/       #   #[cfg(windows)]
│   │   │       ├── topmost.rs #   SetWindowPos + SWP_NOACTIVATE (§4.3)
│   │   │       ├── monitors.rs#   EnumDisplayMonitors, per-monitor DPI (R5)
│   │   │       └── dialog.rs  #   IFileOpenDialog (R9)
│   │   ├── build.rs           #   winres: icon, version info, DPI-awareness manifest
│   │   └── Cargo.toml
│   │
│   ├── api/                   # THE GATEWAY (§5.4). Commands in, Events out.
│   │   └── src/
│   │       ├── lib.rs         #   Gateway: start(), send(Command), events()
│   │       ├── command.rs     #   UI → engine, one variant per user intent
│   │       ├── event.rs       #   engine → UI, incl. async failures (§5.4)
│   │       ├── engine.rs      #   the worker loop; owns core + platform handles
│   │       └── dto.rs         #   types the UI may see — re-exported, never leaked
│   │
│   ├── bridge-gpui/           # THE FIRST BRIDGE (§5.5). GPUI-only.
│   │   └── src/
│   │       ├── main.rs        #   #![windows_subsystem = "windows"] — no console window
│   │       ├── bridge.rs      #   hands the window handle to api (§5.5)
│   │       ├── app.rs         #   GPUI root state; drains Events onto the UI thread
│   │       ├── actions.rs     #   sends Open / Save / SaveAs / TogglePin Commands
│   │       ├── keymap.rs      #   Ctrl+S, Ctrl+Shift+S, pin shortcut
│   │       └── views/
│   │           ├── editor.rs  #   OWNS THE TEXT BUFFER — deliberately not abstracted
│   │           ├── menu.rs    #   hamburger
│   │           ├── titlebar.rs#   ONLY if §10.2 resolves to custom chrome (R11)
│   │           └── status.rs  #   dirty + autosave-failed indicator (§4.4)
│   │
│   └── (bridge-tauri/)        # NOT BUILT. A seam, not a task — see §5.5.
│
├── assets/                    # icon.ico / .icns / .png; fonts if GPUI needs them
├── packaging/
│   ├── windows/               #   WiX .wxs or Inno .iss — NOT MSIX (§7)
│   ├── linux/                 #   deb control, .desktop, AppImage recipe
│   └── macos/                 #   Entitlements.plist, dmg settings, notarise script (§7.1)
├── docs/                      # INTENT: planning & developer documentation
│   ├── README.md              #   the docs map, and the tense rule (§5.6)
│   ├── dev/                   #   how to build/test/package - written once code exists
│   └── decisions/             #   ADRs, one per settled decision (§5.6)
└── scripts/                   #   local dev: run, bundle, regenerate fixtures
```

### 5.2 Rules that keep this honest

**1. The layering must be machine-enforced, or it is a suggestion.** A `core` that
accidentally imports GPUI is one convenience commit away, and then `core` is no longer
headless or testable and the cross-platform plan becomes fiction. Make CI fail on it:

```
cargo tree -p core -i gpui          # must be empty — core is pure
cargo tree -p core -i windows       # must be empty — core has no OS
cargo tree -p core -i platform      # must be empty — core is independent of platform
cargo tree -p api  -i gpui          # must be empty — the port is UI-agnostic
cargo tree -p bridge-gpui -i core      # must be empty: a bridge may only see api
cargo tree -p bridge-gpui -i platform  # must be empty: ditto
```

The last two are the interesting ones: they make *"one gateway"* a fact rather than a
convention. A bridge physically cannot reach around `api` without failing the build.

**2. `platform` backends are cfg-gated modules, not separate crates.** `platform-windows`
/ `platform-macos` / `platform-linux` as three crates sounds tidier, and costs a separate
workspace build per OS plus duplicated trait plumbing. One crate with
`#[cfg(windows)] mod win;` is simpler, and is revisitable if compile times ever complain.

**3. The M0 spike does not live in this tree.** Do it in a throwaway directory. Spike code
is the most dangerous kind of code — it works, so people keep building on it instead of
deleting it. The spike's only output is a yes/no answer plus a note on which GPUI version
and which dialog crate worked. The real tree is then started clean.

**4. `api` knows nothing about bridges.** It must compile with zero UI crates present, and
there is no `#[cfg(feature = "gpui")]` anywhere in it. If `api` starts branching on which
bridge is attached, the seam is gone.

**5. `api` contains no business logic.** It translates and routes. The moment a rule like
*"don't autosave foreign files without an explicit save"* lives in `api/engine.rs`
instead of `core/autosave.rs`, the gateway has become a fourth domain layer and the
testability of `core` is quietly compromised. Review for this.

### 5.3 Two structural notes

- **`build.rs` is where DPI awareness actually gets set (R5).** Per-monitor v2 DPI must be
  declared in the exe's application manifest, not called at runtime — a runtime call is too
  late once the process has already been DPI-scaled. So the manifest is a build-time
  artifact, which is why it sits beside the icon and version info in `winres`.
- **The shipped binary's name is load-bearing.** Portable mode resolves state relative to
  the exe's directory (§5), so renaming the binary relocates the user's notes. Pin it
  early and keep it stable across all five artifacts.

### 5.4 The API gateway

`crates/api` is the single surface the UI is allowed to speak to. One crate, one module
tree, both directions. It is the right call for three reasons, and it forces one design
decision that is easy to get wrong.

**Why it's worth a crate.**

- **The UI becomes replaceable.** R1–R3 say GPUI on Windows is an unproven bet for us. If
  the *only* contract between our logic and our UI is `api`, then losing that bet means
  rewriting the bridge, not the project. That insurance is nearly free to take now and very
  expensive to buy later.
- **The whole app is testable headlessly.** Feed `Command`s, assert `Event`s. No window, no
  GPUI, no display server — a "play a session" integration test runs in CI on all three
  OSes. And because `core` is pure and fast, **tests use the real engine; no mocks.**
- **One place to look.** Every capability the UI can reach is one file. Feature planning
  becomes "what does `Command` need?" instead of spelunking four crates.

**The decision it forces: autosave breaks the request/response model.**

A gateway that is just methods returning `Result` cannot express this app. Autosave
happens seconds after any UI call, on a worker thread, and *can fail* — read-only file,
permission denied, disk full, OneDrive lock. There is no caller waiting for that `Err`.
So the gateway must be **command/event**, not call/return:

```rust
// UI → engine. One variant per user intent. Fire-and-forget.
enum Command {
    Open(PathBuf),
    SaveAs(PathBuf),
    Flush { text: String, revision: u64 },  // Ctrl+S and every autosave trigger
    SetAutosave(bool),
    SetPinned(bool),
    ClearRecents,
    Shutdown,
}

// engine → UI. Delivered on the UI thread. Includes things with no caller.
enum Event {
    Loaded { path: PathBuf, text: String, meta: FileMeta },
    Saved  { path: PathBuf, revision: u64 },
    SaveFailed { path: PathBuf, revision: u64, reason: SaveError },  // ← the async one
    ExternalChange { path: PathBuf },
    AutosaveSkipped { reason: SkipReason },   // disarmed foreign file - ADR-0001
    RecentsUpdated(Vec<RecentEntry>),
}
```

`FileMeta` is where §4.5's do-no-harm data surfaces: encoding, line ending, trailing
newline, read-only flag, and the size-guard verdict. The UI needs read-only and
oversize to render honestly, and `status.rs` needs `SaveFailed` to go amber (§4.4).

**The thread boundary lives here, and it is the sharp edge.** `engine.rs` owns a worker
thread; GPUI runs its logic on its own thread. Events must be marshalled onto the GPUI
thread before touching any view. Two rules: **never block on a channel inside a GPUI
frame**, and **never call into `api` from the worker thread**. Get either wrong and you
get a deadlock that only appears under load. This is the main cost of the design, and it
is paid once.

**The second decision: who owns the live text buffer?**

- **(i) `core` owns it** — every keystroke crosses the gateway. Purest, but chatty, and
  typing latency now depends on a thread hop.
- **(ii) the bridge owns it** — GPUI's text buffer is the live document; `core` owns the *file*
  and receives a snapshot on `Flush`. Gateway stays low-traffic, edits never cross it.

Recommendation: **(ii)**. It keeps keystrokes off the channel and keeps the gateway small.
The cost is that dirty state is split — the bridge knows the buffer changed, `api` knows what
was last written — reconciled by the `revision` counter on `Flush`. Open decision §10.4.

**What it costs, honestly.** Indirection: every feature touches two enums before it
touches logic. Drift risk: `Command`/`Event` lag behind what the UI actually needs, and
someone reaches around the gateway to fix it — which is exactly what the `cargo tree -p
app -i core` check in §5.2 exists to prevent. And a real temptation to let logic pile into
`engine.rs`, hence rule 4.

### 5.5 Bridges: one port, many UIs

A **bridge** is an adapter that owns a UI toolkit end to end and speaks to nothing else in
this repo except `api`. `bridge-gpui` today; `bridge-tauri`, `bridge-egui`, or
`bridge-objc` later if a toolkit ever has to be replaced. `api` is the port, bridges are the
adapters, and the dependency arrow only ever points one way: bridge -> api. The port never
learns that any bridge exists (rule 4, §5.2).

**This works only because the UI surface is small — and it is worth seeing why.** The entire
vocabulary a bridge must carry is: text plus file metadata down, and edit / menu / pin
intents up. That is a genuinely narrow interface. A richer app could not abstract its UI
this way. This one can, because the product is a buffer and a menu.

**What cannot be abstracted: the text editor.** Cursor, selection, IME, undo, soft wrap,
scrolling and multi-byte input are not a widget you can hide behind a trait. GPUI ships a
text buffer; Tauri inherits one from the browser; egui is weak here. If we tried to abstract
the editor we would be writing a UI toolkit, and would finish it sometime around 2029. So:

> **The bridge owns the editor. `api` never sees a keystroke** — only `Flush { text }`.

Note the dependency this creates: it makes **§10.4 resolving to "the UI owns the buffer"**
effectively mandatory. If `core` held the live buffer, every keystroke would cross the port,
and the port would have to abstract text editing — the one thing above that is impossible.
The bridge idea and the buffer-ownership decision stand or fall together.

**The hard part: whoever owns the window owns the window.** `platform` has real window
duties (topmost, position, restore) but it does not create the window — the toolkit does.
And what a toolkit allows varies per bridge: GPUI creates an HWND we can hand to
`SetWindowPos`, while Tauri owns its window *and* exposes always-on-top itself, making a
`platform` call both impossible and wrong. So the division has to be:

- `platform` provides **OS primitives that take a window handle**, and decides nothing.
- The **bridge owns the window** and its lifecycle.
- `api` **routes**, holding the handle the bridge registered.

Which yields a startup order that must be respected, and is easy to get wrong:

```
1. bridge  -> api      query saved session   (rect, scale, maximized, pinned)
2. bridge             create the window AT that rect   <- only the bridge can do this
3. bridge  -> api      register the window handle
4. api     -> platform apply topmost(handle)
```

Step 1 is a synchronous query — the one pragmatic exception to the pure command/event rule,
since the window does not exist yet and there is nowhere to deliver an event. Step 2 is why
geometry *restore* is bridge work while geometry *storage* is `core` work. This is the part
of the design a second bridge will most likely force us to revisit.

**Do not build the abstraction yet.** That is the trap inside the idea. A `trait Bridge`
designed while GPUI is the only implementation will quietly encode GPUI's shape — its
entity types, its frame loop, its thread model — into whatever you labelled "generic", and
the second bridge will then cost more than having no seam at all. The cheap seam is the one
we already have: `api` containing no UI types, enforced by CI. That is all the abstraction
this phase needs. Let a real second implementation **extract** the interface from
comparison, rather than guessing it now from a sample of one.

**A reality check on `bridge-tauri` specifically.** Naming it changes §2. Tauri means a web
frontend, a JS toolchain in the build, and a webview with a separate renderer process —
which is the architecture §2 pitches against ("no Electron-sized footprint, no renderer
process", cold start under ~200ms). Tauri is far lighter than Electron, but it still ships a
renderer and a bundle. Keep it as insurance against R1–R3; if it ever becomes the actual
plan, §2 needs rewriting, not just a new crate.

### 5.6 Documentation and project memory

The code will accumulate decisions faster than anyone remembers them, so the docs have a
type system of their own — location carries meaning, and a validator enforces it.

**There are two trees, split by tense rather than by topic:**

| Tree | Holds | Tense | When it disagrees with the code |
|---|---|---|---|
| `docs/` + `whitepaper.md` | **planning and developer documentation** — what we intend to build, and how to work on it | future / present | **the code wins** — edit the doc |
| `.agents/notes/` | **decision and implementation records** — whether a thing was decided, built, refused, or retired | past | the record stands; a newer note supersedes it |

| Kind | Lives in | Character |
|---|---|---|
| The founding sketch | `whitepaper.md` | one, living, never forked |
| How to build, test, package the code | `docs/dev/` | present tense — **not writable before the code exists** |
| Settled architecture decisions | `docs/decisions/` | ADRs — **append-only** |
| Ideas under discussion | `.agents/notes/proposed/` | working memory, moves as it matures |
| Built, and how it actually works | `.agents/notes/implemented/` | living until removed |
| Decided against, with the reason | `.agents/notes/rejected/` | near-frozen |
| Superseded or obsolete | `.agents/notes/archived/` | frozen |
| Conventions for whoever works here next | `AGENTS.md` | living |
| Reusable agent capability | `.agents/skills/<name>/` | living |

**A plan is not a dev doc.** `docs/dev/` describes code that exists; `whitepaper.md`
describes intent. Promote a plan into `docs/dev/` when it is implemented, not when it is
agreed — which is why that folder currently holds only its map and a schedule of which file
unlocks at which milestone. Writing it early produces a confident description of something
that may never be built.

**The distinction that earns its keep is note vs ADR.** A note is disposable: it argues,
it changes status, it can be deleted. An ADR is a court record: immutable, and edited only
by superseding it with a numbered successor. Confusing them is how repositories end up with
a permanent "decision" file full of live speculation, and a speculating note nobody dares to
edit. When a note gets a real decision, it graduates into an ADR and the note archives.

**Statuses are directories, not tags**, so the state of an idea is visible in the file tree
and `git log --stat` reads like a decision history. The cost is that moving a note is a
three-part edit (file, `status:`, and `whitepaper.md` §10 if the product changed), and a
half-done move leaves a note with a stale status that someone will cite as fact — which is
why the validator checks folder/status agreement specifically.

> `.agents/skills/doc-management/scripts/check-docs.ps1` validates placement, filenames,
> frontmatter, duplicate ids, status/folder agreement, and every `§` cross-reference. It
> runs in `ci.yml`. The full map is in that skill's `SKILL.md`.

**One acknowledged gap:** the four statuses have no state for *decided but not built*, which
is where most of §10 "Resolved" currently sits. `implemented` is defined strictly as "the
code exists and works", and accepted-but-unbuilt decisions stay in `proposed/` with a
`DECIDED:` line, mirrored in §10. If that gets awkward, add a fifth status by proposal —
do not quietly redefine `implemented`, because a folder whose meaning drifts is worse than
no folder at all.

## 6. Cross-platform strategy

Windows first, then macOS + Linux. The abstraction seams above are what make that a
port rather than a rewrite. But the honest ordering of difficulty is not what people
expect:

- **Windows (now):** the target. Everything is designed for it.
- **macOS (later):** low risk. GPUI originated here; Metal backend is the mature path.
  Topmost is a simple `NSWindow.level`. Window positioning is well-behaved.
- **Linux (later):** **highest risk, and it is a protocol problem, not a Rust problem.**
  - **Wayland does not let clients position their own windows.** Placing a window at a
    saved `x,y` is a compositor decision and is *forbidden* to clients by design.
    "Always remember window position" — a headline feature — is not achievable as
    specified on Wayland.
  - **Wayland has no standard always-on-top protocol** either. Some compositors support
    it via zwlr extensions; there is no guarantee.
  - X11 handles both fine (`_NET_WM_FRAME_EXTENTS`, `_NET_WM_STATE_ABOVE`).

  **Consequence:** on Linux the two flagship features may degrade to "size remembered,
  position and pinning best-effort." This needs a decision before we commit publicly to
  Linux parity — see §10. It does not change v1, but it changes what we promise.

## 7. Packaging & distribution

Five artifacts on the final roadmap. They are not five equal amounts of work, and two of
them have external prerequisites that cost money and lead time.

| Artifact | Format | Notes |
|---|---|---|
| **win-install** | MSI (WiX) or Inno Setup | **Not MSIX.** Package identity gives filesystem virtualisation and uninstall-on-update that fight a portable-state app and a tray/hotkey app. Classic installer. |
| **win-portable** | Single `.exe` | Zero install, state lives beside the exe (§5). Also the best artifact for testing and for people who just want it to work. |
| **linux-install** | `.deb` (+ `.rpm` if demand) | Needs a real `.desktop` file, icon, and MIME hints or it won't integrate with the DE. |
| **linux-portable** | AppImage | Self-contained, but GPUI pulls in display-server, fontconfig and graphics libs — bundling those correctly is the fiddly part. |
| **mac-install** | `.dmg` wrapping a `.app` | See the signing gate below. No mac-portable requested; a `.app` in a `.dmg` is already effectively portable, which is presumably why. |

### 7.1 The macOS signing gate — start early

This is the item most likely to embarrass us, because it is **not a code problem and has
a lead time measured in weeks**:

- Apple **notarisation** is required for an app downloaded from the internet to open at
  all. Unsigned or un-notarised → Gatekeeper blocks it with a message most users cannot
  work around.
- Notarisation requires an **Apple Developer Program account (US$99/yr)** and code
  signing with a Developer ID certificate + notarisation via `notarytool`.
- CI needs the certificate and an API key as secrets, and the build must be signed
  *before* notarisation and re-stapled *after*.

**Action:** someone needs to own the Apple Developer account decision now, not at M6.

### 7.2 Windows signing — smaller, but real

An unsigned exe triggers SmartScreen's "Windows protected your PC" blue screen. It's
dismissible, but for a note app you're asking people to run all day, it's a trust hit.
A standard OV certificate removes most of it; an EV certificate removes the warning
immediately but costs more and is increasingly being replaced by reputation accrual.
Decision deferred to §10 — but it's a budget line, not an engineering task.

### 7.3 Linux: sandboxing conflicts with our own features

If we ever ship via **Flatpak**, be aware it directly undermines the product:

- Global hotkeys and system-wide tray icons do not work from inside the sandbox.
- Window positioning and always-on-top are already restricted on Wayland (§6); a sandbox
  adds another layer of "the compositor decides."

A native `.deb`/`.rpm` or an AppImage does not have this problem. Recommendation:
**treat Flatpak as opt-in later, and say plainly that the pinned/hotkey experience is
degraded in the sandboxed build** rather than shipping a broken-feeling app.

### 7.4 Build & release mechanics

- **Build natively per OS in CI** (a GitHub Actions matrix on `windows-latest`,
  `ubuntu-latest`, `macos-latest`). Cross-compiling a GPUI app — especially *to* macOS,
  which needs an SDK and a universal binary — is more pain than the three free runners
  are worth.
- **macOS: ship a universal binary** (`arm64` + `x86_64`) unless we decide otherwise;
  lipo-ing two target builds is straightforward and avoids asking users their architecture.
- **Linux glibc floor:** an AppImage built on a current distro won't run on an older one.
  Build on an old baseline (or use `zig cc` to target an explicit glibc version) and
  state the minimum supported glibc.
- **Versioning:** one semver string driving git tag, binary metadata, installer version,
  and the in-app about box. Set this up in M5, not at first release.
- **Auto-update is not in the five artifacts** but users will expect it. Open decision §10.

## 8. Technical risks

| # | Risk | Impact | Mitigation |
|---|---|---|---|
| R1 | **GPUI as a standalone crate.** It grew up inside Zed's monorepo; building a non-Zed GPUI app outside that repo has historically been rough — unpublished deps, missing docs, API churn. | Could invalidate the whole UI choice | **Spike before anything else.** Prove a hello-world GPUI window builds on Windows from a plain `cargo` project. |
| R2 | **GPUI Windows maturity.** Zed's Windows support is comparatively recent; the renderer path differs from macOS Metal. | Rendering bugs, perf, missing features | Same spike. Pin an exact GPUI version, not a moving git rev. |
| R3 | **GPUI may not expose topmost or the raw HWND.** | Pin is a core feature | Verify in spike. Fallback: create/own the window ourselves via the `windows` crate and hand the handle to GPUI. |
| R4 | **Wayland limits (§6).** | Feature parity promise | Decide scope now, document it, don't discover it in year two. |
| R5 | **DPI awareness.** Windows per-monitor v2 DPI is easy to get subtly wrong — blurry or mis-sized window on a mixed-DPI desktop. | Restored window looks broken | Set DPI awareness explicitly at startup; test on 100/150/200% before calling M3 done. |
| R6 | **Autosave + external editor / sync folder conflict.** A OneDrive/Dropbox-synced notes dir will fight us. | Data loss | Detect external modification before write; keep a rolling backup of the last N versions cheaply. |
| R7 | **macOS notarisation lead time (§7.1).** Needs a paid Developer account, a certificate, and CI secrets — none of it code. | mac-install slips, or ships blocked by Gatekeeper | Decide who owns the Apple account now; wire signing into CI at M7, not at release day. |
| R8 | **GPUI drags in heavy system deps on Linux.** Display-server libs, fontconfig, GPU drivers. | AppImage bundling breaks on some distros | Test the AppImage on an old LTS and a current release, not just the build machine. |
| R9 | **No native file dialog in GPUI.** Open/Save As need the OS dialog. | Core menu items blocked | Verify in the M0 spike. Likely `rfd`, or Win32 `IFileOpenDialog` behind the `platform` trait. |
| R10 | **Silent corruption of foreign files.** Autosaving a file whose encoding or line endings we normalised quietly rewrites someone's repo config. | Trust damage disproportionate to effort | §4.5 do-no-harm rules, plus round-trip tests: load → save → byte-identical for a corpus of CRLF/LF/BOM/UTF-16 fixtures. |
| R11 | **A hamburger menu may mean a custom title bar.** If the menu lives in the title bar rather than the content area, we own window drag, resize borders, snap layouts, double-click-maximise, and rounded corners. | Days of unglamorous work; feels broken if skipped | Decide native-vs-custom chrome before M2. See §10.2. |
| R12 | **A bridge interface designed from one implementation.** A `trait Bridge` guessed while GPUI is the only adapter encodes GPUI's shape into what we call generic. | The second bridge costs more than having no seam at all | Build only `bridge-gpui`. The seam is "api has no UI types", enforced by CI. Extract the trait when a second bridge is actually funded (§5.5). |
| R13 | **`platform` and the bridge both claim the window.** Topmost and positioning need a window handle, but the toolkit creates the window — and what each toolkit permits differs (§5.5). | Startup-order bugs, deadlocks, or unportable code that assumed GPUI's HWND | `platform` primitives take a handle and decide nothing; the bridge owns the window; api routes. Honour the 4-step startup order in §5.5. |

## 9. Milestones

**M0 — Spike (do this first, timebox it).**
Plain `cargo` project, GPUI dependency, window opens on Windows. While there: confirm we
can obtain the raw window handle, set topmost, set an explicit position, and invoke a
native file dialog. **Exit criterion: yes/no on GPUI viability.** If no, we stop and pick
a different UI layer while the cost of changing our mind is still zero. Everything
downstream depends on this.

**M1 — Core engine, headless.**
`core` + `api` crates: document model, open/save/save-as with path rebinding, encoding
+ line-ending detection and preservation, atomic save engine, dirty tracking, recent-files
MRU, session/config serialisation, `resolve_state_dir()`, and the `Command`/`Event` gateway
(§5.4). Zero UI. Fully unit-tested, plus a headless "play a session" test driven through `api`.

**M2 — First usable UI (`bridge-gpui`).**
`bridge-gpui`: editor view, hamburger menu, Open / Save / Save As, native file dialogs,
recent files, drag-and-drop, and reopen-last-file on launch. The bridge imports only `api`,
and the §5.2 checks start running here so the boundary cannot drift later. Follow the
startup order in §5.5. **Exit criterion: you can use it as a notepad.** Also where the
chrome decision (§10.2) gets made — much cheaper now than retrofitted later.

**M3 — Window persistence.**
`platform` trait + Windows backend. Restore, validate against monitors, clamp off-screen,
handle maximised + DPI. Test on 100/150/200% scaling and with a monitor unplugged.

**M4 — Autosave & pin.**
Debounce, periodic flush, blur/close/quit flush, external-change detection, and the
byte-identical round-trip test suite (§4.5). Pin button + shortcut + persisted state.

**M5 — Polish & package (Windows).**
Cold-start time check, icon, crash handling, settings UI, `.notes` file association. Then
the two Windows artifacts: portable `.exe` and MSI installer. **Ship both from CI on every
tag from here on** — packaging that is only done at release time is always broken at
release time.

**M6 — Cross-platform seams.**
Audit `platform` for Windows leakage; macOS backend; Linux decision from §10.

**M7 — mac-install.**
Apple Developer account + certificate + notarisation pipeline (§7.1), universal binary,
`.dmg`. Longest lead time of any milestone and the least code in it — start the account
decision during M5.

**M8 — linux-install + linux-portable.**
`.deb` with proper desktop integration; AppImage with bundled GPUI system deps and a
stated glibc floor (§7.4). Test matrix: one old LTS, one current release.

M1–M4 are roughly independent and could interleave, but M2 is the one that turns this
from a library into an app — worth reaching early. M0 gates all of them. M7's
*administrative* work gates nothing and should start early, because it is the only
milestone with a lead time we don't control.

## 10. Open decisions

Numbering here is **stable**: resolved items are marked in place, never deleted or
renumbered, because references across `AGENTS.md`, the notes, the ADRs, and this document
address decisions by number (§10.3 and friends). Renumbering would silently mispoint them.

### Resolved

- **One note or many?** → Single document per window, file-backed. The menu (Open / Save
  As / Recent files) settles it. No sidebar, no index, no tabs in v1.
- **File granularity** → One file per note, on disk, user-visible. No app-owned database.
- **Multiple windows?** → One window in v1. Geometry and pin state are therefore global,
  which keeps §4.1 simple. Multi-window is a v2 decision and would make geometry
  per-window.
- **Autosave on files the app did not create** (§10.3) → option **B**: `.notes` autosaves
  immediately; a foreign file stays disarmed until one explicit save arms it for the rest of
  that document. Reasoning, consequences and reopening conditions: ADR-0001.

### Still open

1. **Is `.notes` earning its keep?** (§4.5) A distinct extension buys file association, a
   default for new files, and a home for per-file metadata — and costs discoverability,
   cross-app compatibility, and users' notes splitting across two file types. If we store
   no metadata, we should ship `.md` instead.
2. **Native title bar, or custom chrome?** Does the hamburger menu sit in the content area
   under a normal Windows title bar, or do we draw our own? Custom chrome is what makes
   the app look like *our* app, and is also R11: we then own drag, resize borders,
   Win+arrow snapping, double-click-maximise, and rounded corners. Decide before M2.
3. **Autosave policy for foreign files.** → **RESOLVED, option B — see ADR-0001.** The full
   option set is in ADR-0001 and its provenance note. Held at this number on purpose.
4. **Who owns the live text buffer?** `core` (every keystroke crosses the port — purest, but
   typing latency now depends on a thread hop, and the port would have to abstract text
   editing) or the bridge (GPUI holds the buffer, `core` holds the file and gets a snapshot
   on `Flush`). §5.5 makes this near-mandatory: **the bridge owns the editor**, because the
   editor is the one thing that cannot be abstracted across toolkits. See §5.4.
5. **Editing surface.** Plain text, Markdown source-only, or Markdown with live preview?
   Rich text stays out — it remains the single biggest scope risk in the project.
6. **Summon UX.** Global hotkey to pop it up from anywhere? System tray? Both are the
   features that most turn "a note window" into "the note window," and both are
   Windows-specific work.
7. **Linux promise.** Full parity (X11-only, documented), best-effort on Wayland, or drop
   the position/pin guarantees on Wayland and say so plainly?

### Packaging decisions

8. **Who owns the Apple Developer account?** Personal or org account, US$99/yr, and it
   gates mac-install with a lead time we don't control. Needs an answer by M5.
9. **Windows code signing — yes or no?** A real cost line, and it decides whether the
   first thing a user sees is a SmartScreen warning.
10. **Auto-update.** Not in the five artifacts, but expected of a desktop app. A self-updater
    is its own subsystem (download, verify, install, restart) and interacts with portable
    mode — a portable exe updating itself in `Downloads\` is a support ticket. In scope, or
    explicitly out?
11. **Where do these ship?** GitHub Releases only, or also Microsoft Store / Flatpak /
    Homebrew? Store and Flatpak each bring constraints that conflict with §7.3 and portable
    state.

## 11. Next step

M0. Nothing else is worth doing until we know GPUI can open a window on Windows and let
us set topmost and position — R1/R2/R3 all resolve with the same one-day spike, and every
other decision in this document is cheaper to make afterwards.

One thing to kick off **in parallel**, because it is administrative rather than technical
and therefore not gated by the spike: the Apple Developer account (§10.8). It is the only
item on the roadmap whose lead time we do not control.

---

## 12. M0 spike results

Run 2026-09-10 against **gpui 0.2.2** on Windows 11 (10.0.26200), rustc 1.98.0. Built in a
throwaway directory outside this repo, per §5.2 rule 3 — only its findings are recorded here.

### 12.1 Verdict

**GPUI is viable. Proceed to M1.** Every blocking question came back yes. R1, R2 and R3 are
resolved; nothing found suggests re-choosing the UI layer.

| | Question | Result |
|---|---|---|
| Q1 | Plain `cargo` project depending on gpui compiles and links on Windows | **PASS** — 441 crates, ~5 min cold, 1m10s incremental |
| Q2 | A window opens and enters the render loop | **PASS** |
| Q3 | Raw HWND reachable | **PASS** |
| Q4 | Topmost set **and cleared** | **PASS** — `WS_EX_TOPMOST` verified on, then off |
| Q5 | Explicit position/size honoured | **PASS with a finding that matters** — §12.2 |
| Q6 | Native file dialogs | **PASS at the API level** — exist and link; *presenting one still needs a human* |

### 12.2 The one real finding: two coordinate spaces

Requested `(120, 90)` sized `480x320`. Observed:

```
client area    480 x 320        <- exactly what was asked for
frame rect     origin (112, 71), size 496 x 359
chrome deltas   left 8   top 19   right 8   bottom 20
```

**gpui `Bounds` is client space. Win32 `GetWindowRect` is frame space.** They differ by the
window chrome, which is *not symmetric* — 19px above, 20px below, 8px each side, because
Win11 keeps invisible resize borders.

This is exactly the failure §4.1 predicted, now with a measured size. **Persist the frame
rect and restore it as gpui client bounds and the window drifts 8px left and 19px up on
every launch** — a tenth of the screen inside ten sessions, while the app confidently
reports that it remembered. It would have surfaced in M3 as an intermittent, hard-to-
attribute bug. Found on day one, for free.

**Consequences:**

- `platform/geometry.rs` must declare **one** persisted coordinate space and convert at
  both ends. Its "ONE documented unit" comment is now load-bearing, not tidy.
- Recommend persisting **frame** coordinates — that is what "where the window is" means to
  a user, and what §4.1 monitor-clamping must compare against — converting to client space
  only at `open_window`.
- Chrome deltas must be measured per window, never assumed constant: they vary with DPI,
  theme, and whether custom chrome (§10.2) is in effect.
- A regression test must assert a **restore → read-back round trip is stable across two
  launches**. Single-launch correctness cannot detect drift.

### 12.3 API facts worth keeping

- **No topmost API in gpui** — confirmed by source inspection. Pin is our Win32 code
  against the handle, which is exactly the split §5.5 assumed (`platform` primitives take a
  handle, the bridge owns the window). The design held.
- **`WindowBounds` is already `Windowed | Maximized | Fullscreen`**, so §4.1's "persist
  restored bounds *and* a separate maximised flag" maps onto the API 1:1. No workaround.
- **`window_decorations` supports server or client decorations** — the §10.2 custom-chrome
  option is available, not forced. R11 stays a decision rather than a constraint.
- **Dialogs are built in** (`PathPromptOptions`, `prompt_for_paths`, `prompt_for_new_path`):
  no `rfd`, no second event loop owning a window. R9 resolved.
- **`Context::spawn` takes `(WeakEntity<T>, &mut AsyncApp)` and `AsyncApp` has no `quit()`.
  The obvious "quit after the first frame" route does not exist; find another before M2.
- **The Windows backend is native**, not a shim: `directx_renderer.rs`, `direct_write.rs`,
  `directx_devices.rs`, `vsync.rs`, `display.rs` and more. That is what de-risks R2.

### 12.4 Watch items

- Pin **`gpui = "=0.2.2"`** and record why. 0.1.x is yanked and the API moved under it —
  R2's mitigation, now actionable.
- A transitive dep (`proc-macro-error2 v2.0.1`) emits *future-incompat:*; harmless today,
  check on every toolchain bump.
- 441 crates x 3 OSes is a slow CI matrix. Add registry and `target/` caching to `ci.yml`
  at M1 or every run pays for it.
- **Two checks remain undone:** presenting a dialog for real, and a human looking at whether
  the painted frame is correct. Both are minutes, and both belong before M2 is called done.

