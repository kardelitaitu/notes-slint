---
title: The flush must share the refused-load guard
status: proposed
id: 2026-09-15-flush-shares-the-refused-load-guard
created: 2026-09-15
updated: 2026-09-15
relates: [whitepaper §4.5, whitepaper §5.2, whitepaper §5.5, whitepaper §9, whitepaper §10]
decision: null
---

Cited by symbol rather than by line number, on purpose: three workers are moving lines under this file
right now, and every wrong number this session came from a brief that quoted one. Where a line number
is itself the fact, it is named as one.

## The sentence in the code that asked for this note

`Engine::save`'s doc comment closes by naming an asymmetry and declining to fix it:

> [`Engine::flush`] asks nothing of the kind today, and that asymmetry is recorded rather than fixed
> here: closing it changes what a debounced write may do, which is autosave's owner's call, not a
> routing slice's. The ADR sentence this makes true is 0007's `api still applies the refused-load
> guard`.

This is that call, made as an artifact before any code, and it is why the date is today: the routing
slice that recorded it has shipped, so the ask is older than this file. The quoted ADR clause is
[ADR-0007](../../../docs/decisions/0007-save-is-a-conformance.md) in its own words - "`api`
still applies the refused-load guard" - and api's comment is where the deferral was written down.

Nothing here is a discovery about the design. The refused-load guard is the fix for the measured
0-byte overwrite - write the buffer into the file whose `Command::Open` was refused, and a person
loses a document they never saw. `Engine::save_as` asks that question at its head. `Engine::save`
asks it again at step 4b, because a Save may have no caller-named target but always has a target.
`Engine::flush` asks it not at all.

## Scope: which half is a hole, and which half is somebody else's cleanup

The refused-load refusals split by who can see them, and getting this wrong is how the next reader
"fixes" a non-hole.

**The size half is already closed, twice, and is not the asymmetry.** Over the guard, `Engine::open`
answers from the stat alone - `LoadError::TooLarge` - and returns: "nothing opens, no buffer exists,
no `FileMeta` is built, because there is nothing honest to render." There is no buffer to flush, so
the debounced path cannot clobber a file refused for size. That refusal is pinned by
`an_oversize_file_is_refused_from_the_stat_and_cannot_be_overwritten` in `api/tests/session.rs`.
And the standing guarantee is core's, not the port's: `Skip::Oversize` lives in
`Document::should_autosave`, which `Document::should_flush` reaches through, and core states its
own reason in the terms this note would otherwise have to invent - the guarantee is core's, and "a save
the user asked for must not rewrite bytes the app refused to read." Core also says the branch is
unreachable through today's engine, because every `Document::open` call site hard-codes
`oversize: false`, and keeps it anyway: "safe against today's engine and fatal against tomorrow's,
which is the wrong trade to make in the crate whose whole job is do-no-harm."

Two consequences, and both of them are "do not":

- **Do not add a size branch to the flush.** A name refused for size is recorded in the same
  `load_refused_for` state as every other refusal, so the single guard proposed below covers it as a
  side effect. A second, size-specific test in `Engine::flush` would be a second rule about a fact
  the port already remembers once.
- **Do not read the unreachable-flag comment as "growth is detected".** A file that grows past the
  guard *while open* is noticed by no live path: the verdict comes from a stat at open time and there
  is no re-stat. That is a different question from this note's, it belongs closer to the external-change
  work than to the refused-load guard, and it is named here only so it is not mistaken for the hole
  this file is about.

**The read-policy half is the real asymmetry.** A name can be refused *after* a perfectly good document
is already on screen: undecodable bytes, or a load that failed on a sharing violation or a denied ACL.
The Windows error-code knowledge stays core's - `classify_io_error` is what separates those two - and
the port only shapes the answer, through `fail_load` or the decode arm, and both set
`load_refused_for`. Because a refused load moves no generation and replaces no `Document`, the
buffer behind the epoch is still the one the user is looking at, so both of `Engine::flush`'s existing
guards pass: the epoch equality holds, and core answers `should_flush` honestly about the flags it was
handed.

That is why the record put the guard in `api` rather than in core, and core agrees in its own words:
`Document::should_save_manual` carries a paragraph headed NOT A DOOR PAST THE PORT, saying the
refused-load guard "is about a PATH, and core holds no such state: it decides only from what it was
handed." There is no `Document` behind a refused name to consult.

## The reachable pair, narrowed honestly

Two of the three kinds are actually dangerous, and the note is weaker if it implies the third is too.

- **A denied ACL usually denies the write as well.** The flush reaches the write and answers
  `SaveFailed` truthfully from the filesystem. Nothing is lost and no lie is told; the guard would
  only change which layer says no. Real, and not the reason to act.
