---
title: Keyboard traversal for the hamburger popup
status: proposed
id: 2026-09-14-menu-keyboard-traversal
created: 2026-09-14
updated: 2026-09-14
relates: [ADR-0001, ADR-0006, docs/features.md §4.4, 2026-09-14-popup-stays-inside-its-window, 2026-09-14-menu-footer-explains-itself, 2026-09-14-explicit-save-act]
decision: null
---

## Question

The popup has **seven** command rows and a list of recents — six when this note was filed, and
A5's `Save` row made it seven, which `plumbing.rs`'s own census states: "seven command rows
since A5's Save row, seven chord cells". A pointer reaches all of them; the keyboard reaches all of
them **one at a time, by memorised chord** — and the count of those chords is fifteen, not
fifteen-plus-ten: `SHORTCUTS` (`surface.rs:121-146`) is **five command chords**
(Ctrl+O, Ctrl+S, Ctrl+Shift+S, Ctrl+T, Ctrl+Shift+R) **and the ten `Alt+1..0` recents doors**, which
are rows 6 through 15 of the same table. So `Alt+1..0` is inside the fifteen, not an addition to
them. What the popup itself prints is the seven chord cells of those command rows, because a recents
row carries its slot number in its label instead of an `Alt+n` — and it is the status line's legend
that prints all fifteen. There is no way to move a visible current row with
`Up`/`Down` and press `Return`,
and `docs/features.md` §4.4's menu has never promised one, so this is not a regression: it is the
gap the previous slice found when it went looking for what the hand-rolled popup costs.

The `slint` skill states the cost in one line: `ContextMenuArea`'s entries "are exposed to
accessibility frameworks — **a hand-rolled overlay menu is neither**". Ours is hand-rolled by
decision (the rows carry the pin glyph, the checkmark, the shortcut legend, and the frozen needles
key off `row-about := TouchArea { col: 0; row: 5;`), so a screen reader sees a menu-shaped
Rectangle, and there is no focus to be seen either. `Alt+1..0` is why that is not total; it is not
a substitute.

## What was tried, and what the tree said

The traversal needs three keys that must **not** reach the editor: `Up`, `Down`, `Return`. In this
bridge a key that must not reach the caret is swallowed by the outermost `FocusScope` in
`main.slint` with `return EventResult.accept;`, which is the mechanism all fourteen chords and the
Escape dismissal already use.

Two facts came out of doing it rather than reasoning about it.

1. **The arrow keys are not named what they are.** `Key.Up`, `Key.Down`, `Key.Left`, `Key.Right` and
   `Key.Enter` do not exist — the compiler says `'Down' is not a member of the namespace Key`. The
   real names are `Key.UpArrow` / `Key.DownArrow`, and the Enter key is `Key.Return`. Checked by
   compiling all seventeen candidates at once; twelve exist, five of those names are wrong.
2. **One more swallowed key fails the frozen instrument, by name.** With a
   `if (event.text == Key.DownArrow && chrome.menu-open) { return EventResult.accept; }` branch in
   place — compiling, correct, one branch, one accept — the probe bin reports:

   ```
   chords::capture_path_ends_by_rejecting_everything_it_did_not_match ... FAILED
   left: 17   right: 16
   one accept per bound chord, plus Escape: fourteen commands in the table and ONE dismissal branch
   ```

   The row is `assert_eq!(body.matches("EventResult.accept").count(), SHORTCUTS.len() + 2, …)` at
   `probe.rs:2571-2575`, and the right-hand side is a **formula at `:2573`, not a literal 16**: the
   fourteen bound chords, plus Escape, plus the ONE branch that covers both replay keys in a single
   return (`:2566-2569` says so, and it is why S10b moved the count by one statement and not two).
   It *evaluates* to 16 while the table holds fourteen rows, which is what the failure printed as
   `right: 16` — and `left: 17` is this leg's one added accept against that formula. So traversal is
   not blocked by anything about the menu: it is blocked because **the instrument counts the
   markup's accepts, and the instrument is frozen** (ADR-0006 §4). `main.slint` was restored
   immediately; the working tree is clean and the count is 16 again.

## The workarounds, and why each is refused

- **Edit the count in `probe.rs`.** There is no `16` to edit — the row is `SHORTCUTS.len() + 2`, so
  the edit is to the `+ 2`, or a row smuggled into the table to move it. Forbidden either way, and
  it is the same law this repo applied to
  `popup-x` one slice ago: an instrument that acquires assertions after its verdict is a wish, not
  evidence.
- **Widen the Escape branch's condition to also accept the arrows.** Keeps 16 and would pass. It is
  also exactly what the previous bullet exists to prevent — the count is being protected rather than
  honoured — and it is a lie in the markup, since Escape closes and arrows move.
