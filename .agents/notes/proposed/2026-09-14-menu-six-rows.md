---
title: The rows the hamburger should hold
status: proposed
id: 2026-09-14-menu-six-rows
created: 2026-09-14
updated: 2026-09-14
relates: [ADR-0001, ADR-0006, docs/features.md §4.4, AGENTS.md non-goals, 2026-09-14-menu-keyboard-traversal, 2026-09-14-explicit-save-act, 2026-09-14-popup-stays-inside-its-window, 2026-09-14-menu-footer-explains-itself]
decision: null
---

## Asked

On 2026-09-14 (round 8) the menu's contents were specified directly:

```
Open
Recent Files >
Save
Save As
Auto-save (toggle)          <- added in the same reply, "under Save As"
Open in new tab toggle
About
```

The current popup is `Open · Save As · Auto-save (on|off) · Clear recents · Quit · About` plus the
recents as rows beneath. This note is the distance between the two, because three parts of it are
decisions and one is an existing non-goal.

## What follows, item by item

**`Save` — consistent, and already approved.** Round 8 approved building `Command::Save`
(`2026-09-14-explicit-save-act`), so this row is the act ADR-0001 already assumes: its arming rule is
written as "until the user performs one explicit save (`Ctrl+S` or the Save menu item)". Both exist
then, and the footer's "Save As once and it keeps saving" copy changes to name the real door. Touches
`api` (a new `Command`), both bridges' `describe` arms, `route_of`, the chord table.

**`Recent Files >` — a submenu, not inline rows.** This is the one that un-builds recent work rather
than extending it: the recents are currently five-of-ten rows *inside* the popup, which is what the
height clamp (`519df1c7`), the y bound (`140106fd`) and the About x rule (`5c8cf189`) were all sized
around. A submenu means a second surface that opens beside the popup, its own placement against the
host on both axes, its own dismissal, and — because 1.17's `PopupWindow` is a separate native window
while our popup is a `Rectangle` — a child-window question the current `no-frame` transparent window
has never answered. The chord law also moves: `Alt+1..0` currently opens slots 1..10 with the number
printed *in the label*; under a submenu the chord opens a hidden panel, or it still opens the file
with no visible row to point at. Worth deciding on purpose rather than discovering mid-edit.

The count above is the audit's correction, not the author's find: this line said six-of-ten when the
note was written, and the shipped cap is FIVE — `Math.min(5, root.recents-fit)` at chrome.slint:798,
drawn from `recents-fit` at :791. Ten still exist in the model (`MAX_RECENTS`), and the rows six..ten
are the reachable-undrawn ones the same file names at :700; the popup has never drawn more than five
of them, so every number sized "around" this list sizes around five.

**`Open in new tab toggle` — a declared non-goal, so it cannot be built from a chat line.**
AGENTS.md: "Deliberate non-goals: **Tabs or multiple documents per window**", and the instruction that
follows is "Do not add these without a note in `.agents/notes/proposed/` first". This note is that
paragraph, not an implementation. Two things it would need that the app does not have: somewhere to
put a second document (the whole engine is one `Document` per window: `core::Document`, one session
path, one autosave witness), and a *toggle* whose second state means "open the next file in the same
window instead of a new one" — which changes the pin's job. The pin already means "this window stays
on top and keeps the file"; a new-tab toggle overlaps it in the only way that matters (what happens to
the next file you open). If tabs are wanted, the honest version is a proposal that replaces or bounds
the pin's meaning, and it is the single largest scope change available to this menu — the reason
`whitepaper` §8 lists multi-document as a risk rather than a feature.

**`Auto-save (on|off)` — kept, at your reply, and that resolves the guard problem.** It stays the
only pointer door to `Ctrl+T` and keeps the "(on)/(off)" state read-back. Placing it **under `Save
As`** is also where the meaning wants to sit: Save As is the act that arms autosave on a foreign file
(ADR-0001), so the row that shows and toggles the arming verdict follows the row that changes it, and
the footer's two lines sit directly beneath both. The list is then **seven rows**, which is the number
the popup's height, the grid's own row arithmetic and `probe.rs`'s `row: 5` cells all have to be
recounted against — About is no longer the 6th row, so the frozen needles move with it.

**`Clear recents` and `Quit` disappear.** `probe.rs` pins About's cells as `row: 5` and cites "the
same shape as Quit's absence" — under the new list About becomes row 5 anyway only if the recents
leave the grid *and* nothing else shifts, which needs counting rather than hoping. Quit leaving the
menu leaves the close button in the caption slot as the only pointer door out of the app; acceptable
if intended, since the window has no OS frame to fall back on.

## What is already predicted, and is useful here

`39239fd5` (the legend crossing) asserts both directions: every chord cell names a key something
binds, **and** every bound non-slot chord has a cell. Keeping `Auto-save` is what stops that guard
firing on a key with no row. What it *will* fire on is the new pair: **`Save` and `Save As` cannot
both hold `Ctrl+S`.** Today the table binds `("ctrl-s", "Ctrl+S", "Save As", "save-as")` and the popup
prints `Ctrl+S` on the Save As row; ADR-0001 meanwhile says the arming save is "`Ctrl+S` or the Save
menu item", which reads as `Ctrl+S` belonging to **Save**. So one of the two is being displaced, and
the choice is a chord reassignment: it moves a row in the load-bearing table, both bridges' legends,
the synthetic driver that walks the table, and the guard named above. The likeliest pairing is
`Ctrl+S` = Save with `Ctrl+Shift+S` = Save As, which keeps ADR-0001's sentence true and leaves the
shifted chord meaning "the variant that asks where", but it is a user-visible key change and should be
decided on that footing rather than inherited from a table row written before the Save act existed.

That guard also pins `x: root.popup-x;` at exactly one consumer (the 190px menu) and the frozen
expression at one spelling; a submenu that reuses the popup geometry has to place itself, so expect
that count to be part of the conversation.


## Recommendation

**Build it in three steps, and answer one question first.**

1. **Answer the tab question** (below) before any markup moves: replace the row with a real
   "open in a new window" act — which this app *could* mean without tabs, if "new tab" was shorthand
   for "a second window on the same file set" — or drop it, or write the multi-document proposal that
   AGENTS.md asks for and accept that it is the biggest change to the product's shape.
2. Then land the cheap, unambiguous half: **`Save`** (approved, needs `api` + both bridges) and
   **`min-height`/`min-width`** (approved), neither of which depends on the submenu or the tab.
3. Then, separately, the **`Recent Files >` submenu**, sized as its own slice with its own placement
   rules, because it removes work that was measured this week and raises the child-window question.

Keeping the removed acts (`Auto-save`, `Clear recents`, `Quit`) in the popup as a tail, or moving
Auto-save into the footer where its reason already lives, are both smaller changes than letting the
only pointer door to a chord disappear without a line said about it.

## What would settle it

- Whether "new tab" means a second document in this window (a non-goal), a second window (not a
  non-goal, and not what a tab usually means), or a mode switch for where the next Open lands
  (overlapping the pin).
- Whether `Recent Files >` keeps `Alt+1..0` pointing at files or pointing at the panel.
- Where `Ctrl+T` lives visually once its row is gone, and whether the table drops the row with it.
