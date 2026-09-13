---
title: The undo quarantine, and why a startup open must not arm it
status: implemented
id: 2026-09-13-undo-quarantine
created: 2026-09-13
updated: 2026-09-13
relates: [§4.4, §5.5]
decision: null
---

Read this next to `crates/bridge-slint/src/main.rs` — the rule, its witness and its arming all
live in that one file, and the code comments are the authority on what the code claims. This note
is the provenance: what S10b shipped, and the bug S10c had to fix in it.

## Question

A single-document app can open file A, type, then open file B. Slint's `TextInput` keeps **one**
native undo stack per editor instance, and the spike's editor is not recreated per document — so
after a switch, `Ctrl+Z` replays A's edits into B's buffer. The hazard is silent and it is
data-destroying: the user undoes the wrong file's work into the file they are looking at. So:
what does the app do about undo across a document switch, and who gets to keep native undo?

## Options

### A. Recreate the editor per document

A fresh `TextInput` per open, so the stack is always the right file's. Costs the caret, the
selection, the scroll position, and the frame the user is looking at; and it collides with the
conditional-recreation parity rule the generation counter exists to hold (`main.rs:2191-2197`).

### B. Keep native undo and hope the stack is small

Rejected on the facts: the replay is not a rare edge, it is the ordinary case of opening a second
file after typing in the first.

### C. Quarantine it — swallow replay keys once the session has switched documents

`Ctrl+Z`, `Ctrl+Shift+Z` and `Ctrl+Y` are eaten after the first real switch, for the rest of
the session. Undo stays native forever on the first document.

## Recommendation

**C**, shipped as S10b and corrected by S10c — and the correction is the reason this note exists
rather than a line in the spike log. The predicate is a **state rule, not a chord rule**: what is
armed is the session's document generation, not the key. The same keystroke is native undo on the
first document and a swallowed hazard on the second, "which is precisely the shape a table cannot
express" (`main.rs:2204-2213`).

## What is built

- **The rule, as an oracle the tests can hold.** `undo_quarantined(generation, text, ctrl, shift,
  alt) = generation > 0 && ctrl && !alt && (text eq "z" or "y")`
  (`crates/bridge-slint/src/main.rs:2215-2221`). It is `#[cfg(test)]` on purpose: the real
  implementation is the capture-handler branch in the markup, Rust cannot call into it and markup
  cannot call into here, so `the_quarantine_is_a_state_rule_and_the_markup_agrees`
  (`:2986`, assertions at `:3028-3041`) greps the condition out of the mounted Slint and holds
  the pair together. Wiring the Rust fn into the binary would mean inventing a call site that lies
  (`:2211-2213`).
- **It is not a command, and the chord table proves it.** `SHORTCUTS` stays at fourteen rows and
  no undo or redo row was invented (`:3030-3041`): the port has no `Command` for undo and never
  will, so the legend of commands may not claim one.
- **The arming is reported once, visibly.** When the generation first leaves zero, the run prints
  "undo: quarantine ARMED at generation=N — `Ctrl+Z`, `Ctrl+Shift+Z` and `Ctrl+Y` are
  swallowed for the rest of the session (stale stack, cross-file replay is the hazard)"
  (`main.rs:884-899`, latch field `quarantine_reported` at `:2313-2315`). The per-keystroke
  voice is `root.undo-swallowed();` → `report("undo: quarantined …")` (`:1487-1492`).
- **S10c: the adoption policy, so a startup open is not a switch.** `note_adoption` clears the
  dirty witness and steps the generation **only when the loop has already ticked**
  (`p.invocations > 0`) — `crates/bridge-slint/src/main.rs:2171-2189`. That guard is the whole
  fix.

## The S10c story, kept embarrassing

S10b shipped a correct predicate fed by a wrong number. In the S10b run, the app's own startup
sequence — the rebind that answers the reopened-file open, then the `Loaded` that brings the
text back — arrived **before the event loop had ever ticked**, printing
`rebind: epoch=1 gen=1` and `load: epoch=2 gen=2` before
`hwnd = 0x… APPEARED at t+84ms, after the loop spun` (`main.rs:2171-2180`). So the quarantine
armed at generation=2 **with zero document switches**, and the very first `Ctrl+Z` on the very
first file of an ordinary launch was swallowed. The feature killed undo on the document nobody had
switched away from — "the exact opposite of what the capture branch promises".

S10c's answer is that the loop's own tick count is the honest witness for "a person caused this":
an adoption arriving before the first tick is the app coming back to the note it was closed with,
not a switch (`:2172-2180`). The test is deliberately aimed at the policy and not the predicate
(`:3044-3059`): `the_startup_adoption_is_not_a_switch` drives `note_adoption` and asserts it
returns 0 twice, because `undo_quarantined(gen, ..)` "was correct all along and could not fail,
but the gen it was handed came from a policy that stepped on the app's own startup open. A test
that takes the number as an argument cannot catch the code that produces it."

## What Slint 1.17 cannot do, and what that leaves provable

**1.17 cannot deliver a key event.** There is no key-injection API in the toolkit's test surface
(established in S6), so the headless harness cannot press `Ctrl+Z` and watch the swallow happen.
What CAN be proven headlessly, and is, is the pair the behaviour is made of: the predicate as a
table of facts (`z_and_y_are_swallowed_only_after_the_first_switch`, `:2928-2980` — first
document keeps undo, every replay spelling is swallowed after a switch, and non-undo chords are
untouched), and the state that arms it. The per-keystroke needle "can only come from a real hand"
(`:886-890`), which makes the armed-report the strongest witness a run can show: the switch
happened, the branch is armed, the cost is being paid from here on. A manual keypress is the one
remaining proof, and it is not automated.

## What it costs

**The cost is native undo, for the whole rest of the session, after a single document switch** —
including on the second file, which never had anything to undo from the first, and including undo
of edits made *after* the switch. The predicate cannot tell those apart: generation is a session
counter, not a stack, and the only per-document cure is option A's editor recreation with
everything it throws away. That is a real loss on a notepad where switching files is routine, and
it is why the arming is reported out loud rather than swallowed silently.

## Consequences

- Undo behaviour differs by history, not by file: identical actions on identical text, and only one
  of them can be undone. Anyone testing "undo does not work" should be asked how many files they
  opened first.
- The quarantine is not in the chord table, so nothing in the menu or the shortcuts legend
  advertises it; the visible record is the armed report and the per-key line.
- A per-document undo stack (option A, or a stack we own in `core`) is the only path to lifting
  it, and it is unbuilt.
- The generation counter is now load-bearing for a UX rule, not only for geometry parity — changing
  when it steps is a user-visible change.

## Reopening conditions

- A per-document undo stack owned above the toolkit, so the switch does not have to be a hazard.
- Slint (or a test harness) gaining real key injection: the swallow then becomes provable
  end-to-end rather than as a predicate plus a report.
- Any report of a user losing work to the *absence* of undo after a switch, which is the failure
  this design accepts on purpose and should re-open the trade-off against option A.
- Evidence that startup adoptions can arrive after the first tick (a slower first frame, a
  recreate, a relaunch path), which would let a session open already quarantined.
