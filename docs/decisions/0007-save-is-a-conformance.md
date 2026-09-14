---
id: 0007
title: The plain Save is a conformance, not a new affordance
status: accepted
date: 2026-09-14
deciders: [human decision 2026-09-14 round 13, department execution]
supersedes: null
superseded_by: null
relates: [ADR-0001, ADR-0003, ADR-0006, docs/features.md §4.2/§4.4, docs/roadmap.md §9,
          .agents/notes/proposed/2026-09-14-explicit-save-act.md]
---

## Context

Three records already describe a plain Save the port did not have. ADR-0001 names the arming act
as "one explicit save (`Ctrl+S` or the Save menu item)" (0001:29-30), states requirement 4 as
"Save As arms the new path" (0001:44-45), and carries as an accepted negative "press Ctrl+S
once, then it just saves forever" (0001:58). ADR-0003:29 lists the hamburger's menu as "Open /
Save / Save As / Auto-save toggle / Recent files". `docs/features.md §4.4` tables the row and
its chord. And `core` wrote the act into its own type long before anyone could send it:
`document.rs:46-48` — armed "is set only by the constructor, an explicit save, or a Save As".
The code lagged the documents. This is the record of catching up.

## Decision

**0001 is HONOURED, not reversed: `supersedes: null`, and 0001 keeps `status: accepted` with no
`superseded_by`.** A reader who spots the gap between 0001's wording and 0001's affordance:
the wording was never wrong, the affordance was missing, and a missing affordance is a build
queue, not a defect. 0007 is the date it closed; amend nothing in 0001.

