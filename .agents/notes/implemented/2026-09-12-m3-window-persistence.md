---
title: M3 — window persistence on Windows
status: implemented
id: 2026-09-12-m3-window-persistence
created: 2026-09-12
updated: 2026-09-12
relates: [§4.1, §5.5, §12.2]
decision: null
---

## Question

Which rectangle is persisted, and can restore survive what §4.1 promised — a monitor
unplugged, changed scaling, a maximised window — without the window drifting every launch?

## Options

### A. Persist what GetWindowRect answered, restore it as gpui client bounds

One rect, no conversion. M0 (§12.2) measured exactly where this fails: gpui bounds are
client space, `GetWindowRect` is frame space, and the chrome deltas are not symmetric
(8/19/8/20 px on the spike host) — the window drifts 8 px left and 19 px up on every
launch while the app confidently reports that it remembered.

### B. Persist frame pixels in one documented unit; convert once, at the boundary

Storage says one thing — `FrameRect`, frame pixels including chrome — and the bridge
converts to client bounds at `open_window` and nowhere else. Client coordinates are never
persisted.

## Recommendation

**B — and it is built.** The M0 finding made A indefensible before a line of it existed;
every persistent rect in the product is a `FrameRect`.

## What was built

- `crates/platform` — the OS seam that decides nothing: `geometry.rs` (`FrameRect`, the
  crate's one rectangle type, in one documented unit), `windows/monitors.rs` (monitor
  facts — enumeration, surface, scale — that everything above validates a stored rect
  against), `windows/topmost.rs` (`set_topmost`, whose `PinOutcome` is read back from
  `WS_EX_TOPMOST`, not assumed), `windows/paths.rs`.
- Both seams that change a window — `set_frame_rect` and `set_topmost` — issue
  `SetWindowPos` with `SWP_ASYNCWINDOWPOS`, so they return before the change has landed
  instead of blocking the calling thread on the window's owner.
- The **restore-measured-rect rule**: an async move has not landed when it returns, so a
  rect read straight after a move is the PRE-move value. Nothing is persisted from a
  read-back taken right after a move; the port measures for persistence on its own
  session flush tick. `crates/platform/src/lib.rs` documents the obligation; `crates/api`
  honours it.
- Restore follows the fixed startup order (§5.5): query session → create the window at
  the saved rect → register the handle → apply topmost. `Session.rect` is frame pixels;
  the bridge's `bounds_for` divides by the scale the session was saved at.

## Tested

- `crates/platform/tests/geometry_live.rs` — the real-window live test, including a
  parked owner: the reband wait, and proof that a topmost request issued from the wrong
  queue context does not land.
- `crates/api/tests/geometry.rs` and `crates/api/tests/scratch_restart.rs` — geometry
  persistence through the port, and restore/read-back stability across a restart — the
  two-launch check §12.2 says single-launch correctness cannot replace.
- Unit tests pin the `SetWindowPos` flag bits on both writing seams. That an async change
  actually *lands* is deliberately not unit-testable and is owned by the live test.

## Where it diverged

- The bridge-side frame→client conversion is M2 work in flight: `bounds_for` applies the
  saved scale and documents the chrome subtraction that is not done yet
  (`crates/bridge-gpui/src/main.rs`). What is persisted is already correct; the last
  conversion step lands with M2.

## Consequences

- Applying the saved rect is bridge work; storage is core work; neither knows the other —
  `api` is the only place their outputs are joined.
- What platform does *not* do is as load-bearing as what it does: no clamping, no
  work-area snapping, no DPI queries, no window lifecycle. Policy lives above the seam.

## Reopening conditions

Custom chrome (ADR-0003) changes the measured chrome deltas, not the persisted unit.
Reopen only if a second UI toolkit arrives whose windows cannot accept a frame-space rect.
