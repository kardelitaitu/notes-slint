---
title: The explicit save ADR-0001 waits for does not exist
status: proposed
id: 2026-09-14-explicit-save-act
created: 2026-09-14
updated: 2026-09-14
relates: [ADR-0001, ADR-0006, §4.4, §4.5]
decision: null
---

## Question

Should `crates/api` grow a `Command::Save` — the act ADR-0001 names, the spec's menu table
lists, `core`'s own doc comments describe, and the port does not have?

This note is not a feature request. It is the observation that a **settled** decision
(ADR-0001, accepted) has an affordance that was never built, and that the hamburger menu is
where the absence shows up.

## What the code does today

- ADR-0001 decides: "autosave stays disarmed until the user performs one explicit save
  (`Ctrl+S` or the Save menu item)" (`docs/decisions/0001-…:29`), and its consequences section
  expects the user experience to be "press Ctrl+S once, then it just saves" (`:58`).
- `core` agrees the act exists: `armed` "is set only by the constructor, an explicit save, or a
  Save As" (`crates/core/src/document.rs:42`). The only line that sets it is `mark_saved`
  (`:153`), and the only public caller of `mark_saved` is `save_as` (`:174-177`).
- The port has no such command. `crates/api/src/command.rs` offers `Open`, `SaveAs`, `Flush`,
  `SetAutosave`, `SetPinned`, `SetCornerRounding`, `ClearRecents`, `Shutdown`, `RegisterWindow`,
  `GeometryChanged`, `UnregisterWindow`. `bridge-gpui` says so in its own shipped copy
  (`crates/bridge-gpui/src/main.rs:1361-1362`, and `:1002`: "there is no manual Save command").
- `Flush` is not a Save. `should_flush` is a staleness gate on top of `should_autosave`
  (`document.rs:228-233`), so it inherits both refusals: `!autosave_enabled` → `AutosaveDisabled`
  (`:183-185`), `!armed` → `ForeignFileNotArmed` (`:213-215`).

Three consequences follow, and none of them is hypothetical:

1. **A foreign file can never be written to its own path.** `FileKind::Foreign` opens disarmed
   (`.md`, `.txt`, anything this app did not create — `document.rs:26-38`), and the only door to
   `armed` is a `Save As`, which *rebinds* the document to a new path. Editing `todo.md` in place
   is not a thing the app can do.
