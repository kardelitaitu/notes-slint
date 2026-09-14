---
title: A held switch waits for the answer it asked for
status: proposed
id: 2026-09-15-held-switch-waits-for-answer
created: 2026-09-15
updated: 2026-09-15
relates: [§4.2, §4.4, §5.5]
decision: null
---

## Scope, and it is narrower than the bug that found it

A review turned up one root cause with two children. The **retry** belongs to
`.agents/notes/proposed/2026-09-14-autosave-retry-ownership.md`, whose addendum carries the channel
law (a retry is a Save, never a Flush), the three repairs, and the plumbing authority this note
borrows. This note owns one thing only:

> **What a switch may do while it waits for a save answer.**

A switch is an act that replaces the document: Open (`Ctrl+O` or a menu row), a recent-file row, New
document. Save As is not a switch in this sense - the port keeps the buffer through it and emits
`Saved` then `Rebound` - and the close path is not either, because it already holds for a terminal
answer through `settle_says`.

**Nothing described here exists in the tree yet.** The funnel is a design being built, so this is a
law written before its door, not a report about shipped code. That ordering is deliberate and rule 3
explains why.

## What is in the tree today, so the law is anchored

- The pump's only save-answer signal is **a counter**: `pump.saves_settled`, moved once in each of the
  three terminal arms (`Saved`, `AutosaveSkipped`, `SaveFailed`), and read by the close wait. Its own
  comment states the property that makes it unsafe for a switch: "`saves_settled` is ANY terminal
  answer (Saved / AutosaveSkipped / SaveFailed)."
