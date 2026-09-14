---
title: Opening another note must not lose this one
status: proposed
id: 2026-09-14-note-switch-must-not-lose-text
created: 2026-09-14
updated: 2026-09-14
relates: [whitepaper §4.2, whitepaper §4.4, whitepaper §5.2, whitepaper §5.4, whitepaper §9, whitepaper §10]
decision: null
---

## First: this is pre-existing, and it is not ADR-0007 damage

The finding arrived inside a review of
[ADR-0007](../../../docs/decisions/0007-save-is-a-conformance.md),
so the first sentence has to be the one that keeps the record straight: `Engine::open` has never
written the outgoing document, in any commit of this port. The new `Command::Save` did not create
this hole, did not widen it, and is not what makes it reachable - the hole predates the chord, the
menu row, and the conformance argument that justified them. It is worth saying in those words because
the opposite reading is available and wrong: `Event::Loaded` replacing a buffer is the oldest act in
this app, and the bytes it overwrites have been unprotected since M1.

The plan did ask for a guard. `docs/features.md` §4.4 states the single-document shape - "one window,
one file open at a time, **opening a file replaces the current one**" (`:69-71`) - and then, under
"Consequences that need designing, not assuming", requires exactly what is missing:

> **Unsaved-changes guard.** With autosave on, Open-while-dirty is rare - but autosave *can* fail
> (read-only file, no permission, file deleted, disk full, OneDrive lock). So the guard is still
> required. It must not become a modal that interrupts typing. (`:83-85`)

So this note is not proposing a new law. It is reporting that a law already in the plan has no code,
and that §4.4 understates its own case: it reasons from *autosave failing*, and the defect does not
need that. It happens when autosave works perfectly, and far more often when it is off.

## The defect, read off the source

`Engine::open` (`crates/api/src/engine.rs:905`) runs its read gate, decodes, and then assigns the
new document over the old one:

- `:1005-1010` - `self.doc = Document::open(path, file_kind(path), readonly, false)`. That is the
  switch. There is no dirty query, no flush, no refusal, and no write anywhere in `:905-1004`: the
  only save-adjacent tokens in that range are comments about the *read* gate and about the measured
  0-byte overwrite. The outgoing buffer is dropped on the floor with the struct that held it.
- `:1014-1017` - `self.epoch += 1`, announced as "THE BUMP... this buffer is a different document
  than the one the bridge was echoing".
- `:1018-1033` - `Event::Loaded` carries the new text and the new epoch, and `:1034` files the
  path in recents. Nothing between the bump and the emit asks whether the previous document had
  unsaved work.

**Autosave ON.** The only writer of ordinary edits is the debounced `Flush`, and a flush that crosses
the switch is discarded by the epoch guard: `:1253-1258` returns `Event::AutosaveSkipped { reason:
Superseded }` on `epoch != self.epoch`. The premise is stated at `:1246-1252` and it is a good
premise - one equality test against a number only this crate issues - but its consequence is that
**what dies is precisely the text still inside the debounce window**: the last ~750 ms of typing (§4.2's
own trigger list, `docs/features.md:37-39`) never reaches disk. The loss is bounded and small in that
mode, and it is still a loss with no notice.

**Autosave OFF.** Nothing writes except an explicit `Save` or `Save As`. So `Ctrl+O` destroys
every unsaved edit in the buffer, unbounded: a paragraph written over an hour, then one keystroke on a
file dialog. The toggle is a mirror the bridge sets (`surface.rs:1483-1486`) and the port holds no
obligation to have written anything.

