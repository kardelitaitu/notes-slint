---
title: Does "new note" deserve a Command::New?
status: proposed
id: 2026-09-13-new-document-command
created: 2026-09-13
updated: 2026-09-13
relates: [§3, §4.1, §4.4, §5.5]
decision: null
---

## Question

A user who wants a blank note today has one move: quit and relaunch. Does that act deserve a
place in the port — `Command::New` — or is its absence the correct shape of a single-document
app?

**How this surfaced:** the Slint bridge found it while porting the chord table, not while
designing a feature. `menu.rs` binds **Ctrl+T to `ToggleAutosave`**
(`crates/bridge-gpui/src/menu.rs:119`, `:140`), so the reflex Ctrl+N → "new" has nowhere to go and
the shipped table has **no new-document row at all** — Ctrl+O, Ctrl+S, Ctrl+T, Ctrl+Shift+R,
Alt+1–0 (`menu.rs:117-121`). The absence is now asserted in code:
`ctrl_t_is_toggle_autosave_and_there_is_no_new_note_chord`
(`crates/bridge-slint/src/main.rs:2012-2022`), whose failure message says the gap "needs a port
Command and an ADR conversation", and the markup agrees (`crates/bridge-slint/ui/main.slint:154`:
"menu.rs binds Ctrl+T to ToggleAutosave, NOT to a new note"). The gpui bridge never asked, because
`build_menus` has no such row to wire (`menu.rs:188`, action enum at `:39`/`:51`). So the only
bridge that went looking for the act is the one that had to re-spell the table.

## The facts, as the code holds them

- **The port has no new-document act.** `Command` is exactly: `Open`, `SaveAs`, `Flush`,
  `SetAutosave`, `SetPinned`, `ClearRecents`, `Shutdown`, `RegisterWindow`, `GeometryChanged`,
  `UnregisterWindow` (`crates/api/src/command.rs:43-145`). The only command that changes *which
  document the engine holds* is `Open` (`:47`); `SaveAs` rebinds it (`:60-64`).
- **Arming is per-document and cannot be inherited.** `FileKind::Notes` arms autosave on open;
  anything else "is foreign and opens DISARMED", and armed "is set only by the constructor, an
  explicit save, or a Save As" (`crates/core/src/document.rs:26-42`, ADR-0001). Requirement 4
  rides the rebind event: "the path changed, the arming changed with it"
  (`crates/api/src/event.rs:359`, `Rebound` at `:369-378`).
- **The untitled note is already a real file, and it is a SHARED, DETERMINISTIC one.** D69 binds
  an unnamed buffer to `<StateDir>/notes/untitled.notes` — core owns where (`scratch_note_path`,
  `crates/core/src/paths.rs:82`), the port owns the write through the same machinery Save As uses
  (`crates/api/src/engine.rs:1101-1111`), the scratch deliberately does **not** join the recents
  (`:1079-1081`), and an *empty* untitled buffer is skipped as `NeedsPath` rather than
  churning a zero-byte file (`:1091-1100`). Two untitled buffers reuse the same path, no
  `untitled-2.notes` (`crates/api/tests/session.rs:1352`), and three launches never duplicate the
  text (`crates/api/tests/scratch_restart.rs:119`).
- **The scratch bind deliberately does NOT bump the document epoch** — a load-bearing
  non-action, because the bridge mirrors the engine's generation only at the send of an `Open` or
  a `SaveAs`, so a bump there would silently discard the autosave of "the one document this product
  always has" (`engine.rs:1113-1125`, pinned by `the_scratch_bind_does_not_bump_the_epoch`).
  The epoch guard's whole job is that "a Flush whose echoed epoch names a generation the engine has
  already replaced is DISCARDED" (`command.rs:80-91`) — "otherwise an in-flight edit for A lands in
  the file named B, atomically, and reports Saved."
- **§4.4's menu has no New row** (`docs/features.md:73-79`: Open… / Save / Save As… / Auto-save /
  Recent files) — yet §4.4's title rule already presumes the state exists: "`Untitled` for a new
  unsaved document" (`docs/features.md:98-99"). The plan describes a *startup* state, not an *act*.

## Options

### A. There is no new-note act, on purpose

The app is one window, one file, opening replaces the current one, and recents are the way back
to anything (§4.4, §3 — tabs and a workspace are stated non-goals). On this reading the blank note
is not something you create, it is what the app *is* when nothing else owns it: launch with no
restored path and you are typing into `untitled.notes` within a frame. "New" is then
**relaunch**, which is honest, costs no `Command`, no event, and no settings key, and never
contradicts the promise in the README's first line ("never asks you to save").

