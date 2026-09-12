---
id: 0003
title: The app draws its own title bar (custom chrome)
status: accepted
date: 2026-09-11
deciders: []
supersedes: null
superseded_by: null
relates: [whitepaper §9, whitepaper §4.3, whitepaper §4.4, whitepaper §10.2, whitepaper §12.2]
---

## Context

§10.2 asked where the hamburger menu lives: a content-area row under a normal Windows title
bar, or a title bar of our own. Three settled requirements were already leaning on the
answer. §4.3 demands the pin toggle be *visible in the window chrome*; §4.4 demands a dirty
indicator *in the title bar* that goes amber when autosave fails; and the product bet (§2)
is that the window behaviour **is** the product — the frame should look like ours. With
ADR-0002 in place, R11's machinery is a component we verify rather than code we write, so
the remaining question is design and verification, not feasibility.

## Decision

**Custom chrome.** One 34px title bar (the gpui-kit `TitleBar`, its `window_options()` as
the window's base), divided into exactly three regions:

| Region | Contents |
|---|---|
| **Left** | hamburger — opens the §4.4 menu (Open / Save / Save As / Auto-save toggle / Recent files) as a popup anchored beneath it · **pin toggle** — the §4.3 visible chrome toggle, plus its keyboard shortcut |
| **Centre** | document title: a dirty dot (amber on `SaveFailed`) + file name → `Untitled` for a new unsaved document → the app name. Full path always in the tooltip; centred ellipsis truncation, permitted only because the tooltip exists |
| **Right** | minimize / maximize / close, wired through the kit's Windows `WindowControlArea` |

There is **no sidebar button** — the §3 no-sidebar non-goal stands untouched
([`.agents/notes/rejected/2026-09-11-titlebar-sidebar-button.md`](../../.agents/notes/rejected/2026-09-11-titlebar-sidebar-button.md)).

The OS-level window title is still set (`name — app` form) so Alt+Tab, taskbar previews and
accessibility tooling speak correctly even though nothing renders it visibly.

## Consequences

**Positive**

- One band, not two: no native caption stealing a strip above a menu row.
- §4.3's pin toggle and §4.4's dirty/amber indicator get a designed home instead of a
  re-wording.
- The window identity is ours, which is the point of the §2 bet.

**Negative, and accepted**

- R11's residue is owed in full, as a checklist against a purchased component: drag,
  double-click-maximise, Win+arrow snapping *and* the hover snap-layout flyout, caption
  accessibility exposure, rounded corners and shadow, DPI at 100/150/200%. Remote-desktop
  and IME behaviour under custom chrome join the list. All M2 acceptance work.
- §12.2's coordinate trap widens: chrome deltas vary with custom chrome, so M3 must
  measure them in the custom-chrome build. The native 19/8/20/8 numbers recorded in §12.2
  are void for the shipped window.
- The title shows the file's real name with its real extension — §10.1's undecided
  `.notes`-vs-`.md` question changes what users will see in this title bar without
  changing this rule.

**Forecloses**

- The native-chrome layout (menu row under a Windows caption). Supersede to return to it.
- Making the menu's primary entry the native system menu; the hamburger is the entry.

## Reopening conditions

Revert to native chrome via a superseding ADR if M2 verification shows the kit's caption
integration cannot pass the Windows checklist — snap layouts, accessibility, remote desktop
— and the fix lies in `gpui-pre`, out of our reach. The fallback is already implied: menu
row under the native caption, with pin and dirty indicator relocated to a status row.

## Provenance

Decided 2026-09-11, resolving §10.2, in the conversation recorded at
[`.agents/notes/archived/2026-09-11-custom-chrome-titlebar.md`](../../.agents/notes/archived/2026-09-11-custom-chrome-titlebar.md).
Depends on ADR-0002.
