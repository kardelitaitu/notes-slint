---
title: Making "comes back maximised" actually true
status: proposed
id: 2026-09-12-maximized-persistence
created: 2026-09-12
updated: 2026-09-12
relates: [§4.1, §5.4, §5.5]
decision: null
---

## Question

§4.1 promises the window comes back "including when the window was maximised". Is that
promise kept today — and if it is not, what is the smallest change that fixes it without
inventing a `Command` for a fact the engine can already measure?

## What the code does today

The bit exists and is wired on the **read** side only:

- `Session::maximized` — `crates/core/src/session.rs:30`, defaulting to `false` at `:47`,
  and it round-trips through `session.json` (`maximized_and_rect_round_trip_together`,
  `crates/core/src/session.rs:318`).
- `api` honours it: a maximised session is never moved, only pinned
  (`crates/api/src/engine.rs:1160`).
- `bridge-gpui` honours it: it creates the window as `WindowBounds::Maximized`
  (`crates/bridge-gpui/src/main.rs:1505`).

**Nothing ever sets it.** Outside `Default::default()`, test fixtures and test literals there
is no assignment to `maximized` anywhere in the workspace, and no `Command` carries it
(`crates/api/src/command.rs:47-132`: Open, SaveAs, Flush, SetAutosave, SetPinned,
ClearRecents, Shutdown, RegisterWindow, GeometryChanged, UnregisterWindow). A real
`session.json` therefore always says `false`, both read paths are dead in production, and
**the §4.1 promise is currently false**: close the app maximised and it reopens at the
restore rect.

The fact is already within reach. `GeometryChanged` makes the engine measure once through the
seam, and that measurement is `GetWindowPlacement`, whose struct carries `showCmd` beside
`rcNormalPosition` — `crates/platform/src/windows/monitors.rs:68-82` reads it and returns
only the rect. The maximised bit is being discarded one layer down.

## Options

### A. A new `Command::SetMaximized(bool)`

The bridge reports the show state as its own event, mirroring `SetPinned`. Cost: every other
`Set*` command is a **user intent** — the menu asked for it. Maximising is something the user
does to the OS chrome, not to this app, so the variant would name an observation in the
vocabulary reserved for requests. It also adds a call site every future bridge must
reproduce, and per AGENTS.md a new `Command` is a design change that needs this note first.

### B. Fold a placement field into `GeometryChanged`

`GeometryChanged { placement }` with a two-case enum (Normal | Maximized). The variant is
payload-free on purpose, but the reason is **coordinate space**, not payload: a rect from the
bridge drifts the window one chrome-height per cycle — the live bug recorded at
`crates/api/src/command.rs:116-123`. A show-state enum names no space and cannot drift, so B
keeps the one-geometry-report shape and adds no variant.

B has a cheaper form. Because the engine's own measurement already reads `showCmd`, the
bridge can send nothing at all and `platform` can return the show state alongside the rect.
Same outcome, and the change lands in `platform`'s return type rather than in the port's
vocabulary.

## Recommendation

**B, in the measurement form.** Maximised is a fact about the window the engine is already
asking the OS for; `Command` is the expensive vocabulary and should stay reserved for
intents. Keep the rule that no rect ever crosses on this path — a show-state flag is the only
admissible payload, or the drift bug returns. If a bridge later cannot observe the state
without being told, A remains available and B does not foreclose it.

## Consequences

Easier: §4.1 becomes true, the two read paths stop being dead code, and the tests written
against them (`crates/api/tests/geometry.rs:121`, the `maximized` fixtures in
`crates/xtask/src/smoke.rs`) start exercising production behaviour. Old `session.json` files
stay valid — the field already exists and defaults to `false`.

Must be pinned down: that the restore rect keeps being stored *while* maximised (it does
today, through the same measurement), and that "maximised" means the show state at close,
not a request for the next launch.

Out of scope: minimised and tray state. This note fixes one bit, not a placement system.

Until this lands the §4.1 sentence is aspirational, and this note is where that is recorded —
a persisted bit nothing ever writes is worse than no bit, because it reads as implemented.

## Reopening conditions

- A bridge cannot observe a maximise without being told to (no geometry callback on that
  path) → take A.
- The app gains an explicit Maximize menu item or shortcut: that *is* user intent and earns a
  `Command`.
- `GeometryChanged` grows a second field → a dedicated `WindowStateChanged` beats a
  grab-bag; reopen the enum-vs-variant question then.
- §4.1's maximised promise is dropped from the product → delete the field rather than wiring
  it.

## Status update — 2026-09-12 (`8cda7bd0`, `e401e513`, `3324766a`)

**Recommendation B landed in its measurement form**, and the §4.1 promise is now true in code.
Verified against the tree, not against the commit messages:

- **The bit has a writer — one, and it is a measurement.** `crates/api/src/engine.rs:1405-1409`
  maps the platform's show state onto the field: `ShowState::Maximized` sets it,
  `ShowState::Normal` clears it, `ShowState::Unknown` leaves the stored bit alone — Unknown is
  the honest answer for a minimised window (`crates/platform/src/lib.rs:136-142`), not a No.
  It runs on the flush-tick measure (`engine.rs:1375-1411`) and sits inside the MAJOR-5
  move-in-flight guard (`engine.rs:1387-1390`), so the bit is protected from the same stale
  read-back as the rect. This is the "no `Command` for a fact the engine can already measure"
  shape the recommendation asked for: `GeometryChanged` is still payload-free.
- **The Watch wakes on a show flip, not just a rect change** — the failure mode option B had
  to survive. `crates/bridge-gpui/src/main.rs:453-457` adds `Fingerprint { rect, maximized }`,
  read at `:468-474` from `window.is_maximized()`; the reason it is necessary is gpui's own
  note that a maximised Windows window can report identical bounds
  (`main.rs:440-449`). A rect-only diff would have shipped a writer that never fired.
- **Both read paths are live now, not dead code.** The engine's no-move branch
  (`engine.rs:1208`, rationale `:1181-1182`) and the bridge creating the window as
  `WindowBounds::Maximized` (`main.rs:1816-1820`).

**What is still not proven, and why this note stays here.** No automated proof exists of the
whole cycle: `crates/xtask/src/smoke.rs` names `maximized` only inside `session.json` unit
fixtures (`:3031`, `:3393`, `:3415`, `:3446`), and the bridge coverage is headless
(`main.rs:2681-2697`, a synthetic flip). Nothing has launched the real app, maximised it,
quit and relaunched it. `implemented` means the code exists *and works*, so the note stays
`proposed/` — the writer works in tests, the behaviour is unobserved.

**The single act that promotes this note:** maximise the window, quit, relaunch, and see it
come back maximised. On that yes — move the file to `../implemented/`, set `status:` and
`updated:` in the same commit, mark M3's live-proof line closed, and drop the caveat from
README's "Remembers the window" row. On a no — keep it here and record what came back instead,
because "a persisted bit nothing writes" has been replaced by "a written bit nobody has watched
land", which is a different bug and a better one.
