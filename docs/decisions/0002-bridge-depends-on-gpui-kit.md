---
id: 0002
title: The GPUI bridge depends on gpui-kit, not bare gpui
status: accepted
date: 2026-09-11
deciders: []
supersedes: null
superseded_by: null
relates: [whitepaper §5.1, whitepaper §5.2, whitepaper §5.5, whitepaper §10.2, whitepaper §12.3, whitepaper §12.4]
---

## Context

§10.2 — native title bar or custom chrome — was the open decision that could not survive
being deferred past M2 (R11). Whether the custom option is cheap or expensive turns on a
fact the planning set had never stated: **which crate the bridge depends on.** M0 tested
plain `gpui = "=0.2.2"` (§12.4), and `docs/` mentions nothing else. Bare gpui ships no
title-bar component; custom chrome there means hand-rolling window drag, caption-button OS
integration, double-click-maximise, and Win11's snap and accessibility surface — the entire
cost R11 feared.

`gpui-kit` — the published kit wrapping GPUI with a component layer — ships a `TitleBar`
that owns that machinery, including Windows `WindowControlArea` integration, the mechanism
that lets the OS treat drawn caption buttons as real ones. It also ships most of the rest of
what M2 needs: menus, tooltips, icons, notifications. So the framework question is not
merely about chrome; it decides whether the bridge *assembles* its widgets or *builds* them.

## Decision

**`bridge-gpui` depends on `gpui-kit`, pinned exactly — `gpui-kit = "=0.6.1"` at
acceptance — and treats it as its toolkit.** Application code reaches GPUI via
`gpui_kit::*`, widgets via `gpui_kit::component`, default assets via `gpui_kit::assets`.

The layering rules are untouched, which needs stating explicitly because it is the easy
thing to doubt:

- gpui-kit **is** the GPUI bridge's toolkit; "a bridge may import `api` and its own
  toolkit" (§5.2) describes the same single import edge.
- `core` and `api` still never see gpui or the kit; every `cargo tree` check in §5.2 is
  unchanged and still enforced.
- The §5.5 startup order and the bridge-owns-the-editor rule are unaffected.

## Consequences

**Positive**

- R11 shrinks from construction to verification: drag, double-click-maximise,
  caption-button OS integration and DPI-adaptive chrome arrive with the component.
- M2's hamburger menu, the tooltips §4.4's truncation rule requires, and the amber
  autosave-failure indicator have off-the-shelf parts instead of bespoke elements.
- One dependency, per the kit's own contract — not a pile of separately-versioned
  component crates.

**Negative, and accepted**

- **M0 tested a different crate.** gpui-kit 0.6.1 resolves `gpui-pre ^0.3.1`, not the
  `gpui 0.2.2` the spike verified. Every §12.3 API fact — HWND reachability, topmost via
  the handle, built-in dialogs, `WindowBounds`, decorations — is now a claim about a
  forked codebase. Recorded as R14; M2's opening task re-runs the six M0 checks against
  the kit before anything is built on the findings.
- The kit moved fast before acceptance (three releases in the first week it existed). The
  pin is exact; every update becomes a deliberate event with R14's checks re-run.
- A third party's fork-maintenance posture is now load-bearing for the UI — the same bet
  §5.4's bridge seam exists to hedge, priced one layer up.

**Forecloses**

- Hand-rolled chrome on bare gpui. Reverting is a superseding ADR, not a refactor.

## Reopening conditions

Revert to bare `gpui = "=0.2.2"` if (a) M2's re-verification fails on anything the kit
cannot fix — HWND/topmost, dialogs, or caption-button OS integration under `gpui-pre` —
while plain gpui still passes; or (b) the kit's churn starts forcing changes into the frame
loop or the buffer-ownership split (§10.4) that a dependency should never demand.

## Provenance

Decided 2026-09-11 in the §10.2 conversation recorded at
[`.agents/notes/archived/2026-09-11-custom-chrome-titlebar.md`](../../.agents/notes/archived/2026-09-11-custom-chrome-titlebar.md).
The choice was explicitly delegated by the human partner ("what's your recommendation?") —
recorded because a delegated decision is still a decision and this ADR carries its risk.
Taken together with ADR-0003, which assumes it.
