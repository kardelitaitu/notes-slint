---
title: The popup keeps itself inside the window it was given
status: implemented
id: 2026-09-14-popup-stays-inside-its-window
created: 2026-09-14
updated: 2026-09-14
relates: [ADR-0001, ADR-0006, docs/features.md §4.4, 2026-09-14-menu-footer-explains-itself]
decision: null
---

## Question

The popup had a clamp on one axis and none on the other. `popup-x` / `popup-floored` /
`popup-overflow` bound it against the host's left and right; `menu.y` and `about.y` were both
`parent.height` — a fixed drop below the 28px band, as if the window went on forever downward.

Three facts made that a defect rather than a style:

- **Nobody enforces a minimum window height.** Not the `Window`, not the bridge, not `platform` —
  searched, nothing. A short window is reachable by dragging the bottom edge up.
- **Slint clips a child at the client edge silently**: no scroll, no warning, no resize.
- **What gets clipped is the bottom of the menu** — which in this design is the About row (the
  licence obligation) and below it the two footer lines ADR-0001 requires because "Silence is
  forbidden".

And the toolkit's own guidance agrees the missing half is the missing half: the `slint` skill's
recipe for a hand-rolled overlay says to anchor the panel "**clamping both edges**". This repo had
implemented one of those two words.

## What was built

`chrome.slint` gains one function and two flags, and both hanging blocks go through it:

```slint
function popup-top(needed: length) -> length {
    root.host-height <= 0px
        ? Theme.bar-height
        : Math.max(0px, Math.min(Theme.bar-height, root.host-height - needed - Theme.menu-gap))
}
```

- `menu.y` → `root.popup-top(menu.height)`, `about.y` → `root.popup-top(about.height)`: each
  measured by **its own** height, because About (340x260, fixed) runs out of window 76px sooner
  than the menu and a fix that stopped at the menu would leave the licence clipping first.
- `host-height <= 0px` stands the rule down, so a harness that never tells the bar its window's
  height — including anything that instantiates `Chrome` bare — keeps the old constant behaviour.
- `menu-raised` / `about-raised` say out loud when the popup had to rise, in the same voice as
  `popup-floored`: a flag that admits beats a screenshot that hides.

