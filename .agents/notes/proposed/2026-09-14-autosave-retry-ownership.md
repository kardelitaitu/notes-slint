---
title: Who owns the autosave retry cadence?
status: proposed
id: 2026-09-14-autosave-retry-ownership
created: 2026-09-14
updated: 2026-09-14
relates: [§4.2, §5.2, §5.5]
decision: null
---

## Question

A save failed and the user has stopped typing. Does the app try again — and if so, how
often, and who decides? The two shipped bridges answer differently, so this is no longer
hypothetical: the retry rule is today a per-toolkit habit, and §4.4's line — "Silent
autosave failure is the worst possible outcome — the user believes they are saved and are
not" — is only as strong as whichever bridge the user happens to be running.

## The disagreement, as the code holds it

**`bridge-slint` retries forever.** `79af7b32` (STRIP-4b row 1, decided 2026-09-13) made the
`Event::SaveFailed` arm in `crates/bridge-slint/src/surface.rs:1119-1146` restore the send
witness — `last_sent.clear()`, `edited_flag = true` — so the next quiet tick re-sends a
`Flush`. There is no cap and no back-off: a permanent failure is retried **every
`AUTOSAVE_IDLE` (750 ms), indefinitely**, each attempt printing a `retry: … (retry #N)`
line. The stated win is the one that matters: the bytes **land the instant the cause
clears**, without asking the user to type another character to trigger a save.

**`bridge-gpui` re-sends on the next edit only.** Its `Saved | SaveFailed` arm
(`crates/bridge-gpui/src/main.rs:996-1000`) clears the in-flight marker and nothing else.
`flush_due` (`main.rs:2055-2071`) requires `dirty()`, and `dirty()` is
`seen_edits != flushed_edits` (`main.rs:241-242`) — a counter only an edit moves. So after a
failure the gpui bridge goes quiet, and a user who fixes the cause (closes the locking
program, frees the disk) and then walks away has lost the newest edit until they type
again. Same port, same event, two behaviours — and it is the gpui reading that §4.4 calls
the worst possible outcome.

**Three copies of 750 ms.** The cadence is duplicated rather than shared, because the
port's constant is private: `api` holds `AUTOSAVE_IDLE` at `crates/api/src/engine.rs:64`
("in its temporary home"), `bridge-gpui` mirrors it at `main.rs:117-123` with a comment
saying exactly that, and `bridge-slint` mirrors it again at `surface.rs:205`. A cadence with
three owners is how a policy becomes a coincidence.

## The constraint that decides where this can go

**Core has no clock, and `api` is not allowed policy.** `notes-core` schedules nothing: it has
no timer and no thread, and `save::save_document` is a function that returns
`Result<SaveOutcome, SaveError>` and stops there. `api` does own a tick, but `AGENTS.md`
forbids putting a rule there — "A rule living in `api/engine.rs` instead of `core/` is a
silent architecture change" (§5.2) — and `crates/api/src/lib.rs:27` already states the
present position: "`SaveError` carries the copy for a failure, not a retry policy." So the
only things in this repo with a clock are `api`'s engine tick and the two bridge timers. A
retry rule needs a clock **and** a decision, and no crate today is allowed to hold both.

## Options

### A. The port owns it: the engine re-arms after `SaveFailed`

`api` decides when to try again — it has the tick, and it knows the save failed. One
cadence, both bridges, the constants collapse to one. **Cost:** it puts a policy in the
port, which §5.2 rule 5 forbids; and the bridge owns the text (§5.5, "`api` never sees a
keystroke"), so the port cannot re-send bytes it does not hold — it can only ask, which is
a new command in both directions.

### B. The bridge owns it, and the shape gets written down

Each adapter keeps its own rule, but the rule and its cost become a documented obligation
of "a bridge" (§5.5), and gpui gains the loop so the two agree. **Cost:** N bridges, N
cadences, forever; a third toolkit is a third opinion. This is the status quo, restated
nicely.

### C. `core` owns the rule; the bridge keeps the clock

`core` gains a pure decision — given the failure and how many attempts have happened, what
is the next delay, and when do we stop — with **no timer inside it**. The bridge or `api`
still wakes up, asks, and acts on the answer. Headless-testable in `core`'s own style, and
`api` stays a router forwarding a verdict rather than making one. **Cost:** a new pure
module, plus the attempt count has to live somewhere in the port.

## Recommendation

**C**, and settle the cadence in the same move: a back-off with a cap (750 ms, doubling to
a few seconds, retried while the window is open) plus a persistent, visible failed state in
the UI, which §4.4's amber dot already gestures at.

Why not the others. **A** is the shape that would fix the bug today and it needs an
architecture change to do it: "when do we try again" is business logic, and §5.2 rule 5
says that belongs in `core` — the same reason the autosave-arming rule lives there. **B**
is what has already happened by accident; its cost is this note. **C** respects the
constraint that is actually load-bearing: `core` has no clock, so `core` decides *what* to do
and something else decides *when* to wake up — the same split §5.5 uses for the editor,
where the text crosses as one `Flush` and the timing stays at the surface.

Whichever way it goes, do not leave `AUTOSAVE_IDLE` triplicated. If the answer is "the
bridge owns it", say so in §5.5 and either delete `engine.rs:64` or make it public with a
line naming who must use it.

## Consequences

- A `core` retry rule needs history `SaveError` does not carry: `SaveError` is per-attempt, so
  an attempt count (and maybe the first-failure time) joins the port's state. That is the
  part that makes this not small, and this note is its paragraph.
- Back-off trades directly against what `79af7b32` bought: a cap makes a permanent failure
  quieter and makes recovery slower than "lands the instant the cause clears". Decide which
  of the two is worth more before picking the curve.
- Adopting C changes gpui: its present "quiet until the next keystroke" is the weaker
  promise, so it must gain the loop. That is a user-visible change to M2, not a refactor.
- If nothing is decided, the two bridges keep disagreeing, and the disagreement will be
  cited as a bug later with less context than it has now.

## Reopening conditions

Nothing closes this note; it is open. What would force a decision:

- A third bridge starts (§6, mac or Linux): three cadences is unmaintainable, and C stops
  being a design question.
- Any report of "it never saved until I typed again" — that is the gpui arm, and it turns
  A or C from a design change into a bugfix.
- A settings-loaded autosave interval (§4.2): once the cadence is data it cannot live in a
  bridge, and the constant collapses by necessity.
- A disk-full or sync-client incident where retrying every 750 ms is itself the damage: an
  unbounded loop against a failing volume is a resource leak, which argues for C's cap.
- Choosing A anyway — it contradicts §5.2 rule 5, so it needs an ADR, not a commit message.
