---
id: 0006
title: The GPUI bridge is frozen, not deleted
status: accepted
date: 2026-09-14
deciders: [human decision, department execution]
supersedes: null
superseded_by: null
relates: [docs/roadmap.md §9, ADR-0002, ADR-0005]
---

## Context

On 2026-09-14 the human decided the framework question: **Slint is the product.** `notes-slint.exe`
is the thing that ships, and GPUI is dropped from its future. The department's ruling on *how* that
happens is this record: **freeze, then retarget, then delete on a schedule** — never delete tonight.

The reason a drop does not become a `git rm` is the honesty gate §9 carries. M2 reports
**3 of 7 machine-proven**, and those three rows are not abstractions; they are citations to code that
compiles and runs:

- row 1 (window memory) rests on `crates/api/tests/geometry.rs` and on the harness's M9 leg, whose
  exit-6 fixed point was measured against gpui-shaped window bytes;
- row 5 (recents) rests on the label trio in `crates/bridge-gpui/src/menu.rs:230-295` —
  `a_recent_is_labelled_by_the_ports_own_string_at_its_own_slot`,
  `a_missing_file_is_shown_greyed_and_never_forgotten`, `the_list_stops_where_the_slots_end` — the
  only thing proving "the port's string, unedited, at its own numbered slot" is proven *on screen*;
- row 7's scratch half rests on the same startup order `§5.5` that `bridge-gpui` was first to satisfy.

Delete those files and the roadmap does not become "less done" — it becomes **unprovable**: a document
citing paths that no longer exist, which is worse than one citing tests that fail. **Git-can-recover is
not proof-can-recover.** A deleted test can be resurrected from history; an unproven claim cannot be
re-earned retroactively, and no future reader can tell which state they are reading. That asymmetry,
not sentiment about the first bridge, is what this record exists to hold.

## Decision

**`bridge-gpui` is frozen. Its removal is scheduled and gated, not open-ended.**

1. **The freeze law.** No new GPUI features, no new needles, no new surface on `notes-gpui.exe`.
   What exists keeps building and keeps passing. **Rows may not loosen**: no §9 check moves on
   anything gpui-side, in either direction — the crate's rows hold their present status until a Slint
   proof replaces them, and the freeze is no licence to retire a claim early either.
2. **The retarget.** What changes is what the repo faces by default, not what exists: `cargo xtask
   smoke` defaults to the Slint product, CI's smoke rows point there, and the docs say Slint. Today's
   shape is still the old one — the default leg runs the needle schedule, and CI still carries the gpui
   smoke as row **3e** (`.github/workflows/ci.yml:350`) beside the Slint product row at `:574`. So the
   retarget is a work order, stated here so a freeze is never mistaken for "carry on as you were".
3. **The terminal delete happens only when these six are green** — all six, on the Slint side, each an
   *equivalent proof* rather than a ported file:
   - **the recents label trio on product rows**: Slint-side equivalents of the three
     `bridge-gpui/src/menu.rs` tests above, over the product's own rendered rows;
   - **the §4.5 byte-identical bridge leg on Slint `TextEdit`**: a live round trip through the
     product editor, not core's fixtures alone;
   - **the M9 rect fixed point `(0, 0, 0, 0)` on product bytes**: today `run_product_leg` names
     reading a rect as something it deliberately does **not** do
     (`crates/xtask/src/smoke.rs:4203-4211`), so this item is **new harness surface**, not a flag flip;
   - **the dialog flows Open and Save As driven to text-in-window**: `rfd` is already live in the
     product (`crates/bridge-slint/src/surface.rs:541`, unparented and said so at `:444-462`) — the
     gap is the assertion, not the code;
   - **the autosave toggle through the real menu**: row 6's gap, closed on a product menu row;
   - **the STRIP-4 owes at `crates/bridge-slint/src/product.rs:52`**: the recents exists-mark, the
     legend keys in a real bar, and the pin check mark's round trip.

   When the sixth lands, deleting `crates/bridge-gpui` unproves nothing — and no new ADR is needed,
   because this is it.
4. **The probe ruling, settled inside this decision.** `notes-slint-probe` stays frozen.
   `--binary=slint-probe` returns `Leg::NotWired` and exit **2** forever — the harness's own "I did
   not judge", not a machine decline (3) and not an app failure — with its single honest reason at
   `crates/xtask/src/smoke.rs:332`, `:343-348`, `:3508-3525`. What the needles proved is **donated to
   `Leg::Product` as claims**, re-earned there. Never as *probe edits*: an instrument that acquires new
   assertions after its verdict is not evidence, it is a wish.

## Consequences

**Positive.**

- §9's "3 of 7" keeps its referents for as long as it makes the claim: every cited file still
  compiles, still runs, still says what the roadmap says it said.
- The freeze costs no rewrite, no port-by-duplication, no deletion of a working bridge under time
  pressure.
- The exit gate is six **named, checkable** items rather than a feeling, so "when" is answerable by
  anyone with the tree, and the delete is bookkeeping instead of a judgement call.
- `bridge-slint` inherits a specification it did not have to invent: the six items are exactly the
  claims gpui earned first — the strip program's rule, applied to a toolkit swap.
- **ADR-0002 is narrowed, not superseded.** `bridge-gpui` still depends on `gpui-kit` and still builds,
  so its reasoning holds wherever it is cited; what changed is that nothing new is built on it.
  ADR-0005 stands for the same reason — its gpui-side arm is frozen behaviour, not revoked behaviour.

**Negative, and named as cost.**

- **`gpui-kit` stays in the resolve.** `gpui-kit = "=0.6.1"` remains a workspace dependency
  (`Cargo.toml:35`), and so does the duplicate-manifest workaround it forced: the
  `duplicate resource: type MANIFEST (ID 24)` linker failure and the post-link `cargo xtask manifest`
  step that answers it (`Cargo.toml:20-31`, `crates/bridge-gpui/build.rs:3-13`). That machinery is
  load-bearing for a bridge already decided against, and stays so until the delete.
- **Cold CI keeps paying gpui's compile.** Every run resolves and builds the toolkit tree — `gpui-pre`,
  the kit, and the `windows` version it pulls — for bytes the product will never ship. Recurring,
  accepted here, not discovered later.
- Two toolkits remain in the graph, so `arch.rs` keeps two UI roots and every dependency audit pays for
  a framework nobody is developing toward.
- The freeze forbids the cheap fix on the wrong side: a gpui-only bug surfacing in the frozen window
  is answered "not there", and that is a cost this decision takes on purpose.
- **A freeze without an exit gate rots.** That is the risk this record exists to name, and it is not
  small: frozen code becomes "kept" code by inertia, and a bridge nobody deletes becomes a second
  product by accident. **The six items ARE the gate.** Anyone who finds the list shorter than the work
  should widen it before the delete, never after.

## Reopen condition

Two directions, both checkable.

- **Earlier than scheduled**: if the frozen crate starts blocking the product — a resolve conflict, a
  CI arm unsatisfiable while both bridges build, or a §5.2 rule bent to keep gpui compiling — the
  freeze has stopped being cheap and the delete moves up, with every row it unproves marked
  *not proven* in §9 in the same commit. Unproving is allowed; leaving the claim standing is not.
- **Never**: if any of the six is declared satisfied by a *port* of a gpui test rather than by a proof
  over Slint bytes, the gate has not closed and the delete is postponed. The port is the tempting
  version of this decision; the equivalent proof is the one that means anything.
