---
title: Cross-platform strategy
type: planning
owns: ['§6']
status: living
updated: 2026-09-10
---

# Cross-platform strategy

Owns **§6** of this project's plan. Part of the set indexed by
[whitepaper.md](../whitepaper.md) — section numbers are **global across the set**, so a
reference like `§4.5` resolves from any file. Each numbered section has exactly one owner;
the validator fails if one appears in two files.

## 6. Cross-platform strategy

Windows first, then macOS + Linux. The abstraction seams above are what make that a
port rather than a rewrite. But the honest ordering of difficulty is not what people
expect:

- **Windows (now):** the target. Everything is designed for it.
- **macOS (later):** low risk. GPUI originated here; Metal backend is the mature path.
  Topmost is a simple `NSWindow.level`. Window positioning is well-behaved.
- **Linux (later):** **highest risk, and it is a protocol problem, not a Rust problem.**
  - **Wayland does not let clients position their own windows.** Placing a window at a
    saved `x,y` is a compositor decision and is *forbidden* to clients by design.
    "Always remember window position" — a headline feature — is not achievable as
    specified on Wayland.
  - **Wayland has no standard always-on-top protocol** either. Some compositors support
    it via zwlr extensions; there is no guarantee.
  - X11 handles both fine (`_NET_WM_FRAME_EXTENTS`, `_NET_WM_STATE_ABOVE`).

  **Consequence:** on Linux the two flagship features may degrade to "size remembered,
  position and pinning best-effort." This needs a decision before we commit publicly to
  Linux parity — see §10. It does not change v1, but it changes what we promise.

