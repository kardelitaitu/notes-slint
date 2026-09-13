---
title: Slint bridge adoption in flight - what we are measuring as we build
status: proposed
id: 2026-09-13-bridge-slint-spike
created: 2026-09-13
updated: 2026-09-13
relates: [whitepaper §4.1, whitepaper §4.3, whitepaper §4.4, whitepaper §4.5, whitepaper §5.5, whitepaper §8, whitepaper §10.2, whitepaper §12.2]
decision: null
---

> **DECIDED (2026-09-13 04:47, human):** `we want to use slint, lets proceed`. Slint is
> the chosen UI path; `bridge-slint` is the destination. This note is **not** a go/no-go
> trial and contains **no STOP rules** - that framing came with the 04:10 brief and the human
> overrode it at 04:47. What follows is five measurements to keep taking while the bridge is built,
> four risks to engineer *through*, and the licensing consequence that has to be decided before
> anything ships. `status:` stays `proposed` because no working Slint bridge
> exists yet - `implemented` would be a lie (§5.6).

## Question

The decision is made. What remains is to know what Slint adoption actually costs this project,
expressed as numbers we keep measuring on the way in - and, for each one, whether it describes a
risk we engineer through or a consequence we accept and document.

## The frozen setup

| | |
|---|---|
| Build location | `crates/bridge-slint` - **in-tree, promoted, a workspace member**. The brief's out-of-tree scaffold at `C:\dev\bridge-slint-spike` was approved as *temporary* (promote once the walking skeleton runs); promotion beat it to the punch and landed at 05:00 the same morning, so the out-of-tree dir was never created |
| Link to the engine | `notes-api`, now a workspace path dependency like any other crate. Same rule as any bridge: import `api` and its own toolkit, nothing else (AGENTS.md, §5.2) - and now the rule `check-arch` can actually enforce |
| Baseline commit | `0ea7b3e5` - full: `0ea7b3e5fa8cced8d52b57240808fa0e167e1381`, verified this session (`git rev-parse --verify 0ea7b3e5^{commit}`). Subject: `skills: add the slint house guide (SKILL, setup, tools-install, 7 references)`, 2026-09-13 04:42 +0700. **This is the GPUI-only baseline every `G` number belongs to** |
| What followed it | `038c1755` - `slint spike 0: workspace learns a second bridge`, 05:00:37 +0700: `Cargo.toml` gains `crates/bridge-slint` as a member, `crates/xtask/src/arch.rs` and `smoke.rs` are edited, and `crates/bridge-slint/{Cargo.toml,src/main.rs,ui/README.md}` are added. `git rev-list --count 0ea7b3e5..HEAD` is now `1`: the base stopped being HEAD eighteen minutes after it was frozen |
| Slint version | `1.17.1` (crates.io `max_stable_version`, read 2026-09-13), matching the house skill's `>= 1.17` target. Pin it exactly, as §8 R14 demands of `gpui-kit` |
| Tree state at write time | promotion has **already begun**, uncommitted: `M Cargo.toml` (workspace members now include `crates/bridge-slint`), `M crates/xtask/src/arch.rs`, `M crates/xtask/src/smoke.rs`, and `?? crates/bridge-slint/` holding `Cargo.toml`, `src/main.rs`, `ui/README.md` |

Why the SHA matters more under adoption than it did under the trial framing: freezing it was
originally about a fair comparison. With the decision taken, the point is a **reproducible
baseline** - `0ea7b3e5` is the last commit where GPUI is the only bridge, so every `G` number
below must come from that tree (a second worktree or a `git stash`-free checkout, not from HEAD,
which now builds a second bridge and shares one `target` dir and one lock file with it), and
every `S` number from a pinned Slint version. Under the trial framing the risk was drift; at 05:00
the drift happened, which is the argument for writing the SHA down before the code rather than
after.