2. **With Auto-save off, nothing can be written at all.** The bridge's tick does not even send
   (`surface.rs`'s `!p.autosave` guard), and the engine would refuse it as `AutosaveDisabled`.
   The toggle is therefore not "off until I say save" — it is "off, and there is no saying".
3. **The fastest route into this is a drag.** "A drop is an open" (ADR-0006 §6), so: drag
   `todo.md` onto the window, type, and discover there is no way to keep it except Save As to a
   second file. For the app whose §4.5 promise is byte-fidelity with foreign files, that is the
   opposite of the promise.

## Why the menu is where this lands

`docs/features.md` §4.4's table has five rows, and the built popup has four of them:

| spec | built | note |
|---|---|---|
| Open… `Ctrl+O` | Open `Ctrl+O` | ✓ |
| **Save** `Ctrl+S` | — | **absent, and `Ctrl+S` is spent on Save As** |
| Save As… `Ctrl+Shift+S` | Save As `Ctrl+S` | the chord is the first bridge's, ported row for row (`surface.rs:121`) |
| Auto-save (visible check) | Auto-save `Ctrl+T` | ✓ |
| Recent files | the recents stack in the body, `Alt+1..9` | the doc's list is now the worse of the two |

The absence is already *recorded* rather than invented in the markup
(`ui/chrome.slint:601`: "no New and no plain Save"), and the AGENTS rule about it is
asymmetric on purpose: a `Command` variant is a design change that deserves a paragraph.
This file is that paragraph.

**ADR-0001's requirement 1 shipped today.** "Silence is forbidden. While disarmed, the UI must
show why nothing is being saved" (`:37`) is now the popup's footer
(`plumbing::skip_words`, drawn by `chrome.slint`'s footer block). That is the half this
repository could build without new vocabulary. The requirement's *other* half — that a user
facing a disarmed file has an act to perform on the spot — still points at a row that does not
exist. The footer's sentence is the compromise the first bridge already chose, and it is honest:

> a file this app did not create: Save As once (Ctrl+S) and it keeps saving

Its test reads `Ctrl+S` out of the chord table rather than hard-coding it, so if the binding
moves, the copy fails loudly (`plumbing.rs`'s tests). **The sentence is a decision-shape
assertion: build `Save` and this is where the change is noticed.**

## Options

**A — Build the act ADR-0001 already assumed.** `Command::Save { text, revision }` meaning
"write the buffer to its own path now, arm the document, and do it whether or not auto-save is
on". Costs, honestly: a 12th variant moves the count tests in both bridges; `bridge-gpui`'s
exhaustive `describe` and `Command` matches must grow, which the freeze *demands* rather than
forbids (precedent: `CornerRoundingFailed` today); `core` needs a save gate that is not
autosave's, and that is a behaviour change inside the do-no-harm crate, so it owes its own
tests; and the chord question reopens — `Save` wants `Ctrl+S` and `Save As` wants
`Ctrl+Shift+S` per the spec, which is a **binding change on a table the live needles walk**.
Benefit: §4.4's muscle memory, ADR-0001 satisfied literally, foreign files savable in place.

**B — Decide that foreign files are not edited in place.** Then fix `features.md` §4.4 (drop the
`Save` row, keep `Ctrl+Shift+S` off the table), and write a short ADR superseding ADR-0001's
affordance — because an append-only record whose stated user act the app refuses to offer is a
trap for the next reader, not a decision. Cost: the app says of the commonest file kind you can
hand it, "you may read and edit this, and then re-choose its name".

**C — Nothing.** Not on the table: it leaves the arming rule half-built — the silence is
explained, but the way out is a rename.

## Recommendation

Build `Command::Save` — it is the option this repository has already agreed to. ADR-0001 names the
act and its chord, `core`'s arming rule is written as if it existed, §4.4 lists the row, and both
bridges' shipped copy currently works around the absence by pointing at Save As instead. The other
two options are documentation edits that turn the absence into a policy; if that is the policy, say
so in a new ADR rather than letting the code discover it by omission.

The arming rule needs no new decision — ADR-0001 already grants it. Two things are genuinely open,
and both are expensive to change afterwards, so they belong in prose before any Rust:

1. **`Save` bypasses the autosave toggle, by definition.** If it consults `should_autosave` it is
   `Flush` with a new name, and "auto-save is off" is precisely the case it exists to serve. So
   `core` grows a second gate — in the crate whose entire job is do-no-harm, which is why it owes
   its own tests rather than a reuse of the autosave ones.
2. **What `Save As` is bound to once `Ctrl+S` is taken.** The spec says `Ctrl+Shift+S`; the shipped
   table says `Ctrl+S`, ported row for row from the frozen bridge, and the live needles walk that
   table — so a rebind is not a label change, it changes what the evidence means. Rebind in
   `bridge-slint` alone and the two bridges' tables diverge for the first time; rebind in both and
   `ADR-0006`'s freeze is being edited for convenience, which is the one thing it forbids.

The footer's `ForeignFileNotArmed` sentence is where any of this lands first: it promises "Save As
once (Ctrl+S)", reads the chord out of the table in its own test, and fails loudly the day the
binding moves. That is deliberate — a sentence that orders a user to press a key is part of the
chord table's contract surface.

## What would settle it

- One reproduction: drag a `.md` in, type, try to keep it. The footer will name the dead end.
- The M2 checklist row that cites `Save` (`docs/roadmap.md`) — its status may not move on
  either bridge's account pre-CI (`git remote -v` is empty), which is its own reason to decide
  the question in prose first.
- `crates/api/src/engine.rs:1116` and `:1122` hold the same doc comment twice, from `2d71da8e`.
  One copy describes the pre-fix behaviour ("write the snapshot") and one the fix ("write THE
  TEXT IT WAS GIVEN"). A reader who picks the wrong one re-introduces a real bug, so that
  deletion belongs with whichever option is chosen here.
