---
title: No instrument tests a pointer
status: proposed
id: 2026-09-15-no-instrument-tests-a-pointer
created: 2026-09-15
updated: 2026-09-15
relates: [whitepaper §9, whitepaper §5.5, whitepaper §4.2, whitepaper §12]
decision: null
---

## Honesty gate first: what the docs claim, and what they do not

The overstatement is narrow and it is in two places, both fixable with a sentence.

- **`README.md`'s M2 row** says the product "passes its own smoke leg locally
  (`cargo xtask smoke --binary=slint`)". True as written - and a reader who knows the leg contains
  four pointer modes will hear more than it says, because those four have never reported a verdict.
- **`chrome.slint`'s hamburger-ask comment** claimed the click handler and the synthetic act "run
  one". A worker narrowed it today (`825ccae9`) to what is actually known, and the narrowed text is
  now the best statement of this finding anywhere in the tree:

> Both routes arrive here (the press at menu-touch's clicked arm, the counter the `changed` arm below
> reads), and from this line on there is exactly one implementation. What is NOT shared, and has never
> been tested, is the road from a pixel to this callback: every instrument that opens the popup raises
> the counter, which bypasses hit-testing entirely, and the only legs in the repo that press real
> pixels are smoke modes 4-7 - which carry no recorded green run anywhere. So do not read this comment
> as "the pointer path works": it says one body from the callback in, and nothing further.

Everything else already tells the truth, and this note should be read alongside it rather than
correcting it:

- **`docs/roadmap.md` §9 lists the pointer-dependent claims as owed, not proven** - check 6 is
  marked "**Autosave toggle** - **not proven.**" with "the menu toggle driving them is not", and the
  owed list carries "the autosave toggle through the real menu - check 6, closed on a product menu
  row". §9 also names what no harness reaches: "a real Explorer drag rather than a synthesised one".
- **`docs/dev/testing.md` states what smoke proves without any pointer claim**, and uses
- `docs/dev/testing.md` states what smoke proves and claims no pointer coverage at all: it names
  the probe binary's leg "**not built, not launched, not judged**" (`Leg::NotWired`) and describes the
  `slint` row as the product contract (`Leg::Product`) without ever saying a pixel was pressed.
- **§9 rows do not move pre-CI in either direction**, per `AGENTS.md` and the honesty gate written
  into §9's own text ("a row that can only travel one way is not a gate"). This note therefore
  produces a doc edit and an unproven-surface record. It produces no status change, and it is not
  evidence that anything regressed.

## The finding

`notes-slint-probe.exe` **never presses a pixel.** Every menu opening and every About opening it
drives is a property write: `set_toggle_asks` and `set_about_asks` bump counters that Chrome reads
in `changed toggle-asks` and `changed about-asks` arms. Chrome then routes the ask through the same
handler a click reaches - so everything *downstream* of the callback is genuinely shared, and
everything *upstream* of it is untraversed.

Two details that make the shape precise, both from the markup:

- The counter exists because of a toolkit limit, not a choice: a component callback cannot be invoked
  from Rust through the parent, which is why the About bit says its ask is "Forwarded by the mount so
  a synthetic act and a real row click go through ONE door (the same counter pattern as toggle-asks
  above, which cost a build to learn)". The root's own callbacks *can* be invoked, which is why the
  close-request door is a real callback invocation - `invoke_quit_asked` - while the popup doors are
  numbers.
- The probe's dismissal arm is the same shape: it "bumps this number and Chrome closes BOTH surfaces
  - the same two lines a backdrop click or an Escape reaches, entered through the other trigger."
  Elegant, and it is precisely why the instrument is so good at testing logic: it enters at the
  handler. The cost is that it never learns whether a handler can be reached by a pointer.

## The only pixel-pressing instruments, and the absence of a record

Four smoke modes and one drag procedure:

- **MODE 4 - LEG A: THE MENU ANSWERS A REAL CLICK** (two presses, placed by the harness);
- **MODE 5 - LEG B, THE REFUSAL PROOF** (same two presses, the Open row instead of the toggle);
- **MODE 6 - ITEM 4, THE NATIVE DIALOG ON CAMERA**;
- **MODE 7 - LEG C: THE DRAG CLOSES THE POPUP** (recorded in the source as "b76c277c's promise, first
  machine proof");
- and the drag door itself, `Drag-Band`: "press, MOVE IN STEPS, release. The steps matter - Slint's
  `TouchArea` has no 'dragged' callback, so a drag is its 'moved' handler".

They carry **no recorded green run anywhere in the tree**, and the checks that establish it are
mechanical:

- The reflog holds **393 commit subjects** for this branch and **none** of them contains `gaps=`,
  `SMOKE FAIL`, or the `smoke: menu` prefix those legs print. The legs landed in `069018ef`,
  whose subject is exactly 90 characters: "test(xtask): the menu answers a real click - and the
  native dialog finally shows on camera". A subject that asserts a verdict with no run attached to it.
  (The subject is what the reflog can show; whether the body carried a run is the part I could not read
  without git plumbing, so it is recorded as unverified rather than repeated as fact.)
- **No markdown file in the tree cites these legs at all.** A grep over every `.md` for
  `menu-click`, `mouse_event`, `SetCursorPos`, `NOT JUDGED` and "did not move" returns nothing
  from `.agents/notes`: §9's only reference is a drift correction observing that "those lines are
  the menu legs' P/Invoke block today". So the record is not merely unfavourable - it is absent, which
  is worse than the brief that commissioned this note described.
- The print that a green run *would* produce is
  "`smoke: menu-click: INFO - hamburger {} | row {} | toggle lines={} active={} | cursor moved -> {}`",
  and INFO is not a verdict; the following legs can also fall back to "`smoke: menu-click: NOT RUN`",
  which takes "the three legs after it" down with it.

## Two independent runs today, and what they measured

Neither run is a verdict on the product; both are evidence about the instrument. Provenance, since it
matters to anyone re-reading this: the two runs were performed by other workers today and are recorded
here as their measurements - the print formats, the seed rect, the injection calls and the markup claims
in this note are all read from the tree by its author, and neither run reproduced a recorded artefact
anywhere in the repo, which is itself part of the finding.

1. **Through smoke:** the presses landed on our window 3 of 3, `WindowFromPoint` named the product
   HWND under the press, the cursor visibly moved to the target pixel - and the product printed
   nothing. No `menu-shown` transition, no row answer, no refusal.
2. **Through the built product directly:** a worker ran `notes-slint.exe` by hand with its own
   injected presses, at coordinates derived from the same smoke arithmetic. Three presses;
   `menu-shown` never became true.

Both runs sat **below the relevance of the window floor** - the harness seed is `120,90,800,600`
(`DEFAULT_RECT_HINT`), and the harness's own comment notes that "at 800x600 the 6-row menu fits
comfortably" - so the floor that landed today is not what is interfering, and neither is the size
guard. Nothing was read off a diff; the two runs are the observation.

## Why the instrument is the prime suspect, not the product

The harness posts **relative button events that carry no coordinates at all**:
`mouse_event(2, 0, 0, 0, Zero)` for the down and `mouse_event(4, 0, 0, 0, Zero)` for the up -
`dx` and `dy` both zero, no absolute flag. It is not a statement about where to click; it is a
statement that a button changed state, and the operating system routes it to wherever the foreground
thread's cursor happens to be. Every coordinate the leg computes is used to *move the cursor* first
(`SetCursorPos`) and to ask `WindowFromPoint` afterwards; the press itself is blind.

Which means **`WindowFromPoint` naming our window proves GEOMETRY and never proves DELIVERY.** The
harness knows this in one place - its own assertion reads "a press that WindowFromPoint says was not
ours proves nothing about the menu" - and the guard it wrote for the door ("public static extern void
mouse_event" is asserted to still be in the script) protects the *existence* of the press, not its
arrival.

And markup is independently exonerated, so this is not a "something covers the bar" bug:

- The mount-order claim is a needle, not a hope: `assert!(editor < backdrop, "the editor must be
  below the catcher")` and `assert!(backdrop < chrome, "the catcher must be below the popup")`.
- The backdrop **cannot** be eating the click while the menu is shut: its `visible` and `enabled`
  both follow `chrome.menu-open || chrome.about-open` - "a closed menu cannot swallow a click - the
  editor keeps every pixel the instant the popup is gone" - and it is deliberately declared between
  the editor and the bar, the only sibling position where it is above the text and below the popup.

So the shortest honest reading is: a coordinate-blind injection aimed at a cursor nobody has proven is
where the arithmetic thinks it is. That is a property of the instrument.

## What hover actually gates, measured from the markup

`has-hover` is read **fourteen times** in `chrome.slint`, and every one of those reads sits in a
visual property: `opacity` or `colorize`. Concretely: four title-bar cells (menu, pin, minimise,
maximise), the close glyph twice (its tint and its colour), the seven fixed menu rows, and the
recent-files template row. **No state transition anywhere in the markup reads it.** A hover changes
what a surface looks like and nothing about what a surface does.

So the surfaces that genuinely need a pointer - and therefore have **zero coverage of any kind** - are
the pressed-gated ones:

- the **title-band window drag**, whose entire mechanism is `pressed` plus `moved` while pressed,
  with no callback that a timer can bump;
- the **backdrop click-away**, whose `clicked` arm is the only way that surface closes by pointing
  (Escape and the `close-asks` counter reach the same two lines, which is exactly why the *policy* is
  tested and the *pointer* is not).

That is the structural finding: the instrument is very good at logic and the app's pointer-owned
surfaces are invisible to it. Not a defect in the probe - the probe was built to be deterministic -
but the tree has been reading its determinism as coverage.

## A consequence that already shipped today, and why it is survivable

The About decline is now gated in markup (`if root.about-fits { root.about-open = true; }` inside
`changed about-asks`, one line, one write, no else), and the sentence explaining it is keyed to the
state a person is looking at rather than to the act:
`if unfit && ui.get_menu_shown() && !tick_about_said.get()`. Since no instrument can open the menu by
pressing, and the product-side refusal leaves no trace Rust can see, **no instrument currently reaches
that branch's print.**

The code already says so, in the comment above that condition: "THE ASYMMETRY, STATED NOT HIDDEN: a
refused POINTER press leaves no trace Rust can see - the bit simply never changes, and a row that
declined emits nothing - so the sentence is keyed to the state a person is looking at".

What proves the branch, then, is a needle and the compiler:
`the_about_act_is_gated_on_the_panels_own_terms_and_the_decline_speaks` reads the source and asserts
that the decline line is still there and still quotes the floor's own consts. That is a real guarantee
- it is what keeps the sentence from silently copying a literal - and it is not observation.

This is acceptable **only** because of what the branch is: a guard that can refuse too much, cannot
corrupt anything, and has a truthful fallback in the write path. The worker explicitly declined to
loosen the `get_menu_shown()` key to manufacture a print, and that refusal was correct - a log line
produced by weakening its own precondition is the artefact's worst kind of evidence.

What is worth watching is the general shape, not this instance: **a branch that no instrument can
reach can still ship, and nothing in the gate notices.** Every future "proven by a source-text needle"
claim should be read as answering *does this line exist*, never *does this path work*.

## What this note must not be used to say

**It is not a claim that the app is keyboard-only or pointer-only.** No test has ever established that
a pointer works, and no test has established that it does not; the row's `clicked` handler is real
markup that nothing in this repo has ever watched fire. The honest statement is exactly one sentence:

> **M2 pointer interaction is untested.**

Anyone reading this note as evidence that the menu does not respond to a click has over-read it. The
two measurements below exist because the question is genuinely open, and either answer is worth having.

## The ask: two measurements, both of which need a human

Both move a live mouse cursor, so neither can be delegated to a worker or run inside CI, and both are
decidable in about a minute by a person at the machine with the app open.

1. **A hover-tint read.** Park the cursor away from the window; capture the hamburger cell; move the
   cursor onto the cell; capture again. If `Theme.hover-bg` appears in the second frame, then Slint
   received pointer motion and the smoke failure is product-side. If nothing tints, the four pointer
   legs are an instrument finding, and the app's own claim about its rows should be re-read as untested
   rather than as failing. This one needs no code change at all - it needs eyes and a screenshot.
2. **A zero-pixel control.** Open the menu through the product's own ask door - the same
   `about-asks`/`toggle-asks` path the instrument uses, driven however the person prefers - and
   then press a row with a real click. If a row answers a popup that was opened by property, the
   `TouchArea`s work and only the hamburger press in the harness was lost, which localises the whole
   finding to the harness's blind injection. **This one needs an edit to
   `crates/xtask/src/smoke.rs`**, which has been deliberately left closed all session; the ask is
   therefore explicit rather than implied, and it should be scheduled as its own slice with its own
   green run recorded in the commit that gets it.

## Recommendation

1. **Record the surface as unproven, in prose, before anything else.** The claim "M2 pointer
   interaction is untested" costs nothing and prevents the most likely future error: a worker citing
   the smoke leg as pointer evidence. This note is that record; it is also the artifact that the README
   edit and the §9 owed-list edit should cite.
2. **Fix the two overstatements, as a doc-only change:** qualify the README M2 row so "passes its own
   smoke leg" cannot be read as "its pointer modes ran" (name them as unjudged), and leave
   `chrome.slint`'s narrowed comment standing - it is already the honest version and it should not be
   softened back.
3. **Run the two measurements above.** They decide which crate owns this work, and they are the only
   cheap way to find out. Do not open `smoke.rs` before measurement (2) is scheduled as its own
   slice.
4. **Do not extend the probe to fake a pointer.** A synthetic arm that pretends to hit-test would
   produce exactly the false confidence this note is about. If pointer coverage is ever wanted, it
   belongs on the legs that already inject at the OS - with the injection fixed to be absolute, and
   with a delivery proof, not a geometry proof.
5. **Move no §9 row.** Not up (there is no green run to promote) and not down (nothing regressed, and
   §9's rows were already owed). The ledger stays as written; this note adds a sentence to it.

## What this note does not do

It edits nothing outside itself: no README change, no §9 change, no ADR, no code, no probe arm - all of
which the finding argues for but does not perform. It does not touch an `implemented/` note, and in
particular it does not rewrite what the strip or the ledger records; those describe what was built and
earned, and this is a claim about what was never measured. It does not re-open ADR-0006, and it takes no
position on the floor, which is exonerated above rather than assumed innocent.

## What would change this recommendation

- **A recorded green run of modes 4-7** - then this becomes an `implemented/` record about how the
  pointer path was earned, and the README needs no apology.
- **The hover read landing on the product side** - then the finding is a real defect, it gets a fix
  slice with its own note, and the harness's relative-injection question becomes a task rather than a
  suspicion.
- **Any Slint release that changes pointer routing or hit-testing** - which would make even the
  current "the instrument cannot reach it" claim stale, since today's shape is a property of the
  pinned 1.17.1 behaviour and of winit's delivery, not a law of the toolkit.
- **A human deciding the pointer path is not worth machine proof** - a legitimate call for a
  frameless single-window note app, and if it is taken, it should be written down as a
  `rejected/` note with this file as its evidence rather than left as a silence in §9.