- **Undecodable but writable.** A `.doc` - or any non-text file - dropped into the path of a note this
  app has open and loaded. The re-open refuses at decode, the editor still shows the old body, autosave
  still has a target, and the write succeeds. Somebody's readable bytes are replaced by our text.
- **Shared-open but writable.** Another program holds the file in a mode that refuses our read; we
  refuse the load; it lets go; the debounce fires and we win the file. The interval between the refusal
  and the write is precisely the interval the guard exists for.

Both real, neither exotic: they are the ordinary company of a text editor in a folder with other
programs in it.

And the bytes destroyed are the *newer* ones. The refusal was produced by what is on disk now; the text
in the buffer came from an earlier successful read of that path. So this is not the neutral hazard of
"our stale text loses to newer text" - it is somebody else's file being replaced by ours, at a name
where we had already looked and said we would not read that.

## The premise this decision was made on, recorded because it was false

The director left the asymmetry open on the argument that **an autosave refusal is invisible by
design**, so refusing on the debounced path would buy no honesty and cost some surprise.

That premise is false as of the work that surfaced autosave verdicts, and the note is incomplete
without the correction. An autosave refusal today is a sticky reason in the menu, a status-line
sentence, and a *settled* answer to an honest shutdown: the pump's `Event::AutosaveSkipped` arm
exists precisely because "Autosave OFF was never silence - the port answers this event", and it was
added after the close wait burned its full 2.0 s on every session with autosave off. An invisible
answer is still an answer the port emitted.

> **invisible is not the same as silent, and a guard does not need a dialog to be honest.**

So the argument that decided the question does not hold, and what it was holding is unpleasant on its
face: leaving `flush` free makes writing foreign bytes **unacceptable if a person presses a key,
acceptable if nobody does**. That is the wrong axis for a law about somebody else's file. The axis that
is right - which bytes the app declined to read - does not care who asked.

This is a SCOPE paragraph, not a downgrade of the earlier decision: the decision was made on a premise
that has since been disproved, and the honest record is the premise, the disproof, and the new answer.

## Where the rule lives, and what it must not become

`api`, in `Engine::flush`, and **called, not copied**.

Three reasons the location is forced, in order of how many alternatives they close off:

1. **Only `api` holds the fact.** `load_refused_for` is port state, and core has no input for it -
   the NOT A DOOR PAST THE PORT paragraph is core declining the jurisdiction, not core forgetting it.
2. **A new `SkipReason` in core would legislate in a crate with no input for the law.** It would also
   put a path-shaped verdict into an enum whose other members are all facts about a `Document` - and
   the two paths that do have the fact would keep their existing arms anyway.
3. **A bridge-side check would make a second buffer authority.** The bridge owns the text and the
   window, not the decision about which file the port refused, and `AGENTS.md` forbids reaching
   around the seam for a missing judgement. The §5.2 tension in putting *any* rule in `api` is real,
   and it is answered by what this rule is: not business logic about notes, but a port remembering its
   own refusal. Core stays the judge of flags.

**But count first, because the obvious patch is the wrong patch.** The predicate - "is the name I am
about to write the name whose open I refused", answered by `identity_key` on both sides because
Windows paths are case-insensitive but not case-preserving - is written out twice already, once in
`Engine::save_as` and once in `Engine::save`. Adding a third literal copy in the flush would make
three identical restatements of one gate in one file, and this repo already wrote the law that governs
that, in `Engine::save`'s own step-3 comment:

> the gate is CALLED, never copied - a predicate restated here is a second rule, and the two would
> drift.

So the change is not three lines in three arms. It is **one private predicate** - the shape is
`refused_target(&self, path: &Path) -> bool` - called from three arms, with the two existing copies
folded into it. Smaller diff than the copy version, and the one the file's own comment requires.

Two more constraints on the shape, both already decided elsewhere:

