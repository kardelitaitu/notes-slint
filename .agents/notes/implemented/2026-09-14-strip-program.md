---
title: The strip program - the Slint bridge stops being a probe
status: implemented
id: 2026-09-14-strip-program
created: 2026-09-14
updated: 2026-09-14
relates: [§4.4, §5.5, §9, §12]
decision: null
---

`ec5904b0..e8aa2ac3` is twelve commits, every one landed 2026-09-14, and their combined effect is
a change of tense: `bridge-slint` stopped being an instrumented spike - a probe that measured the
window contract and then hid itself - and became a shipped product with its own exe name, its own
licence obligation, and its own machine leg. The commit messages are the authority for every
number below; this note is the map, not the evidence.

## Question

The spike proved the window contract by *saying sentences about itself*. A product cannot keep
doing that: `notes-slint.exe` must not self-hide, must not override its own state dir, and must not
print a needle schedule into a user stderr. So how much of a 3,700-line probe can be thrown away,
and how much has to be moved somewhere honest first?

## Options

**A. Write the product root from scratch.** Clean, and it re-decides - eventually badly - every rule
the spike discovered the hard way: the locked refusal, the do-no-harm witnesses, the generation
ladder, the title contract.

**B. Carve the probe.** Move each decision out of the probe into a module both roots compile, in the
commit that moves it, guards and all. The product root stays thin by construction and the probe
keeps its name and its schedule.

**C. One binary, behaviour switched by an env var.** Cheapest to type, and it ships a product with a
hidden mode that prints the probe lines - the exact lie this strip exists to end.

## Recommendation

**B**, and hold the law that made it mechanical, because it is the part to re-read before editing
this crate: *a product root may own no `Event` arm and may not call `send()` itself*. Every event is
read by `surface::drain`, every command goes through `plumbing::send` (`08b22e50`, `c1591cd3`). When
a product need turned out to be unexposed, the decision moved out of `probe.rs` into `surface.rs` in
the same commit - never into a second copy in the root.

## What the tree is now

| File | Owns | Compiled by |
|---|---|---|
| `src/probe.rs` | the instrument: `main`, the timed acts, the witnesses that *are* acts, the chord tests | `notes-slint-probe.exe` |
| `src/product.rs` | §5.5 and nothing else: state dir by the port rule, place, show, handle, register, when the loop wakes, what a close means | `notes-slint.exe` |
| `src/surface.rs` | the shared decisions: `drain`, `Pump`, `text_pump*`, the dialog mailbox, `lock_verdict`, `legend`, `caption_glyph`, the drift guards | both |
| `src/plumbing.rs` | `report` (the single owner of the `notes-gpui: ` prefix), `send`, `hwnd_of`, `arm_drop_target`, the fingerprint witnesses | both |
| `src/ui_gen.rs` | the `slint!` block, verbatim - it does live outside a crate root in Slint 1.17.1 | both |

`use crate::*;` is gone from the crate. `product.rs` still carries `#![allow(dead_code)]` with its
reason in the header: a dozen instrument items are used by the probe and not yet by this root, and
calling them from here would mean writing the acts into the product.

## The dated lie and its funeral (`ec5904b0` to `c1591cd3`)

STRIP-0 declared two bin names over ONE source file and said so in the manifest **with a date**
(2026-09-15), because everything downstream fences against which exe is the product: the icon and
version resource, the application manifest, the portable folder, the `%APPDATA%` identity, the smoke
leg, and the `check.rs` slint-build row. STRIP-2b half 2 ended the sharing, and `c1591cd3` records
the proof rather than this note restating it: the cargo warning about one file in multiple build
targets went to zero, and the two exes stopped being the same bytes. The name every owner fences
against, `notes-slint`, was flipped last and kept building throughout, so no check ever had to be
lied to.

## The ordering fact §5.5 was silent on (`c1591cd3`)

Measured, not hoped: **there is no HWND after `ui.show()`.** The product printed `startup: NO HWND
after show` on a real window - the same line the probe prints at its own startup, which is why the
probe registers again from a later tick. winit materialises the platform window the first time the
toolkit pumps events. Both startup-order comments read "create window, show, read the handle,
`RegisterWindow`" as though the handle were simply there (`probe.rs:11-13`,
`crates/bridge-gpui/src/main.rs:5-8`); neither had written this down. The product now re-asks **once
per wake until it holds a handle, then never again** (`product.rs:283-304`). If it never appears, no
drag lands and no pin applies, and the startup print is the evidence: the contract failing loudly
rather than quietly.

## Honest shutdown lives after `ui.run()` returns (`2fc924f3`)

The brief asked for the final flush and join *inside* the granted-close callback. `Gateway::close`
own doc forbids that exact place - never call it from the engine thread - and the note above its
bounded join records the hang that rule replaced: **12 s timed, >15 s reproduced, no panic, no log,
no event**, because the thread calling `close()` from the window own close callback is the one
thread that owner needs in order to pump (`crates/api/src/gateway.rs:444-458`). So the callback only
counts and grants. After `ui.run()` returns: `drop(tick)` FIRST (that is what releases the gateway
and events clones the wake closure held), then a last compare plus a bounded 2 s wait for the
answering `Saved`, then `UnregisterWindow`, then disarm the drop lease, then `close()` - and a
failed join is a trace line plus a **nonzero exit**, never a panic. An `Abandoned` join waits again
for 10 s on the channel `Disconnected`, which is gpui shape and gpui ceiling. Driven from outside
the process by user32 `PostMessage(WM_CLOSE)`; `taskkill` was explicitly not used, because a kill is
not a close.

