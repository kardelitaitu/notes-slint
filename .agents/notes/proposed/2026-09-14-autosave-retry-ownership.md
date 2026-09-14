---
title: Who owns the autosave retry cadence?
status: proposed
id: 2026-09-14-autosave-retry-ownership
created: 2026-09-14
updated: 2026-09-15
relates: [§4.2, §5.2, §5.5, §4.4]
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

---

## Addendum, 2026-09-15: the channel law, found by review rather than by design

A review of the shipped `Ctrl+S` found that this note's question has already been
answered by accident, and answered wrongly. The cadence debate above survives it; one of
its consequences does not, and the wrong sentence is named below rather than edited away,
so a reader of the original finds the correction where the error was.

`59de9c59` made A5 real: `save_now` sends `Command::Save` and adopts the send witness at
the moment of the ask, in ADR-0007's own words - "it sends Save with the pair it would
have flushed and ADOPTS that pair as its send witness (bump `edits`, resync `last_sent`,
clear the pending debounce)". The comment gives the reason the pairing is taken at the
ask: "the epoch stamp exists precisely so the SENDER owns the pairing."

On failure the bridge does what this note already documented: clears `last_sent`, sets the
edited flag, and the retry that follows is **the ordinary debounce pump sending
`Command::Flush`**.

**That loop is structurally incapable in the DEFAULT case.** The path the review walked:

1. A foreign file is open. `armed` is set only on write success - core's `mark_saved` is
   the one site that sets it - so a file whose FIRST save failed stays un-armed
   permanently.
2. The pump re-sends a `Flush`, and `should_flush` refuses it on `ForeignFileNotArmed`.
3. With auto-save off it refuses on `AutosaveDisabled` instead. Same silence, other gate.
4. The skip arm restores nothing - it settles, words the reason, and leaves `last_sent`
   alone.
5. And the send already re-synced `last_sent`, so the comparison lane early-returns
   forever.

Four independent ways to lose a person's newest text, in the ordinary case of one failed
`Ctrl+S` on a file the app did not create - against a README whose first sentence promises
an app that "never asks you to save". Each of the four is a gate that exists for a reason,
which is what makes the composition a design error rather than a bug in one place.

### The law

> **A retry is a Save, never a Flush.**

The reason is the pair of gates above, and why passing them is not a loophole.
`Document::should_save_manual` differs from the debounced rule by exactly two gates BY
DESIGN and says so: `AutosaveDisabled` is bypassed because the toggle is a standing
instruction about unattended writes, and `ForeignFileNotArmed` is bypassed because
ADR-0001 arms a foreign file on an explicit save - "An explicit Save IS the arming act."
A gate that refused the retry until the file was armed "would make its own arming act
unreachable - the deadlock ADR-0007 exists to avoid." So a retry routed through `Flush`
is a retry routed through the gate the retry exists to pass. The channel is not a detail
of the cadence; it decides whether the cadence can ever fire.

This also honours what A5's own comment conceded about refusals: "the refused-load guard,
the Clean refusal and the read-only verdict belong to the engine's ANSWER and not to
anything decided here." The answer lane is where a retry's fate is decided, and `Save` is
the command that has one.

### The shape, with its end

- The bridge's back-off re-sends `Command::Save`: **750 ms, doubling, to a cap**.
- **At the cap it falls to a terminal state**, and the terminal state is not silence: the
  witness stays dirty, the next user act carries the text, and the status line keeps
  saying what failed. Retrying a doomed `Save` forever would be the resource leak this
  note's own reopening conditions warned about, now with a write attempt attached to it.

### Where the cadence does NOT go, said plainly

Not into `api`: it has no clock for this and `AGENTS.md` forbids the rule - "A rule living
in `api/engine.rs` instead of `core/` is a silent architecture change" - and
`crates/api/src/lib.rs` still states that `SaveError` "carries the copy for a failure, not
a retry policy." Not into `core` either, as a timer: core has no clock, and this note
already wrote the sentence that settles the split - core decides *what* to do and something
else decides *when* to wake up. Option **C** survives this addendum unchanged for the
**curve**, because a pure `attempt -> delay -> stop` decision is exactly the rule core can
own and test headless. What the addendum changes is the **payload**, and the payload was
never in dispute.

