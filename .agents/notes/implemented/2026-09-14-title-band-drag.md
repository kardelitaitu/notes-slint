---
title: The orphan that turned out real - the title band now moves the window
status: implemented
id: 2026-09-14-title-band-drag
created: 2026-09-14
updated: 2026-09-15
relates: [§5.5, §9, §12]
decision: null
---

An unattributed 198-line patch was found sitting in the working tree. It looked exactly like
someone's abandoned experiment, which is the kind of diff this repo deletes on sight. It was
instead reviewed, trimmed by one line and landed as 363b758f - and the reason it was kept is
that the gap it filled was PROVEN real before the code was trusted. The title band on
notes-slint.exe now drags the window, and the whole contract - motion AND storage - has a
witness.

## Question

crates/bridge-slint/src/product.rs grew from a probe into a product, and somewhere in that
split the file called main.rs became product.rs + surface.rs + plumbing.rs +
title_contract.rs. A lane with no session, no note and no upstream commit left behind
.agents/drag-wip.patch (+198 lines over product.rs and surface.rs) whose report line read like
a completion claim: the title-band drag was hooked. Adopt a stranger's diff, or throw it away
as unverified work?

The cost of adopting blind is the cost this project already named for unwired UI: a callback
Rust never registers compiles, runs, and does nothing quietly. So the question was never "does
this patch look right" - it was "is the hole it claims real, and does the patch fill it".

## Forensics: the gap was real, and it was a known shape

Three checks, in order, all cheap:

1. THE MARKUP EMITTED. chrome.slint:216 declares the drag-delta callback, :220 declares
   drag-ended, and :529-545 is the machine that fires them - TouchArea has no dragged in Slint
   1.17, so a drag is the moved callback guarded by the pressed bit, its down edge sampling the
   pointer and its up edge calling root.drag-ended() (:536) and root.drag-delta(dx, dy) (:545).
   main.slint forwards both (:100-101 declared, :408-412 mounted on Chrome). Signal leaving the
   markup: yes.
2. HEAD ANSWERED NOTHING. Grepping the product surface for on_drag_delta / on_drag_ended before
   the adoption returned zero. Both callbacks were fired into an empty registration slot; every
   band drag was swallowed.
3. THE INSTRUMENT DID ANSWER. probe.rs:1492 has listened since S5 - drag_by on the probe's own
   forwarding handler.

Point three is the one that decided it. The product passing signals to nobody while the frozen
probe handles them is the STEP-B bug class: the unwired-chord hole recorded in
2026-09-14-slint-chord-wiring.md, where the same shape hid behind needles that read clean
because the instrument was wired and the product was not. A patch that closes a hole the graph
says exists is not a stranger's experiment; it is a fix whose author forgot to arrive.

## Adoption review: one line removed, nothing invented

The diff was read, not waved through. It was structurally sound - the arithmetic split from the
window call (drag_destination pure, drag_by holding the handle), the maximised refusal inherited
from probe.rs:1724-1742 (a maximised window reports its position as <-8,-8>, so read-add-write
is arithmetically perfect and semantically wrong, and it once made the PORT store -8,52 as a
normal position and destroyed a restore point while every delta needle read cleanly), and one
GeometryChanged per drag rather than per frame.

One thing was dropped: a redundant weak.upgrade() in the maximised refusal path. ui is already
upgraded at the top of that function; a second upgrade is the same handle twice, a shadow that
can fail independently of the copy already in hand. rustfmt applied. The +198 became 197
insertions and 2 deletions in 363b758f, and that is the entire review delta.

## What headless proves, and what only live proves

The two halves prove different things and are kept apart on purpose.