## Geometry: an episode, not a count (`2fc924f3`)

`GEOMETRY_QUIET` 250 ms and `GEOMETRY_FORCE` 1 s are gpui numbers, taken so the two bridges agree
about when a rect is official - the guards police the two constants themselves. What is deliberately
**not** inherited is the probe cap of two sends per session: that was a measurement budget for one
act walk, and a user who keeps dragging has to end up with the port holding the rect the window is
actually at. `2fc924f3` quotes the live run - two sends in one episode, and the next startup reading
back exactly where the last one left the window. The decision is a pure fn (`settle_says`) separated
from the measuring so it is testable without a window.

## The panic hook - code and unit only (`2fc924f3`)

Installed at the top of `main`, before the port is even asked for a snapshot, so a panic in startup
is caught too; it writes through `report()` and then calls the default hook, which is what keeps a
nonzero exit code. Only the format function is unit-tested: **no live panic was provoked**, because
shipping a panic trigger in a binary people run is not a thing.

## The licence row (`62ef5527`)

`LicenseRef-Slint-Royalty-free-2.0.md` grants itself on **either** of two conditions and only one is
open here: condition (b) is a badge on a download page, and this project has no download page, so
clause 2(a) is the live obligation - display the `AboutSlint` widget in an About screen reachable
from the top-level menu. It is now the 6th hamburger row, and the placement was HUMAN-approved
2026-09-14. Three facts to have before anyone tidies it:

- `AboutSlint` is reachable **only through the style re-export** - `std-widgets.slint`, whose fluent
  copy re-exports it at line 6 (`crates/bridge-slint/ui/chrome.slint:49`) - the same door
  `ui/main.slint` uses for `TextEdit`. Omitting that import broke the tree once already: the markup
  error is the cause and the `E0432` on `crate::Spike` in BOTH bins is its echo.
- The row has **no chord cell** and `SHORTCUTS` stays at fourteen: a pointer act with no `Command`
  behind it is not a command, and the legend must not lie about it. A test asserts the legend never
  sees About.
- A hand-drawn `MadeWithSlint` is **forbidden by a guard** (`probe.rs:2070`). The widget is the
  compliance; artwork that merely resembles it is not.

## Smoke: three artifacts, two judged, one honest decline

`71baf034` added `cargo xtask smoke --binary=gpui|slint|slint-probe` (`81831027` made the profile a
function, `exe_rel(profile)`, so the build step and the launch step cannot disagree about which file
they mean). `e8aa2ac3` wired the middle one. In `crates/xtask/src/smoke.rs`:

- `gpui` - the needle schedule, unchanged.
- `slint` - the **product contract**: its startup lines on its own captured stderr under the
  `notes-gpui: ` voice, still alive at 45 s, a *visible* window given a real `WM_CLOSE`, the close
  and `shutdown: joined cleanly` lines, and exit code 0 **by itself**. A force-kill cannot PASS
  whatever code it ends with (`:4329-4378`), and its own PASS line names its limits: it proves none
  of the rect, the pin, the recents trace or the maximised cycle (`:4386-4392`).
- `slint-probe` - `Leg::NotWired`, exit **2**, deliberately not 3: 3 claims the *machine* cannot host
  the check, which is a statement this decline is not entitled to make - a box with a perfect window
  station answers the same 2 (`:3490-3544`).

What is owed to wire the probe leg is the schedule itself, not a flag: **17** timed `*_AT` acts live
in `probe.rs` (`PIN_AT` through `LOCK_AT`), and the harness reason says plainly that porting that
schedule is its own slice of work. No run log of `--binary=slint` is archived in this tree, so the
machine proof of the shutdown sequence that *is* recorded is the `2fc924f3` live run.

## Three parity rows the bridge owed the port (`79af7b32`)

- **Retry.** `SaveFailed` now re-arms the send witness, so a refused save goes out again on the next
  quiet tick instead of the newest edit sitting there unheard - §4.4 calls that the worst failure
  this app can have. The cost is stated in the code, not hidden: a *permanent* failure now retries
  every `AUTOSAVE_IDLE` forever, where gpui re-sends only on the next edit. If the two bridges are to
  agree, that retry rule belongs to the **port**. Raised there, not invented here.
- **Copy.** The status line printed the `Debug` of a public-contract enum; the port declares its
  `Display` as the user-visible wording (`crates/api/src/event.rs:188-191`), so it is used now, and a
  grep-grade guard forbids `reason:?` returning.
- **The missing mark.** Checked in the port first: `RecentEntry::exists` exists
  (`api/src/event.rs:182`) and D13 says a vanished path stays in the list, greyed - so this was a
  bridge gap, not a port gap, and no bridge stats a path. Rows read `display (missing)`; gpui
  `disabled(!exists)` parity is admitted to be weaker, because 1.17 text rows have no per-row colour
  hook. It also caught a lying comment: the mark was claimed as core, the code mapped
  `entry.display` alone, so the mark was **nobody**.

## What M2 still owes

Not code - evidence and eyes. `.github/workflows/ci.yml` carries the slint build and clippy rows,
but `git remote -v` is empty, so that workflow has never run anywhere and nothing in this record is
CI-proved. The remaining human eye-pass list: the popup, caption and About glyphs on a real screen,
a real Explorer drag rather than a synthesised one, the over-8-MiB refusal as a user sees it, and
the read-only caret.
