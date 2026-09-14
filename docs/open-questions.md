---
title: Open decisions
type: planning
owns: ['§10']
status: living
updated: 2026-09-15
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

- **One note or many?** → Single document per window, file-backed. The menu (Open / Save /
  Save As / Recent files) settles it. No sidebar, no index, no tabs in v1.
- **File granularity** → One file per note, on disk, user-visible. No app-owned database.
- **Multiple windows?** → One window in v1. Geometry and pin state are therefore global,
  which keeps §4.1 simple. Multi-window is a v2 decision and would make geometry
  per-window.
- **Autosave on files the app did not create** (§10.3) → option **B**: `.notes` autosaves
  immediately; a foreign file stays disarmed until one explicit save arms it for the rest of
  that document. Reasoning, consequences and reopening conditions: ADR-0001.
- **Which act counts as that explicit save** (the affordance §10.3 was waiting on) →
  ADR-0007: `Command::Save`, on `Ctrl+S`, *is* the arming act ADR-0001 named. 0001 is
  honoured, not reversed — the wording was never wrong, the affordance was missing, and a
  missing affordance is a build queue. Its failure path now retries (§10.16). The argument
  is `2026-09-12-arm-on-explicit-save-path` — overtaken, and deliberately still in
  `proposed/`; the note that asked for the decision is archived as
  `2026-09-14-explicit-save-act` with its `decision:` set.

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
   editing) or the bridge (the bridge holds the buffer — `bridge-gpui` first, `bridge-slint`
   now shipping it — and `core` holds the file and gets a snapshot on `Flush` or on an
   explicit `Save`). §5.5 makes this near-mandatory: **the bridge owns the editor**, because
   the editor is the one thing that cannot be abstracted across toolkits. See §5.4.
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

### Opened by the UI, 2026-09-12 → 15

The first eleven slots were written before a window existed. These eleven were opened by the
two bridges and the records that came with them. Each names its note, which lives in
`.agents/notes/proposed/` and holds the options and the recommendation; this list is the
map, not the argument. **None of them is settled by a row moving** — `git remote -v` is
empty, so no CI run exists to make a status move legal in either direction, and an entry
below that describes missing evidence is a sentence about the instruments, never a verdict
about the product.

12. **Does a blank note deserve a place in the port?** A user who wants an empty note has
    one move today: quit and relaunch. `Command::New` waits on three answers a chord cannot
    supply — what a New costs the previous scratch, whether a fresh scratch is armed under
    ADR-0001's file-kind rule, and what the epoch does. A bridge clearing its own buffer is
    the shape the note refuses. `2026-09-13-new-document-command`
13. **What protects text when the note is switched?** `Event::Loaded` replaces the buffer
    and §4.4's unsaved-changes guard has no code, so an Open-while-dirty can drop text —
    with autosave working, and more often with autosave off. The hole predates
    `Command::Save`. Open: which side sequences the save-before-switch, and what happens on
    the refusal. `2026-09-14-note-switch-must-not-lose-text`
14. **What may a switch do while it waits for the save answer?** The narrower law: release
    on the answer that was actually sent, never on a counter, and on a refusal hold and tell
    until a second press confirms. The note says plainly that none of it is in the tree yet
    — a law written before its door. `2026-09-15-held-switch-waits-for-answer`
15. **May a debounced `Flush` write where an open was refused?** `Engine::save` and
    `Engine::save_as` both refuse to write a file whose open was refused; `Engine::flush`
    does not ask, so autosave can put a buffer into a document nobody saw. ADR-0007 already
    states the guard applies. Closing the asymmetry decides what an autosave says when the
    answer is no. `2026-09-15-flush-shares-the-refused-load-guard`
16. **Who owns the retry after a failed save?** The user stopped typing; does the app try
    again, how often, and who decides? The two bridges answer differently and ADR-0006 keeps
    one frozen. The channel law the note's addendum carries — a retry is a Save, never a
    Flush — now has code on the shipping side, a back-off that caps into a terminal state
    rather than silence, which leaves the real question: should `api` or `core` own the
    cadence, or is a per-toolkit habit the answer? `2026-09-14-autosave-retry-ownership`
17. **Which rows does the hamburger hold?** The chord half is answered and shipped — ADR-0007
    moved `Ctrl+S` to Save and `Ctrl+Shift+S` to Save As, in the one table that prints both
    the popup and the legend. Open: a `Recent Files >` submenu, where Auto-save, Clear
    recents and Quit sit once rows are added, and a "new tab" row whose plain reading is the
    declared non-goal above (one document per window, one window in v1) and therefore needs
    the proposal paragraph AGENTS.md asks for before it can be drawn.
    `2026-09-14-menu-six-rows`
18. **Does the popup traverse by keyboard?** A pointer reaches every row; the keyboard
    reaches them one at a time by memorised chord. The popup is hand-rolled by decision, and
    the note records what that costs: no accessibility exposure, no visible focus.
    Traversal would need a sanctioned re-earning of the frozen bridge's row counts, so "no"
    is a live option — to be written down in `rejected/` with its consequences, not left
    silent. `2026-09-14-menu-keyboard-traversal`
19. **A popup and a tooltip on a bar that refuses `Root`?** ADR-0003 drew both: the
    hamburger opens the §4.4 menu as a popup, and the centre may ellide only because the full
    path is always in a tooltip. M2 is deliberately `Root`-free, because `Root` would be a
    second owner of window-scoped text selection. Mount, hand-roll inline, or ship less than
    the ADR drew — and say which. `2026-09-12-titlebar-root-overlay-question`
20. **Who owns the minimum window size?** Dragging the bottom or right edge inward clips the
    About panel, and one of the lines it loses is the reason a file is not being saved.
    Markup, product bridge, or `platform` — and which promise bends when a floor rewrites a
    rect the user chose (§4.1 clamps a *position*, not a size)? The shared markup is read by
    the frozen instrument too, so a property there is not a product-only change.
    `2026-09-14-window-gets-a-floor`
21. **Does v1 notice a file changed outside the app?** §4.2 asks for mtime plus hash before
    overwriting. The port declares `Event::ExternalChange`, one bridge renders it, and no
    code produces it. Either it gets built as a scoped slice with an exit criterion, or the
    intent docs stop claiming it — and the note is explicit that the doc half waits on the
    same CI evidence that would make any §9 row move legal. Nothing here restates detection
    as shipped. `2026-09-14-external-change-event-is-never-emitted`
22. **What may we claim about the pointer?** No instrument here has ever watched a pixel
    reach a handler, and the four press legs carry no recorded green run. The open question
    is evidence, not behaviour: what the docs may say about M2's pointer surface until a
    person makes the two reads the note asks for. The honest sentence is exactly
    `M2 pointer interaction is untested` — which is not a claim that the menu is broken, and
    not a claim that the app is keyboard-only. `2026-09-15-no-instrument-tests-a-pointer`

