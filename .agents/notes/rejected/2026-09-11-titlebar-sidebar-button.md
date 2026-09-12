---
title: Sidebar toggle button in the title bar
status: rejected
id: 2026-09-11-titlebar-sidebar-button
created: 2026-09-11
updated: 2026-09-11
relates: [whitepaper §3, whitepaper §4.4, whitepaper §5.5, whitepaper §10.2]
decision: null
---

## Question

The first title bar proposal (2026-09-11, the §10.2 conversation) placed a sidebar toggle
left of the hamburger. §3, §10 and §4.4 each state v1 has no sidebar. What would the button
do, and does the idea survive being asked?

## Options

### A. Reinterpret the slot: it is the pin button

§4.3 already demands a visible pin toggle in the window chrome, and the proposed layout had
no home for it. Renaming the slot's contents adds zero scope and closes a real gap.

### B. A transient recent-files panel

§4.4's recents presented as an overlay rather than menu entries. A presentation of an
existing feature — but panels that hold file lists are how sidebar conversations start.

### C. A persistent file-browser sidebar

Reopens a named non-goal and re-scopes §3 and §4.4 wholesale. It also erodes the §5.5
argument for why the bridge seam can be this narrow at all: "the product is a buffer and a
menu." A sidebar means the port carries file browsing, not just files.

## Recommendation

**Reject as proposed; take A on its way out.** The left cluster becomes hamburger + pin
(ADR-0003), the §4.3 toggle finally has a home, and the no-sidebar non-goal stands
untouched. B is not banned forever — see reopening.

## Consequences

- The title bar ships with no panel affordance; nothing in §3 needed editing.
- "Sidebar button" in a future mockup now points here before anyone re-argues it.

## Reopening conditions

Reconsider **B** if real use shows the menu's recent-files list painful to navigate — the
cap of 10 is doing load-bearing work, and a transient picker is the smallest honest fix.
Reconsider **C** only inside the v2 file-browsing conversation §3 already defers to, and
then as its own note with a §3 revision and a re-examination of how narrow §5.5's seam
must stay.
