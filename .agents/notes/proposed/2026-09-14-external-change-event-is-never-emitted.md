---
title: ExternalChange is declared, rendered, and never emitted
status: proposed
id: 2026-09-14-external-change-event-is-never-emitted
created: 2026-09-14
updated: 2026-09-15
relates: [§4.2, §4.5, §5.4, §8, §9, §10]
decision: null
---

> **NOT DECIDED:** nothing here is agreed. No status row in §9 or README moves because of this
> note, and the claim is neither asserted nor deleted in it. Both options below stay open; the
> note exists so the gap is on the record while the decision waits on CI.

## Question

Does v1 detect a file changed outside the app before overwriting it (§4.2), or not? The port
declares the event that would announce it, one bridge renders it, and **no code produces it** —
while the milestone text lists external-change detection among what M4 delivered. Which of
those is the truth: the prose, or the absence of a producer?

## What was found

Every line below was read in this session.

- **Declared.** `crates/api/src/event.rs:440` — `ExternalChange { path: PathBuf }`, under the
  doc comment at `:438-439`: "Something outside the app changed the file on disk. The bridge
  decides what to do about it; the port only says it happened."
- **Produced by nothing.** The port has exactly two ways an `Event` enters the channel:
  `Engine::emit` (`crates/api/src/engine.rs:2326-2333`) and two `event_tx.send` calls in
  `crates/api/src/gateway.rs:325` and `:342` (`StateDirUnusable`, `SettingsCorrupt`). None
  names this variant. Repo-wide the identifier appears in ten places, and all ten are the
  declaration, an exhaustive `match` that cannot fail to compile, a fabricated fixture, or a
  consumer's arm: `event.rs:440` (decl), `:630` (`Clone`), `:664` (`variant()`), `:715`
  (the test's `all_events`), `:766` (a comment); `crates/bridge-gpui/src/main.rs:1228` and `:1230`
  (the render arm and its sentence), `:2788` and `:2846` (the vocabulary fixture and its name
  list).
- **Claimed by the intent docs.** `docs/roadmap.md:178-180` — "**M4 — Autosave & pin.**
  Debounce, periodic flush, blur/close/quit flush, external-change detection, and the
  byte-identical round-trip test suite (§4.5)." `docs/features.md:47-48` — "**External
  edits.** If the file changed on disk while we held it, detect it (mtime + hash) before
  overwriting. v1 behaviour: warn and keep both, do not silently clobber." §8's R6 mitigation
  still reads "Detect external modification before write" (`docs/risks.md:25`). The variant
  first appears in the plan's own enum sketch, inside §5.4 (`docs/architecture.md:272`).
- **Milestone rows, untouched here.** `README.md:72` — "| M4 | **Done.** Autosave and pin. |";
  `README.md:14` — "M4 … built and tested".

**Reproduce it yourself** — the emit-site probe returns nothing (exit 1):

```sh
rg -n 'Event::ExternalChange' crates/api/src/engine.rs crates/api/src/gateway.rs
```

And the whole-tree view, which shows the absence's shape — every hit is the port describing the
variant to itself, or a bridge consuming it:

```sh
rg -n 'ExternalChange' crates/ docs/
```

## Why the variant exists but is unreachable: declared alongside its siblings, never wired

The note and ADR record is silent. The commit record is not. What was probed:

- `.agents/notes/**` — no note proposes, accepts, or names this variant. The closest prior
  record is the same absence, found six days earlier and described without the type:
  `.agents/notes/proposed/2026-09-12-arm-on-explicit-save-path.md:87-93` ("A §4.2 promise B and
  C would lean on does not exist … Nothing in `crates/core/src/save.rs` compares the target's
  current bytes or mtime with what was loaded … Do-no-harm today is **format** preservation,
  not **ownership** preservation"), and its reopening condition at `:170`, "Move to **A** if
  external-change detection cannot be built and a real user clobbers" something. So this is the
  second note to record one missing mechanism: the first was about the save path, this one is
  about the event that would have reported it.
- `docs/decisions/` — zero hits for `external`, `watch`, `debounce`. **No ADR claims it.**
- Commit messages, via the readable reflog `.git/logs/HEAD` (401 entries), grepped for
  `external|watch` — three hits, none of them a file watcher: "feat(core): external revision
  note on Document with a single D11 flush gate" (`5a07c4c3`, amended as `7a8d7085`), which is
  core's D11 revision field; "feat(bridge): hamburger popup, per-event trace lines, show-state
  watcher" (`3324766a`), which polls window show state; and a docs-commit wording.
- **The pickaxe, since run** — `git log -S ExternalChange --oneline` (executed in the parent session on
  2026-09-14; this session had no shell tool, so the probe is cited, not re-derived). Four
  commits have ever changed how many times the name appears in the tree, and each is accounted
  for:

  | Commit | Message | What it did to the name |
  |---|---|---|
  | `473f785e` | planning docs: whitepaper v0.0.1 … | the plan named it first, in the §5.4 enum sketch (`docs/architecture.md:272`) |
  | `33e29c89` | docs: split whitepaper into docs/ tree; record ADR-0002/0003; add gpui-kit skills | the plan's snippet moved into the docs tree — a docs change, not a code change |
  | `2b6561b0` | feat(api): the port — Command/Event vocabulary, FileMeta, SaveError, WindowHandle | the variant was **declared**, with the rest of its enum |
  | `needa6ae0` | feat(bridge): wake on engine events and render every variant | a **consumer** was wired: the render arm in `crates/bridge-gpui/src/main.rs` |

  Not one of them added a producer. That is the entire history: named in the plan, declared into
  the vocabulary commit beside its siblings, carried across the docs split, given a bridge's
  render arm — which is what AGENTS.md instructs, to render what the port can say rather than
  reach around it — and never wired to anything that could send it. So the shape of the gap is
  no longer a guess: the bridge half of `needa6ae0` is the same commit that makes every
  `Event` variant renderable, which is why the arm exists and why nothing notices the absence.

  The limit first, because it scopes the two claims under it. `git log -S` walks **first-parent
  history from HEAD**, and it counts commits where the number of occurrences changed — it does
  not read each diff. So this probe cannot see a producer added and removed inside a single
  commit, nor one that lived only on a branch never merged into walked history. What it does
  establish is the present and the walked past: no producer exists in any crate now, and of the
  four commits above, none is a producer's commit by its own message, by the file it names, or by
  what the tree holds today. "Never wired" is a claim about this repo's walked history and current
  tree — not about every branch anyone ever pushed — and the Reopening conditions below name the
  residue that would change it.

  What the record then retires, with that scope attached: it is **not a regression** — no commit in
  the walked history removed an emit site, because an added-then-deleted producer would have
  landed in the four-line list as two more entries. And it is **not a decision that outran its
  code** — no ADR, no note, and no commit message ever proposed or accepted external-change
  detection. It is a variant declared alongside its siblings and never wired: the enum was written
  from the §5.4 sketch, and the one member whose producer needs new machinery is the one that got
  none. No severity is claimed for that: it is a declared contract with an unbuilt side.

## Option A — build it: a watcher in core, emitted from the engine

A real feature slice, not a plumbable match arm.

- **Something must ask.** No `Command` in `crates/api/src/command.rs` carries "watch this
  path" — the vocabulary is Open, SaveAs, Flush, SetAutosave, SetPinned, SetCornerRounding,
  ClearRecents, Shutdown, RegisterWindow, GeometryChanged, UnregisterWindow. Either a new
  variant (and a bridge that sends it), or no new variant at all: a **core-side** answer
  riding the existing 750 ms tick (`AUTOSAVE_IDLE`, `crates/api/src/engine.rs:64`, serviced by
  `on_tick` at `:590`) — a stat is cheap, and the tick already exists.
- **Where the decision lives.** The mtime + hash comparison *decides* something, so it belongs
  in `core`; `api` routes and translates (AGENTS.md: "`api` contains no business logic").
- **The hard constraint.** No blocking I/O added to core's save path, and detection may not
  rewrite anything while it looks — §4.5's do-no-harm applies to the checker as much as to the
  writer. Autosave is asynchronous, so a detection has no caller to return `Err` to and must
  travel as an Event.
- **What A does not buy by itself.** §4.2's v1 behaviour is "warn and keep both".
  `ExternalChange { path }` carries no before/after bytes and no policy; shipping the producer
  without the policy moves the gap, it does not close it.
- **Exit criterion, if A is chosen:** a test that changes the file behind the engine's back and
  asserts the event arrives *and* the foreign bytes survive the next flush.

## Option B — correct the intent docs so they stop claiming it

What it would touch: `docs/roadmap.md:179` (drop "external-change detection" from the M4 line),
`docs/features.md:47-48` (move the requirement to a later milestone or into §10),
`docs/architecture.md:272` and `crates/api/src/event.rs:440` (the variant becomes
declared-for-later, or goes), and the R6 mitigation at `docs/risks.md:25`.

**Why it is parked, and why nothing above was edited.** Removing the claim from the §9 milestone
text is a **status move**, and AGENTS.md's gate forbids one pre-CI in either direction: "No M2
check's status moves on either bridge's account pre-CI … The honesty gate is symmetric: a row
that can only travel one way is not a gate, it is a narrative." The sentence names M2 checks;
the same symmetry is what governs the M4 wording under examination (`README.md:72`), and the
condition it cites holds here — `git remote -v` is empty and no workflow has ever run, so there
is no CI account on which any row may move. Deleting the claim would also erase the reason the
variant exists, which is exactly the quiet half of the trade. **So B is parked pending CI, and
this note is the placeholder that keeps the claim from being quietly asserted or quietly
deleted.**

## Second instance of the same shape: the debounce ignore

`crates/api/tests/debounce.rs:69` —

```text
#[ignore = "the engine has no document debounce yet: flush() saves per Flush, AUTOSAVE_IDLE gates only session/settings - see the FINDING note above"]
```

with the FINDING at `debounce.rs:21-29`: the debounce this file was written to prove "DOES NOT
EXIST … This test ran red with Saved revision 1 arriving in 0.04 s: three Flushes, three
saves." The machine check is switched off, and the prose around it is stronger than the check:

- `README.md:23` — "Autosaves | Continuously, **debounced**, atomically."
- `docs/roadmap.md:179` — M4 includes "Debounce".
- `.agents/notes/implemented/2026-09-12-m4-autosave-pin-recents.md:62` lists
  `crates/api/tests/debounce.rs` under "## Tested" as "the 750 ms cadence: fires once when
  quiet, re-arms after firing" — that note contains no occurrence of `ignore` anywhere.
- The honest one, for contrast:
  `.agents/notes/implemented/2026-09-13-show-bit-two-sample-latch.md:168-170` counts the suite
  as "138 `#[test]` in `crates/api` less the one documented `#[ignore]` at
  `crates/api/tests/debounce.rs:69` (no document debounce yet — a different finding, unchanged
  by this commit)".

Same failure mode as `ExternalChange`, one rung lower: not an event with no producer but a test
that would catch it, disarmed — while three documents, one of them an `implemented` record,
describe the mechanism as present. The `#[ignore]` reason is currently the most accurate
sentence about document debounce in the tree.

**This is the note's second live instance, and it is still open.** The index sentence —
`README.md:23`, "Autosaves | Continuously, **debounced**, atomically." — asserts a behaviour the
suite deliberately declines to prove, and it has now been flagged twice while working the intent
docs without being changed. It was not changed here, on purpose: softening or striking that word
is a **status move**, so it is gated. The symmetry that stops an M2 or M4 row travelling **up**
before CI is the same thing stopping the word from being deleted — AGENTS.md's reason is exactly
this: "a row that can only travel one way is not a gate, it is a narrative." So this note records
the gap, edits no README, and does not touch the `#[ignore]`. The owner of the gate decides, and
the two honest exits are the same pair as above: build the document debounce so the test can be
un-ignored, or stop advertising the word — with CI's account, not this note's.

## Recommendation

**Hold both options, and do the cheap probes first.** Concretely:

1. Keep this note standing as the record. Do not restate external-change detection as shipped
   anywhere, and do not delete `docs/roadmap.md:179` or any M4 row pre-CI — Option B is a §9
   move, and the gate is symmetric.
2. Provenance is settled by the pickaxe record above: declared with the vocabulary in
   `2b6561b0`, rendered in `needa6ae0`, never wired. So Option A is a build, not a revert — there
   is no deleted producer to resurrect and nothing to bisect.
3. If the app is meant to detect external edits in v1 — §8's R6 (a sync-folder conflict costs
   the user data) is the argument for it — choose Option A, as a scoped slice with the exit
   criterion above, not as an emit bolted onto a bridge.
4. If it is not meant to, then Option B happens **with** the CI evidence that makes a row move
   legal, in one commit that also names where the promise went (a §10 entry), not as a quiet
   doc edit.
5. The debounce word is **not** separable after all, and this note's first draft claimed it was:
   softening or striking `debounced` in `README.md:23` is a status move, so it waits on CI exactly as
   Options A and B do. What survives the correction: one of `README.md:23`, `docs/roadmap.md:179`
   and `debounce.rs:69` is wrong, and it is the human's call which — but the call is gated, not
   free.

## Consequences

- The easy path — deleting one phrase from the milestone line — is now a decision with a named
  owner rather than an edit.
- The variant stays declared, so nothing about the compiling record changes: `bridge-gpui`'s
  exhaustive arm (`main.rs:1228`) and both vocabulary fixtures stay valid. Removing it is a
  vocabulary change that both ledgers already police in comments (`event.rs:766`,
  `crates/bridge-gpui/src/main.rs:2837-2840`), so it cannot happen quietly either.
- §4.2's mtime + hash requirement is now recorded as unimplemented in a file that outlives the
  session that found it. Anyone citing `docs/roadmap.md:179` as evidence finds this note cited
  back.
- Forecloses nothing: Option A remains fully available, and choosing it later is not an
  admission that the milestone line was wrong — it is the line becoming right.

## Reopening conditions

- CI exists and has run (a remote is configured) → rows may move in either direction, and
  Option B becomes legal; re-decide then.
- A real clobber is observed — a user loses bytes to OneDrive, Dropbox, or a second editor →
  Option A stops being a feature slice and becomes a fix; §8 R6 has materialised.
- A producer turns out to have existed — inside one of the four listed commits' own diffs, or
  on a branch that `git log -S` never walked — then this note becomes a regression record and
  Option A shrinks to a re-wire.
- Document debounce lands → the `#[ignore]` at `debounce.rs:69` is deleted, and this note's
  second instance is superseded by whatever record says so.

## What would make this note resolvable — and who decides

Resolvable when exactly one of these is true **and recorded**:

1. **A producer exists.** Some code path emits `Event::ExternalChange`, and a test that edits
   the file behind the engine's back sees it and proves the foreign bytes were not clobbered.
   This note then moves to `implemented/` carrying that file:line.
2. **The intent docs stop claiming it,** in a commit that also names the CI state which made
   the move legal. This note then goes to `archived/` with `decision:` pointing at the §9 edit
   or the ADR that recorded the move.
3. **The human decides v1 will not detect external edits** → an ADR (append-only) plus a §10
   entry, and this note to `archived/` pointing at the ADR.

**Owner: the human.** Every option ends in a sentence — in §9, §4.2, or README — that says
either "we ship this" or "we do not", and this repo's own gate says such rows move on CI's
account, not an agent's. An agent may build Option A, after which the record simply follows the
code; only a person may retire a promise, and only a person may keep one that has no producer.
No severity is assigned in this note, and none should be inferred from its existence.
