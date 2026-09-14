---
title: Who owns the minimum window size
status: proposed
id: 2026-09-14-window-gets-a-floor
created: 2026-09-14
updated: 2026-09-14
relates: [whitepaper §4.1, whitepaper §5.2, whitepaper §5.4, whitepaper §5.5, whitepaper §9, whitepaper §10, whitepaper §12.2]
decision: null
---

# Who owns the minimum window size

This is the artifact `c42b21d0` named. Its last paragraph is the ask, quoted so nobody has to
trust my paraphrase of a commit message:

> THE FLOOR MAY RETURN via a real .agents/notes/proposed/ note plus a probe arm that re-earns the
> narrow-host leg, or a sanctioned needle edit - the ask is the artifact. The clipping bug it was
> written against (a dragged-shrunk window cutting the bottom off the licence panel) is still open
> and still nobody's; only the fix is gone.

It carries no `> **DECIDED:**` line, deliberately. The reverted commit opened with "Approved in
round 8", and the revert's finding (c) is that no round 8 exists anywhere in this repo; a proposal
that repeats an unverifiable authority is how the first one died. This note asks. It does not
report an answer it cannot point at.

It is also, per `2026-09-14-popup-stays-inside-its-window` Recommendation 1 and
`2026-09-14-menu-footer-explains-itself`:191-193, the third note in a row to say somebody should
own this and then not own it. That is the debt this file exists to close.

## Question

A person drags a window's bottom or right edge inward and the app clips its own obligations: the
About panel loses lines, and one of the two lines it loses is the reason a file is not being saved.
Nothing in the tree stops that. So: **who owns a minimum window size - the markup, the product
bridge, `platform`, or nobody - and by what mechanism that leaves the frozen instrument intact?**

Two constraints make this a design question rather than a one-line change:

1. `crates/bridge-slint/ui/main.slint` is **shared by both bins**. `ui_gen.rs:17-20` mounts it for
   the crate, and `probe.rs:1880` pulls the same file into the instrument's own test module with
   `include_str!`. A property written there is a change to the behaviour of a frozen needle, not
   only to the product.
2. The promise being bent is §4.1's: the window "comes back the size and in the place you left
   it". A floor that rewrites a rect the user chose is a product decision, and §4.1 already says
   the persisted rect is the user's, clamped only for **position** when a monitor disappears.

## The clip is arithmetic at HEAD - and nothing in the tree ever reaches it

**It is real, and it needs no measurement to state.** With no minimum anywhere, at host width 180:
`popup-left(340) = Math.max(0, Math.min(8, 180 - 340 - 8)) = 0px` (`chrome.slint:230-234`), so the
panel's left edge is already at the client edge and 160px of a 340px obligation sits past it. At any
host height below 262, `popup-top(260) = Math.max(0, Math.min(28, H - 262)) = 0px`
(`chrome.slint:198-202`) and the panel tops out at y=0, so `260 - H` px are cut off the bottom. No
placement rule can recover it, because both rules' floors are already 0: `popup-top` cannot go
above the bar and `popup-left` has no margin left to spend. `Chrome` can keep a popup inside the
host it is given; it cannot make a host bigger than itself. `chrome.slint:179-183` and
`:764-769` already say this in the file's own words - "Nothing in this app enforces a minimum
window height - not the Window, not the bridge, not `platform`".

**And it is unmechanted everywhere, not merely unasserted.** Repo-wide there are **zero** hits for
`WM_GETMINMAXINFO`, `MINMAXINFO`, `ptMinTrackSize` or `min_width` under `crates/`. `platform`
has no `WndProc` in `src` at all: the only window procedure under `crates/platform` is a test
fixture (`crates/platform/tests/geometry_live.rs:140`, whose `type WndProc` sits at `:474`).
`core`'s `Rect::clamped_to` (`geometry.rs:110-124`) moves a rect; it never edits `w` or
`h`. `bridge-gpui` builds `WindowOptions` from `bounds_for(&initial)` (`main.rs:1574-1586`) with
nothing size-related in it, and has no licence panel to clip - zero occurrences of `About` anywhere
in `crates/bridge-gpui/src`, and its native menu rows are Open / Save As / Open Recent / Clear /
Auto-save (`menu.rs:193-209`). **So this is `bridge-slint` work only: no second bridge is affected,
and this note does not promise parity it has no subject for.**

