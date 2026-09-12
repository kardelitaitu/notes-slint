# docs

**Planning and developer documentation.** Everything here describes *intent* — what we mean
to build, and how to work on it. It is human-facing and part of the product's source.

The other documentation tree is [`.agents/notes/`](../.agents/notes/), which records
*decisions and what actually got built*. See the tense rule below; it is the one that stops
these two trees from drifting into contradiction.

## The plan is split by area, but numbered globally

[`../whitepaper.md`](../whitepaper.md) is the **index**. The plan itself lives in one file
per area:

| § | Area | File |
|---|---|---|
| §1 – §3 | Product: what it is, why, v1 scope | [`product.md`](product.md) |
| §4 | Core behaviour: persistence, autosave, pin, files, `.notes` | [`features.md`](features.md) |
| §5 | Architecture: crates, `api` port, bridges, repo layout, docs model | [`architecture.md`](architecture.md) |
| §6 | Cross-platform, and Wayland's hard limits | [`platforms.md`](platforms.md) |
| §7 | Packaging: the five artifacts, signing gates, CI | [`packaging.md`](packaging.md) |
| §8 | Technical risks R1 – R14 | [`risks.md`](risks.md) |
| §9, §12 | Milestones M0 – M8, and the M0 spike results | [`roadmap.md`](roadmap.md) |
| §10 | Open decisions | [`open-questions.md`](open-questions.md) |

**Section numbers are global across the set, not per-file.** `§4.5` means the same thing
wherever it is written, and ~40 such references already exist. That is why splitting the
document did not renumber anything. Two rules keep this from rotting:

1. **One owner per numbered section.** A section appears in exactly one file. The validator
   builds the anchor set from the whole plan set and fails on a number claimed twice — that,
   not filename, is what "never fork the plan" means now.
2. **Every area doc declares `owns:`** in its frontmatter. A file that doesn't say what it
   owns is a file whose content can quietly grow into another area's remit.

Adding an area: create `docs/<kebab-area>.md` with frontmatter, give it a section number
nobody else holds, add a row to the index in `whitepaper.md`, and run the validator.

## Also in this folder

| Folder | Holds | Written when |
|---|---|---|
| [`decisions/`](decisions/) | Architecture Decision Records — the settled *why*, append-only | A decision is genuinely made and worth its weight |
| [`dev/`](dev/) | Developer documentation: building, testing, packaging, debugging | **After the code exists.** Writing it before is how you produce a beautiful description of something that was never built |

## The tense rule

Every document is either *forward-looking* or *a record*. Put it in the tree that matches:

| | Home | Tense | If it disagrees with reality |
|---|---|---|---|
| What we intend to build | the plan set above, `docs/dev/` | future / present | **The code wins.** Edit the doc. |
| Whether something was decided, and what became of it | `.agents/notes/` | past | The record stands. Add a newer note that supersedes it. |

Two consequences that matter in practice:

1. **`docs/dev/` is thin on purpose until M2.** Developer docs about code that does not
   exist are not documentation, they are speculative fiction with headings. Promote a plan
   into `docs/dev/` when it is implemented — not when it is agreed.
2. **No second index.** `whitepaper.md` at the root is the only entry point. A
   `docs/overview.md` that duplicates the map will be the one people read, and it will be
   the one that goes stale.

Full rules: [`.agents/skills/doc-management/SKILL.md`](../.agents/skills/doc-management/SKILL.md).