HEADLE§ - 4 unit tests, committed. drag_destination is pure, so the DPI half needs no window:
identity at scale 1.0 (exactly what every instrument needle measured), 10 logical px at 150
percent = 15 physical px, sub-pixel rounding, and a non-finite or zero scale falling back to 1.0
instead of freezing the note in place or parking it somewhere undefined. Plus a grep guard
asserting BOTH halves still exist - the markup emitting and the product listening - because the
failure this episode is about is precisely one-half-existing. cargo test -q -p
notes-bridge-slint: 20 lib + 38 integration, green. This retires the debt probe.rs:1717-1757
documents out loud (it reads physical and writes LOGICAL, correct only because the probe is
pinned to scale 1.0): the product reads physical and writes PhysicalPosition, one conversion, in
one function.

LIVE - a run, not a test. The app built from this diff was driven with a real synthetic mouse:
press on the band at left+200 / top+10 with WindowFromPoint confirming the press is over the
note's own hwnd, 10 absolute moves of +6 px, release. GetWindowRect left went 4200 -> 4236. The
bridge logged "drag: from <4200,200> by <6,0> at scale 1 -> asked <4206,200>, reads
<4206,200>" - asked == reads, the intention and the toolkit's fact printed side by side. Then
WM_CLOSE, session.json read back as x=4236, and a relaunch restored the window at exactly
L=4236 T=200: dLeft 0, dTop 0. Motion and storage, the whole contract.

## Honest limits of that evidence

- NO LIVE 150 PERCENT EVIDENCE. The machine reports dpi=96, scale 1.0. The 150 percent path is
  proven by unit tests and nothing else. Anyone citing "a DPI-correct drag" should cite the
  arithmetic, not a window that moved on a 96-dpi desktop.
- 36 PX OF A 60 PX POINTER PATH. Only 6 of the 10 synthetic moves reached the app - Windows
  coalesces WM_MOUSEMOVE under a scripted cursor. Every delta that did arrive produced exactly
  its own 6 px with asked == reads, so the shortfall is input delivery in the harness, not
  arithmetic in the bridge. Reported rather than smoothed over: "the window moved 60 px" would
  have been the comfortable and the false sentence.
- THE LIVE PROOF DOES NOT SURVIVE cargo clean. target/dragproof.ps1 was deliberately throwaway
  and is ignored only incidentally, by the /target rule - it is not a committed test and nobody
  can re-run it later. That is a cost recorded, not a virtue.

## The decoy that would have fake-confirmed all of this

Storage was verified by reading session.json - and there are two candidate state directories,
only one of them live. resolve_state_dir takes the PORTABLE branch when <exe_dir>/data exists
(product.rs:96 probes it), and a debug build's exe lives in target/debug, so the file the app
actually writes is target/debug/data/session.json. An assertion aimed at %APPDATA% notes-gpui
would read a stale directory, find the old x, and "confirm" that persistence works while
measuring nothing at all. When a check passes for the wrong reason, the first thing to suspect
is the path it read.

## The stray, which is not a bug claim

Once, BEFORE the commit, the driven window disappeared and then outlived its own WM_CLOSE by
about 20 seconds - the harness kept polling a rect for a window nobody could see. Single
occurrence, pre-adoption build, never reproduced before or since, no artefact beyond this
paragraph. It is not reported as a defect and it did not block the adoption. It is a watch-list
item: if a hidden-window or unresponsive-close case recurs - start with the R2 close contract,
where declining a close must leave the window visible - and read the decoy section above before
believing any persistence number.

## S1 (2026-09-15): frame debt, and the hold that made the live read safe again

The episode above is dated 2026-09-14 and describes the state the drag had reached when it
shipped: read the window once per gesture, ignore it afterwards, accumulate the pointer's travel
locally. That shape was correct against a band that re-anchored every event, and 1085ef63 stopped
the band re-anchoring - ui/chrome.slint now HOLDS `last-x`/`last-y` at the press sample, so each
emitted delta is the OUTSTANDING ERROR between pointer travel and travel the frame has applied.
The consequence was not noticed at the commit and is fixed in b12b7246: summing cumulative
quantities is quadratic. Five events 6 logical px apart against a note that lands every ask asked
for 396, 402, 408, 414, 420 - a gesture worth 30 px that outruns the hand, because each event was
credited with the whole error AND with the events before it.