`Command::Save { text, revision, epoch }`. Flush's shape, not SaveAs's, and the difference is
the reason: SaveAs rebinds, so the bridge mirrors the generation at `Wire::rebind`; Save writes
the *current* path, so without the stamp an in-flight Save lands a previous document's text at
the file the user switched to and reports Saved (command.rs:80-91, verbatim: "an in-flight edit
for A lands in the file named B, atomically, and reports Saved"). The rule lives in `core` as
`should_save_manual` (document.rs:302-319): `should_autosave` minus exactly two gates — it
bypasses `AutosaveDisabled` (that case is why Save exists; the method takes no
`autosave_enabled` argument and the absence *is* the decision) and `ForeignFileNotArmed`
(0001:29 *is* this act; `armed` is deliberately unread, since an explicit save sets it) — and
retains `ReadOnly`, `Oversize` and the Clean refusal, while `api` still applies the refused-load
guard (engine.rs:1142-1153) that `core` cannot see (document.rs:286-289).
**An unchanged buffer is refused, `AutosaveSkipped { Clean }`, never written:** the gate asks
`dirty` rather than `revision > saved_revision`, because a hand-triggered save has no queue to
be stale (document.rs:306-308). A Skip is an answer, so command.rs:78-79's "must not go silent"
is satisfied without touching the disk; a write on a clean buffer buys an mtime bump and a new
content hash — the signal the external-change work is meant to consume — and would let a stray
Ctrl+S create a file for an empty scratch note. **This overrules the drafting recommendation to
write and answer `Saved`; do not "fix" it back.** `note_revision` before `mark_saved`, without
exception, and `apply_edit` first when the command's revision is above core's counter
(document.rs:291-301): the anchor's `debug_assert!(revision <= noted)` compiles out in release,
so it is a tripwire, not the contract. Answers `Event::Saved` then `Event::Rebound` carrying the
UNCHANGED path and epoch, precedent engine.rs:1311-1336. `api` owns the no-path branch by
reusing that scratch arm; no bridge grows a dialog, because SaveAs remains the only act that
names a file. Every `Save` is answered by exactly one of `Saved` / `SaveFailed` /
`AutosaveSkipped`. A file the port refused to read is answered `SaveFailed` with `SaveError::NoTarget`, not a skip: a skip names a verdict about the document, and the document is fine — it is the target that is forbidden. On send the bridge retires the flush that Save replaces — it sends `Save`
with the pair it would have flushed and **adopts that pair as its send witness** (bump `edits`,
resync `last_sent`, clear `pending_at`), exactly as the SaveAs dialog answer does at
surface.rs:657-663 — because `should_flush` absorbing the duplicate today is an accident of the
Clean rule, not a design, and the epoch stamp exists precisely so the SENDER owns the pairing.

**Asynchronous, unchanged.** `Save` returns nothing; a failure is `Event::SaveFailed { path,
revision, reason }` and only that (AGENTS.md). A `Command::Save` answering `Result` is refused
here before it is proposed. Where a leg must wait, the wait is the bridge's or the harness's and
takes `Gateway::close()`'s shape — a typed verdict on a bounded join (gateway.rs:430-448) —
never a callback; the port does not call into the UI.

**The chord.** Ctrl+S is Save; Ctrl+Shift+S is Save As — §4.4's table, adopted unchanged —
bridged in `bridge-slint` alone. `bridge-gpui` is frozen (ADR-0006 §1) and has no Save to bind:
rebinding it leaves a dead key, forces edits to its own earned guards (menu.rs:301/:325/:341),
and shifts `menu.rs:230-295`, which roadmap.md:66/:111 and ADR-0006:24-27 cite **by line range**.
A moved citation is how evidence becomes unfalsifiable. No §9 row names a chord, so no row moves.

## Consequences

**Positive.** 0001 satisfied literally; a foreign file editable in place, which §4.5's
do-no-harm promise always implied and the app could not do; autosave-off is no longer "off, and
there is no saying"; and the by-id chord walk (probe.rs:304) keeps the needles driving the acts
they drove once the table grows.

**Negative, and accepted.** *The two bridges now explain one skip differently, on purpose:*
`plumbing.rs:153` reads "a file this app did not create: **Save once** (Ctrl+S)…" while frozen
`main.rs:1369` keeps "**Save As once** (Ctrl+S)…", each true only of its own keymap. This
replaces the written-but-unenforced parity law at `plumbing.rs:146-149` with a class rule: **a
skip sentence describing a verdict is identical on both bridges; a skip sentence naming an act
is each bridge's own, checked against that bridge's own table.** `NeedsPath` is unchanged on
both. `editor_roundtrip.rs:517` still arms through SaveAs at the seed's own path and stays
green. A clean-buffer Save visibly does nothing but explain itself, which will read to some
users as a dead key.

**Forecloses.** A synchronous save path. A second chord table. `trait Bridge`. Editing
`bridge-gpui`'s chord, menu or legend surface for consistency: here its keymap is
evidence-with-cited-lines, not product. Any write on a clean buffer. And, restating ADR-0006 §4
as the rule it is: **the protected artifact is the probe's report, not its source** — adding an
assertion that can only fail is permitted in a probe-compiled module, while adding a line to
what the instrument reports, or an assertion a roadmap row could later cite, is not.

## Reopening conditions

- The day `bridge-gpui` is deleted (all six ADR-0006 §3 re-earnings green), the pair **must**
  reconverge; until then, converging them is a bug.
- A gpui bug whose fix is a chord, legend or menu-row edit ⇒ ADR-0006's *earlier-than-scheduled*
  reopen, taken on its own terms, unproving every §9 row it costs in the same commit. Not a
  silent exception.
- Any §9 line-range citation resolving to something other than the proof it names.
- Any `Save` answered by nothing, twice, or by both `Saved` and `SaveFailed`; any `Save` that
  writes when `should_save_manual` said Clean.
- If `Save` ever names a file — a dialog, a path, a prompt — §4.4's no-modal rule returns and
  0007 reopens.

## Provenance

`.agents/notes/proposed/2026-09-14-explicit-save-act.md` (Option A) specified
`Save { text, revision }` **without `epoch`** and predicted `bridge-gpui`'s exhaustive matches
"must grow" to keep compiling. Both were wrong, and the record stands deliberately: without the
stamp, a Save queued behind an Open writes the previous document's text into the new document's
file and reports Saved; and no bridge matches on `Command` (gpui main.rs:3363/:3395/:3434 are
single-variant test matches with `other => panic!`; both bridges only *send*), so a new variant
breaks no compile. **This ADR governs the contract.** The note moves to `archived/` with
`decision: docs/decisions/0007-save-is-a-conformance.md` — not to `implemented/`, which means
*the code exists and works*; the chord decision is recorded at
`.agents/notes/proposed/2026-09-14-menu-six-rows.md:123-143`. All six quoted pointers
(0001:29-30, 0001:44-45, 0001:58, 0003:29, document.rs:46-48, command.rs:80-91) were read
directly against the tree before transcription; one v1 pointer (`document.rs:41-42`) was wrong
and is corrected here. A third gap was found afterwards, by the test-honesty review of the Save leg: nothing here said which answer a refused-load `Save` gets, and the choice lived only in `Engine::save`'s refused-load guard and in its pinning test `a_save_cannot_write_the_file_whose_open_was_just_refused`; the Decision now decides it. The by-id chord walk added four assertions to the probe target, and that
is permitted under ADR-0006 §4 read as a rule about what the instrument **reports** rather than
about test count — the five driven acts and their order are pinned unchanged, so nothing the
verdict certifies was widened; and a floor or mirror claim is a PRODUCT claim that re-earns on
`Leg::Product`, which is why the needles that landed later sit in `product.rs` for the donation
reason, not the both-bins reason.
