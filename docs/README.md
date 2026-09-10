# docs

**Planning and developer documentation.** Everything here describes *intent* — what we mean
to build, and how to work on it. It is human-facing and part of the product's source.

The other documentation tree is [`.agents/notes/`](../.agents/notes/), which records
*decisions and what actually got built*. See the tense rule below; it is the one that stops
these two trees from drifting into contradiction.

## Contents

| Folder | Holds | Written when |
|---|---|---|
| [`decisions/`](decisions/NNNN-slug.md) | Architecture Decision Records — the settled *why*, append-only | A decision is genuinely made and worth its weight |
| [`dev/`](dev/) | Developer documentation: building, testing, packaging, debugging, code organisation | **After the code exists.** Writing it before is how you produce a beautiful description of something that was never built |

The founding sketch is **not** here — it is [`../whitepaper.md`](../whitepaper.md), at the
repo root, and there is exactly one of it.

## The tense rule

Every document is either *forward-looking* or *a record*. Put it in the tree that matches:

| | Home | Tense | If it disagrees with reality |
|---|---|---|---|
| What we intend to build | `whitepaper.md`, `docs/dev/` | future / present | **The code wins.** Edit the doc. |
| Whether something was decided, and what became of it | `.agents/notes/` | past | The record stands. Add a newer note that supersedes it. |

Two consequences that matter in practice:

1. **`docs/dev/` is thin on purpose until M2.** Developer docs about code that does not
   exist are not documentation, they are speculative fiction with headings. Promote a plan
   into `docs/dev/` when it is implemented — not when it is agreed.
2. **Never fork the whitepaper.** No `docs/planning.md`, no `whitepaper-v2.md`, no
   `docs/roadmap.md`. A second copy of the product definition is a guarantee that in three
   months someone reads the wrong one. The validator fails the build on filenames matching
   `whitepaper` or `roadmap` anywhere except the root.

## What goes where, in one line each

- **New idea, not decided** → `.agents/notes/proposed/`
- **Decided against** → `.agents/notes/rejected/`
- **Settled decision, worth a permanent record** → `docs/decisions/NNNN-…`
- **Built, and needs explaining to the next person** → `.agents/notes/implemented/`
- **How to build / test / debug the code** → `docs/dev/`
- **Product or architecture thinking** → `whitepaper.md`

Full map: [`.agents/skills/doc-management/SKILL.md`](../.agents/skills/doc-management/SKILL.md).