**Timeline, because the order of these four facts is the whole point:** 04:10 approval to measure
Slint; 04:42 house-skill commit, frozen as the baseline; 04:47 re-aim to adoption - *we want to use
slint, lets proceed*; 05:00 `spike 0` puts the crate in the workspace. The pre-registered
measurements below were therefore written *before* any Slint code existed, which is the only sense
in which "pre-registration" still means anything here: they are checkpoints, not gates.

### Concurrent writer observed

`038c1755` was **not this session's commit.** While the brief this note was written against still
put the spike out-of-tree, a **concurrent writer** landed the in-tree version at 05:00:37 -
`crates/bridge-slint` as a workspace member, `Cargo.toml` +15/-1, `arch.rs` 102 changed lines,
`smoke.rs` 115, `Cargo.lock` +1988/-65, `crates/bridge-slint/ui/README.md` +7 (re-verified against
`git show --numstat 038c1755`; the 102/115 are diffstat totals, 100+2 and 96+19). The Manager then
directed **in-tree-canonical** - my brief has that at 05:10, ten minutes after the commit, while the
manager's own journal logs the directive at 11:15 inside its "DRIFT 3" entry and names this same
commit as a concurrent writer's. Flagged rather than averaged: either timestamp puts the direction
*after* the code, which is the fact the paragraph exists to keep - the direction ratified a state
already on HEAD; it did not cause it. Which means "out-of-tree was superseded", as Option A
below puts it, understates the mechanism: it was superseded by another actor's commit plus a later
direction, not by the reasoning in this note, and that reasoning was written against a tree that had
already moved. Recorded because this is twice today that two sessions edited the same plan object at
once, and only the commit graph says who moved what.

### Non-goals, still

- **`bridge-gpui` is not deleted by this note, and deleting it is not a step here.** It is
  the baseline for all five measurements and the only bridge that currently works. Retiring it is a
  §9 milestone decision - real now that §8 R12's unfunded-second-bridge objection is spent, but
  later, and with evidence.
- **No `trait Bridge`.** Two implementations still do not license a guessed interface. The
  seam is `api` having no UI types, which `check-arch` enforces. If Slint needs
  an `api` event GPUI never needed, **add the event**; never reach around the port
  (AGENTS.md, §8 R13).
- **No markdown inside the Slint crate.** One correction to the brief's stated reason, because it
  will be repeated: the validator recurses `*.md` from the **repo root** only
  (`Get-ChildItem -Recurse`, excluding `target`, `node_modules`, `.git`,
  `dist`, `build`), so markdown sitting in an out-of-tree spike was always invisible to it - the
  rule was never really about the validator, despite how it was briefed. What the rule *is* about is
  placement: no row of the map covers markdown under `crates/`. It was broken once, by the promotion,
  on purpose and for a good reason - and has since been repaired the cheap way; see the item under
  Consequences.

## The five measurements: build checkpoints, not verdicts

Quantities to record repeatedly as the bridge grows. None can stop the work; the human decided the
work. What each feeds is a choice about *where* a cost lands.

### C1 - pin ownership (§4.3)

**Measure:** with pin on, read `WS_EX_TOPMOST` off the HWND after first show, after opening
and closing Slint's own native menus, after losing and regaining focus, after maximise, and after any
toolkit-owned popup, tooltip or overlay. Compare against the `api`-verified
`Applied` for `SetPinned` - never against the bridge's own optimistic state
(§4.3: pin persists, and pin is what the port says it is).

