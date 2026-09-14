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
  `popup-floored`: a flag that admits beats a screenshot that hides. The flag admits and the system
  does not listen — none of the four new flags is bound in `main.slint` or printed anywhere, which
  is recorded under *Known limits of this ladder* rather than smoothed over here.

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

## What was read off the code, and what was sampled

The heading used to say *measured, not asserted*. That overstated it in one direction and hid the
other: most of the ladder is arithmetic read off `chrome.slint`, and arithmetic is the better
evidence here because it can be re-checked without a render. Re-derived from the file for this
repair — `Theme.bar-height` 28px, `Theme.menu-pad` 4px, `Theme.menu-gap` 2px,
`Theme.menu-row-height` 20px (`theme.slint:126, 183-184, 199`), and `menu.rows-bottom` =
`rows.y + rows.height` = 4 + (6·20 + 5·2) = **134** (`chrome.slint:822, 855`):

- `recents-budget` (`:786-790`) = H − 28 − (134 + 4) − footer − 2 − 2 = **H − 214**;
- `recents-fit` (`:791-792`) = ⌊(budget + 2) / 22⌋ = **⌊(H − 212) / 22⌋**, and `recents-shown`
  (`:796-798`) is that capped at 5 and by the length of the list;
- `menu.height` (`:837-839`) = 8 + 130 + 46 + 22n = **184 + 22px per shown recent**;
- `popup-top` (`:198-202`) = max(0, min(28, H − needed − 2)), and `menu-raised` is that value
  being **strictly** below 28.

One term is not a constant: the footer's 44px is `1px + menu-gap + notes.min-height` (`:756-758`),
and `notes.min-height` is the layout's own demand for the three caption lines this harness feeds it.
That is the single number a render supplies, and it is exactly why the caveat at the end of this
section says the table's rows are not invariants.

The preview harness (`slint-viewer --screenshot --backend software --load-data`), with the root
`Window`'s height patched and `host-height` fed `root.height` exactly as `main.slint:371` feeds it
in the product. Ten recents seeded, the footer carrying a two-line reason plus the file line.

| window | menu: top → bottom | menu height | state |
|---|---|---|---|
| 640 | 29 → 322 | 294 | whole, five recents |
| 300 | 26 → 297 | 272 | whole, four recents, risen 2px (re-derived, not re-sampled) |
| 258 | 27 → 254 | 228 | whole, two recents — at rest, see below |
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

**Two cells were wrong, and both are repaired against the arithmetic above rather than against a
render.** The `300` row read `29 → 278 | 250 | whole, three recents`, which is the **280** host's
answer pasted one row up. At H=300 the budget is 86px, the fit is ⌊88 / 22⌋ = **four** recents,
`menu.height` is 184 + 88 = **272**, and `popup-top` = min(28, 300 − 272 − 2) = **26**: risen by 2px,
bottom at 298 of 300, whole — so the row now reads 272 / four recents, and `250 / three` belongs to
280 and 299 alike, not to 300. The `258` row is the mirror-image error, a sampled pixel read as a
state: `popup-top(228)` = min(28, 258 − 228 − 2) = min(28, 28) = **28 exactly**, so the popup is
*at rest* there with 2px of slack to spare and `menu-raised` is **false**, because the flag asks for
strictly below the bar. Every reading in the top column carries ±1 (29 for a computed 28, 13 for a
computed 14) — the border the sample reads from inside — so that column cannot resolve a 2px rise at
all. Where a row says "risen" and the arithmetic does not say it too, it is not evidence; the rows
that survive this are the ones whose height and recents count solve exactly, and the 640 / 236 /
220 / 214 / 200 / 190 / 170 cells do.

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

### Known limits of this ladder

**The two tiers do not know about each other, and 26px of window is lost in the gap.** The recents
clamp budgets as if the popup will always sit at rest under the bar — the first term it subtracts
*is* `Theme.bar-height` — while the rise tier may spend up to all twenty-eight of those pixels,
holding back only `menu-gap`. Showing n recents costs **H ≥ 212 + 22n** to the clamp, while the same
n rows stay whole from **H ≥ 186 + 22n** once the popup may rise: a **26px band at every tier** in
which the list has already given up rows the rise would have kept. The un-costed band is therefore
hosts 186 + 22n through 211 + 22n: **296–321** for the fifth recent, 274–299 for the fourth, and the
same 26px all the way down. Inside each of
them the list is one row shorter than the window could have carried, and the top of the ladder makes
it concrete: at H=300 the shipped arithmetic draws four recents (272px, risen 2px), while five would
have been whole — 294px of menu at `popup-top(294)` = 300 − 294 − 2 = 4px, bottom at 298 of 300. The
fifth recent is refused below 322 and the rise would have held it from 296, so 26 pixels of window
are spent on nothing. Nothing in the repo costs this: no test asks what the rise tier would have
spared. Whether the band *should* be spent is a decision rather than a bug to fix quietly — a whole
menu bought with the bar, against one fewer recent bought with nothing — and it belongs with the
minimum-height question in Recommendation 1.

**The raise is invisible from outside.** Four flags exist to admit it — `menu-raised`
(`chrome.slint:203`), `about-raised` (`:205`), `about-floored` (`:235`) and `about-overflow` (`:240`)
— and not one of them leaves the file that declares it. `main.slint` forwards only the older x-axis
pair (`popup-floored`, `popup-overflow`, `:169-170`), and the instrument prints only those
(`probe.rs:1837`), so no needle, log line or status string can see a risen popup or a floored
licence panel. Three of the four are guarded, and guarded against *declaration* rather than
behaviour: `plumbing.rs:703-704` asks only that the markup still contains the two
`property<bool>…-raised` lines, and `:893-896` that `about-overflow`'s expression still reads
`root.host-width<about.width`. `about-floored` is guarded by nothing at all. So the line in *What was
built* — "a flag that admits beats a screenshot that hides" — is true of the flag and false of the
system: the admission is unread, and wiring it needs a voice that is not the frozen instrument's,
which is the same re-earning `2026-09-14-menu-keyboard-traversal` is waiting on.

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
