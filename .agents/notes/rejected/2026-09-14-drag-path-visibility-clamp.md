---
title: A drag-path visibility clamp — band-aware min_visible at the point of the drag
status: rejected
id: 2026-09-14-drag-path-visibility-clamp
created: 2026-09-14
updated: 2026-09-14
relates: [whitepaper §4.1, whitepaper §5.2, whitepaper §5.5, whitepaper §12.2]
decision: null
---

## Question

`Rect::clamped_to` promises that at least `min_visible` pixels of a saved rect land inside the
work area, and the port calls it with 32 (crates/api/src/engine.rs:1463, `MIN_VISIBLE`). A user
who drags a note by its title band can leave only a strip of BODY on screen, and 32 pixels of
body is not 32 pixels of *grab thing*: the band is the only handle this app has — §10.2 settled on
custom chrome (ADR-0003), so there is no OS caption to catch, and §12.2 is the coordinate trap that
follows from it. So: should the visibility law be re-cut in band terms, and
applied where the drag happens?

This note records the ruling: **no new law.** The reachability question was measured against the
drag arithmetic, and the answer is that the state worth rescuing cannot be reached by dragging,
and the states that can are already covered by the clamp that exists. The band-aware number is
named, not built.

## What the measurement says

Two facts, both read off the code, neither desktop-measured — and §Consequences names the one
gap that still owes a real screen.

**1. A drag is cursor-bounded, and the bound is the press point.** The product's only position
writer is `window.set_position` in `drag_by` (crates/bridge-slint/src/surface.rs:1798), fed by
`DragEpisode::advance`, which adds the pointer's own accumulated travel to the corner sampled at
the gesture's FIRST delta (surface.rs:1686-1713). The markup's half is the same arithmetic in
reverse: `max-band` emits `drag-delta(mouse-x - last-x, ...)` with the anchor HELD at the press
sample (crates/bridge-slint/ui/chrome.slint:542-566), so a delta is outstanding pointer travel
and nothing else. There is no force in this path except the pointer, and the pointer stops at
the edge of the desktop. A gesture that pressed `P` physical px into the window can therefore
put the window's left edge no further off than `-P`.

**2. A re-grab cannot extend it.** A second gesture starts where its press lands, and its press
must be inside the band. Press at screen x = 0 and there is nowhere left to walk: the delta
cannot go negative, so nothing moves. Press further right and the press point is by construction
already ON screen, which is the recoverable state. The worst reachable case is a fraction of the
rect off, not all of it — on the default 800-px-wide note against a 1920-px work area, at most
about a quarter. And every one of those keeps far more than 32 px inside, so `clamped_to` leaves
them alone: the correct verdict, not a hole. The user left the window there.

**3. The single mover is pinned by a test, so fact 1 cannot rot quietly.** `drag_by` is the only
place the product file moves a window, and that is an assertion, not a comment:
`the_band_that_moves_the_window_is_listened_to` counts
`src.matches("set_position(").count() == 1` over the non-test half of surface.rs
(crates/bridge-slint/src/surface.rs:2672-2676). A second writer — a snap helper, a menu "move
to", a keyboard nudge — would break the cursor bound, and it would break that test first. That
is reopening condition (i).

## Options

### A. Clamp on the drag path: ask the OS for the work area on every delta, in the bridge

Correct-looking, and refused by the layering gate before anything else is argued: a bridge may
import `api` and its own toolkit and nothing else in this repo (§5.2). The rule rows
`bridge-sees-only-api` and `bridge-names-no-ffi` are in crates/xtask/src/arch.rs:290-322, and
`work_area_for_rect` is a `notes-platform` fact
(crates/platform/src/windows/monitors.rs:125). Option A is therefore either a platform
dependency in a bridge or a hand-declared Win32 call — the reach-around AGENTS.md names by name.
The third door is a command-and-event round trip across the seam per mouse frame, for a
behaviour fact 1 says is unreachable. Rejected on layering and on cost.

### B. Clamp on store: correct the rect in core when it is persisted

The shape that looked cheapest, and it does not exist to be chosen. `GeometryChanged` is
**payload-free** (crates/api/src/command.rs:153): the bridge reports THAT geometry changed,
never what it is. Core is not handed a rect to clamp — the port MEASURES one,
`Engine::measure_rect` calling `restore_frame_rect` and writing the read-back into
`session.rect` (crates/api/src/engine.rs:1804-1836). So B is a second clamp beside the first, in
a crate that never sees the OS answer it would need. Worse, it ratchets against
`push_restore_rect` (engine.rs:1697) on the maximised lane, where the port writes the stored
frame back through the door it read it from: a store-time correction would be re-read as the
window's own truth and corrected again. One law, one place, one measure. Rejected.

### C. Band-aware, asymmetric `min_visible`