**Posture - ownership is declared, not contested:** `api` and `platform` remain
the **sole writers** of `WS_EX_TOPMOST`. Slint's own always-on-top window property stays
**unused**, and the build should assert it reads back false, so the unused-ness is a fact rather than
an assumption. That is the seam in one line: the bridge owns the window, `platform` takes a
handle and decides nothing, and pin is decided in `core` (§5.5's startup order, §8 R13).

**If drift shows up anyway:** a bug with an owner, not a verdict. Fix forward by suppressing the
toolkit-side path that sets the style, or by having the bridge create the window and hand Slint the
HWND - already the documented §8 R3 fallback, and what `platform` exists for. Re-measure
rather than measure once, because silent style re-assertion is the one failure that would degrade pin
from a feature to a best-effort, and that is a §4 change, not a rendering glitch.

### C2 - frame-rect landing (§4.1)

**Measure:** restore a saved rect on each side of a two-monitor setup at 100 % and 150 %, read the
rect back once the window settles, record the delta in physical pixels per monitor per scale.
`core/geometry.rs` stores one persisted space; §12.2's two coordinate spaces - client
versus frame, asymmetric chrome, per-monitor DPI - are exactly what is being re-tested.

**Posture:** measure inside the build, fix forward, no gate. But record *where* the correction lands,
because that is the design content of this checkpoint. A delta that is a pure function of scale
factor belongs in `core` or `platform`, next to the storage that made the
promise. A delta that exists only for Slint, on Windows, on a particular monitor arrangement is the
shape to **escalate rather than absorb**: a table keyed by monitor inside a bridge means the toolkit
knows where our rect lands and core does not, which redefines §4.1 per toolkit - i.e. reopens M3. M3
is closed and machine-proven, maximise-close-relaunch included, and it stays protectable only while
the correction is toolkit-neutral.

### C3 - frameless chrome (§4.1, §10.2)

**Measure:** lines of Rust plus `.slint` for a draggable, resizable, min/max/close-capable
window with our title bar, at 100 % and 150 %, and whether Win+arrow snapping and
double-click-maximise survive.

**Posture - decided:** the custom title bar **will be built in Slint**. The budget is its GPUI-side
equivalent, **915 LOC** measured today (`titlebar.rs` 552 + `menu.rs` 363,
`wc -l` at the base commit), and it lands in a **later slice**, not in the walking
skeleton. Treat 915 as a floor with a caveat: that number was the *purchased* version - ADR-0002
bought `TitleBar` from gpui-kit rather than building it - so the Slint bar is genuinely new
work, and §8 R11's risk is the honest estimate rather than a port.

**What transfers unchanged:** R11's verification checklist is toolkit-independent by construction -
drag, double-click-maximise, Win+arrow snapping, snap-layout flyout, caption accessibility exposure,
rounded corners, DPI at 100/150/200 %, remote desktop, IME. That list is the acceptance test for this
slice no matter which toolkit draws the bar.

### C4 - build and iteration latency (§2)

**Measure, one session, one machine, interleaved `G S G S G S G S G S` (five pairs; report
median and worst, never a cherry-picked best):**

- `G` = warm `cargo build` of `bridge-gpui` after a one-line edit in
  `main.rs`.
- `S` = warm `cargo build` of the Slint bridge after the same-shape edit.
- Plus keystroke-to-visible for a pure layout change on each side - the A/B split below.

**Baseline rule, kept from the original brief on purpose:** the GPUI number must come from the **same
session on the same machine**. **`2m11s` and `1.7s` from CI are context, never
the divisor** - different machine, different cache warmth, different dependency graph, and one of
them is a cold checkout. Record the pin beside every `G` figure, since §8 R14 already notes
the GPUI side of this comparison is a moving target (`gpui-kit 0.6.1` resolving
`gpui-pre ^0.3.1`, where M0 tested plain `gpui 0.2.2`).

**Why keep measuring something decided:** the request arrived as “GPUI iteration feels
slow”, and two latencies hide in that sentence. **A. rebuild latency** - keystroke to running
binary; Slint does not obviously win it, as `slint-build` adds a proc-macro pass over
`.slint`. **B. iteration-loop latency** - change to visible; here
`slint-viewer --auto-reload` genuinely skips the Rust rebuild for layout work, which is
what the house skill prescribes. If only B improved, write that down plainly: the want was a
hot-reload loop, and the bridge change bought it as a side-effect. Not a verdict on Slint - a note on
what to expect, so the same complaint does not come back in three months wearing a new name.

### C5 - text editing, the one axis (§4.5, §5.5)

**Measure:** `wc -l` of every bridge line that exists only to give the user a text buffer -
caret, selection, double and triple-click, word and line motion, clipboard, undo, IME, scrollbar,
composition - and how much of that Slint's `TextEdit` / editable `TextArea`
genuinely covers. Reference: `crates/bridge-gpui/src/editor.rs` is **3715 LOC**
(`wc -l`), 47 % of that bridge's 7900 lines. This is where adoption pays off or it does
not: §5.5 makes the editor the bridge's own precisely because text editing is the one thing that
cannot be abstracted across toolkits, so a real widget is the only architectural gain on offer.

**Posture: measure inside the build and fix forward - explicitly not a go/no-go.** Two behaviours,
both §4.5 do-no-harm and §8 R10 rather than gates:

- **1 MiB echo.** Paste a 1 MiB buffer, type continuously for 10 s, record keystroke-to-echo median
  and p95, dropped frames, and whether the app stays responsive. If Slint is worse, the answer is
  virtualisation or an intermediate buffer inside *our* bridge - a rendering problem gets a rendering
  fix, which is exactly why §5.5 put the editor on this side of the seam.
- **paste-CR and paste-CRLF.** Paste text containing a lone `CR` and `CRLF`
  sequences, then diff what reaches `api` `Flush { text }` byte-for-byte against
  what was pasted. **Any normalisation is a defect with an owner, not a decision point** - and note
  why it is nasty: if the widget rewrites \r to \n internally, the bytes arrive at
  `core` already changed, so nothing in `save.rs` can catch it and the fix has to
  live in the widget, in interop, or in a buffer that keeps the file's line endings *outside* the
  editor. The `roundtrip_port.rs` corpus is the shape of the test; a lone-CR fixture joins
  it.

Deleting thousands of lines by importing a widget that rewrites a user's file would trade our largest
maintenance liability for our smallest possible one. Measure the reduction, and hold the
byte-faithful diff as the condition on counting it as a reduction at all.

## LICENSE - read from the terms, not from memory

Sources retrieved 2026-09-13: `LICENSE.md` and
`LICENSES/LicenseRef-Slint-Royalty-free-2.0.md` from
`github.com/slint-ui/slint` `master` (both HTTP 200), plus crates.io metadata for
the `slint` crate. `slint.dev/slint-license.html` is a **404**; do not cite it.

The framework is **triple-licensed, at our choice**:

1. **Royalty-free Desktop, Mobile, and Web Applications License v2.0** - no cost, proprietary
   desktop/mobile/web, **embedded excluded**.
2. **GPLv3** - no cost, any platform including embedded, copyleft on the combined work.
3. **Commercial** - paid; the only one covering embedded systems.

Documentation and examples are MIT, so snippets copied from them carry no obligation.

**Royalty-free 2.0 conditions, as written:**

- **§2 Attribution, either/or.** (a) Display the `AboutSlint` widget *in an "About" screen
  or dialog accessible from the top level menu* - with the clause that bites us: "In the absence of
  such a screen or dialog, display the widget in the 'Splash Screen' of the Application." Or (b)
  display the Slint attribution badge **on a public webpage, preferably where the binaries can be
  downloaded from**, easily found by visitors to that page.
- **§3 Limitations.** The Software may not be distributed or made publicly available **alone**,
  without integration into an Application. It may not be used **within embedded systems**. An
  Application that **exposes the APIs** of the Software, in part or in total, may not be distributed.
  License and copyright notices in the source may not be removed or altered.
- **A portable zip is still distribution.** §2 attaches to all five §7 artifacts, not only to the
  installer, and §7.4's build mechanics change nothing about that.

**The consequence, stated plainly: this is product surface, not a dependency detail.** Our menu is
Open, Save As, Recent files, Auto-save (§4.4) - **there is no About screen today** - so (a) means
*inventing one*, a §4 feature row of its own, while its splash-screen fallback collides with the
cold-start budget in §2. Option (b) moves the cost to the download page, a §7 distribution decision:
the badge has to live where the binaries are actually downloadable, and a portable zip shared by file
link has no such page. GPLv3 removes the attribution surface but puts the combined binary under
copyleft, against README's current "all rights reserved" standing - and this project has not chosen
its own license at all yet.

> **FLAGGED FOR THE HUMAN.** Adoption is decided; which of the three licenses is being accepted, and
> which attribution surface we therefore owe, is not. This is the one item in the note that cannot be
> engineered through by writing code, and §7.4's release mechanics are where it becomes invisible if
> it is deferred past M5.

**UNVERIFIED - my reading, not counsel, and no authoritative text found for:** (i) whether a private,
never-distributed dev build triggers §2 at all, since §2 reads as attaching to *distribution*; (ii)
whether a page under this repo's `docs/` counts as "a public webpage where binaries can be
downloaded"; (iii) whether §3's "exposes the APIs" clause could ever read on an application that
statically links Slint. All three go to counsel before a shipping decision. None of them gates the
build.

## Report format: done, not declared

The house skill's rule is inherited unchanged: **never declare UI work done without looking at a
render.** For every checkpoint record, before it is written anywhere as a fact:

1. `slint-viewer --check <file>.slint` passes - compiles, prints diagnostics.
2. `slint-viewer --screenshot <file>.slint` produces an image, and **a human looks at it**.
   Both are `1.17+` features and we are pinned to `1.17.1`.
3. The screenshot path and the raw numbers go in the record. Interaction claims - C1's topmost reads,
   C3's drag and snapping - need the MCP server (the house skill's debugging reference), not a still
   image of a static layout.

