---
title: How the chords got wired - the micro-wave, the draft store, and what is still owed
status: implemented
id: 2026-09-14-slint-chord-wiring
created: 2026-09-14
updated: 2026-09-14
relates: [§4.2, §4.3, §4.4, §5.5, §9]
decision: null
---

`1668a5d1..76a36433` is six commits, all landed 2026-09-14, and between them `notes-slint.exe` stops
being a window that describes itself and starts being a window that answers a key press. Before this
wave the markup fired every one of its callbacks into a Rust side that had never registered a handler
(`product.rs:347-351` says so in past tense); after it, Ctrl+T toggles auto-save, the pin strip pins, a
recent row opens, a relaunch comes back holding the document it had, and the caption's four frame asks
are hooked. The commit messages are the authority for every claim below; this note is the map, not the
evidence.

## Question

STRIP-2b shipped a product root that owns no `Event` arm and no `send()` of its own - the root law - and
left it with a legend of fourteen chords, a hamburger menu and zero handlers. How does the wiring get in
without writing the probe's decisions a second time, and how much of it can a single 15-minute box
actually hold?

## Options

**A. One full-box brief: wire the whole thing.** Tried first. Three coder lanes took it and none of the
three landed a diff; the losses were environmental - a lane quiet for eleven minutes and then
interrupted, not a lane that could not do the work. A brief that spans two files, four behaviours and
the gate is a brief a box cannot finish.

**B. The isolation micro.** One file, one function, one commit per lane, no gate duty, and an explicit
commit-or-revert at the box edge. Each step's brief ends by naming the one thing the next step calls.

**C. Let the product root call the probe's wiring.** Cheapest to type, and it is the move the strip
program exists to forbid: a second copy of a decision instead of moving the decision. Not attempted.

## Recommendation

**B**, because it is the pattern that turned three dead lanes into four landed commits, and because the
diffstat is the proof of the method rather than a claim about it: `1668a5d1` is +108 in `surface.rs` and
nothing else; `19d4de5f` is +33/-8 in `product.rs` and nothing else; `48f0d6e7` is +120/-8 across the
same two files; `6dd19637` is +151/-17 across those two and no others. Nobody re-decided a rule, and no
step had to hold the whole shape in one box - each lane only had to call the one before it. Re-read this
before briefing another bridge slice: the failure A demonstrated was about the size of the ask, not about
the difficulty of the code.

## What each step is

- **A - `1668a5d1`: the wiring gets a home in `surface`.** One new function, `wire_callbacks`
  (`crates/bridge-slint/src/surface.rs:1266`), taking `ui`, the gateway, the `Pump` and the dialog
  sender: open-asked, save-as-asked, autosave-asked, open-at-index, quit-asked. It goes in `surface`
  because it is policy - it names which `Command` a user act means - and the root law says a product
  root may own none.
- **B - `19d4de5f`: the product calls it.** One hook line, placed after the window exists and after
  `Pump` holds the stored auto-save bit that two of those handlers read (`product.rs:347-358`), and the
  mailbox created beside it gets its reader: one `try_recv` per wake inside the 8 ms tick
  (`product.rs:463-471`), never an await, so nothing on the loop ever waits for a modal. The reader is
  not decoration - `ask_dialog` answers *through* that channel even when `SLINT_NO_DIALOG` skips the
  modal, so an Open ask with no reader would be a silent dead end. The root still sends nothing itself.
- **C - `48f0d6e7`: the document comes back, and the caption's last four asks are hooked.** The two
  sections below. This is the commit that made the app usable across a relaunch.
- **CI - `67d1d682`, mid-wave:** the product gets judged off this machine too. Why that row is not yet
  evidence is owed item 4.
- **Hygiene - `6dd19637`, and its tests at `76a36433` (HEAD when this note was written).** Three fixes, below.

## The draft store, and the C1 bug that reads like a missing feature

`session.json` holds **window facts and a path, never text**: `Session` is rect, monitor id, scale,
`maximized`, `pinned` and `path: Option<PathBuf>` (`crates/core/src/session.rs:22-41`). The words of a
note nobody has named live at `<StateDir>/notes/untitled.notes` - a deterministic name core owns
(`scratch_note_path`, `crates/core/src/paths.rs:67-84`, D69: no pid, no counter, so two launches share
one scratch and last-writer-wins) - written by api through the SaveAs machinery it already has
(`crates/api/src/engine.rs:1040-1043` for the identity test, :1290 for the bind). **That is the fact to
have before touching restore: an untitled draft is a file, and the session only names it.**