**The order in which things give way** is the whole design, and it is now three deep: the recents
yield rows first (a row that isn't drawn is still reachable by `Alt+1..0`); then the popup rises and
pays with the bar's lower pixels; and only if the window is smaller than the *irreducible* menu does
anything clip. The footer is in no tier — it yields to nothing, which is what the previous slice
bought and this one preserves: `footer-height` is subtracted from the budget before the list is
counted, and the footer's own block reads no `host-height` at all.

**What was deliberately not touched: `popup-x`.** The frozen instrument prints it
(`probe.rs:1837`, `popup-x=… floored=… overflow=…`) as recorded evidence, and ADR-0006 §4 forbids an
instrument acquiring new assertions after its verdict. So the width rule stands exactly where its
verdict left it — including the known gap that About is 340px wide while `popup-x` is computed for a
190px `Theme.menu-width`. That is a real overflow on a narrow host and it is recorded below rather
than quietly fixed, and `the_popup_keeps_its_bottom_inside_its_window` now pins the formula so a
future edit has to mean it.

## What was measured, not asserted

The preview harness (`slint-viewer --screenshot --backend software --load-data`), with the root
`Window`'s height patched and `host-height` fed `root.height` exactly as `main.slint:371` feeds it
in the product. Ten recents seeded, the footer carrying a two-line reason plus the file line.

| window | menu: top → bottom | menu height | state |
|---|---|---|---|
| 640 | 29 → 322 | 294 | whole, five recents |
| 300 | 29 → 278 | 250 | whole, three recents |
| 258 | 27 → 254 | 228 | whole, two recents, risen 2px |
| 236 | 27 → 232 | 206 | whole, one recent |
| 220 / 214 | 29 / 27 → 212 / 210 | 184 | whole, no recents |
| 200 | **13** → 196 | 184 | whole — the popup now overlaps the bar |
| 190 | **3** → 186 | 184 | whole |
| 170 | 1 → 167 | 167 of 184 | clipped: the window is smaller than the menu |

| window | About: top → bottom | state |
|---|---|---|
| 640 / 320 / 300 | 29 → 286 | whole at rest |
| 280 | **19** → 276 | whole, risen |
| 270 | **9** → 266 | whole, risen |
| 262 | **1** → 258 | whole — the last height that can hold it |
| 240 / 200 | — | clipped (258 and 199 measured of 260) |

The arithmetic is exact rather than approximate: at H=200 the measured top is **13**, which is
`200 − 184 − 2 = 14` plus the 1px border the sample reads from inside. And the cost was checked
where it was paid — sampling the bar's stroke rows inside the popup's x-range found popup colour
(42,42,42) over them at H=200 and H=190 and not at H=214, which is the overlap the comment promises
and nothing more than that.

Before this change the same ladder clipped the menu below **212** and About below **288**.

Renders are at `%TEMP%\clamp-menu-*.png` and `%TEMP%\clamp-about-*.png`. This agent declares no
image input, so every check above is numeric pixel sampling rather than a look — which for geometry
is the stronger claim, but it is not a substitute for a person seeing whether a risen popup reads as
a rescue or as a glitch.

## Recommendation

1. **Somebody should own the minimum window height.** The menu can now keep itself whole, but only
   down to the size of its own contents; below 186px (menu) and 262px (About) something clips, and
   no clamp can fix a window smaller than the thing inside it. The number that makes both whole at
   worst is **~264px**. A `min-height` on the `Window` in `main.slint` is a one-line change with a
   restore-path consequence (a persisted smaller rect must be clamped on the way in), and it changes
   what a person can do by dragging an edge — so it is a design decision, not a menu detail, and it
   belongs to whoever owns the window. Ask, or propose it in `.agents/notes/proposed/`; do not leave
   "nobody enforces this" as the steady state.
2. **`popup-x` is not a clamp, and About has now been taken off it.** The expression is
   `Math.max(8px, Math.min(Theme.menu-inset, host - menu-width - 8px))` and its floor (8) sits *above*
   the inset (4) its ceiling is capped against, so it returns **8px on every host** — arithmetic at
   twelve widths, then pixels: the menu's left edge is x=9 (8 + border) at 1200, 800, 400, 360, 250,
   200 and 190 alike. `popup-floored` is true everywhere, so the flag that exists to "admit the
   remainder" reports a tautology, and the branch its own comment calls "the interesting case — a
   narrow host where the ceiling takes over" cannot execute. The rule is frozen evidence
   (`probe.rs:1837`, pinned by a count in `plumbing.rs`) so it stays exactly where its verdict left
   it; what changed is that About — 340px where the expression assumes 190px — now has its own
   `popup-left(about.width)`, resting at the same 8px so the two panels still line up, ceiling at the
   host, and **a floor of zero, because a cut licence is worse than no left margin**. Measured: on a
   340px host the panel sits at x=1 with its right edge at 338, whole, where the inert rest left it
   hanging 6px off the edge; at 800/400/360 it did not move at all, which is the point. Below 340 no
   placement exists and `about-overflow` says so against `about.width` rather than a second copy of
   the literal that `probe.rs:2105` pins.
   The remaining item is the frozen half: making `popup-x` a real clamp needs fresh evidence, which is
   the same re-earning keyboard traversal is waiting on (`2026-09-14-menu-keyboard-traversal`).
3. **The menu is not accessible, and the skill says so in terms.** `ContextMenuArea`'s entries "are
   exposed to accessibility frameworks — a hand-rolled overlay menu is neither". Ours is hand-rolled
   by decision: the rows carry the pin glyph, the checkmark, the shortcut display, and the frozen
   probe's needles key off `row-about := TouchArea { col: 0; row: 5;`. So the gap is not total (every
   row has a chord) but a screen reader sees a menu-shaped Rectangle. Worth a `proposed/` note with
   reopening conditions rather than a migration on a whim.
4. **Promote the preview recipe into the `slint` skill.** Second slice in a row whose proof had to
   be a rendered harness because the popup's inputs are markup-only and this session cannot drive
   the OS mouse. It is no longer a one-off.

## What is still not proven

- **No live product witness of a raised popup.** Reaching these heights in the real window means
  resizing it, and the OS input door was proved shut in an earlier round (no foreground rights from
  this session). The renders are the product's own markup with a real `host-height`, which is the
  same input `main.slint` supplies — but it is not the product.
- **186 and 262 are the floors for *these* contents.** A reason string that wraps to three lines, a
  longer file line, or a translated row label moves them; the clamp moves with them because it reads
  `footer-height` and `menu.height` rather than a constant, but the numbers in the table are not
  invariants.
- **Whether covering the bar reads correctly to a person.** At 190px the popup sits over the
  hamburger and the pin. Nothing was measured about that being *good*, only about it being whole.
- **About's horizontal overflow at narrow widths** (Recommendation 2) is unfixed on purpose and
  unmeasured in this slice.
