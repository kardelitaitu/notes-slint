---
title: Core behaviour
type: planning
owns: ['§4']
status: living
updated: 2026-09-11
---

# Core behaviour

Owns **§4** of this project's plan. Part of the set indexed by
[whitepaper.md](../whitepaper.md) — section numbers are **global across the set**, so a
reference like `§4.5` resolves from any file. Each numbered section has exactly one owner;
the validator fails if one appears in two files.

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

- A visible toggle in the window chrome — the custom title bar's left cluster, beside the
  hamburger (ADR-0003) — plus a keyboard shortcut.
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
- **Drag-and-drop a file onto the window to open it.** The fastest path for this kind of app —
  and, contra the first version of this bullet, which priced it as "nearly free", **not free on
  either toolkit**: gpui-pre rewrites the shell drop into an internal `active_drag` a bridge
  cannot reach, and Slint's winit backend has no arm for the event at all, so it falls into
  `_ => {}`. What ships instead is a drop target owned in `platform`, armed through one
  synchronous, thread-affine call at the port (ADR-0004) and polled by the engine into the **same
  `Command::Open` the menu uses** — so a drop is an open, arms no new autosave rule (ADR-0001
  unchanged), and adds no `Command` and no `Event`. Built and shipping on the Slint bridge;
  `bridge-gpui` does not arm it (the S7 slice was not run).
  Chain: [2026-09-13-file-drop-chain](../.agents/notes/implemented/2026-09-13-file-drop-chain.md).
- **Title display.** The title bar's centre shows, in order: the open file's real name with
  its real extension (§10.1 changes the default, not this rule), else `Untitled` for a new
  unsaved document, else the app name. A dirty document carries a dot that turns amber on
  `SaveFailed` — the same slot, both states. Full path in the tooltip, which is what
  licenses truncating the name. Chrome spec: ADR-0003.
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