The honest form of the complaint: `min_visible` is PIXELS OF THE RECT, per axis, the same number
on all four sides. A grab handle wants a bigger minimum on the TOP edge than on the bottom, and
wants it derived from the bar, not from title-bar grab folklore. Named, not built — the
three-line addition on `Rect::clamped_to` is what shipped in place of a rule. Rejected today
because the state it defends is the one facts 1 and 2 say a drag cannot produce.

### D. Nothing, but say it out loud

Document the units of the promise that exists, pin the paired negative in a test, and keep the
band-aware number in this note.

## Recommendation

**D.** The law is unchanged because nothing about its reach changed; what changed is that its
scope is written down where a reader will find it. Three artifacts, one commit:

- crates/core/src/geometry.rs: `Rect::clamped_to` now states that `min_visible` is pixels of the
  rect (physical), NOT a grabbable band — and that the logical 28 px of `Theme.bar-height`
  (crates/bridge-slint/ui/theme.slint:125) is ~42 physical px at 150 %, so a bottom-edge park
  leaving 32 px of body inside the work area passes today with the band off-screen. Reachable at
  RESTORE time (a saved rect, a changed monitor), never by drag.
- crates/api/tests/geometry.rs:
  `a_partially_off_rect_on_a_connected_monitor_is_restored_untouched_and_unreported` — exactly
  one move, the saved rect VERBATIM, and no `GeometryNotRestored` anywhere in the stream. It is
  the paired negative of the two rows that prove the clamp fires, and without it "the clamp
  protects you" misreads as "the clamp always acts", which is a different and wrong law.
- This note, so the next reader finds the argument instead of re-running it.

## Consequences

- The band-off-screen state is a documented property of the restore law rather than a bug report
  waiting to happen, and it lives in a `core` doc comment, so both bridges inherit the sentence
  without either owning it.
- **Zero overlap still resolves a work area, and that is what makes the clamp fire at all.**
  `work_area_for_rect` asks `MonitorFromRect(&win_rect, MONITOR_DEFAULTTONEAREST)`
  (crates/platform/src/windows/monitors.rs:132-137), whose own comment records that the flag is
  documented never to return null: a rect touching nothing still gets a monitor and a real
  `rcWork`, so the port clamps against the nearest screen instead of failing. Stated as
  read-off-code. It has NOT been measured on a desktop with a monitor unplugged, and this note
  does not claim it was.
- **The fake cannot see the nearest-monitor choice at all.** `host_mock::Host::work_area_for_rect`
  answers the SAME configured work area for every rect it is asked about
  (crates/api/tests/support/host_mock.rs:399-411), so which monitor a rect is nearest to is not
  fakeable in this suite. The new row proves the port's decision GIVEN an answer; the
  nearest-monitor pick itself lives to a smoke leg.
- `MIN_VISIBLE = 32` now appears in three places: the const
  (crates/api/src/engine.rs:1463), the two pre-existing clamp rows that call
  `saved.clamped_to(Rect::new(0, 0, 1920, 1032), 32)` to compute their own expectation
  (geometry.rs:119 and :619 as they stood before this commit), and the new row's fixture guard.
  One literal, deliberately NOT refactored: a test that imports the production constant cannot
  show that the production constant is what broke.
- Nothing is foreclosed. Option C is a parameter change in `Rect::clamped_to` plus a per-axis
  split — no new seam, event or settings key. That is exactly why naming it is cheap enough to
  do instead of building it.

## Reopening conditions

Reopen **C** (band-aware, asymmetric `min_visible`) on any one of these:

1. **A mover that is not the cursor.** Fact 1 is a pointer bound, and surface.rs holds it only
   because `set_position` has exactly one caller. A snap integration, a menu "move to a corner",
   or a keyboard-move chord puts a rect on screen that no pointer travelled through, and the
   measurement is void. The one-mover assertion catches the first two; a keyboard move routed
   through `drag_by` would not, which is the row to watch.
2. **macOS or Linux, where the work-area law differs.** §6 already records that Wayland
   restricts positioning outright, and macOS's visible frame is not Win32's `rcWork` — no
   taskbar equivalent, a menu bar with different semantics. If a non-Windows port measures a
   different reachable fraction, or cannot position at all, the number becomes per-platform
   policy and belongs in a new note carrying that platform's measurement.
3. **One reported bottom-edge park.** That is the case `min_visible` genuinely misses today: body
   keeps 32 px, band keeps none. It is not reachable by drag (fact 1), so a report showing it is
   evidence of a mover or a monitor geometry this note has not modelled — and C becomes the fix,
   with the report attached.

Reopen **B** only if `GeometryChanged` ever carries a payload. At that point "core measures" has
stopped being the law and this note is wrong about the code, not merely about the choice.