The slint product's hole follows straight from not reading it: the product cloned the session for its
rect and read every field of it except `path`, so a relaunch showed an empty editor while the draft's
bytes sat on disk. bridge-gpui had the identical bug and named it at its own STEP 5
(`crates/bridge-gpui/src/main.rs:1706-1720`, found in `b343543`); this is the twin, and it arrived with
the same reasoning - the fix is **one `Command::Open` of existing vocabulary**, whose answer is the
`Loaded` arm in `drain` that already existed (buffer adoption, the epoch, the title, the lock verdict),
so it is a send and not a subsystem. It lives at `surface.rs:1229-1257` (`restore_from_session`) exactly
because the root owns no send, and is called from `product.rs:323-333`. Order is not load-bearing: the
engine queues a command posted before `RegisterWindow`, and gpui sends its STEP 5 pre-loop too. What IS
load-bearing is the ask - **no path, no ask**: a first launch, or a draft that was never written, comes
back empty, which is the truth about it, and the product prints `startup: the session named no document,
so nothing is asked for` rather than being silent.

## The four remaining caption asks (`48f0d6e7`, C2)

Each is a verbatim move of the probe body it mirrors, minus that body's measurement witnesses
(`probe.rs:1316-1320`, :1410, :1448, :1453, with :1693-1703 and :1825-1833 behind them), and each obeys
one rule: the act is a `Command` out, or it is a `ui.window()` call, and nothing is rendered from the
click.

| Ask | What it does | Where |
|---|---|---|
| `toggled-pin` | `Command::SetPinned(!confirmed)` - **the port's last confirmed bit**, not the widget's guess | `surface.rs:1345-1357` |
| `clear-recents-asked` | `Command::ClearRecents`; the list redraws from the event that answers it | `surface.rs:1358-1366` |
| `toggle-max` | `window.set_maximized(!is_maximized())` plus `Command::GeometryChanged`, and the report line names `caption_glyph`, because the window's own bit decides which asset shows | `surface.rs:1367-1385` |
| `minimize-requested` | `window.set_minimized(true)` and **no command** - a park is not a resting place to store | `surface.rs:1386-1400` |

The stale dated allow on `wire_callbacks` died in this commit; the two surface fns that are dead in the
*probe* root carry a per-item allow there instead, which is the honest direction for an allow.

## Hygiene, because all three were found by watching the product run (`6dd19637`)

- **The register storm, latched.** The retry guard used to be `tick_drops.borrow().is_none()` alone, and
  `arm_drop_target` leaves that `Option` **empty when it refuses** (`plumbing.rs:255-262`) - so a handle
  the platform would not take was re-registered, re-armed and re-printed every 8 ms: ~125 tries a second,
  forever. The decision is a pure fn (`register_says`, `product.rs:132`) and it closes the latch on a
  success *or* on the second refusal of the SAME handle; a new handle is a different window and owes its
  own budget (`product.rs:386-418`).
- **Minimised is not a place.** Windows parks the window at -32000,-32000 at 160x28
  (`plumbing.rs:186-194`), and `GeometryChanged` makes the port measure the *live* window - so telling
  it about a park is how a park gets persisted. `Fingerprint` carries the bit and the root now skips
  every comparison, store and send while it is set; the pending episode fires on the wake after a
  restore, on the real rect (`product.rs:427-459`).
- **The autosave-off close tax.** The close path waited for a `Saved` that, with auto-save off, is never
  coming - a guaranteed 2.0 s per close. `saves_settled` counts *any* terminal answer (Saved /
  AutosaveSkipped / SaveFailed: `surface.rs:686`, bumped at :1099, :1141, :1172) and the wait now exits
  on either counter moving, with the answer named in the verdict line (`product.rs:511-544`).

`76a36433` pins two of the three down as unit tests -
`the_register_latch_closes_on_success_and_on_the_second_same_refusal` and
`an_answered_flush_ends_the_close_wait_even_when_nothing_saved` - so the next reader inherits verdicts,
not prose.

## Evidence, and what each line does not buy

- **Gate at `48f0d6e7`, as the manager journal records it:** clippy 0, `cargo test --workspace` 31 ok,
  `check-arch` 6/0, `check-ci` 17/0, xtask 176, `cargo xtask smoke --binary=slint` **PASS in 45.8 s**, and
  the gpui leg PASS. `6dd19637` and `76a36433` were re-verified narrower: bin tests 16 + 34, arch 0.
- **Re-measured at HEAD `76a36433` while this note was written:** `cargo test -p notes-bridge-slint
  --bins` = 16 + 34 passed, 0 failed; `cargo test -p xtask` = 176 passed; `check-arch` = 6 crates, 0
  violations; `check-ci` = 17 steps, 0 violations; `check-deps` = 6 members, 0 violations;
  `check-unsafe` = 0 violations. No full clippy or workspace run was re-executed here, so the wide
  verdict stands at the `48f0d6e7` gate above.
- **Real-input end-to-end at `48f0d6e7`: PASS** - a driver posting keystrokes and mouse events through
  `AttachThreadInput` flipped Ctrl+T **both ways**, pinned **both ways** with a `WS_EX_TOPMOST` readback
  rather than the app's own say-so, adopted a recent via Alt+1, restored the document across a relaunch,
  and closed on request **#1 granted** (`product.rs:480-484`).
