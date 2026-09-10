---
title: Autosave behaviour on files the app did not create
status: archived
id: 2026-09-10-autosave-foreign-files
created: 2026-09-10
updated: 2026-09-10
relates: [whitepaper §4.2, whitepaper §4.5, whitepaper §5.4, whitepaper §10.3]
decision: docs/decisions/0001-autosave-arms-on-explicit-save.md
---

> **DECIDED 2026-09-10 and superseded by** [ADR-0001](../../../docs/decisions/0001-autosave-arms-on-explicit-save.md):
> option B. Recorded here because it contains the alternatives considered; the ADR is
> now the authority.

## Question

When the user opens a file this app did not create — `C:\project\package.json`, someone's
`README.md`, a log file — should autosave write to it without an explicit save?

The tension is real and one-sided: **autosave is the feature that makes this app good for
the user's own notes, and the same feature that rewrites other people's files.** Opening a
foreign file is deliberate; overwriting it three seconds later, silently, may not be.

## Options

### A. Autosave in place, always

Matches the feature name and the mental model. Simplest rule, no modes, nothing to
explain. `core/autosave.rs` needs no knowledge of provenance.

Costs: the app will rewrite a file the user opened to *look at*. A read-only-by-accident
file in a repo gets modified on disk with no affirmative act. Worst case is a dirty working
tree someone else has to explain.

### B. Explicit save arms autosave, per document

`.notes` files autosave immediately. Foreign files require one `Ctrl+S` / **Save** before
autosave engages, and until then behave like a conventional editor (dirty indicator, and a
guard on Open/Close).

Costs: two modes, which means one of them will confuse someone. Needs `Command::Flush` to
carry or consult an "armed" flag, and needs `Event::AutosaveSkipped { reason }` so the UI
can say *why* nothing was saved — an unexplained silent no-save is worse than either
option. That event variant exists only if we choose B.

### C. Autosave to a shadow copy, reconcile later

Never touches the foreign file; keeps a recovery copy and lets the user adopt it.

Costs: a reconciliation UI, an extra on-disk location, and a save model the user cannot
predict. Effectively introduces a second source of truth. Highest complexity by a wide
margin, and it complicates the crash-recovery story in §4.2 rather than simplifying it.

## Recommendation

**B**, with `AutosaveSkipped` surfaced in the status indicator so the reason is always
visible.

Reasoning: the `.notes` / foreign distinction already exists in the product (§4.5 defines
the do-no-harm rules for foreign files precisely *because* they are different). Reusing it
as the autosave boundary costs little, while option A imports an unbounded blast radius
into an app whose whole selling point is that it saves without being asked. The risk profile
is asymmetric: B's cost is a mode some users find odd; A's cost is modified files the user
did not intend to change.

The one thing that must not happen is silence. B is only acceptable if the UI explains
itself.

## Consequences

- Makes B: `Command::Flush` gains an armed/ungated distinction;
  `Event::AutosaveSkipped { reason: SkipReason }` becomes load-bearing and must render.
- Makes B: "did my note save?" needs an answer that differs by file type — the menu
  should show the autosave state *for the current document*, not globally.
- Forecloses: a single global autosave toggle that means the same thing for every file.
- Neutral: §4.5's encoding and line-endings rules apply under every option.

## Reopening conditions

Re-open toward **A** if user testing shows the arming step causes people to lose work or to
misread the indicator — the safety benefit is worthless if the cost is unsaved notes.
Re-open toward **C** only if we add multi-device sync, at which point a shadow copy is
needed anyway.

## Blocked on

Nothing in code, but it changes `core/autosave.rs`'s interface and the `api` event
vocabulary, so it should be settled before M1 is scaffolded.
