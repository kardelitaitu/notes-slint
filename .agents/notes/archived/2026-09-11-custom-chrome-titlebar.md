---
title: Custom chrome and the title bar design
status: archived
id: 2026-09-11-custom-chrome-titlebar
created: 2026-09-11
updated: 2026-09-11
relates: [whitepaper §3, whitepaper §4.3, whitepaper §4.4, whitepaper §10.2, whitepaper §12.2, whitepaper §12.4]
decision: docs/decisions/0003-custom-chrome-titlebar.md
---

> **DECIDED 2026-09-11 by** [ADR-0003](../../../docs/decisions/0003-custom-chrome-titlebar.md)
> and, for the framework question, [ADR-0002](../../../docs/decisions/0002-bridge-depends-on-gpui-kit.md).
> Kept because it records the options considered; the ADRs are now the authority.

## Question

The first title bar proposal: hamburger left, filename centre (fallback `notes-gpui`),
window controls right — plus a sidebar toggle left of the hamburger. Three decisions hide
inside it: where the menu lives (§10.2), which crate the bridge depends on, and what "no
file open" reads as. A fourth piece, the sidebar button, collides with §3's non-goals.

## Options

### A. Keep native Windows chrome

Menu gets its own row under the caption. §4.3's "visible toggle in the window chrome" and
§4.4's "dirty indicator in the title bar" must be re-worded to say "status row" — a
requirement quietly rewritten because the frame was kept. Zero R11 cost, split identity,
two sentences of §4 downgraded.

### B. Custom chrome via gpui-kit (chosen → ADR-0002 + ADR-0003)

One 34px bar: hamburger + pin left, title centre, OS-integrated window controls right.
`TitleBar` brings drag, double-click-maximise and Windows `WindowControlArea` caption
integration, so R11 becomes a verification checklist rather than a subsystem. The catch M0
did not cover: the kit resolves `gpui-pre ^0.3.1`, not the tested plain `gpui 0.2.2` —
new risk R14; M2 opens by re-running the six M0 checks against the kit.

### C. Custom chrome on bare gpui

Same outcome as B, hand-rolled: drag regions, caption-button OS integration, snap flyout,
accessibility exposure. Rejected — it re-prices R11 at full cost for no different result.

## Recommendation

**B**, with the centre title reading *file name → `Untitled` → app name* rather than the
working title `notes-gpui` specifically — the whitepaper records that name as TBD, and a
provisional name baked into chrome buys a rename sweep later. The sidebar button is not
adopted (see its rejected note); the pin toggle takes its slot beside the hamburger, which
was homeless in the original sketch anyway.

## Consequences

- §10.2 marked resolved **in place**, stable numbering intact.
- R11 re-worded from "decide" to "decided; verification owed"; new R14 added; §12.4's pin
  guidance amended, not deleted.
- M3 geometry must measure chrome deltas in the custom-chrome build, per §12.2 (§10.2's
  consequence that was always pending this decision).

## Reopening conditions

Through the ADRs, not here: caption-integration failure reopens ADR-0003; gpui-pre
verification failure reopens ADR-0002; the sidebar itself reopens through its own note.