**The narrow-host leg measures a different panel.** The instrument's 180px arm is
`probe.rs:1036-1037` (`2 => w.set_size(LogicalSize::new(180.0, 600.0))`), step 3 restores the
original size (`:1038-1044`), and About is only opened at step 8, `ui.set_about_asks(...)`
(`:1053`) - after the host is wide again. The line that prints is `popup_words()` (`:1835-1842`),
which reports `popup-x` / `popup-floored` / `popup-overflow`, i.e. the **190px menu**
(`theme.slint:181`). So the frozen run never shows the licence at a narrow host. The clip is not
just unasserted; it is never exercised.

## What the revert actually charged

`6aafbe6b` added exactly 13 lines to `main.slint` - ten comment lines, `min-height: 264px;`,
`min-width: 340px;`, one blank - at `:33-45`, immediately under `resize-border-width: 8px;`.
`c42b21d0` removed the same 13; HEAD's `main.slint` is byte-identical to the fix's parent. It
carried no test, no needle, no note, no §9 row. The revert's three findings, restated against the
code:

- **(a) critical - it deleted the input state of a frozen needle without touching the needle.**
  `min-width: 340` turns `probe.rs:1037`'s `set_size(180, 600)` into a request for a **different
  window**. The arm still runs, still prints, and measures a clamp instead of the geometry it was
  built for. The freeze (ADR-0006, Decision item 1: "rows may not loosen") reached from the window
  side, which is the same violation in a different coat.
- **(b) major - two overflow flags became unreachable-by-construction.** `popup-overflow` lights
  only below a ~202px host (`8 + 190 > host - 4`; the expression is inert at 8px for every host -
  see `chrome.slint:208-219`), and `about-overflow` is `host-width < about.width`, i.e. below 340
  (`:240-241`). A 340 floor kills both. With `no-frame: true` (`main.slint:29`) there is no OS
  edge to argue with, so neither could ever set again. This repo's own reason for those flags is
  that a flag that admits beats a screenshot that hides; a flag that can never light is worse,
  because it also passes review.
- **(c) the authority was not in the tree.** That is the sentence this file is the answer to.

## Options

### A. An unconditional floor in the shared markup - **fatal**

`min-width: 340px; min-height: 264px;` on `Spike inherits Window`. This is what shipped and what
was reverted: findings (a) and (b) are its, and `probe.rs:1880` (`include_str!`) is the reason a
"product-only" markup edit is not product-only. Dead.

### A-prime. Two inert knobs in the markup, the decision in the bridge - **recommended**

`main.slint` gains two properties that decide nothing:

```slint
in property <length> floor-width: 0px;
in property <length> floor-height: 0px;
min-width: root.floor-width;
min-height: root.floor-height;
```

and `product.rs` makes the call once, before the only `set_size` the product has
(`:355`, with `set_position` at `:356`): `ui.set_floor_width(340.0)` /
`ui.set_floor_height(262.0)`. The probe never calls those setters, so both knobs stay at `0px`
there and `180x600` at `probe.rs:1037` measures exactly what it measures today.