Status per checkpoint is **RECORDED / NOT MEASURED**, with the commit and the pinned version beside
each. There is no PASS, because there is nothing left that could fail. What stays forbidden is
declaring a checkpoint covered with no render behind it: a chrome claim without a screenshot is an
unchecked assertion, which is the failure mode §12 exists to avoid. And no checkpoint may be quietly
re-scoped once a number exists - if the measurement does not fit the question, the question was the
finding.

## Options still live

The toolkit choice is off the table. What is still genuinely open:

### A. Out-of-tree until the walking skeleton runs, then promote (what was briefed)

Scaffold at `C:\dev\bridge-slint-spike\`, path-dep on `../notes-gpui/crates/api`, no workspace
churn while the shape is unknown. Cost: while out-of-tree, `cargo xtask check-arch` cannot see the
new bridge and `cargo build` from the root does not build it, so CI proves nothing about it - the
seam is unguarded exactly when it is most likely to be crossed. **Superseded by events: `038c1755`
chose B at 05:00.** Kept because the reason matters the next time a spike is briefed this way.

### B. Promote now (what happened)

A workspace member from the first commit: reviewed cheaply, covered by CI and by the layering gate
immediately, and honest about the project's direction. Cost, as executed: the gate's own source
(`arch.rs`) was edited in the same commit as the thing it gates, and the crate arrived with a
`ui/README.md` that no row of the placement map covers - so the docs validator has been red since
`038c1755`. Both costs are small and both are payable now; neither should be paid twice.

### C. Two bridges in-tree indefinitely

Not a choice to make now. It becomes one whenever `bridge-gpui` stops being built, which on this
trajectory it eventually will - and that is a §9 milestone decision with §8 R12 attached. Until that
decision is taken, the extra build in the workspace is the price of the comparison, and `G`/`S` side
by side is exactly why it is worth paying.

### C. Two bridges in-tree indefinitely

Not a choice to make now. It becomes one whenever `bridge-gpui` stops being built, which on
this trajectory it eventually will - and that is a §9 milestone decision with §8 R12 attached.

## Recommendation

**B, as landed - and now pay the two small debts `spike 0` took on credit: the gate edit and the
docs gate.**

The sequencing question answered itself while this note was being written, and the answer was the
better one: a member of the workspace is a bridge CI can judge. What is left is to make that
commit's implicit choices explicit. Take the five checkpoints from here - `G` numbers from a
`0ea7b3e5` checkout, `S` numbers from `crates/bridge-slint` with Slint pinned exactly, the pair
recorded on the same machine in the same session - and engineer the four risks as stated above. In
parallel, not after: answer the licensing question, because it is the one item here whose lead time
is not ours.

Sequencing judgement, so it is not lost: **C1 first, C5 second, C2 alongside the first two-monitor
test, C3 last.** C1 is the load-bearing invariant - a bridge that cannot own its window styles is a
demo rather than a bridge, and it is fixable only by the §8 R3 path, so learn it while there is
nothing to rewrite. C5 is the payoff and needs the longest runway. C3 is genuinely later: most
expensive, least architectural, and already budgeted at 915-LOC-equivalent in a subsequent slice.

And one thing no checkpoint asked for: **make the `crates/xtask/src/arch.rs` change a decision with
a test behind it.** The gate is the only artifact in this repo whose job is to keep a second bridge
honest about what it imports, and `038c1755` edited the gate - **102 lines of `arch.rs`, 115 of `smoke.rs`** - in the same commit as the thing it
gates. That is legitimate - it is the promotion the plan called for - but it is now history, so the
fix is not a revert: it is a test proving a `bridge-slint` that imported `notes-core` or
`notes-platform` still fails `check-arch`, and a paragraph in the ADR that supersedes ADR-0002
saying why admitting a second toolkit changed the rule and what it did not change.

## Consequences

- **`bridge-gpui` stops being the product surface on this trajectory**, while remaining the
  baseline for every number above and the only working bridge until the skeleton runs. When it is
  retired - not *if*, on this framing - §8 R12's objection expires with it, and that is the moment a
  bridge abstraction may finally be worth designing, from two implementations instead of one.
- **ADR-0002 and ADR-0003 go from settled to contingent.** Both are gpui-kit-specific: the purchased
  `TitleBar` and the custom chrome. ADRs are append-only, so building a title bar in Slint
  does not edit ADR-0003 - it earns ADR-0004 superseding 0002 (the gpui-kit dependency) and restating
  0003's chrome decision for a toolkit that ships no chrome. §10.2 reopens with it.
- **The intent docs name the wrong toolkit; the cheap half is now edited.** README's stack bullet
  says Slint is swapping in for GPUI behind the same bridge rule, and points here. What still says
  GPUI: `whitepaper.md`'s one-paragraph summary - "Rust engine, GPUI frontend behind a bridge", plus
  its statement of the single biggest risk in GPUI-viability terms - and the working title
  `notes-gpui`. Those are *intent*, so the code wins and they get edited as adoption lands, but the
  name of the repo is the one thing a decision record cannot quietly fix. Still owed by whoever lands
  the promotion, and out of this note's fence: `whitepaper.md` and `docs/architecture.md` - recorded
  here so it does not go stale silently.
- **The layering gate is load-bearing in a new way.** `arch.rs` enforces "a bridge imports
  `api` plus its own toolkit and nothing else" (§5.2). If admitting Slint loosened the
  family rule in a way that also loosens it for the next member, the seam is gone while the gate
  still prints clean - which is the specific way this project's most important invariant dies. The
  test to add: a hypothetical `bridge-slint` importing `notes-core` must still
  fail.
- **Was an open item, breaking the docs gate from `038c1755` until 05:17 the same day:**
  `check-docs.ps1` exited **1** with `crates\bridge-slint\ui\README.md ORPHAN - no rule in the
  placement map covers this location`. That file's own text says it exists because git cannot store
  an empty directory. Two fixes were on offer: the placement map gains a row for markdown under
  `crates/` - which means editing the `doc-management` skill *and* its validator, since a README
  under a crate is normal Rust practice and the map currently pretends otherwise - or the
  placeholder stops being markdown. **Taken the second way: `ui/README.md` is now `ui/NOTES.txt`,**
  text unchanged. The placeholder's job is to make a directory exist in a clone and to sit inside
  `smoke.rs`'s Slint freshness root, neither of which needs a markdown extension; inventing a map row
  for one placeholder would bend the rule that exists to exclude exactly this. `check-docs.ps1`
  exits 0. Worth noticing this was the no-markdown hazard firing at the moment of promotion, one
  directory inside the fence.
- Left out-of-tree, the scaffold's `Cargo.lock` and `target` dir sit outside this
  repo and outside every CI job. Acceptable for scaffolding, not as a destination - which is the
  argument for scheduling promotion rather than letting it drift.

## Reopening conditions

- **Reopen the go/no-go framing** - the one this note carried until 04:47 - if C1 shows
  `WS_EX_TOPMOST` moving after an `api`-verified `Applied` *and* the
  §8 R3 fallback (bridge creates the window, hands Slint the HWND) does not close it. That is the one
  finding which cannot be engineered forward without redefining §4.3, and it warrants its own note
  rather than a workaround buried in a crate.
- **Reopen §4.1's persistence promise** if C2's delta proves irreducible to something
  toolkit-neutral, i.e. per-monitor *and* per-toolkit. That is M3, reopened with evidence rather than
  nostalgia.
- **Reopen the retirement question for `bridge-gpui`** the first time the Slint bridge
  passes the §8 R11 checklist. Until then it is the baseline, not legacy.
- **Reopen C5's fix-forward posture** if a lone-`CR` paste survives to
  `Flush { text }` in a build anyone could ship. §4.5 and §8 R10 outrank the editor line
  count, and the round-trip fixtures gate CI on purpose.
- **Reopen the licensing section** on any decision to distribute, not merely to build - and
  immediately if this project's own license is chosen, since "all rights reserved" plus GPLv3 is not a
  combination that survives contact with a release.


---

## What the lane has actually produced (2026-09-13, S1-S4 + platform S1/S2)

Appended as a record, not a rewrite: the five measurements above still stand and are
still open. Everything below was built and measured this day; where a claim has no
number behind it, it says so.

### The spike, and the findings it cost
Out-of-tree first (a standalone workspace at C:/dev/bridge-slint-spike), then reconciled
in-tree as canonical. On slint 1.17.1: compat-1-2 must be enabled or the slint! macro
refuses; the root unsafe_code = forbid cannot survive a member whose dependency expands
ItemTreeVTable_static (E0453 x8), so that crate relaxes to deny locally; widgets are not
builtins (TextEdit must come from std-widgets.slint); Rectangle.color is deprecated for
background; a handler may not assign an in property, so anything the UI writes locally is
in-out; export takes no trailing semicolon and relying on the implicit last-import
re-export is deprecated; a property may not be named row (Rectangle already answers to
it); and inside a changed handler the Window builtins need a receiver - close() is an
unknown identifier, self.close() is the door.

### C1 (one topmost writer) held under a live two-writer test
Slint's own always-on-top property is never named in the markup, and the pin bit is
rendered only from Event::Pinned. Measured on the real window: pinned: true
WS_EX_TOPMOST=1 readback HELD across 1s, port silent on a repeat ask - reproduced every
run. The HWND is genuinely not available before show() (appearing between t+61ms and
t+270ms), and RegisterWindow DOES get an answer in-run: an earlier "no answer" finding
was a poll artifact of the harness, not an engine fact, and the fix was to drain the
event channel every tick (drains went 2 -> 1178).

### The text loop, and what it proved about the port
buffer -> Command::Flush on the bridge's own 750 ms quiet cadence, edit-to-flush
measured at 750.99 ms; CRLF normalised on the way in only (mirroring
bridge-gpui/src/editor.rs:2026), bytes kept as core's problem. Do-no-harm proven on a
seeded CRLF file: 32 bytes in, hash-equal=1, flush-since-open=0. Slint does NOT
normalise a lone CR - a 17-byte probe with one CRLF and one lone CR read back 17 bytes
with CRs=2 LFs=1 - so normalise-on-entry is load-bearing for this toolkit too. The 1 MiB
echo (set_buffer ~18 ms) is a property-in/out number, not paint latency, and is not
quoted as typing latency.

### Close and quit, which is where the data-loss path was
on_close_requested declines the first request (KeepWindowShown, window verifiably still
visible) and grants the second; on the granted close the bridge sends a final Flush if
dirty, then Gateway::close(), and prints which arm of Exit came back - QueueClosed /
Abandoned(Duration) / Panicked all matched, the first bridge in this repo to do so. The
quit race was reproduced deliberately: a character appended on the same tick as the
close cannot be saved by the debounce, and the bytes landed (exit: final flush sent
(58 bytes rev=1 epoch=3) -> exit drain: 1 event(s) accounted for, the voice xtask smoke
parses). Still NOT proven: an OS-initiated close (taskbar X / Alt+F4) on a no-frame
window - self.close() reaches the same callback, but nothing here can press a real
caption control.

### Frame risk, and the reason it became an architecture rule
With no-frame: true plus resize-border-width the restore rect did not ratchet across two
cycles: the maximised overhang (-8,-8,3440x1392) never reached storage and every
port-measured line stayed 800x600 at 120,90. What could NOT be read from a bridge is
GetWindowPlacement.showCmd / WS_MAXIMIZEBOX - and that is the finding, not a detail:
naming the OS in a bridge was convention, not instrument, until this day. check-arch now
carries bridge-names-no-ffi, DirectOnly so a toolkit's own OS lineage below it stays
legal (gpui's dialog through gpui-pre-windows, rfd through windows-sys), with
raw-window-handle exempted because Command::RegisterWindow asks a bridge to hold exactly
that type. While that rule was being written the first check-unsafe run printed
files scanned: 0, unsafe: 0 on a tree holding 104 raw unsafe uses - a silent-green gate -
because it read cargo metadata without --all-features/--filter-platform, so optional and
target-gated deps resolved to nothing and every member was skipped as published. One
flag; the fix is pinned by its own test.

### What is NOT done
- File drop exists only down-stack: WindowBackend::take_dropped_paths (default
Vec::new()) plus platform's arm/disarm/take_buffered. No bridge consumes a drop yet and
no real drag has ever been performed - the buffer tests pass with no window, no
apartment and no mouse. arm/disarm are also unreachable from any legal caller today (a
bridge may not import notes-platform), so S3 needs either a trait seam or a port
command before this is more than machine-verified plumbing.
- The title's APP_NAME is env(CARGO_PKG_NAME) on both sides, so Alt+Tab reads
notes-bridge-gpui in one bridge and notes-bridge-slint in the other: same rule,
different package name. Flagged as a product decision; nobody has prettified it.
- The double-click-to-maximise emitter, recents polish, and dirty/save-failed/autosave
fed from real port signals are queued (S4b). The chrome module is mounted; the strip is
S2's, not a re-implementation.
- Evidence: there is none to hand over. The requested
C:/dev/bridge-slint-spike/cbm-evidence/ does not exist. Run logs were transient stderr
files and the out-of-tree spike was declared redundant once the in-tree member became
canonical, so nothing was archived. Every number above is reproducible-by-running, not
an artifact.

