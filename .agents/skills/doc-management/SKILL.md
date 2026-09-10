---
name: doc-management
description: Place, create, and lifecycle-manage every documentation artifact in this repository. Use when adding, renaming, or moving any markdown file; when a design decision is proposed, accepted, rejected, or superseded; or when asked "where does this doc go?". Also use to validate existing placement.
---

# Documentation management

Every markdown file in this repo has exactly one correct location. This skill defines the
map, the note lifecycle, and the invariants. When in doubt, run the validator (§6).

## 1. The placement map

There are **two documentation trees, and they are separated by tense, not by topic:**

| Tree | Holds | Tense | Audience |
|---|---|---|---|
| `docs/` + `whitepaper.md` | **Planning and developer documentation** — what we intend to build, and how to work on it | future / present | humans, contributors |
| `.agents/notes/` | **Decision and implementation records** — whether a thing was decided, built, refused, or retired | past | agents and humans, as working memory |

The tie-break question is: *"is this describing intent, or recording an outcome?"* Intent
goes in `docs/`; outcomes go in `.agents/notes/`. If they disagree later, **the code wins**
— `docs/` is edited, `.agents/notes/` is left standing and superseded by a newer note.

Decide what the artifact **is**, then put it where the table says. Never invent a new
location; if nothing in the table fits, stop and add a row here first (via a note in
`proposed/`).

### Planning & developer docs

| The artifact is… | It goes in | Mutability |
|---|---|---|
| The founding sketch: product definition, architecture, risks, roadmap | `whitepaper.md` (root) | Living. **Exactly one.** Never fork it into `*-v2.md`. |
| How to build, test, package, or debug the code | `docs/dev/<topic>.md` | Living — and **not writable before the code exists.** See `docs/dev/README.md` |
| A settled architectural decision, accepted after deliberation | `docs/decisions/NNNN-slug.md` (ADR) | **Append-only.** Never edit an accepted decision; supersede it. |
| Agent instructions, repo conventions, invariants | `AGENTS.md` (root) | Living |
| Human-facing intro: what this is, how to get it | `README.md` (root) | Living |
| A reusable agent capability | `.agents/skills/<skill-name>/SKILL.md` | Living |
| Build, package, or release automation | `scripts/` | Living |
| Installer manifests, `.desktop` files, dmg/entitlements inputs | `packaging/<os>/` | Living |

### Decision & implementation records

| The artifact is… | It goes in | Mutability |
|---|---|---|
| A feature or behaviour idea on the table | `.agents/notes/proposed/` | Fluid |
| An idea decided **against**, kept so nobody relitigates it | `.agents/notes/rejected/` | Near-frozen |
| Something **actually built**, documenting how it works | `.agents/notes/implemented/` | Living until removed |
| Anything superseded, obsolete, or no longer operative | `.agents/notes/archived/` | Frozen |

### The boundary that gets crossed most

Two confusions, both expensive, both preventable:

- **A note is not an ADR.** A note is *working memory*: an idea being argued about, moving
  between status folders, cheap and disposable. An ADR is a *court record*: decided,
  reasoned, immutable. When a note gets a real decision, it **graduates** into an ADR, sets
  its `decision:` field, and moves to `archived/`. The note is not the record. Confusing
  them yields a permanent file full of live speculation, or a working note nobody dares to
  edit.
- **A plan is not a dev doc.** `docs/dev/` describes code that exists. Product and
  architecture intent belongs in `whitepaper.md`. Promote a plan into `docs/dev/` when it is
  *implemented*, not when it is *agreed* — otherwise you are maintaining fiction that reads
  with authority. `docs/dev/README.md` lists which file unlocks at which milestone.

## 2. Note file naming

```
YYYY-MM-DD-short-kebab-slug.md
```

Date is the **creation** date and never changes when the note moves folders. Slug is 2–5
words. Example: `2026-09-10-autosave-foreign-files.md`.

## 3. Note frontmatter (required)

```yaml
---
title: Autosave on foreign files
status: proposed
id: 2026-09-10-autosave-foreign-files
created: 2026-09-10
updated: 2026-09-10
relates: [whitepaper §10.3, whitepaper §4.5]
decision: null
---
```