This is the mechanism/decision split §5.2 exists to enforce: markup provides a door, the bridge
provides the number, `api` stays ignorant of UI types, and `platform` decides nothing - which is
what its own header demands (`lib.rs:10-12`: "whether to clamp, snap, centre, **enforce a minimum
size** ... All of that is policy and lives above this crate").

### B. `WM_GETMINMAXINFO` / `ptMinTrackSize` via a new `api` Command - **the only true bound, and not affordable**

Only the window manager can make a drag actually stop, so B is the technically correct answer: a
resize border (`main.slint:30`, `resize-border-width: 8px`) is winit's, and a clamp applied after
the fact is a rubber-band, not a wall. But there is no `WndProc` in `platform`, so B means new
`unsafe` plus subclassing a window winit owns - writing into somebody else's window procedure -
and it means a new `Command` variant (`command.rs:43-162` has none: Open, SaveAs, Flush,
SetAutosave, SetPinned, SetCornerRounding, ClearRecents, Shutdown, RegisterWindow, GeometryChanged,
UnregisterWindow) carrying a product number across the seam. That last half is the problem: 262 and
340 are not OS facts. B puts a decision in platform clothing, and it is exactly the shape AGENTS.md
calls "a design change dressed as a small feature". Worth recording as the eventual answer for
*drag* behaviour; not this slice.

### C. A bridge-side clamp - **mechanically unable**

There is no resize hook to hang it on: no `resized` callback is used anywhere in
`crates/bridge-slint/src`, the product writes a size exactly once (`product.rs:355`), and the only
interactive geometry writer is the drag path's `window.set_position(PhysicalPosition::new(..))`
(`surface.rs:1950`), which is position and not size - pinned to exactly one call site by the count
at `surface.rs:3270-3274` ("drag_destination must be the only place this file moves the window").
So C is a poll: notice the window is too small, `set_size` it back, and meanwhile the
too-small rect has already been persisted, because `GeometryChanged` fires from the drag and the
port **measures** the live window (`engine.rs:1804-1836` per the drag-path note).
A rubber-band that saves the stretched state on its way back. Worse than A′, which costs one line
more and no polling.

### D. Make the footer adaptive - **attacks the cause, edits frozen evidence**

A shorter panel cannot be cut off. But the panel *is* its obligation text, and its first edit is
the pinned box: `width: 340px;` / `height: 260px;` at `chrome.slint:1001-1002`, asserted verbatim
at `probe.rs:2104-2111` ("the About panel has no fixed box to fill"). That is frozen evidence, so D
is a sanctioned-needle-edit conversation, not a slice. It is also the option to raise *after* the
floor exists, because a floor makes scrolling unnecessary at the sizes the app ships at.

## Recommendation

**A-prime, plus an instrumentation slice in the product only.** Concretely:

1. Add the two knobs to `main.slint` and bind `min-width` / `min-height` to them. Default `0px`,
   which is the no-floor state, so the shared file changes no behaviour until a caller opts in.
2. Set 340 x 262 in `product.rs` immediately before `window.set_size(want)` at `:355` - with the
   derivation in the comment beside it, not a claim of measurement.
3. **Mirror the two About flags before claiming anything about them.** Today `about-floored`
   (`chrome.slint:235-236`) and `about-overflow` (`:240-241`) are markup-internal bindings with no
   mirror on `Spike`: `main.slint` mirrors out `menu-shown` (`:156`), `about-shown` (`:158`),
   `popup-x` / `popup-floored` / `popup-overflow` (`:168-170`) and nothing else, so there is no
   `get_about_overflow()` to call and **no needle can read them at all**. Repo-wide,
   `about-overflow` occurs only in its own binding and in a static text guard
   (`plumbing.rs:893-895`). Adding the mirrors is the substance of the proof step - it is new
   plumbing, not an existing signal being wired to a new print.
4. Print them from the product, read them from the product. See "What ADR-0006 permits" for why the
   probe is out of bounds and `xtask` is out of scope.

**The numbers, with their derivation** - these are read off markup, not measured:

- **262** = the About box's own `height: 260px` (`chrome.slint:1002`) + the single
  `Theme.menu-gap` 2px (`theme.slint:184`) that `popup-top` subtracts (`chrome.slint:198-202`). **No
  border term:** Slint centres a Rectangle's border on its geometry edge, so `height: 260px` is
  already border-inclusive - `i-slint-core` takes a `half_border_width` on both sides of the border
  (`graphics/border_radius.rs:165-199`, `outer` and `inner`) and clips children by that same
  `border_width` inside the item box (`item_rendering.rs:523`, `:544`). 263 and 264 each add a border
  that is not additive. What is left is a stated choice, not an arithmetic dispute: **260** is the
  strict obligation, **262** is the obligation plus the gap the markup itself asks for - this note
  picks 262. It is the taller of the two content floors; the menu's is 186, quoted from
  `2026-09-14-popup-stays-inside-its-window`:172-175 rather than re-derived here.
- **340** = the panel's width, `chrome.slint:1001` - the value below which `about-overflow` says
  there is no placement at all. It is also the value `popup-x` was never computed for (that rule
  assumes `theme.slint:181`'s 190px menu).
- **Both are window-floor numbers** - they bound `Spike inherits Window` (`main.slint:16`), the
  client area. The popup's own height arithmetic, footer included, is a different sum and is already
  satisfied by `popup-top`'s rise; the floor does not replace it.
- **An honest wrinkle: 340 guarantees not-clipped, not the margin.** `about-floored` lights below
  **356** (= 8 + 340 + 8, from `popup-left`'s own arithmetic), and at any host ≤ 348 the panel sits
  at x=0 with no left margin. A 340px floor therefore buys a whole licence and a flush-left licence.
  If somebody wants the 8px too, the number is 356 and that is a bigger change to what a person can
  drag - so 340 is recommended as the floor, with the flush-left consequence stated rather than
  discovered. And a correction the revert's finding (b) does not cover: **`about-floored` stays
  reachable across 340..356 under a 340 floor**, so the floor does not kill that flag - only a 356
  floor would. The two flags finding (b) executed are `about-overflow` and `popup-overflow`, and it
  is right about those two.
- **Where 264 came from, recorded so the confusion cannot return.** `chrome.slint:997-999` says "a
  window under 290px tall would otherwise cut the licence text", and taken as a live requirement that
  number overshoots by exactly the 28px bar: with `popup-top` applied, a host under 290 **raises** the
  panel rather than cutting it, and it stays whole to 262. The sentence is a true counterfactual about
  the unraised resting place (28 bar + 260 panel + 2 gap = 290), which is exactly why it reads as a
  floor - and it, plus a border added twice, is where 264 came from. `chrome.slint` is NOT edited for
  it: that file is outside this chain's fence, and its 340/260 literals are pinned twice over by
  needles (`probe.rs:2104-2111` and the counts in `plumbing.rs`).

## A second chain in the same file: the geometry watch has no baseline

This half is not about the floor. It predates it, it outlives it, and it is worth landing **even if
the floor is never approved** - which is why it is fenced here rather than in a note of its own: the
revert made the two inseparable in one direction only, and a reader who takes the floor and leaves
this will take the worse half of the bargain.

**The defect, read off the tree.** `product.rs:466` seeds the settle watch with
`Settle::default()`, and its `seen` field is an `Option<Rect>` that defaults to `None`
(`:190`). On the first wake `same` is therefore false - `is_some_and` on a `None` answers false -
so `!same && !parked` at `:577-580` records `changed_at = Some(now)`: **a first measurement is
booked as a change from nothing.** While the rect then sits still, the same branch never runs again,
so `changed_at` is never refreshed, and `moved_for` at `:582` grows monotonically from that first
wake. `settle_says` (`:199-200`, `GEOMETRY_QUIET` 250 ms at `:111`) fires `>= QUIET` on that clock,
so `Command::GeometryChanged` is sent on **every launch with no user input at all**. The comment at
`:531-535` promises the opposite - "no rect is sent on the FIRST frame (nothing changed, and a send
the port treats as a move would let a restore be re-stored with the toolkit's rounding drift)" - and
that is true for the 8 ms between wakes (`:469`) and false by 250 ms. The suppression exists; it
just does not cover the case it names.

**Why that is merely wrong today and fatal once a floor exists.** `GeometryChanged` is a bare
trigger with no payload (`engine.rs:783-790`: "the only rect a bridge can produce is its own space",
and the drift bug that killed the payload is §12.2's own trap - 390,278,1010,698 frame in,
398,297,1002,678 client persisted). The engine answers by **measuring** through the platform seam and
persisting the read-back, never the hint. So today an idle launch re-persists a rect the person did
not change: wasteful, invisible, recoverable. Put A-prime's floor under it and the same clock writes
something else: a session that says `120x120` restores, is clamped up to `340x262` by the floor, is
measured at `340x262`, and **is written back as `340x262`**. `Rect::clamped_to` clamps position and
never touches `w`/`h` (`geometry.rs:110-118`), so nothing in any crate ever pushes it back down. One
run of the product permanently rewrites a person's saved window - a ratchet, and one that no act of
theirs caused.

**The decision, as adjudicated.** Option **(i)**: *a first measurement is a BASELINE, not a change.*
Seed `seen` from the first read and leave `changed_at` as `None` when there was no previous rect, so
nothing is "moved for" until a second, different reading exists. Extract it into a pure function
beside `settle_says` - the file already separates decision from measuring for exactly this reason
(`:195`, "so it can be tested without a window") - and give it one needle in `product.rs`'s own
`mod tests`. Note what does **not** change: `settle_says` itself keeps its body, so
`assert!(settle_says(GEOMETRY_QUIET, None))` at `:826` stays green untouched. That is tightening on a
clock the frozen instrument does not own, not loosening, and it touches no §9 row.
**(iv) rides along as an addendum**, cheap and in the same file: when the first baseline differs from
`want`, print `geometry: floored <want> -> <measured>` in the trace lane, so the divergence between
what the port stored and what the window is speaks in the run instead of being inferred from a diff.

**Why not the alternatives.** (ii) - keep the send, refuse the *write* - buys nothing over (i) except
permanence: the live window is floored under every option, so (ii) only makes a write that no act
asked for unrecoverable. (iii) - exempt the restore path from the floor - is not reachable, because
Slint and winit clamp programmatic sizes at the toolkit (see unknown 1: `set_min_inner_size` plus
`adjust_window_size_to_satisfy_constraints`); exempting restore would mean raising the floor late,
which is a rubber band plus a second law about when a size is allowed to be true.

**The order this imposes.** The watch fix is a **precondition** of the floor, not a sibling of it.
`session.json` is a memory of an act; the floor is a policy applied at presentation. Overwriting a
memory with a policy on a launch where the person did nothing converts "we remembered what you chose
and decline to show it that small" into "you never chose it". This root has already refused that shape
twice, in the same function: the minimised park must never be told (`:539-540`, "MINIMISED IS NOT A
PLACE"), and the first frame must never be sent (`:531-535`). A baseline is the third instance of the
same law, and it is currently the one that is broken.

**What it does to the machine-proven claim, said carefully.** §9's row 1 rests on "the restore rect is
a fixed point across one cycle" (the leg is S8, `smoke.rs:8841`, and its verdict string is
`:5296`). Today that PASS is satisfied by an idempotence of the *write itself*: the port stores what
`frame_rect` says, and that store happens on every launch whether or not anything moved - so a second
cycle equaling the first is a tautology about a read-back, not a proof about memory. After (i), an
idle launch writes nothing, so the same PASS asserts two things instead of one: restore fidelity
**and** a proven no-op. `PERSIST ok` (`:3216-3221`) then becomes harder, not easier, to earn - its
only remaining producer is the harness's own real `MoveWindow` (`smoke.rs:2221`) - and the leg's
own `WARNING - MoveWindow changed nothing on screen, so the persist half was not exercised`
(`:3185`) already admits that dependency, which is what the sentence has always claimed.
direction, because no workflow has run (AGENTS.md's honesty gate). What is owed is a check, not an
edit: run the geometry leg on the patched build and confirm `PERSIST` and `RELAUNCH` still earn their
words from the move alone - and if a leg goes red there, that is the fingerprint of this bug, not
evidence against the fix.

## Why this is not the rejected drag-path visibility clamp

`.agents/notes/rejected/2026-09-14-drag-path-visibility-clamp.md` exists, and this is not it. That
note is about `Rect::clamped_to`'s `min_visible = 32` - how much of a saved rect must land inside a
**work area**, i.e. *position*, in `core`, at **restore** time; its rejected options were a
work-area query on the drag path in a bridge (refused by the §5.2 layering rule), a second clamp at
store time (refused because `GeometryChanged` is payload-free, so core is never handed a rect), and
a band-aware asymmetric `min_visible` (named, not built, because the state it defends is not
reachable by dragging). This note is about **size**, in the bridge that owns the window, applied
where the size is written; it touches neither `clamped_to` nor `MIN_VISIBLE` nor the work area, and
it asks no bridge to import `platform`. One overlap is real and is admitted: the old note's fact 1
(a drag is cursor-bounded) is why its state was unreachable - and a resize drag is bounded by the
pointer the same way, which is precisely why B (the OS wall) is the only mechanism that stops the
hand rather than mopping up after it. A-prime is the mopping-up, chosen because the wall costs unsafe
and a new Command. That is a difference of cost, not of kind, and a reader who thinks the old
rejection covers this has to say which axis it applies to.

## What ADR-0006 permits here

- **Adding a needle that READS `about-overflow` is tightening, and allowed.** Decision item 1 bans
  new needles on `bridge-gpui` and forbids rows *loosening*; a product-side assertion that makes the
  unreachable-by-construction state visible again narrows the gap the freeze was arguing against.
  It is also, strictly, more than a read - see (3) above: the flags have no mirror, so the assertion
  cannot exist until one is added in `main.slint`, and that addition is the change to review.
- **No §9 row moves.** None of the seven checks is a minimum-size check, so nothing here can pass or
  fail one; the "3 of 7" ledger stays as written, and this note does not claim otherwise.
- **Proof is product-side only, and never a new probe arm.** Decision item 4 rules the probe out by
  name: `--binary=slint-probe` is `Leg::NotWired`, exit 2, forever, and "an instrument that
  acquires new assertions after its verdict is not evidence, it is a wish". So the re-earning of the
  narrow-host leg the revert offered ("plus a probe arm") must NOT be taken as an invitation to edit
  `probe.rs`: the arm stays as it is, and the new assertion lives where the product can carry it.
- **A live, drag-the-border proof is out of scope here.** That is S5-class work - the drag slice
  (`main.slint:103-104`) - and `xtask/src/smoke.rs` drives real presses and chords (its S9b item
  5, around `:4435-4443`) but never drags a resize border; adding that leg is **new harness
  surface**, the same category ADR-0006 flagged as owed-and-not-owned for the M9 rect item
  (`smoke.rs:6097-6107` now, where the ADR's `:4203-4211` pointer has drifted). Not approved by
  this note. If a floor needs live proof, ask for the harness separately.

## Known unknowns, stated as unknowns

1. **Does Slint honour markup `min-*` against a PROGRAMMATIC `set_size`? Mechanism named, outcome
   still owed.** The mechanism is Slint's own constraint path: `i-slint-core` - the runtime crate
   `slint` re-exports - keeps a window's minimum inner size, applies it to programmatic resizes
   through `set_min_inner_size` plus `adjust_window_size_to_satisfy_constraints`, and on the creation
   lane winit clamps the requested inner size against `min_inner_size` before the window exists. So
   A-prime's knob is not fighting `set_size` at `product.rs:355`: it sits on the path that call
   already travels. **That is a design claim read off the toolkit's architecture, not a measurement.**
   The case that matters here is restoring a persisted rect of, say, 120x120 and seeing what the
   window actually comes back as, and neither this note nor `6aafbe6b` has that run; until it lands,
   item (2) is the same kind of claim and neither is evidence. Both names also sit behind a pinned
   1.17.1, and this crate has already learned that a Slint bump can move a private door
   (`product.rs:227` says it of the focus one) - so the mechanism is the shape to verify, not a
   settled fact about future versions. If the seeded run shows the stored rect winning, the knob
   bounds user drags only and the restore path stays open, and **that is a reopening trigger, to be
   recorded as a new note, not quietly patched around.**
2. **Is `min-width: 0px` the no-op its absence is?** The mechanism answer is that `i-slint-core`
   reports `min_size` as `None` unless the constraint is greater than zero - which is why a `0px`
   default should be inert rather than a zero-valued clamp, and inertness is the whole safety argument
   for A-prime, since the probe never calls the setters. Again: read off the design, not measured. It
   must be checked before any markup lands, because if an explicit zero behaves differently from an
   absent property, the probe moved and the revert's finding (a) is back.
3. **Logical vs physical px.** 262 and 340 are **logical** (Slint lengths); the persisted rect is
   **physical** - `product.rs:353` divides `session.rect.w/h` by `scale` before `set_size` - so at
   150% the 340px floor is a 510 physical px minimum, and §4.1's promise that the window comes back as
   left is now conditional on a floor the user never chose. Where that interacts with
   `Rect::clamped_to`'s physical `min_visible` on a small monitor is unmodelled here. The two units
   already coexist on one window: the size write at `product.rs:355` takes a `LogicalSize`, the drag
   write at `surface.rs:1950` emits a `PhysicalPosition`, and the floor knob joins the first.
4. **The provenance of "measured."** `6aafbe6b`'s message calls 186 and 262 "measured floors"; the
   same message records that its session could not inject a pointer or a key, so no live drag was
   sampled. What is verifiable today is the derivation above, from constants. A live sample of the clip
   point: **unverified**.
5. **What the user should not be able to do.** Whether a person may shrink a note below its own
   contents at all is a product judgement (§10 has no row for it). A-prime makes the judgement visible
   and single-pointed; it does not make it for the human.

## What would change this recommendation

- **B becomes the answer** if a rubber-band is judged worse than new `unsafe`: the wall is only
  reachable through `WM_GETMINMAXINFO`, and it is the option a person dragging the border can
  actually feel. It would need its own note, a Command shape that carries no product numbers, and
  an answer to "who owns the window winit created".
- **D becomes the answer** if the About obligation can be met by a shorter panel; that needs a
  sanctioned needle edit at `probe.rs:2104-2111` first.
- **A-prime is wrong, and gets a new note, if** unknown (1) or (2) resolves against it - a knob that
  does not bind a programmatic size, or a zero that is not inert, leaves the bug open while the code
  looks fixed, which is the exact failure mode the revert was written about.
- **The whole question goes away** if a frame returns: with an OS caption, the WM enforces a minimum
  whether or not this repo asks. §10.2 says it will not.