**THE HOLD BECAME THE LOAD-BEARING PART.** The cure is to trust the live read again: the ask is
`drag_destination(here, dx, dy, scale)` - frame plus outstanding error - which resolves to
`origin + pointer_travel` on both branches, apply-landed and apply-refused, with no accumulator in
between. `DragEpisode` keeps its origin (the release line measures displacement from it), its gap
and its printed bit; `travel` survived as a WITNESS computed from the same two numbers, not as a
runner. What retired is the integrating role, and the reason it is safe is entirely in the other
file. That is a coupling, so it is now a guard and not a comment:
`the_hold_is_what_makes_the_read_safe` slices the `moved =>` block out of ui/chrome.slint, strips
the comment lines (that block documents the abandoned re-anchor in prose, quoting the very
assignment it refuses, so a raw grep finds a false positive on the commit that fixed the bug) and
asserts no `last-x =` write reaches it. Advance the anchor and this goes red here rather than
shipping a note that outruns the cursor.

**THE PROBE NOW DIVERGES, DELIBERATELY.** probe.rs:1713-1760 drives its synthetic drags with the
old read-modify-write decomposition. It is FROZEN (ADR-0006) and it is the instrument, not the
product: it still measures what it was built to measure, and its verdicts keep meaning what they
said on 2026-09-14. Citing it as evidence about the product's drag arithmetic after this commit is
the error - it is evidence about the shape the product used to have.

**DPI MID-GESTURE IS NOW A FREEZE, NOT A GUESS.** `DragEpisode` carries one new field,
`scale: Option<f32>`, settled by the press delta. A later delta whose `window.scale_factor()`
differs spends that field and the episode stops writing position for the rest of the gesture; the
transition prints once (`scale changed mid-gesture, release to re-grab`) and the release resets.
The two candidate repairs each discard a fact the other keeps - re-sample the origin and you throw
away the pointer, re-base the anchor and you throw away the frame - so the gesture waits for the
next press. A field on the episode, not on the Pump, because a scale that disagrees with its own
gesture is a per-gesture fact and must die with it like `origin`, `last` and `printed` do.