- **`status` must equal the folder it lives in.** This is the single most common
  documentation bug and the validator rejects it.
- `id` is stable forever; treat it as a key, not a path.
- `relates` points at whitepaper sections, using `§` numbers.
- `decision` is `null` until the note graduates to an ADR, then a repo-relative path.
- Body structure is free, but every note should state the **question**, the **options**,
  and a **recommendation** — a note with no recommendation is a ticket, not a note.

Use `templates/note.md` to start.

## 4. Lifecycle

```
                    ┌─── accept ───> write ADR ─── build ───> implemented
                    │                                            │
   (new) ──> proposed                                              ├── removed ──> archived
                    │                                            │
                    └─── reject ───> rejected ──── relitigated ──┤
                                                                  │
   implemented / rejected ──── superseded or obsolete ───────────>┴─> archived
```

**Four states, and one honest gap.** There is no status for *accepted but not yet built*
— which is where most of this project's decisions currently sit. Do not stretch
`implemented` to cover it: `implemented` means **the code exists and works**. Until then,
keep the note in `proposed/` with `status: proposed` and a one-line `> **DECIDED:** …` at
the top, and mirror the decision into `whitepaper.md` §10 "Resolved". If that gets
uncomfortable, propose a fifth status as a note rather than quietly misusing the four.

### Transition procedure

Moving a note between status folders is a **three-part edit, all in one commit**:

1. `git mv` (or equivalent) the file to the new status folder. Path changes, filename does
   not.
2. Update `status:` to match, and bump `updated:`.
3. Update `whitepaper.md` §10 if the decision changes anything in the product or
   architecture — and check whether a `relates:` section still points somewhere real.

Leaving a note in the wrong folder with a stale `status:` is worse than deleting it,
because it will be cited as fact.

## 5. Editing rules

- **ADRs are append-only.** To change one: write a new ADR, set the old one's `status:
  superseded` and add `superseded_by:`. Never rewrite history — the record's whole value
  is that nobody can.
- **One `whitepaper.md`.** Large restructure is fine; a `whitepaper-archived.md` is not.
- **No new top-level `.md`** beyond `README.md`, `AGENTS.md`, and `whitepaper.md`.
  Anything else is evidence the table above needs a row, not a new root file.
- **Section anchors are load-bearing.** `whitepaper.md` uses `§N` numbering and ~60
  cross-references depend on it. When inserting a section, renumber **and** fix every
  inbound `§` reference in the same edit. Grep for `§` before committing.
- Never store user text, note *contents*, or secrets in any doc. Notes about `.notes`
  files are fine; actual notes are user data and never in the repo.

## 6. Validation

Run after any documentation change, and fix before reporting the work done:

```
pwsh .agents/skills/doc-management/scripts/check-docs.ps1
```

It checks:

- **status/folder agreement** — the one that gets cited as fact when wrong
- note filename format, frontmatter completeness, duplicate `id`s
- a `## Recommendation` section in every note, and `## Reopening conditions` in rejections
- orphaned `.md` files, and extra root-level markdown
- **forked intent** — any `whitepaper`/`roadmap` filename outside the root
- loose files in `docs/` (that root holds only `README.md`)
- **premature dev docs** — anything in `docs/dev/` besides its README while no `Cargo.toml`
  exists, because the tense rule above is unenforceable in the abstract, but this is not
- every `§` cross-reference, resolved against `whitepaper.md` headings *and* its numbered
  decision items (so `§10.3` is a real anchor), skipping code fences

Exit code 0 means clean. Note what this list cannot check: whether an ADR's *reasoning* is
sound, or whether `docs/dev/` prose still matches changed behaviour. Those are review jobs;
the validator is a floor, not a ceiling.

## 7. When asked "where does this go?"

Answer from §1 in one line, name the file you will create, and say whether it is a note or
an ADR — because that choice is the one people get wrong, and getting it backwards means
either a court record full of speculation or a working note nobody dares to edit.
