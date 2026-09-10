# decisions (ADRs)

The court record. A settled architectural decision, once, permanently.

**Append-only.** Never edit an accepted ADR's decision text. To change one, write a new ADR,
set the old one's `status: superseded` and `superseded_by:`. The entire value of an ADR is
that nobody can quietly revise the reasoning after the fact.

## Format

Filename `NNNN-short-kebab-slug.md`, zero-padded, sequential from `0001`. The number is
assigned at acceptance, never reused, and never renumbered.

```yaml
---
id: 0001
title: Core engine is UI-agnostic and headless
status: accepted          # proposed | accepted | deprecated | superseded
date: 2026-09-10
deciders: []
supersedes: null
superseded_by: null
relates: [whitepaper §5]
---
```

Body: **Context → Decision → Consequences.** Positive *and* negative consequences; an ADR
that lists only benefits was written by someone selling the decision rather than recording
it.

## Notes vs ADRs

An idea being argued about is a **note** (`.agents/notes/proposed/`) — cheap, disposable,
moves between status folders. A decision that has been made is an **ADR** — expensive,
immutable, never moves.

When a note gets a real decision: write the ADR, set the note's `decision:` field to its
path, and move the note to `archived/`. The note is not the record and should not pretend
to be one.

## Source

The founding decisions of this project live in `whitepaper.md` §10 ("Resolved") until code
exists to make them real. Promote them here as ADRs as they are actually committed to —
not before, and not all at once.