- The answers themselves are more informative than the counter. `Event::Saved { path, revision }` and
  `Event::SaveFailed { path, revision, reason }` both name what they answer - `Saved`'s doc says "A save
  succeeded, and the buffer at `revision` is now what is on disk" - while `Event::AutosaveSkipped {
  reason }` names nothing at all. It is a verdict about a gate, not about a send.
- The port has already legislated this exact distinction. `Loaded`'s `epoch` field: "**The bridge does
  not own a counter and must not keep one**: it stores this value on `Loaded`/`Rebound` and echoes it
  back ... That is what makes the guard an echo check rather than a race between two independent
  counters."
- And the stale case is already a category: `SkipReason::Stale` - "the stale Flush was DISCARDED, and
  this says so". A held request whose generation moved is a thing the port can name.

## Rule 1: release on the answer the funnel SENT, never on a counter

Release when the event names the `(path, revision)` pair the funnel sent, and only then.

The failure mode is concrete and it is a silent one. A save is in flight; the person asks for an Open;
the switch is held. Meanwhile a debounce tick answers `AutosaveSkipped { reason: AutosaveDisabled }`, or
`Clean`, for a send that was never at risk. `saves_settled` moves. A funnel released on that counter
now opens the new file **while the newest text of the old one has never reached disk** - the precise
loss the funnel exists to prevent, arriving as an unrelated event, with the unit suite green.

That is why the counter is the wrong type of fact: it is a **rate**, not an **identity**. It answers
"did something answer", and the question a switch must ask is "did MY send answer".

Three precedents in this repo already choose identity over recency:

- `Engine::save_as` decides its guard by asking `refused_target(path)` - the *identity* of the target -
  and its comment refuses any looser rule: "A refusal of some OTHER name does not follow the buffer
  around ... holding THAT hostage is the c92494f3 bug in reverse."
- `Loaded.epoch`'s law, quoted above: echo what you were told; do not run a second counter.
- The send witness itself: `save_now` adopts "the pair it would have flushed" at the ask, because "the
  epoch stamp exists precisely so the SENDER owns the pairing."

Matching costs no new state: the funnel is created at a send and already knows the pair it sent. And a
hold that waits for identity is not a hold that can wait forever - it is bounded by the retry's cap and
terminal state in the other note, which is where an answer that never arrives ends up.

## Rule 2: on refusal, hold and tell - and a second press is the confirmation

When the answer says the newest text did not land, the switch does not happen. The hold is shown, in
words, and the person presses the same act again to confirm it.

This is the rule a reader will dispute, so the argument is written out rather than asserted.

- **Switching anyway re-creates the loss the funnel exists to prevent.** The point of holding an Open is
  that the buffer is about to be replaced by another file's text. If the bytes that were just refused
  are the newest copy of the person's note, replacing them is not "doing what was asked"; it is
  destroying the only copy in the name of responsiveness. The README's promise is that the app "never
  asks you to save" - a promise about not asking, and it has never been a promise to never refuse.
- **Refusing the thing asked is only silent if the reason is not on screen** - and the reason already
  is. The failure lane has two channels today: the explain channel (`publish_explain`, which is what
  puts a reason where a person is looking) and the sticky `why` line. Rule 1 of the other note's
  repairs is exactly what makes the sticky line truthful here: with a precedence rule, the line says the
  disk refused, instead of being overwritten by a setting's sentence that tells the person to perform
  the act that just failed. Hold-and-tell therefore costs no new UI surface; it spends one that exists.
- **A dialog is ruled out, twice.** `docs/features.md` §4.4 on the unsaved-changes guard: "It must not
  become a modal that interrupts typing." And ADR-0007 lists the same rule among its reopening
  conditions: "If `Save` ever names a file - a dialog, a prompt - §4.4's no-modal rule returns and 0007
  reopens." So the design space is not "dialog or lose text". With a dialog forbidden, **a second press
  of the same act is the only honest confirmation available**: a person who pressed the act again, while
  the reason was on the screen, has read the reason. No setting, no checkbox, no timeout that decides
  for them.

Two disciplines come with it, and both are load-bearing:

- **The hold is one request deep, not a queue.** A newer switch replaces the held one (the newest intent
  wins) and the replaced one never fires. A queue would make confirmation impossible - the second press
  would be queued behind the first rather than answering it - and would let a switch the person has
  since abandoned fire late.
- **The second press must not be swallowed by the first hold.** It is the confirmation; treating it as
  a duplicate is how the design would become a dead end.

Two presses is friction. The answer is that the friction appears only where a write has already failed,
which is the one moment in this app where friction is cheap and silence is expensive.

## Rule 3: the tick delay is a feature under hold-and-tell, a hazard under switch-anyway

The funnel defers a switch to the next post-drain step of the product tick, so the hold is up to one
tick long. Which of the two that is depends entirely on what the release does with it.

- **Under hold-and-tell the delay is the mechanism.** The tick is where the answer arrives, where the
  identity is checked, and where a released switch therefore runs with the bytes already on disk. One
  tick, bounded, and invisible.
- **Under switch-anyway the same delay is a hazard.** During it the person keeps typing. A switch that
  fires "on time" on a timer rather than on an answer then discards text written *after* the send - a
  loss this design would own by construction rather than inherit by accident, and the shape is
  indistinguishable in the log from the failure it was meant to prevent.

Say it plainly, because it is the reason the release shape was chosen before the door was built and not
after: **the cheap option was not rejected for cheapness.** Waiting costs the same lines as not waiting
- one predicate instead of a comparison against a counter. What differs is whether the delay is bounded
by an answer or by a clock. A later reader must not mistake the shape for an accident of implementation.

## The residue this design accepts, in the thinker's own words

**A person who ignores the reason on the screen and never presses twice keeps their text unsaved - and
knows it.** That is the most this design can promise, and it is worth writing down as accepted rather
than engineered away, because the alternative to it is one of the two things rule 2 forbids: switch
anyway, or ask in a modal.

The residue has a cousin that the hold cannot fix: the disk is still refusing. A held switch does not
retry the write, does not free space, and does not un-set a read-only bit; it only declines to make the
refusal permanent by overwriting the buffer. The write attempt's fate belongs to the retry's terminal
state in the other note. Two different jobs, and a reader who conflates them will expect this hold to
be a recovery.

## No new ADR, and the three things that would force one

ADR-0007 does not need reopening: its send-witness clause is **incomplete rather than false**. Adoption
at send stands untouched ("it sends Save with the pair it would have flushed and ADOPTS that pair as its
send witness"), and "one answer per `Save`" stands untouched too - a second `Save` issued by a retry is
a second *command*, each of which is still answered exactly once. The count of commands is not the count
of answers, and the funnel adds neither: it holds a UI act, not a port interaction.

Three things would change that, and each of them is a design change rather than a detail:

1. **Arming moving** off write success - e.g. onto the ask, or onto the hold - which is exactly the fix
   the other note refuses, and which would make ADR-0001's promise false in its own words.
2. **A new `Command` or `Event` variant appearing** to carry what the funnel needs. Worth noticing that
   rule 1 needs none: `Saved` and `SaveFailed` already carry `path` and `revision`. So if the slice ever
   demands a new answer kind to match an identity, that is evidence the design drifted, not that
   the port was thin.
3. **The funnel ever being demanded of the probe.** It is opt-in data installed only by product
   wiring; an instrument that never installs one has nothing to release. If a future slice starts
   wanting the funnel inside `notes-slint-probe.exe`, the frozen instrument has gained product
   behaviour and that is an ADR, not a commit message.

## Evidence, because the honesty gate is symmetric

Whatever lands, the proof that the instrument is untouched is **the probe report diffed against a
recorded run after the funnel lands** - not a comment saying it is unaffected. The report is text with a
fixed line set, so a diff is a fact rather than an opinion: if any line moves, the funnel leaked
into the instrument and the design is wrong, not the baseline.

Two limits on this note, stated so nobody borrows authority it does not have. **No §9 row moves in
either way** pre-CI, per `AGENTS.md`; and everything here was read off source in a docs session with no
shell - no runtime behaviour is claimed, and the loss scenario in rule 1 is an argument about the shape
of the state, not a measured failure.

## The order of work the two notes imply

So a reader can pick this up cold. Four slices, in this order, each with its own green run recorded in
the commit that lands it.

1. **The door.** The one post-drain step in `product.rs`'s tick - the place that already holds a
   gateway, right where the tick calls `drain(&tick_events, &tick_pump, &ui.as_weak())` and then
   `text_pump(&tick_gw, ...)` on the same wake. It does nothing on its own; it makes sending possible
   from a place that sees the answered events. Not between the panic-hook take and `Gateway::start`:
   that region is sliced by file index and asserted against.
2. **The retry trio** - the `pending` field, the `why` precedence rule, and `retry_says` - in the other
   note, with the retry channel switched from `Flush` to `Save`. First, because a retry that re-sends
   `Flush` is dead, and every rule here about refusal presumes a refusal that can actually be produced.
3. **The funnel.** Hold one switch, carry the `(path, revision)` pair it sent, release on that
   identity, hold-and-tell on refusal, second press confirms. Built against the door from slice 1, so it
   never grows a second route to the port.
4. **The pure-decision tests.** `retry_says`, and the release predicate - which deserves the same
   treatment, so `switch_says` in the house `*_says` idiom (`settle_says`, `floored_says`), asserting
   on a function rather than grepping for an arm. Last, because the decisions have to exist before they
   can be made pure; and this is the slice that retires the grep-grade pin the other note names as the
   bug.

## Recommendation

1. **Adopt the three rules as written.** Rule 1 first, because it is the only one with a data-loss
   argument behind it and the only one that can be satisfied by accident as well as on purpose; a
   counter-based release would pass every test in this repo today.
2. **Adopt hold-and-tell with the second press as the confirmation**, and the one-request-deep hold, in
   the same slice as the funnel - the residue is acceptable only when the telling is real, so the `why`
   precedence repair is a precondition of this rule, not a companion to it.
3. **Build in the order above**, four slices, each carrying its recorded run.
4. **Keep the funnel out of the instrument by construction, and prove it with the diffed report**
   (see Evidence above). Do not accept a comment that says the probe is unaffected.
5. **Write no ADR now.** Write ADR-0008 the moment any of the three triggers fires, and treat trigger 2
   - a new answer kind needed to match an identity - as a smell on this design rather than a port gap.
6. **Hand the residue to the intent tree** when the guard is written up: `docs/features.md` §4.4
   owns the unsaved-changes guard, and an accepted residue is a product-visible behaviour, so it
   belongs there - as an edit owed by whoever lands the switch, not by this note.

## What this note does not do

It amends nothing outside itself, and it is not a report of a defect in shipped code: the funnel
does not exist yet, so no behaviour described here has been observed either working or failing. It
does not touch `docs/`, an ADR, a probe arm, or any `implemented/` record; it does not move a §9
row; and it does not decide the retry question, which stays in the note that owns the cadence.

## Reopening conditions

- **If a save answer ever stops naming a revision**, rule 1 cannot be honoured by matching and the port
  needs an event change - trigger 2 above, and an ADR rather than a workaround.
- **If the second press stops being available** - a switch reachable only through a surface that is
  itself held - then hold-and-tell has no confirmation channel, and §4.4's no-modal rule has to be
  revisited on its own terms rather than quietly broken.
- **If a held switch ever needs to survive re-entry** (two queued intents), the one-deep discipline
  breaks, the replaced request stops being dead, and rule 3's hazard argument returns in full.
- **If CI lands and §9 starts moving**, the evidence rule changes from a locally diffed report to
  whatever the workflow records; the symmetry of the gate survives, its artefact does not.