**CLOSE IS SAFE; THE SWITCH IS NOT.** This asymmetry is the whole shape of the bug, and the safe path is
already built: after `ui.run()` returns, the shutdown does a final compare-and-flush and then waits a
bounded `SAVE_WAIT` (2.0 s, `product.rs:118`) for a terminal answer - `product.rs:741-753` lists
the order ("(2) one last compare of buffer against what was sent, (3) wait BOUNDED for the Saved that
answers it"), and `:759-778` performs it, exiting on either `saves` or `saves_settled` moving.
Quit asks the port to write. Open does not.

## Two corroborations, and one honest narrowing

**1. The dirty witness is deleted by the arrival, whatever the bridge sent.** `note_adoption`
(`surface.rs:705-708`) clears `edited_flag` and `pending_at` on **every** `Loaded` or
`Rebound` - it is one function for both events (`:680-687`), because "an adoption is not a user
edit". So the moment the new document arrives, the bridge's own knowledge that it was dirty is gone.
Any fix that waits for a `Loaded` and *then* decides must capture dirty-ness before this runs, i.e.
before the switch, which is another way of stating where the fix belongs.

**2. Save As already reads the buffer at the act, and shipped before 0007.** The Save As dialog reply
(`surface.rs:657-663`) reads `lf(&ui.get_buffer())` and re-arms `p.last_sent` in the same pair of
statements, with the reason at `:651-656`: a snapshot taken before the dialog "pays for it: anything
typed while the dialog is up is saved under a name for text that is already gone". That handler is
precedent for the shape B-prime needs - *read the buffer when the act completes, not when it was asked*
- and it is older than the ADR under review.

**3. The narrowing, said rather than skipped.** A review can plausibly describe a race: the bridge
sends `Save`, then `Open`, and something interleaves. It cannot happen that way, because the command
channel is one `mpsc::channel()` pair, FIFO and unbounded (`crates/api/src/lib.rs:211-219`) - an
`Open` queued *behind* a `Save` cannot overtake it. The reachable shape is the reverse:
**`Save`-after-switch, where the Save is stale the instant it is sent** - `engine.rs:1483-1490`
guards `save` with the same equality test as `flush` and discards it as `Superseded`, and
`command.rs:91-99` documents that this is why `Save` carries Flush's shape: "a Save whose
generation the engine has already replaced is DISCARDED... exactly as a stale Flush is". Narrower than
the review described. Same bug, and the narrowing matters: it means the fix is a *sequence*, and
sequences are what one process owns.

## Options

### A. The bridge restores dirty intent on Superseded - **rejected; it is not a fix**

Say it plainly so nobody re-proposes it: re-arming `last_sent` and the dirty witness with document A's
text while the editor shows document B is a **corruption generator**. It defeats the premise of the
guard at `engine.rs:1246-1252` - the mismatch "can only mean the text was buffered under a document
this engine has since replaced" - and it would hand the next flush A's bytes stamped with B's epoch,
which is the exact stale-write the epoch was invented to kill. Three tests already pin that discard as
intended behaviour, and none of them is a mistake to be edited away:
`manual_save.rs:513` (`a_stale_save_is_discarded_as_superseded_and_writes_nothing`),
`session.rs:1425` (`a_stale_flush_never_lands_in_the_file_that_replaced_its_document`),
`scratch_restart.rs:625` (`a_flush_stamped_before_the_restore_open_is_discarded_not_written`).

### B. The engine flushes before it switches - **right law, wrong home**

`api` cannot flush what it cannot see: **the port holds no buffer.** `command.rs:52-58` is explicit
that `Save As` carries the text because "the bridge owns the buffer... and it is why the engine holds
no text of its own". To make the engine flush on switch it would need either (a) a handshake event plus
an ordering protocol across the seam - which is the port calling into the UI by another name, and
AGENTS.md's "add the event instead" does not license an event whose purpose is to wait on a keystroke
buffer - or (b) caching text in `api`, which re-creates the stale-write hazard the epoch stamp exists
to kill and moves a buffer across the boundary §5.2 and §5.4 exist to keep clean. A rule about the
*outgoing* buffer is a rule about the only place buffers live.

### C. Ask the human - a confirm dialog on Open-while-dirty - **rejected on the plan's own words**

This is §4.4's nominal shape, and §4.4 rejects it in the same breath: the guard "must not become a modal
that interrupts typing" (`docs/features.md:85`). It also mis-assigns the cost: a dialog on every
switch makes the common case (autosave on, nothing dirty) pay for the rare one, and it teaches a person
to press Enter through a question that sometimes means it. `Save As` is the only act in this app that
asks a human a question with a modal, and that asymmetry is a design, not an oversight.

### B-prime. The bridge sequences it - **CHOSEN**

Every user Open ask already aims at one door; make it one *function*. Today there are two sends plus
the startup one: the dialog reply (`surface.rs:644-649`, reached from `on_open_asked` at
`:1451-1453`, whose comment at `:1444-1446` already claims "the row, the Ctrl+O chord and any future
native menu land HERE... the same door the recents rows use"), and the recent row
(`surface.rs:1497-1502`). Startup restore (`:1414-1425`) is the third send and stays alone: it has
no previous buffer to lose, which is the same fact `note_adoption` treats as the baseline
(`:709-717`). The behaviour: **if the buffer is dirty, send `Save` first, hold the `Open` until
its terminal answer arrives, then send `Open`.**

The constraints it must meet, and does: no new `Command` variant; no text in `core` or `api`; the
port still never calls the UI; `bridge-gpui` untouched, so no ADR-0006 or roadmap line-range citation
moves. It is also the least novel move available, which is the point - the close path already performs
this exact wait, and it already counts **the right family of answers**: `saves_settled`
(`surface.rs:753-763`) counts `Saved`, `SaveFailed` *and* `AutosaveSkipped` (incremented at
`:1247`, `:1295`, `:1333`) rather than `Saved` alone, precisely because "with autosave OFF the
port does not go silent, it answers `AutosaveSkipped`... after which no `Saved` ever comes". **Reuse
that witness. Do not invent a second one**, and keep it a count, not a bool, so the waiter answers to
its own flush and not to a save cycle left over from earlier in the run.

## The failure branch, and the residue we accept

If the write is refused - `ReadOnly`, `Oversize`, `SaveFailed` - **the bridge still switches**,
leaves the dirty witness intact, and puts the reason in the status line. That is the channel this app
already uses for skips: `skip_words` (`plumbing.rs:149-163`) has a sentence for every one of them,
including the one this defect produces today - `Superseded` renders as "the edit belonged to a note
that has since been replaced, so it was discarded" (`:156-158`) - and the strings are shared with
`bridge-gpui` by policy, not coincidence (`:146-148`). No dialog: §4.4 forbids the modal, and
`Save As` stays the only act that asks a human a question.

The residue, named so nobody has to discover it later: a document whose write is refused is still
switched away from, so its unsaved text survives only until the next adoption clears the witness
(`surface.rs:707-708`) - which is to say it is gone on the next switch. And a save that fails *inside*
the wait window costs the person up to `SAVE_WAIT` before the switch happens at all. Both are smaller
than what is fixed; neither is zero.

## bridge-gpui carries the same defect, unfixed by the freeze

It must be written here rather than inferred from who owns which file, because a Slint-only fix reads
like an accident of custody. `crates/bridge-gpui/src/main.rs` sends `Command::Open` with no pre-write
at `:1732` (startup), `:2309` (asks), and a file dropped on its window "arrives as Command::Open"
(`:1939`, `:2017`). The loss lives in the shared `open`, so both bridges lose both ways. The
divergence ADR-0007 accepted was about **copy** - two bridges wording the same skip - and it stayed
tolerable because `skip_words` is deliberately identical. **This divergence is about behaviour**,
which is the kind that costs a person their text, and it should be recorded as the concrete price of the
freeze rather than as a boundary technicality: ADR-0006 keeps `bridge-gpui` compiling and cited, and
that is precisely why a product fix in one adapter is not a product fix. If the freeze is going to cost
something, this is what it costs - §10 and ADR-0006's delete gate should carry that sentence, and this
note cannot write either.

## Scope: what the M4 and §9 evidence never tested

`docs/roadmap.md:178-180` defines M4 as "Debounce, periodic flush, **blur/close/quit flush**,
external-change detection, and the byte-identical round-trip test suite (§4.5)". §4.2's trigger list
(`docs/features.md:37-39`) is the same set: idle debounce, periodic, "immediately on focus loss,
window close, and app quit". **Switching documents is in neither list.** Every green autosave row
therefore proves that a buffer reaches disk across a *trigger the plan enumerated*, and none of them
tests the one path where the port itself deletes the buffer's identity (`engine.rs:1017`'s bump plus
`surface.rs:707-708`). Check 6 (`roadmap.md:73-74`, "the menu toggle driving them is not [tested]")
and check 7 (`:75-78`, "the idle/blur flush through a live window is still owed") are the same gap in
different words: the owed evidence is about acts, and this act has never been on the list.

This is a **scope paragraph, not a downgrade.** Per AGENTS.md's honesty gate - no M2 check's status
moves on either bridge's account pre-CI, in either direction - no §9 row changes here, and no row's
meaning quietly narrows either: the rows are true about what they measured. What changes is that a
second act needs measuring, and the sentence to write beside it when it is done is "Open-while-dirty
across a real window", which nothing in the tree says today.

## Known unknowns, stated as unknowns

1. **Whether `Save` is always the right pre-write verb.** `Save` writes the current path, and a
   draft with no path is bound to its scratch file (`command.rs:100-101`), so the pre-write should be
   the command the buffer's own identity implies - the same decision the close path already makes when
   it flushes. Settle it by reading `should_save_manual`, not by guessing at a dialog.
2. **Which dirty witness the guard trusts.** The bridge has `edited_flag`
   (`surface.rs:765-768`) and `dirty` (`:855`); core has `Document::should_save_manual`. The
   guard must name one. The honest candidate is the one an adoption has not already cleared - which is
   also why step 2 of the Recommendation captures before the save rather than after the answer.
3. **What a second act inside the wait window does.** A person who hits `Ctrl+O` twice while the first
   is waiting produces two saves and one switch, or a queue this note has not modelled. That needs a
   needle, not an argument.
4. **"Rare" is unmeasured.** §4.4 asserts Open-while-dirty is rare with autosave on. Nothing has
   counted it, and the debounce window alone makes every fast switch a small loss - which may be the
   common case rather than the rare one.

## Recommendation

**B-prime.** Concretely, in this order:

1. Name the funnel: one function that both user-facing Open doors call (`surface.rs:644-649` for the
   dialog reply, `:1497-1502` for the recent row), leaving `restore_from_session` (`:1414-1425`)
   untouched and saying why in the comment.
2. Capture the dirty verdict **before** the save goes out - `note_adoption` erases it on arrival
   (`:707-708`).
3. Send `Save`, then hold the `Open` until `saves_settled` moves (`:753-763`, incremented at
   `:1247`, `:1295`, `:1333`), reusing `SAVE_WAIT` (`product.rs:118`) and the
   `flush_verdict` report shape (`product.rs:779-787`) rather than writing a second waiter.
4. On any terminal answer, send the `Open`. On a refusal, keep the dirty witness and put
   `skip_words`' reason in the status line (`plumbing.rs:149-163`).
5. Prove it where the proof can be read: a bridge-side test that a dirty buffer's bytes are on disk
   *before* `Loaded` arrives for the next file, and a trace line at the switch - product-side, per
   ADR-0006. **No new probe arm**, and none of the three `Superseded` needles edited.

**Landing order, because this note is the authority for the workstream:** the B-prime slice does not
launch until this file is committed, and the **chord slice lands first** - B-prime sends the command the
chord makes reachable, so a sequence whose verb does not exist yet is untestable. Nothing here was
committed by this note, and no §9 row moved.

## Neighbours in `proposed/`, named so the doors are not plumbed twice

`rejected/` holds nothing on this question - the guard has never been decided against - but four open
notes touch the same doors, and one of them is this note's dependency:

- **`2026-09-14-explicit-save-act`** ("The explicit save ADR-0001 waits for does not exist", which
  recommends building `Command::Save`) **is the chord slice** the landing order above waits for. Its
  command is the pre-write; this note sends it, it does not create it.
- **`2026-09-12-arm-on-explicit-save-path`** is the same family one level down - a save the plan
  assumed and the port could not express. Read it before choosing the pre-write verb (unknown 1).
- **`2026-09-13-new-document-command`** refuses to "let any bridge fake the act locally", and that
  refusal is *not* an objection to B-prime: creating a document is a decision the port owns, while
  sequencing two commands the port already accepts is the bridge's own business - it owns the buffer,
  which is the entire reason options A and B fail. If `Command::New` ever lands, it is a third switch
  door and inherits this guard.
- **`2026-09-14-menu-six-rows`** decides what the hamburger's Open row is, and
  **`2026-09-14-autosave-retry-ownership`** wants a persistent, visible failed state in the UI. Both
  are the same real estate as steps 1 and 4 of the Recommendation: the funnel should be the row's
  target, and the switch-time reason should reuse that channel, not open a second.

## What would change this recommendation

- A decision to make the switch a port-level contract (a `Command::Open` that carries the outgoing
  text, or an event that requests it) - that is option B, and it would be an architecture change to
  §5.2 and §5.4 worth an ADR, not an edit to this note.
- Evidence that holding the `Open` is user-visible as a stall: if `SAVE_WAIT` shows up as a dead
  window on ordinary hardware, the sequence must become non-blocking (park the switch as a pending ask),
  and that is a different, larger design.
- `New document` landing before this: it is a third switch door with no incoming file at all, and it
  inherits the same guard or becomes the worst version of this bug.