**Two sentences in the body above are now wrong, and are named rather than rewritten.**
"Adopting C changes gpui: its present 'quiet until the next keystroke' is the weaker
promise, so it must gain the loop" - it must gain a **Save** loop, not a Flush loop; a gpui
retry that sends `Flush` inherits the same four gates and stays just as dead. And "a cap
makes a permanent failure quieter and makes recovery slower" - under the channel law a cap
no longer trades against recovery, because the terminal state hands the write to the next
act instead of parking forever.

### Three repairs, one root cause in three faces

All three are the same mistake wearing different clothes: **"unsaved" is encoded as a
string comparison and as a side effect of whichever event arrived last.**

1. **Stop encoding "unsaved" as a string comparison.** Clearing `last_sent` cannot make an
   *empty* buffer differ - the pump's `identical` test is `text == p.last_sent`, and ""
   equals "" - so the branch clears the edited flag, prints "edited and edited back - the
   bytes are identical, nothing sent", and early-returns on a buffer that never went
   anywhere. Carry an explicit **pending** field instead: one bit meaning "bytes exist that
   the disk does not have", independent of what any string looks like. The same field fixes
   the stranded deletion, where deleting everything leaves the witness equal to the buffer
   and the deletion with no way to be noticed. A lane that must compare less text than it
   stores has already lost.
2. **`why` needs a PRECEDENCE rule, not another unconditional overwrite.** Both lanes write
   `p.why` with no order between them: the failure lane writes "save failed: <reason>" and
   the next skip writes `skip_words(reason)` over it. The result tells a person to perform
   the act that just failed - the menu explains that the file needs one explicit save, over
   the sound of that save being refused by the disk. The rule: **a verdict about the disk
   outranks a verdict about a setting**, and a setting's sentence may be dropped, never
   allowed to overwrite a disk's. This is not a wording task: the two lanes disagree about
   reality and one of them is being allowed to shout.
3. **The grep-grade pin was the bug, not the test.** The present guarantee that the witness
   comes back on failure is a source-text lookup, which is why wrapping the restore in `if
   autosave_on` would keep the suite green while making case 3 above the normal path.
   Extract a pure `retry_says(...)` decision in the house `*_says` idiom - the precedent is
   in the same crate: `settle_says` for the close wait and `floored_says` for geometry - and
   assert on the function. A pure predicate is testable without a window, and it cannot be
   silently wrapped.

### One thing this addendum explicitly does NOT propose

**Do not move arming to a failed ask.** It is the tempting fix: it would make the retry's
`Flush` pass `should_flush`, and it costs one line. It also arms a file whose bytes never
landed, which turns the promise into its opposite - `api`'s own save documentation names
that exact state, "rendering `save once and it keeps saving` about a file that has just
been saved once", and it is why the `Rebound` announcement exists at all. Arming is a claim
about bytes on disk. A retry that needs arming to work has the wrong channel.

### Plumbing authority, because a code slice depends on it

- **Neither the retry nor a held switch can send from inside `drain`.** Its signature is
  `drain(events, pump, weak)` - there is **no gateway** between that signature and the wire
  block - and its callers are the product tick, once per wake, and the probe, all the way
  through its arm walk. Anything that must send has to be handed the gateway, and `drain`
  deliberately is not: it is the one place both binaries share, and a send inside it would
  be a product decision made on the instrument's account.
- **The door is ONE post-drain step in the product tick**, which already holds a gateway:
  the tick calls `drain(&tick_events, &tick_pump, &ui.as_weak())` and then, on the same wake,
  `text_pump(&ui, &tick_gw, &tick_pump)` - quoted as an exact literal, because that string is
  one of the three the ordering test slices for, so the citation is the thing the test reads.
  A step placed after the drain therefore sees both the answered events and the wire, which is
  the only place in the product where a retry can be issued without inventing a second route
  to the port.
