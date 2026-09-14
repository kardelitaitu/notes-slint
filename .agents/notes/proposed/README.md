# proposed

Ideas on the table. Not decided, or decided but not yet built.

- Frontmatter `status: proposed`.
- State the **question**, the **options**, and a **recommendation**. A note with no
  recommendation is a ticket, not a note.
- If the idea is agreed but no code exists yet, add `> **DECIDED:** <the decision>` at the
  top and keep it here. Do **not** move it to `implemented/` — that folder means the code
  works.
- When the decision is real and settled, write an ADR in `docs/decisions/`, point `decision:`
  at it, and move this note to `archived/`.
- Before proposing anything, grep [`../rejected/`](../rejected/) — relitigating a settled
  rejection wastes the reader's time and yours.

## What is on the table (eleven, as of 2026-09-15)

Grouped by what each note is waiting on — one line each; the note holds the argument, this is the map.

**The save path — five.** What a write may do, and to which file.

- [`2026-09-12-arm-on-explicit-save-path`](2026-09-12-arm-on-explicit-save-path.md) — was the
  `ctrl-s` keybinding the arming act ADR-0001 names, or was a rule missing? **Overtaken by**
  [ADR-0007](../../../docs/decisions/0007-save-is-a-conformance.md), which built the real act; it stays
  here as the argument, and moving it is its owner's call.
- [`2026-09-13-new-document-command`](2026-09-13-new-document-command.md) — does a blank note
  deserve a `Command::New`, or is quit-and-relaunch the correct shape of a single-document app?
- [`2026-09-14-autosave-retry-ownership`](2026-09-14-autosave-retry-ownership.md) — after a
  failed save, who owns the retry cadence — the two shipped bridges answer it differently today.
- [`2026-09-14-note-switch-must-not-lose-text`](2026-09-14-note-switch-must-not-lose-text.md)
  — opening another note replaces the buffer and §4.4's unsaved-changes guard has no code;
  pre-existing, not a cost of the new Save.
- [`2026-09-15-flush-shares-the-refused-load-guard`](2026-09-15-flush-shares-the-refused-load-guard.md)
  — `Engine::save` and `Engine::save_as` both refuse to write a file whose open was
  refused; `Engine::flush` does not, and this note makes that deferred call an artifact first.

**The menu, and how a user reaches it — three.**

- [`2026-09-14-menu-six-rows`](2026-09-14-menu-six-rows.md) — the distance between the rows
  specified on 2026-09-14 and the popup the product draws; three of them are decisions, one is a stated
  non-goal.
- [`2026-09-14-menu-keyboard-traversal`](2026-09-14-menu-keyboard-traversal.md) — a pointer
  reaches every row and the keyboard reaches them one at a time by memorised chord — should the popup
  traverse?
- [`2026-09-12-titlebar-root-overlay-question`](2026-09-12-titlebar-root-overlay-question.md)
  — ADR-0003's bar wants popup and tooltip, and M2 is deliberately `Root`-free. Which
  gives?

**The window's own surface — two.**

- [`2026-09-14-window-gets-a-floor`](2026-09-14-window-gets-a-floor.md) — who owns a minimum
  size — markup, product, or platform: a small window clips the About panel, and one clipped line is
  the reason a file is not being saved.

**The record itself — where prose is stronger than the machine check.**

- [`2026-09-14-external-change-event-is-never-emitted`](2026-09-14-external-change-event-is-never-emitted.md)
  — `Event::ExternalChange` is declared and rendered and produced by nothing, while §4.2
  still asks for mtime + hash; the choice is framed here and no status row moves.
- [`2026-09-15-no-instrument-tests-a-pointer`](2026-09-15-no-instrument-tests-a-pointer.md)
  — no instrument has ever pressed a pixel, the four press legs have no recorded green run, and the
  honest sentence is exactly `M2 pointer interaction is untested`.

The two notes filed 2026-09-15 do **not** belong together, and the grouping above says why: the flush
note is a save-path question about which write is allowed, while the pointer note is an evidence note
about what the instruments can see — so it sits beside `ExternalChange`, not beside the menu
notes it is often quoted with. Dates are filing dates, not categories.
