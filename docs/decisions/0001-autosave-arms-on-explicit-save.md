---
id: 0001
title: Autosave is armed by an explicit save on files this app did not create
status: accepted
date: 2026-09-10
deciders: []
supersedes: null
superseded_by: null
relates: [whitepaper §4.2, whitepaper §4.5, whitepaper §5.4, whitepaper §10.3]
---

## Context

Autosave is a headline feature: the app never shows a save dialog and never asks. The app
also opens ordinary text files — `package.json`, someone's `README.md`, a log — which it did
not create and which may be under version control, shared, or read-only on purpose.

Those two facts collide. Autosaving a foreign file rewrites it seconds after the user opened
it, with no affirmative act of saving. §4.5 already forbids silently changing a foreign
file's encoding or line endings; this is the same class of harm, one level up — changing a
file the user may not have intended to change at all.

The blast radius is asymmetric. For a `.notes` file, "it saved without asking" is the
product. For someone else's repo config, it is an unexplained dirty working tree.

## Decision

**Autosave engages immediately for `.notes` files. For any file this app did not create,
autosave stays disarmed until the user performs one explicit save (`Ctrl+S` or the Save menu
item), after which it is armed for that document for the rest of its session.**

The distinction reuses the boundary that already exists in the product: the same
`.notes` / foreign split that drives §4.5's do-no-harm rules.

Supporting requirements, all part of this decision:

1. **Silence is forbidden.** While disarmed, the UI must show why nothing is being saved.
   `Event::AutosaveSkipped { reason }` exists for this and must be rendered, not merely
   defined. An unexplained silent no-save is worse than either alternative.
2. **Armed state is per document, not global.** Opening a foreign file must not inherit the
   armed state of the file before it.
3. **The menu's auto-save toggle reports the state of the *current* document**, so the user
   can tell "autosave is off" from "autosave is off for this file until you save it once".
4. **Save As arms the new path.** The user just chose a location explicitly; requiring a
   second save would be pedantic.

## Consequences

**Positive**

- Foreign files are safe from unrequested modification, while `.notes` keeps the
  zero-friction experience that is the product's point.
- One `.notes`/foreign concept serves both this rule and §4.5, instead of two similar-but-
  different special cases.

**Negative, and accepted**

- Two modes where option A had one. Some users will find "press Ctrl+S once, then it just
  saves forever" odd, and it will need to be explained.
- The global auto-save toggle can no longer mean one thing for every file — a small
  violation of the app's otherwise simple settings surface.
- `core/autosave.rs` needs an armed/ungated concept, and `Command::Flush` must consult it.

**Forecloses**

- A single global autosave toggle with identical behaviour across all file types.
- Option C (shadow copies) as a *default* behaviour; it remains available as a future
  addition if sync lands, which is where it would actually be needed.

## Reopening conditions

Move to always-autosave (**A**) if testing shows the arming step causes people to lose work
or to misread the indicator — the safety benefit is worthless if its cost is unsaved notes.
Revisit shadow copies (**C**) only if multi-device sync is added, at which point a shadow
copy is needed independently of this decision.

## Provenance

Decided from [`.agents/notes/proposed/2026-09-10-autosave-foreign-files.md`](../../.agents/notes/proposed/2026-09-10-autosave-foreign-files.md),
which records the alternatives considered. Note statuses are `proposed`/`implemented`/etc.;
ADR statuses are `proposed`/`accepted`/`deprecated`/`superseded`. The two vocabularies are
deliberately separate: `accepted` here means the decision stands, not that the code exists.
