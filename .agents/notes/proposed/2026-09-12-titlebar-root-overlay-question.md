---
title: Title bar popup and tooltip without Root
status: proposed
id: 2026-09-12-titlebar-root-overlay-question
created: 2026-09-12
updated: 2026-09-12
relates: [whitepaper §4.3, whitepaper §4.4, whitepaper §5.5, whitepaper §10.2]
decision: null
---

## Question

ADR-0003's title bar wants two pieces of the kit's overlay machinery: the left region's
hamburger opens "the §4.4 menu as a popup anchored beneath it", and the centre region's
centred ellipsis truncation is "permitted only because the tooltip exists" — full path
always in the tooltip. M2 is deliberately Root-free: `crates/bridge-gpui/src/menu.rs:1-28`
records why (Root coordinates window-scoped text selection through the same machinery that
owns the editor's IME marked range; a second owner of selection is how D51/D52 get quietly
broken). How does the title bar ship its popup and tooltip — mount Root, hand-roll without
an overlay, or defer both and ship less than the ADR drew?

## What the sources actually say

- **Tooltip hard-requires Root.** `gpui-component-0.6.1 src/tooltip.rs:263-288` resolves
  `Root::tooltip_overlay(window, cx)` inside `if let Some(...)`; without a Root mounted,
  `window.root::<Root>()??` (`root.rs:466-473`) is `None` and the show/hide handlers are a
  **silent no-op** — the tooltip never appears and nothing reports why.
- **Root is the second owner menu.rs feared — on its face.** Root's render calls
  `TextSelection::activate_scope` **every frame** and paints `TextSelectionLayer`
  (`root.rs:577-608`, esp. :580-581 and :595), owns the window's `tooltip_overlay` and
  `native_menu_overlay` as children (:597-598), clears window selection when modals open
  (:316-318, :401-403), and styles the window from `Theme` (:579, :591-593). The window is
  currently component-free by recorded decision (`main.rs:1182-1186`).
- **The popup half is softer than assumed.** The kit `PopupMenu` never names `Root` —
  `menu/popup_menu.rs:12` builds on gpui's `anchored`/`deferred` primitives and sits in a
  container like `Popover` (:313-315), which wraps `gpui_base::Popover`
  (`popover.rs:15`), and neither file references Root. So the hamburger popup *may* work
  without Root; that is unverified, and Root's render hosting an overlay layer is exactly
  what that family expects to sit under it.

## Options

### A. Mount Root and verify/accept the selection conflict

`Root::new(surface, window, cx)` as the window's root view. Gets tooltips, the popup
container family, and later dialogs/sheets for free. Costs: reverses the recorded M2 slice
condition ("the brief allowed this slice only if the menu does NOT need Root",
menu.rs:3-6) — not fatal, but a recorded decision is now a reopened one; the D51/D52
conflict is *documented, not proven* (menu.rs:5, `tests/ime_seam.rs:564`'s manual recipe is
the test that would have to pass on a Root-mounted build); and Theme styling enters the
window whether or not anything else uses it.

### B. Hand-roll the popup and tooltip inline in the titlebar layout

No floating window, no Root, no kit overlay. The bar is one 34px region inside the single
root view we already own: a hover reveal in the bar itself (full path in the existing
status line, which menu.rs's `legend()` already feeds), and the hamburger popup as a last
painted child absolutely positioned beneath the trigger — plain layout and paint order,
not an overlay system. Costs: keyboard navigation, click-away dismissal, and window-edge
clamping are ours to write (the kit gives them for free); the look is ours to keep boring.

### C. Defer hamburger + tooltip; ship pin/title/dot only

The smallest slice, and it matches menu.rs:26-27's "deliberately NOT here" list. But it
ships in open conflict with an *accepted* ADR-0003: the hamburger is the ADR's left region,
and the centre truncation rule loses its basis ("permitted only because the tooltip
exists"). The chords already are the menu on Windows (menu.rs:19-24), so the *function*
survives; the *design* does not.

## Recommendation

**B.** The conflict Root poses is the one this codebase already paid to avoid, and the
verified facts cut against A's price: the tooltip requires Root but silently no-ops without
it (the worst kind of dependency), while Root's per-frame selection-scope activation
(`root.rs:580-581`) is precisely the second-owner shape menu.rs:1-6 names. B keeps the
window Root-free, keeps D51/D52's ownership story untouched, and the ADR's *intent* — full
path always discoverable, menu reachable from the bar — survives via the status line and an
inline popup. If B's dismissal/clamping work turns out bigger than a slice, fall back to C
and record the divergence from ADR-0003 in the implemented note; do not quietly ship a bar
the ADR does not describe.

## Consequences

- Easier: the window stays exactly what main.rs:1185 declares it — no Theme, no Root, no
  component; the kit's tooltip no-op trap never gets stepped in; the M2 budget (one window,
  one editor, 13 Events, 10 Commands) holds.
- Harder: two small behaviours are hand-maintained. ADR-0003's centre-region *mechanism*
  ("full path always in the tooltip") is not built as written — the divergence is recorded
  here and in the eventual implemented note, and only graduates to a superseding ADR if the
  status-line path proves worse than the tooltip it replaces.
- Forecloses nothing: B does not unmount anything, so A remains available the day Root's
  selection machinery is proven harmless here.

## Reopening conditions

- The kit popup family is verified to render and dismiss correctly in a Root-less window
  (paint order, Escape/click-away) → take the kit `PopupMenu` for the hamburger instead of
  the hand-rolled column; the Root question then only concerns the tooltip.
- A Root-mounted build passes the D51/D52 manual IME recipe unchanged (`ime_seam.rs:564`)
  *and* a concrete need for kit dialogs/sheets/overlays lands → A stops being speculative
  and becomes the cheap option.
- The hand-rolled popup needs floating-window semantics (overflowing a maximised window's
  edges, multi-monitor clamping beyond layout) → inline layout is the wrong tool; reopen
  between A and C, not B.
- ADR-0003 is superseded for other reasons → this note follows it into the folder that
  implies.