- **A nested `FocusScope` in `Chrome`, with focus moved into the popup while it is open.** Slint
  would deliver the keys there and the frozen count in `main.slint` would never move. It is refused
  on this file's own invariant: `main.slint:189-190` says the chord scope's authority comes from
  being outermost, that no imperative `focus()` routes a key, and that `claim_focus` is tolerated
  *specifically because it routes no key*. A popup that steals focus to win a keystroke is that rule
  inverted. It would also fight the startup caret handoff described in `product.rs:385-392`.
- **Let the arrows through and drive the selection anyway.** The cheapest by far, and the worst:
  `Down` would move the caret *and* the selection, and `Return` on a row would insert a newline into
  the user's document. That is buffer corruption bought for one line of markup.

## What would be built if this is approved

`Chrome` owns all of it, and none of it is contentious:

- `in property <int> cursor-row: -1;` over the index space `0..5` = the six command rows,
  `6 + i` = the recents — **bounded by `recents-shown`, not by `recents.length`**, so the selection
  can never sit on a row the height clamp refused to draw. A selection you cannot see is the same
  kind of lie the footer exists to prevent.
- Each row's existing band becomes
  `background: root.cursor-row == N ? Theme.menu-select-bg : Theme.hover-bg`,
  `opacity: row-X.has-hover || root.cursor-row == N ? 1 : 0` — one ternary per row, hover and
  selection distinguished because they answer different questions ("where the pointer is" vs "where
  `Return` will land"), and `menu-select-bg` derived like `hover-bg` is (`Palette.foreground
  .transparentize(k)`), not picked.
- One `function activate(row: int)` that maps the index to the callback the row's `clicked` already
  calls (`open-asked`, `save-as-asked`, `autosave-asked`, `clear-recents-asked`, `quit-asked`,
  `about-asked`, `open-at-index(row - 6)`) and then closes the popup — so the row order lives in
  exactly one more place, in the same file whose cell numbers the frozen needles already pin, and a
  guard asserts the two agree.
- `reset to -1` in the existing `changed toggle-asks` / `close-asks` handlers, so reopening starts
  unpointed rather than pointing at a row the clamp may have just un-drawn.
- In `main.slint`, three branches beside Escape, wrapped in the same condition shape Escape uses,
  and **accepted** — the accepts in the markup go 16 → 19 while `SHORTCUTS.len() + 2` still says 16,
  which is the exact shape of the failure recorded above.
- Guards: traversal never exceeds what is drawn; `activate` covers every cell the probe names; the
  select band is not the hover band; and `Up`/`Down`/`Return` are named by their real constants.

## Recommendation

**Ask for one sanctioned re-earning of the accept count, then build the above.** Three specifics:

1. The number should move **deliberately and with its reason written down** — a line in
   `docs/decisions/` (an ADR amending ADR-0006's frozen rows, or a dated paragraph in the strip
   record, whichever `doc-management` says owns it) naming that the row counts
   `SHORTCUTS.len() + 2` — fourteen chords, Escape, and the one replay branch that answers for both
   `z` and `y` — that a menu now has three more swallowing branches, and that the count is a
   *factual claim about markup* rather than a verdict about behaviour. The alternative — leaving it
   frozen forever — silently converts the freeze from "don't re-judge the past" into "this menu can
   never have a keyboard", which is not what ADR-0006 argued for.
2. If the answer is **no**, say so in `rejected/` with the consequence recorded on the same page:
   the popup is pointer-and-chord only, it is not exposed to accessibility frameworks, and §4.4's
   menu is deliberately narrower than a native menu. A declined proposal that keeps its consequences
   written down is a decision; a silent one is a debt.
3. If the answer is **yes to something bigger** — migrating the popup to `ContextMenuArea` (or a
   `PopupWindow`) so the toolkit owns traversal, focus and accessibility wholesale — that is a
   different slice with a much larger frozen-needle footprint (row cells, the backdrop catcher, the
   About row's `col: 0; row: 5;` shape, and the `set_close_asks` count). Worth naming as an option
   in the same conversation, because it is the only path where the accessibility half is *fixed*
   rather than painted over: traversal highlights a row, but a screen reader still would not see it.

**Undecided and not this note's to decide:** whether a person dragging a window to 190px should see
the popup cover the hamburger at all (the previous slice chose "whole menu, covered bar"), and the
~264px minimum window height that note recommends somebody own — a `min-height` makes the traversal
and the footer both simpler to reason about, and it is the same governance question in smaller
clothing.

## Two facts worth keeping wherever this lands

- `Key.UpArrow` / `Key.DownArrow` / `Key.Return`; there is no `Key.Enter`, and no `Key.Up`/`Down`/
  `Left`/`Right`. This belongs in the `slint` skill's gotchas list next to the unit-type row, and it
  is the second gotcha this slice found by compiling rather than by guessing — the first being the
  CRLF `'\r'` in a flattened guard, already recorded in `plumbing.rs`.
- The frozen instrument's accept count is a **load-bearing constraint on menu work**, and that was
  not knowable before trying to add a key. It should be discoverable: if this note is declined, its
  title and first paragraph are the thing a future reader needs, not the diff.