- **The flush arm answers `SaveFailed` carrying `SaveError::NoTarget`**, the same verdict the two
  explicit paths state for the same refusal, so the UI renders one reason and not two. No new enum
  variant, no new event shape: an autosave write failure already arrives as `SaveFailed`, which
  `AGENTS.md` requires ("A save failure has no caller to return `Err` to - it must arrive as
  `Event::SaveFailed`"), and `SaveError` has a Display precisely because the reason must be
  renderable.
- **Placement is not free: the guard goes after the no-path arm.** `bind_scratch` is deliberately
  outside the refused-load guard and says so - "the refused-load state protects a FOREIGN file from a
  blind overwrite, and the scratch is this port's own freshly created file". A guard bolted to the top
  of `Engine::flush` would put the port's own scratch under a foreign-file rule.

## Do not touch the retry semantics while you are in that file

Out of scope, stated so the next person does not improve it: a skip the port answers must not quietly
turn into a witness resync. Measured for this note, the tree has two lanes and no exception - plus one
rule that was handed down and has since moved, recorded rather than inherited:

- The `Event::AutosaveSkipped` lane resyncs nothing. It settles (`saves_settled`, the answer name),
  words the sticky reason, and leaves `last_sent` and the edited flag exactly as they were: the buffer
  stays intended and goes out again on the next act. That is the property worth keeping.
- The `SaveFailed` lane is the one that restores the witness, and it already exists.
- **`SkipReason::NeedsPath` is not an exception any more.** The rule came to this note as "skips other
  than NeedsPath do not resync the send witness". No such branch exists in either bridge today, and the
  reason is visible in api: NeedsPath was demoted when the shared scratch bind landed - it "survives as
  the FALLBACK (its meaning is now \"we could not make a file for it\")" - and is emitted from
  `bind_scratch` only when an empty untitled buffer has no text to lose, or the scratch directory
  cannot be made. So read the rule as **all skips**. If a coder finds a NeedsPath-specific resync this
  note has missed, that is a finding to report, not a detail to follow.

Choosing `SaveFailed` over an `AutosaveSkipped` for the new refusal therefore has a consequence that
must be accepted out loud rather than discovered: the failure lane **restores** the witness - clears
`last_sent`, sets the edited flag, bumps the retry count, re-sends on the next quiet tick - because a
save failure that cannot retry silently loses the newest edit. That is the right property here, and it
is the second reason for the lane choice, after the Display. It comes with the cost the same comment
already names: a permanent failure retries every idle interval, forever, and the printed count is what
keeps the loop audible instead of mysterious. A refused-load target is a *permanent* failure until a
person does something about it, so this slice is the first to make that loop reachable by design rather
than by accident.

Do not invent a cap or a back-off here. The retry rule belongs to the port, and that argument is already
being had in `.agents/notes/proposed/2026-09-14-autosave-retry-ownership.md`; importing its conclusion
early would be a scope expansion wearing a guard.

**Verify, do not re-add.** After the change, the witness behaviour must be exactly what it is now: the
skip lane leaves `last_sent` alone and settles; the failure lane restores it. If a test needs the skip
behaviour to change in order for this guard to pass, the guard is in the wrong place.

## Evidence, and what it proves

This is a behaviour change to a **finished milestone** - M4's autosave gains a refusal state it never
had - which is exactly the case that goes through an artifact rather than a comment. This file is that
artifact, written before the code, so the ask can be shown to predate the patch.

It unproves nothing:

- **No §9 row names this path.** The roadmap's open checks are about the UI leg and the byte-identity
  corpus; none asserts that a flush writes whatever the buffer holds onto a refused name.
- **"autosave writes the buffer" is not weakened** by "autosave does not clobber a file the app refused
  to read". The second sentence is why the first is safe to advertise.
- The corpus leg is untouched: `crates/api/tests/roundtrip_port.rs` already asserts, on its
  disarmed-fixture path, that "a refused flush must write nothing", and this change can only add a
  refusal, never remove one.

**The re-earning is one `api` unit test with a mutation proof**, driven by the existing test-support
fake in `crates/api/tests/manual_save.rs` (a per-file `Harness`, not a shared support module) - the same
`Harness` that already walks a refused re-open
in `a_save_cannot_write_the_file_whose_open_was_just_refused` - sibling in shape to that Save case and
named for this path:

1. Open a small real file, hand-save it (the arming act), keep the epoch.
2. Replace the file's bytes with something this app refuses - the undecodable arm is the right fixture,
   because it is the one that cannot also refuse the write.
3. `Command::Open` the same path, wait for the refusal on its named reason, assert no generation
   moved.
4. `Command::Flush` the buffer. Assert **exactly one** `Event::SaveFailed`, carrying
   `SaveError::NoTarget` for that path and the revision the write would have anchored - not `Saved`,
   not a silent drop, and not an `AutosaveSkipped`, which would answer with a reason no Display
   reaches.
5. Assert the foreign bytes on disk are unchanged.

Removing the called predicate must fail that test - the mutation proof, which is the whole standard, and
achievable in `api` with a temp dir and no window.

**And not a `Leg::Product` needle.** Product legs earn claims about what rendered and what a window
did; this is a port law the fake sees completely on both sides of the seam. Putting it on a product leg
would spend a window claim to prove a filesystem claim and leave the unit lane unproved. `bridge-gpui`
takes no new needles at all
([ADR-0006](../../../docs/decisions/0006-gpui-is-frozen-not-deleted.md)), and this change needs none.
and this change needs none.

### Coder pre-check, before writing anything

Confirm no existing `api` or `core` test pins the **current** flush-writes-anyway behaviour. If one
does, it is a discovered contract and not a red to delete: read it, and give the note that recorded it
a sentence. My pass over `crates/api/tests` found no such pin. One neighbour is worth re-checking at
coding time rather than trusting here: `roundtrip_port.rs` asserts the refusal *reason* for foreign
fixtures (`SkipReason::ForeignFileNotArmed`) and that the bytes are untouched. Since the new predicate
fires **before** `should_flush`, any fixture in that corpus which is a name the port would refuse to
read would start answering a different event there - and that case is stopped on, not fixed by editing
the assert.

## Recommendation

**Land it, in this order and no larger.**

1. Add the private predicate in `api` that answers "is this path the name my open refused", by
   `identity_key` on both sides, and **call it from the three write arms** - `save_as` and `save`
   fold onto it, `Engine::flush` gains the call - so the gate exists once in the file.
2. The flush arm answers `SaveFailed { path, revision, reason: SaveError::NoTarget }`, placed after
   the no-path `bind_scratch` arm so the port's own scratch stays outside a foreign-file rule. No new
   variant, no new event, no core change, no new `Command`, no bridge change.
3. Add the unit test above with its mutation proof. Run the corpus leg and the manual-save lane
   unchanged; if either moves, go back to the coder pre-check instead of adjusting an assert.
4. Leave the retry semantics exactly as they are and verify that they are - skip lane: witness untouched
   and settled; failure lane: witness restored. The forever-retry consequence of choosing `SaveFailed`
   is accepted here in one sentence and argued where it belongs, in the retry-ownership note.

Why this shape rather than the alternatives, once: the fact lives in the port, so the rule lives in the
port; the law is already written twice, so it becomes a function before it becomes a third copy; the
answer already has an event, so no event is added; and the visible cost of the choice - an autosave
that will keep trying to do the right thing forever on a file it refused - is named here rather than
found later in a status line.

This note owns nothing outside itself. It does not amend
[ADR-0007](../../../docs/decisions/0007-save-is-a-conformance.md), which already says what becomes true
when this lands ("api still applies the refused-load guard") and needs no edit to cover a third path it
never excluded. It moves no §9 row in either direction before CI. It does not decide the retry
question, the status-sentence question, or the read-gate question below.

## What this note cannot answer, and a human owns before it ships

1. **Should the read gate also refuse a file it can open but cannot decode - should the pre-read policy
   own that verdict too?** The decode refusal needs bytes to exist and so can never move above the read;
   the open question is whether the two arms should *present* as one decision, because today a person
   gets one sentence for a name the policy rejected before the stat and a different sentence for a name
   rejected after reading it. Whether that is a defect or the honest shape of two different facts is a
   product call, and it is not this slice.
2. **Does an oversize foreign file deserve a plain-language status sentence?** The pieces are uneven by
   history: `SaveError` has a Display because the UI must render the reason, while `SkipReason` has
   none, which is why the bridge words that one itself. A refusal for size currently reads however
   whatever lane it landed in reads it. Making that a sentence a person can act on is worth doing and
   belongs to the status-line work, not to a guard.

Neither is required by this change, and neither should be smuggled into it.

## Neighbours in proposed/

- `2026-09-14-autosave-retry-ownership.md` owns the retry rule this note deliberately inherits.
- `2026-09-14-external-change-event-is-never-emitted.md` owns noticing that a file changed under us,
  which is the growth case this note explicitly disclaims, and which would make refused-vs-written a
  user-visible distinction.
- `2026-09-14-note-switch-must-not-lose-text.md` is the mirror-image asymmetry in the same port: a
  path that should flush and does not, against this one's path that could write and should not. Both are
  the same failure of nerve about the debounced lane, and both records read better beside each other.

## What would change this recommendation

- **A foreign format becomes a supported open target.** Then "the app refused to read this" stops being
  a rare accident and becomes a state the write path must reason about properly, which is bigger than a
  predicate.
- **The external-change work lands,** turning refused-vs-written into a distinction a user can see and
  act on - which may mean a refusal on the debounced lane needs a better answer than a retry loop.
- **Any test is found pinning today's behaviour.** That is a contract with a reason attached, and it
  earns its own note rather than a deleted assert.
- **The retry rule is decided** by the retry-ownership note in a way that makes `SaveFailed` the wrong
  lane for a permanent refusal. If a cap or a back-off arrives, this guard's answer may have to become a
  settled skip instead - a real argument this note would lose, and it should lose it loudly.