- **That step must NOT sit between the panic-hook take and `Gateway::start`.** The region is
  sliced by file index and asserted against - `product.rs`'s own module header enumerates
  what is paid for it, including `smoke.rs`'s `PRODUCT_CLOSE_NEEDLES` - so inserting there
  moves a slice a test already counts, for no behavioural reason whatever.
- **The probe must not gain the behaviour.** The funnel the retry and the switch share is
  **opt-in data installed only by product wiring**; a probe that never installs one has
  nothing to release, so it releases nothing and its report cannot move. That is the
  guarantee in the shape of the code rather than in a comment - the same
  instrument-preservation rule that keeps `notes-slint-probe.exe` mounting the very same
  `Spike` while owning none of the product's doors.

### What this addendum changes in the recommendation

The Recommendation section above stands: **C**, core owns the curve, the bridge keeps the
clock, and the rule becomes an obligation of "a bridge" so a third toolkit inherits it
instead of inventing it. Add to it, in this order: the retry's **channel** is `Save`
(the channel law); the retry needs the **pending** field and the `why` precedence to be honest
about itself; and the guarantee must move from a grep to `retry_says`. The
"Consequences" bullet that has gpui gain "the loop" is read as the Save loop for the reason
above - a Flush loop there would be a second implementation of a dead design.

### The neighbour this addendum creates

The retry stays this note's. **What a switch may do while it waits for an answer is not** -
that law lives in
`.agents/notes/proposed/2026-09-15-held-switch-waits-for-answer.md`, which shares the
funnel, the plumbing constraint above, and the discipline of answering rather than waiting.


### Evidence moved: the transcript changed, and that is a re-earning owed, not a regression

`7f6d40e7` landed the channel law, and it changed what the frozen instrument **reports** -
which is a re-earning owed under ADR-0006 §4, not a regression, and this paragraph exists so
that nobody reads the old transcript and concludes the bridge got worse. Four lines were
**removed**: the family `retry: a failed save restored the send witness (#1..#4)`, because
the probe's read-only-lock act presses Save on a file it cannot write, the old lane cleared
`last_sent`, and the pump re-sent a `Flush` 750 ms later and was refused again - the doomed
loop this addendum condemns, running inside the instrument and being read as normal output
(`probe.rs` carries no `retry` line at all today). One line **changed**: the chrome verdict
moved from `save-failed=true dirty=false` to `dirty=true`, so the old record contained a
failed save reporting the document CLEAN. Nothing was **added**: the honest else-branch
sentence saying a refusal had no explicit Save behind it was deleted before the commit
precisely because ADR-0006 §4 protects what the instrument reports and an adapter slice may
not add to it - the branch survives as a comment in
`a_refused_save_owes_a_retry_and_the_retry_carries_the_same_pair`, where it is asserted
rather than printed. Measured: the save-failed family went 17 lines to 13, unique normalised
product lines 140 to 139, the six `do-no-harm` lines and the nine arming lines are
unchanged, and all seven overlay triples are byte-identical. Two gaps, and they are not the
same gap. The **call is pinned**:
`an_owed_retry_decides_a_save_and_the_door_is_called_after_the_drain` unwraps `expect("the
door is called in the tick")` on the literal call, so deleting the call panics that test, and
one review claim did not survive reading it. The **send inside it is what no test reaches**,
because a send needs a live gateway and a queue: `retry_door` names its own `send(gw, ...)`
"the one untested line", and its test says plainly that it "does NOT prove a byte reached the
channel". Between those two sits a third, weaker fact - the SaveFailed arm's write is guarded
only by a source-slice assert, so gating just that write on the autosave toggle would keep the
suite green. And the ask: whoever re-earns a menu or save claim against that transcript must
cite the **new** line set, because a verdict earned against four doomed lines is not evidence
about the fixed bridge.
