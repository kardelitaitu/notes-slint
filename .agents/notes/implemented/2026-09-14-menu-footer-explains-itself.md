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

| capture | popup height | brightest amber in the footer band |
|---|---|---|
| committed `47b14241` (footer only) | 184px — grew the predicted 46px | 137,105,50 |
| working tree at the time, recents block + both fixes (now `79d11e86`) | 294px | 218,158,57 |

**What those two numbers do and do not say.** I first read the 137 as amber composited over the
editor rather than over `menu-bg`, and from that concluded the footer was hanging outside its box.
That reading is withdrawn, and the reason is worth keeping: a later live capture showed the product
paints its body at `#292929` and its popup at `#2a2a2a` — **one unit apart** — so in the real window
that comparison cannot distinguish inside from outside at all; it only worked in the viewer because
an unpainted root window is darker than either. What 137 vs 218 actually measures is ink *coverage*
(thin antialiased caption strokes may contain no fully covered pixel), which is a weaker thing.

What does support the two layout changes:

- `GridLayout` is no longer sized by leftover height (`parent.height - pad*2`), so it cannot
  stretch six rows into whatever room the popup happens to have; it is sized by the row arithmetic
  itself, the seam below reads that geometry (`rows-bottom: rows.y + rows.height`, the layout's own
  measure, not a re-derivation), and the footer anchors to the bottom edge its tail reserves
  (`y: parent.height - root.footer-height`). The footer's position is now structural instead of
  agreeing with the box by coincidence at one particular content height.
- The one editor-coloured row inside the popup (y=301) is the **divider, not a hole**: dyeing
  `background` on the temp copy turned exactly that row magenta, 21px above the popup's bottom edge.
  It reads near `#171717` because `Theme.bar-edge` is **`Palette.border`** — a platform brush, not a
  literal — which is also why "a hairline and a see-through hole look identical" is true here for a
  reason I had wrong: the hairline is the OS's own border colour, and the body is one unit from the
  popup's fill.

Unit level: 3 new tests in `plumbing.rs` (the copy's exhaustive skip list, the file line's exact
sentences, and grep guards for "no `TouchArea` in the footer / one writer per setter / both users
spend the same row arithmetic"). `cargo test -p notes-bridge-slint` → 38 + 56 + 4 green, clippy
`-D warnings` clean, fmt clean.

## Recommendation

Three things, in the order they cost least:

1. **Do not let `GridLayout.height` go back to leftover height.** Nothing in the test suite can see
   it — the unit guards check strings, not geometry — and nothing in a screenshot can either,
   because the popup's `#2a2a2a` and the body's `#292929` are one unit apart, so a footer that
   escapes its box does not look like it escaped. The check that survives that fact is structural:
   the popup's measured height equals the sum of its declared parts (`pad*2 + 6*row + 5*gap`, plus
   the recents term, plus the footer term), and the hairline sits `footer-height` above the box's
   bottom edge. If those two agree, the footer is in the box.
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

- **The footer's pixels are proven in a preview, not in a live window.** The fixes are committed in
  `79d11e86`, and the render that shows them correct came from `--load-data`. A real window can be
  photographed here, but its menu cannot be opened from this session: `SetForegroundWindow` is
  refused even with the `AttachThreadInput` dance, and without the foreground a synthetic press is
  swallowed as an activation click — and any keystroke sent anyway goes to whoever *is* focused,
  which is why the harness must read that gate's answer rather than ignore it. `cargo xtask smoke`
  declines this class of run as **exit 3, "not the app's fault"**, and says in its own header that
  "the CLICK half STAYS MANUAL". So "a person can open this menu and read the footer" is a manual
  check, and it is still owed.
- **What a live run here cannot do, it can still feed.** Seeding the private profile's
  `session.json` with `"path"` makes the product restore the file through its own startup door, and
  a real window of a throwaway copy came up on a real foreign `.md`:
  `load: path=…\scratch.md … meta(read_only=false oversize=false armed=false)` — the disarmed
  verdict the footer exists to render, arriving as a real `Event::Loaded` in a real window, next to
  `corners: round (the window is normal)` and `recents: rendered 1 row - slots 1..=1`. So the
  footer's **inputs** are live-proven; only its on-screen placement in a live window is not.
- `file-words` says "lines" nowhere: `FileMeta` carries no line count, so the manager's commit
  message that promises "lines" overstates what this renders. What it renders is encoding, BOM,
  line endings, trailing newline, writability, the size guard, and the arming verdict.
- **The reason line has no live witness yet.** The file line above is proven by a real `Loaded`. The
  other half needs an edit: typing into a disarmed file is what makes the bridge send `Flush` and
  the engine answer `AutosaveSkipped(ForeignFileNotArmed)`, and this session cannot put characters
  into the product's caret without first winning the foreground it cannot win. So that sentence in
  the menu is currently evidenced by unit tests and by the frozen probe's needles, not by a window
  anybody watched.
- The popup can now outgrow a short window: the recents cap of five exists precisely because
  `Chrome` cannot ask the window how tall it is. With both blocks the popup wants ~294px against a
  minimum window height — unmeasured, and the same open fit question the width clamp already
  answers with `popup-floored`/`popup-overflow`.
