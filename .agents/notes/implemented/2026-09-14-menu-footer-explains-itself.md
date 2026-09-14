---
title: The menu footer explains the document
status: implemented
id: 2026-09-14-menu-footer-explains-itself
created: 2026-09-14
updated: 2026-09-14
relates: [ADR-0001, ADR-0006, docs/features.md §4.4, 2026-09-14-explicit-save-act]
decision: null
---

## Question

`docs/features.md` §4.4 asks for the autosave reason **in the menu**, and ADR-0001 requirement 1
says "**Silence is forbidden.** While disarmed, the UI must show why nothing is being saved"
(`docs/decisions/0001-…:37`). Both bridges already *had* the words — `bridge-gpui`'s `skip_words`
and `meta_words` have rendered them in its status line since M2 began. `bridge-slint` rendered
neither: it drained `Event::AutosaveSkipped { .. }`, swallowed the reason, and put `describe()`'s
Debug shape on a status line that the next event erases.

Could the popup carry an explanation without growing a single `Command` or `Event`?

## What was built

A **footer** below the popup's six command rows, holding two sticky strings:

| fact | source | words owned by |
|---|---|---|
| why nothing is being saved | `Event::AutosaveSkipped { reason }` (7 variants), `Event::SaveFailed { reason }` | the bridge for the skip (no `Display` reaches `SkipReason` — that is api's design); **the port** for a failure (`SaveError`'s Display "IS the user-visible copy", `api/src/event.rs:188-191`) |
| what this file is | the `FileMeta` on `Event::Loaded` / `Event::Rebound` | the bridge, in bridge-gpui's exact sentence: `utf-8 · crlf · newline none · writable · within the guard · autosave not armed` |

The seam, one line each: two `String` fields on `Pump` (`why`, `file_words`) → six write sites in
`surface.rs::drain` and the autosave toggle → **one** publisher,
`plumbing::publish_explain(pump, weak)` → two `in property <string>` on `Spike`
(`why-not-saved`, `file-words`) → `Chrome`'s footer block, two `if`-created `Text`s under a 1px
hairline. No vocabulary changed: `core`, `api`, `platform` and `bridge-gpui` are untouched.

Three judgements worth keeping:

- **It is not a row.** No `TouchArea`, no chord, no place in the six-row count. The popup's height
  spends `6 * row + 5 * gap` once and adds the footer as one term outside it — so the probe's
  guards that count the 6th row's cells stay true, and a click on an explanation cannot exist.
- **The two voices for one skip are deliberate.** `describe()` keeps `AutosaveSkipped
  reason=ReadOnly` because `probe.rs:540` quotes that string as recorded evidence, and an
  instrument's evidence is not edited after its verdict (ADR-0006 §4). The menu is not an
  instrument, so `plumbing::skip_words` holds the user's sentence — copied from bridge-gpui so
  two bridges cannot explain one skip two ways.
- **The copy is tested against the chord table.** `ForeignFileNotArmed` says "Save As once
  (Ctrl+S)", and the test reads `Ctrl+S` out of `SHORTCUTS` rather than hard-coding it: rebind
  the key and the menu's promise fails the build. That is a deliberate decision-shape assertion —
  see [the save note](2026-09-14-explicit-save-act.md), which this footer is half of.

## What was measured, not asserted

`slint-viewer --screenshot --backend software` on a throwaway copy of `ui/` (patched only with the
`width`/`height` the root takes from Rust), driven through the **product's own door**:
`--load-data` sets `toggle-asks: 1`, which is the number `Chrome`'s `changed toggle-asks` handler
toggles `menu-open` on. The two strings arrive through the same properties Rust writes. Pixels are
sampled, because this session cannot see images.

| capture | popup height | amber glyph core, footer band | reading |
|---|---|---|---|
| committed `47b14241` (footer only) | 184px — grew the predicted 46px | **137,105,50** | that is amber composited over the **editor** `#171717`, not over `menu-bg` `#2a2a2a` → the block was hanging **outside the box that paints its background** |
| working tree, recents block + both fixes | 294px | **218,158,57** | amber over menu-bg ✓ inside the box |

What made the difference is that the rows region stopped being "whatever height is left". The
`GridLayout` is now sized by the row arithmetic itself rather than `parent.height - pad*2` (which,
once the popup grew for a footer, stretched six rows into the space the footer needed), the seam
below it reads that geometry — `rows-bottom: rows.y + rows.height`, the layout's own measure rather
than a re-derivation — and the footer anchors to the bottom edge its tail reserves,
`y: parent.height - root.footer-height`, instead of computing a position from nominal numbers.

The one remaining editor-coloured row inside the popup (y=301) is the **divider, not a hole**:
dyeing `background` on the temp copy turned exactly that row magenta, 21px above the popup's
bottom edge.

Unit level: 3 new tests in `plumbing.rs` (the copy's exhaustive skip list, the file line's exact
sentences, and grep guards for "no `TouchArea` in the footer / one writer per setter / both users
spend the same row arithmetic"). `cargo test -p notes-bridge-slint` → 38 + 56 + 4 green, clippy
`-D warnings` clean, fmt clean.

## Recommendation

Three things, in the order they cost least:

1. **Do not let the two geometry terms disappear.** They are the difference between the footer
   painting inside its box and outside it, and nothing in the test suite can see that — the unit
   guards check strings, not compositing. Whoever lands the next `chrome.slint` commit should
   re-run the one-number check: a screenshot of the open popup, brightest amber pixel in the
   footer band, **~218 means inside, ~137 means the grid is stretching again.**
2. **Anything else that hangs under the grid should read `menu.rows-bottom`**, the measured seam,
   and never re-derive a position from `6 * row + 5 * gap`. The first version of this footer did,
   and it is the reason the seam exists now.
3. **Promote the preview recipe into the `slint` skill.** A throwaway copy of `ui/` patched with
   the `width`/`height` the root takes from Rust, driven through `--load-data` — including
   `toggle-asks: 1`, which opens the popup through the product's own handler instead of a synthetic
   click — is how a popup with no data behind it was measured at all. The recents cap and the
   popup-vs-window fit question will need it again within a slice, and right now it lives only in
   this note's prose and a temp directory that will not survive.

The wider menu question this footer sits inside — the missing explicit `Save`, and whether
`Save As` keeps `Ctrl+S` — is [proposed separately](2026-09-14-explicit-save-act.md) and is not
answered here.

## What is still not proven

- **The geometry fixes are not committed.** They live in `crates/bridge-slint/ui/chrome.slint`
  while another window is mid-edit in the same file (moving the recents into the popup). If they
  are ever reverted, the footer goes back to painting outside its box — the test above is one
  screenshot and one number: an amber core near **137** rather than **218**.
- `file-words` says "lines" nowhere: `FileMeta` carries no line count, so the manager's commit
  message that promises "lines" overstates what this renders. What it renders is encoding, BOM,
  line endings, trailing newline, writability, the size guard, and the arming verdict.
- No live end-to-end: the footer was rendered through `--load-data`, not by dragging a real `.md`
  into a real window and watching `AutosaveSkipped` arrive. The arm that words it is reached by
  probe-era synthetic runs, which do exist, but nobody has re-run one against this markup.
- The popup can now outgrow a short window: the recents cap of five exists precisely because
  `Chrome` cannot ask the window how tall it is. With both blocks the popup wants ~294px against a
  minimum window height — unmeasured, and the same open fit question the width clamp already
  answers with `popup-floored`/`popup-overflow`.
