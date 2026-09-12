---
title: Product
type: planning
owns: ['§1–§3']
status: living
updated: 2026-09-10
---

# Product

Owns **§1–§3** of this project's plan. Part of the set indexed by
[whitepaper.md](../whitepaper.md) — section numbers are **global across the set**, so a
reference like `§4.5` resolves from any file. Each numbered section has exactly one owner;
the validator fails if one appears in two files.

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

