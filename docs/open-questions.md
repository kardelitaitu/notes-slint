---
title: Open decisions
type: planning
owns: ['§10']
status: living
updated: 2026-09-11
---

# Open decisions

Owns **§10** of this project's plan. Part of the set indexed by
[whitepaper.md](../whitepaper.md) — section numbers are **global across the set**, so a
reference like `§4.5` resolves from any file. Each numbered section has exactly one owner;
the validator fails if one appears in two files.

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
2. **Native title bar, or custom chrome?** → **RESOLVED: custom chrome — see ADR-0003**,
   enabled by ADR-0002 (the bridge's dependency). One 34px title bar: hamburger and pin
   toggle left, filename centre (`Untitled` fallback), window controls right. No sidebar
   button. The full option analysis is in the ADRs and their provenance note. Held at this
   number on purpose. The consequence for R11 is recorded there: decided does not mean built.
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