- **What that E2E is not.** The driver is nowhere in the tree (`git grep AttachThreadInput HEAD` finds
  nothing tracked) and no run log of it is archived, so the verdict is a journalled observation, not a
  re-runnable check - unlike the smoke legs, which anyone can run. And the run **did make a click** to
  aim the pointer, while every chord lives on the markup's outermost `FocusScope`
  (`crates/bridge-slint/ui/main.slint:188-195`, whose own comment also forbids an imperative `focus()`
  on the caret's account). That combination is why owed item 1 could not be closed on **that** run's
  authority; it closed later, on a keyboard-only control run with no click at all (item 1, `a12d58b3`).

## What is still owed - the honest list

1. **Focus at startup - CLOSED, shipped in `a12d58b3`.** The question this row carried - first keys
   dead until a pointer press lands, or driver artifact? - is answered by a control run, not by
   argument: the same keyboard-only `AttachThreadInput` driver, no click anywhere, **PASSes** on the
   fixed build (`focus: taken before the loop`, then a real Ctrl+T flips auto-save) and **FAILS** with
   zero chord lines on the pre-fix `76a36433` build. So the dead first keys were real, and they are
   gone. The mechanism is `claim_focus` (`product.rs:240-257`): ask the outermost scope once and
   accept only a **visible** focus item, via `WindowInner::from_pub(..).set_focus_item(..)` with
   `FocusReason::Programmatic` - the door Slint's generated `.focus()` itself compiles to, borrowed
   because 1.17's public `slint::Window` has no focus API at all. **The caveat, stated plainly:** the
   250-wake give-up path (`FOCUS_TRIES`, ~2 s) has never been exercised - the fast path lands on every
   run observed, including under `cargo xtask smoke --binary=slint` - so the retry arm and its failure
   line are vouched for by reading, not by watching.
2. **The per-field `Pump` allows.** `product.rs:1-14` is now the honest version of an allow: it names
   what is left (`asked`, `hold_reported`, `ticks`, `last_bucket`, `strokes`, `quarantine_reported` and
   the act-machine counters after `surface.rs:700`) and says plainly that replacing the blanket
   `#![allow(dead_code)]` with per-field allows on ~25 lines is **OWED, not done** - in its own words,
   "this box spent itself on the two features".
3. **The probe leg, Option B - unstarted.** `cargo xtask smoke --binary=slint-probe` still returns
   `Leg::NotWired` and exit **2** (`crates/xtask/src/smoke.rs:332-358`, `not_wired_reason`), because the
   only schedule the harness speaks is the gpui needle one; seventeen timed `*_AT` acts live in
   `probe.rs`, and porting them is its own slice. `docs/dev/testing.md` keeps the decline on the record.
4. **CI has a slint row and has never run it.** `67d1d682` added the `slint_build`, `slint_clippy` and
   advisory `xtask_smoke_slint` steps to `.github/workflows/ci.yml` (:507, :530, :574) plus the matching
   rows in `crates/xtask/src/check.rs` - and `git remote -v` prints nothing at HEAD, so that workflow has
   executed nowhere, ever. Every verdict in this note is local.
5. **The eye-pass list carried from the strip record**, now with one more row from item 1: the popup,
   caption and About glyphs on a real screen, a real Explorer drag rather than a synthesised one, the
   over-8-MiB refusal as a user sees it, the read-only caret, and typing the first sentence after a
   double-click launch.

## Consequences

Easier: the product root needs no further per-ask edit to take a new command - a new row's handler goes
into `wire_callbacks`, and the root's only future change is a reader for whatever answers it. The draft
store stops being a mystery to anyone wiring restore on this bridge, and the storm / park / tax failures
now have latch functions with unit tests instead of comments.

Harder, and worth naming: the chord surface is now *live* in a product, so a chord that routes to nothing
is a user-visible dead key rather than a probe curiosity - which is what the `SHORTCUTS` and `route_of`
guards at `surface.rs:110-177` are for. The shipped focus claim carries its own risk in one `use` line:
`slint::private_unstable_api::re_exports` (`product.rs:72`) is not public surface, so **any Slint version
bump is the tripwire** - re-read `claim_focus` against the new `i-slint-core` and re-run the keyboard-only
control before trusting the first keystroke of a post-bump build. And `docs/roadmap.md` §9 still counts
M2 exit items on gpui's proofs: nothing here promotes a row there on its own authority, because §9's own
rule is that the CI evidence trail has to exist first.

## Reopening conditions

- THIS ONE HAS A DATE AND A RESULT: the focus fix landed (`a12d58b3`) and a run with **no** click kept
  its first keys, so the question stayed a bridge bug and did not move up to the markup's focus model.
  Reopen it there if a future no-click run loses the first keys anyway.
- If a third bridge lands, "one wiring home in `surface`" stops being enough and the shared module wants
  its own crate; do not answer that by copying `wire_callbacks`.
- If the per-field allow in item 2 is ever satisfied by *deleting* the measurement half of `Pump` rather
  than naming it, the probe loses its schedule. Those fields belong to the probe; they are not dead code.