Cost, stated plainly: the muscle memory is real and the chord table cannot satisfy it — Ctrl+T is
taken by a different act and re-spacing the table is a user-visible change. And relaunch is not
"free": it is a 200-400 ms window teardown for one keystroke's worth of work.

### B. `Command::New` — reset to untitled, with its own event

The act the user means, expressed once in the port: the engine abandons the current document
identity, binds the scratch, and answers with a rebind-shaped `Event` (a new variant or
`Rebound`) so the title, the dirty slot and the session path all move together. Arming must be
answered explicitly: a fresh scratch is a `.notes` file, so the file-kind rule says **armed**
(`document.rs:32-34`) — "reset to untitled **and Disarmed**" is therefore a *deviation* from
ADR-0001, not an application of it, and it needs its own sentence somewhere. The epoch question
resolves the other way: a New **must** bump the generation, because unlike the scratch bind at
`engine.rs:1113` the bridge's buffer genuinely changes identity, and an in-flight flush of the old
text must be discarded rather than written to the new note.

The hard part is not the port, it is the collision: **a New destroys the previous untitled note.**
One shared scratch + "the app never loses what you typed" = a New that either (i) wipes text the
user has no way to get back, (ii) saves it forward somewhere, which invents a naming scheme the two
scratch tests forbid, or (iii) opens a dialog, which is the save-prompt this product exists to
avoid. All three are product changes, not chords. Per AGENTS.md a new `Command` variant is a
design change and deserves this paragraph, which is where it is.

### C. Bridge-local buffer clear (a rejected preview)

The bridge empties its own text editor and shows `Untitled`, sending nothing. **This one is not
available at any price**, because the buffer and the document identity live on opposite sides of
the seam by design (§5.5: the bridge owns the editor, the port owns the file):

- The engine still holds the *previous* document — say a foreign `.md` the user armed earlier with
  a Save As. The next debounced `Flush` carries the new note's text (or its first keystrokes) with
  the **old epoch**, which passes the guard, and autosave writes a stranger's content into that
  `.md`. That is precisely the failure `command.rs:88-91` names, and it is silent because it
  reports `Saved`.
- ADR-0001's discipline is bypassed by construction: arming is per-document and moves only with an
  open/save/rebind (`document.rs:40-42`), so a bridge-side "new document" inherits the old
  document's armed state — the exact thing requirement 2 forbids.
- The port's view of the world (session path, recents, the title's filename rule, the dirty dot)
  disagrees with what the window shows, and §4.4's title rule becomes a lie with a real file name on
  it.
- Nothing on the bridge side can undo (1): the flush is already in the debounce when the clear
  happens.

## Recommendation

**A for now — do not add `Command::New`, and do not let any bridge fake the act locally (C).**
Not because the reflex is wrong, but because the three unresolved facts above are product
decisions wearing a chord's clothes: what a New costs the previous scratch, whether a fresh
scratch is armed or disarmed against ADR-0001's own file-kind rule, and what the epoch does.
Shipping the chord before those answers exist buys a keybinding and spends the app's one promise.
Keep `ctrl_t_is_toggle_autosave_and_there_is_no_new_note_chord` as the standing guard: it is what
stops the gap being closed by a bridge that "just clears the buffer".

If B is taken, it enters with all three answered in the same commit: (1) a scratch-collision rule
that never loses text silently; (2) an explicit arming verdict, stated as a deviation or as an
application of ADR-0001; (3) an epoch bump plus the mirrored rebind, so an in-flight flush of the
old buffer is discarded rather than written into the new note. That is the minimum shape, and it is
an ADR conversation, not a menu row.

## Consequences

Choosing A keeps `Command` a vocabulary of *intents the port can honour alone* and keeps the
scratch's single-path rule intact. It also leaves a known UX hole, deliberately, in code that
asserts the hole — which is the point of this note. It forecloses nothing: B's shape is written
down, and none of A's reasoning survives intact once the scratch gets a per-note name, so a future
note supersedes this one cleanly rather than amending it.

## Reopening conditions

- **A per-note scratch naming scheme exists** (the collision in B becomes answerable) → reopen; this
  is the main unlock, and `a_second_untitled_note_reuses_the_same_scratch_path` is the test that
  would have to change meaning first.
- **The app gains a document-switcher, tabs, or multi-window** (§10's "one window in v1" relaxes) →
  New is then one of a family of document acts and should be designed with them, not before.
- **A user-visible report of the missing act** — not "would be nice", an observed "I lost my note" /
  "how do I start over" — → reopen with that report as the evidence.
- **Ctrl+T moves** (autosave gets another chord or a menu-only toggle) → the table question changes
  independently of the port question; still not a reason to add `Command::New`.
- **Any bridge proposes clearing the buffer locally** → that is option C. It needs no new analysis;
  point at `command.rs:80-91` and at ADR-0001 requirement 2.
