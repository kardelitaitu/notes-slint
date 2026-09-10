# notes

**Decision and implementation records.** Whether a thing was decided, built, refused, or
retired — the project's working memory, kept apart from planning.

The other tree is [`docs/`](../../docs/), which holds *intent*: planning and developer
documentation. The split is by tense, not topic. A note records an outcome; a doc describes
intent. When the two disagree, **the code wins and the doc gets edited — the note stays**,
because it is a true record of what was believed at the time.

A note is **cheap and disposable**. An ADR in `docs/decisions/` is **permanent and never
edited**. If you find yourself treating a note like a court record, or an ADR like a
scratchpad, they have been confused.

## Folders

| Folder | Meaning | Read it when |
|---|---|---|
| [`proposed/`](proposed/) | On the table, not yet decided. May carry a `> **DECIDED:**` line awaiting code. | Before arguing for or against a feature. |
| [`implemented/`](implemented/) | Built. Documents how it actually works, including where it diverged from the proposal. | Before changing that area of code. |
| [`rejected/`](rejected/) | Decided against, with the reason. | Before re-proposing something. **Check here first.** |
| [`archived/`](archived/) | Superseded or obsolete. Not operative; kept for history. | Rarely. Archaeology only. |

`implemented` means **the code exists and works** — not "we agreed to it". There is
deliberately no "accepted but unbuilt" status; see §4 of the
[`doc-management` skill](../skills/doc-management/SKILL.md).

## Rules

- Filename `YYYY-MM-DD-short-kebab-slug.md` — creation date, unchanged when the note moves.
- Frontmatter `status:` must **match the folder**. A stale status on a misfiled note is the
  most dangerous kind of documentation, because it gets cited as fact.
- Moving a note is a three-part edit in one commit: move the file, update `status:` and
  `updated:`, update `whitepaper.md` if the product changed.
- Full rules: [`.agents/skills/doc-management/SKILL.md`](../skills/doc-management/SKILL.md).
- Start from [`templates/note.md`](../skills/doc-management/templates/note.md).