**THE MINIMISED FRAME IS REFUSED BEFORE IT IS READ.** A parked window reports <-32000,-32000>
(the measured fact in plumbing.rs's `Fingerprint::minimized`), and adding an outstanding error to
a parking place is the maximised `<-8,-8>` bug in a different costume - perfect arithmetic, a
restore point made out of -32000. The guard sits above the read for the same reason the maximised
arm does: a refusal must not sample an origin, must not write a position, and must not reach
`advance`. It reuses `drag_refused_shown` rather than adding a second latch, so one line per
refusal streak and nothing to keep in step.

**WHAT THIS DOES NOT SETTLE.** Items (2) and (3) of the honest-limits list above are retired by
this change: the delta-vs-read question is now answered by the decomposition rather than by which
quantity happened to be starved on someone's desktop, and a scripted 6-of-10 delivery shortfall
no longer changes where the note ends up, because the ask is pointer-referenced instead of
count-referenced - four delivered events of a ten-event gesture produce the same final target as
ten. Item (1), NO LIVE 150 PERCENT EVIDENCE, is NOT retired and is now slightly sharper: the
freeze path exists only because a mid-gesture scale change is unhandleable, and it has unit
evidence and no window evidence at all.

## The live field, run 2026-09-15 against b12b7246

The same three gitignored harnesses in `/target`, one window at a time, each closing with a
`Stop-Process` on the PID it started. dpi=96, so scale 1.0 - the DPI limit above still stands.

- **A (12 x +1 px, release on the band):** asked == reads on every line; released at
  <4260,200>, `travel (12, 0)`; relaunch L=4260 T=200, `dLeft 0, dTop 0`.
- **B (12 x +5 px, release 45 px BELOW the band):** exactly ONE `drag: from` line for the whole
  gesture (the once-per-episode print, not once per frame), one release, `travel (60, 45)` = the
  pointer's own path; relaunch L=4320 T=245 exact.
- **C / D (60 px as six steps / as one step):** 4200 -> 4260 and 4260 -> 4331, `travel (60, 0)`
  and `travel (71, 13)` - the one-step gesture picking up 13 px of y the scripted cursor carried
  with it, which is what a pointer-faithful witness is supposed to say.
- **THE WORK-AREA CLAMP LEG, run for the first time.** Three legs park the note, walk 8 steps
  INTO the constraint, hold 400 ms, then walk 8 steps back out:
  E-left park 80,300 out 8 x -20, F-right park 6000,300 out 8 x +20, G-top park 2000,80 out
  8 x -10. The per-step rect sequence is UNIFORM on every leg - `s-20/0` eight times then
  `b20/0` eight times, `s20/0`/`b-20/0`, `s0/-10`/`b0/10` - and the note ends each leg exactly
  where it parked (80,300 / 6000,300 / 2000,80), with `relaunch rect=2000,80` confirming the
  saved rect. THIS is the claim the accumulator could not hold: while the frame is refused,
  `here + error` walks the debt forward one step at a time, and releasing it does not pay back a
  jump. Under sum-of-cumulative the return leg would have had to spend the whole accrued
  triangle at once; no such spike appears in any of the three sequences.
- **The 400 ms hold is a second, unplanned witness.** It crosses `DRAG_EPISODE_GAP`, so each
  clamp leg runs as TWO episodes and prints two `drag: from` lines - and the travel the release
  reports (80,0), (-80,0), (0,40) is the SECOND episode's ask-minus-origin, not the gesture's
  whole path. That is the gap net doing exactly what `a_stranded_release_cannot_carry_over...`
  says it should, observed live; it also means the release line is per-episode and a reader
  comparing one against a scripted pointer path must count the pauses.
- **NOT RUN: the teleport leg.** The decomposition is tested at its exact numbers by
  `a_frame_that_teleported_under_the_drag_is_paid_for_once` (press frame <1000,500>, cursor
  <1050,512>, frame taken +300/+150, outstanding error <-310,-150> -> ask <990,500>), but no
  harness moves a window under a live drag mid-gesture, so that remains unit-grade only.

## RecommendationAdopted and shipped: the delta lands in surface.rs::drag_by (:1478) on top of the pure
drag_destination (:1460), registered by wire_callbacks at surface.rs:1421 (on_drag_delta) and
:1434 (on_drag_ended). Keep three rules from this episode:

1. A FIRED CALLBACK WITH NO REGISTERED HANDLER IS A BUG CLA§, NOT A TODO. Where markup emits a
   signal, grep the product for the listener and cite both. The guard in
   the_band_that_moves_the_window_is_listened_to exists so this cannot regress silently.
2. FOREIGN DIFFS ARE JUDGED AGAINST THE GRAPH, NOT AGAINST THEIR OWN ME§AGE. The patch's claim
   was worthless; chrome.slint:216 / :220 firing into zero product handlers while probe.rs:1492
   listened was the evidence. Review is still mandatory - it is what found the duplicate
   weak.upgrade().
3. NAME THE FILE THAT ANSWERS, IN THE COMMENT. main.slint:98 pointed at drag_by() "in main.rs"
   after main.rs had been split apart; a comment naming a dead file is how the next orphan fails
   to find the hole. Fixed in this pass, at the drag comment only. The same stale pattern
   survives elsewhere in main.slint (:57, :92, :122, :182, :201, :205, :219, :403) and was
   deliberately left alone - out of this fence, but now a known stale set instead of a surprise.

## Reopening conditions

- A 150 percent (or any non-1.0 scale) machine becomes available: re-run a dragproof-style pass
  live. Until then the DPI claim is unit-test-grade only.
- The stray disappearance recurs: the watch-list item becomes a bug, and the maximised-refusal
  path plus the R2 close contract are the first two places to look.
- If target/dragproof.ps1 is ever worth keeping, promote it into a committed harness (docs/dev/
  or the smoke xtask) rather than citing a script that a clean build deletes.
