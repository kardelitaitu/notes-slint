---
title: Rounded corners without an alpha channel
status: implemented
id: 2026-09-14-rounded-corners-dwm
created: 2026-09-14
updated: 2026-09-14
relates: [ADR-0002, ADR-0003, ADR-0006, docs/risks.md R11]
decision: null
---

## Question

`notes-slint` draws its own title strip on a `no-frame` window, so on Windows 11 its corners are
square: `docs/risks.md` R11 and `docs/decisions/0003-custom-chrome-titlebar.md:50-53` both list
"rounded corners and shadow" as M2 acceptance work that was bought from the toolkit and never
verified. How should the Slint bridge get rounded corners — and what does each answer cost the
seam?

The trigger was a human report on 2026-09-14: the window "needs rounded corners when it is not
maximized, usually the trick is to have transparent background", with upstream's
[`examples/custom-titlebar`](https://github.com/slint-ui/slint/tree/master/examples/custom-titlebar)
named as the model. This note records what was measured against that suggestion, because the
answer turned out to be no for two independent reasons.

## What was measured (2026-09-14, Windows 11 Pro build 26200)

All of it against the live `notes-slint.exe` and a throwaway `slint!` project outside the repo,
using this workspace's exact dependency declaration (`slint =1.17.1`, `default-features = false`,
features `compat-1-2`, `backend-winit`, `renderer-software`, `raw-window-handle-06`).

1. **The frame is not what the markup comment says it is.** `ui/main.slint:24-30` frames the risk
   as "an undecorated winit window loses `WS_MAXIMIZEBOX`". `GetWindowLongW(GWL_STYLE)` on the
   running window reads `0x16CF0000` — `WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPSIBLINGS |
   WS_CLIPCHILDREN`, with `WS_POPUP` **clear** and `WS_MAXIMIZEBOX` **set** — and
   `DWMWA_EXTENDED_FRAME_BOUNDS` equals `GetWindowRect` exactly, with the client origin at the
   window origin. So Slint keeps the standard styles and makes the whole window client area:
   Win+arrow and the maximize box survive (R1's worry, answered), and there is no non-client
   caption for the OS to drag.
2. **Alpha does not reach the screen on this renderer.** A probe window with `no-frame: true` and
   `background: transparent`, containing a pure `#FF0000` rounded rectangle inset 24 px, reads
   `#000000` in the band that should show the desktop. The opaque control (same window,
   `#101010` background) reads `#101010` in that same band, and the antialiased edge of the drawn
   radius reads `#E10000` — so Slint computes the alpha and the radius correctly, and the last
   step drops it. Corroborated in source: `renderer-software` presents through
   `softbuffer 0.4.8`, whose only transparency-adjacent code in the whole crate is an X11 comment;
   and `i-slint-backend-winit-1.17.1` reads `window_attributes.transparent` in exactly one place,
   `renderer/femtovg.rs:215`, which this workspace deliberately does not build (root
   `Cargo.toml`: `default-features = false` "drops femtovg / glutin / OpenGL"). winit 0.30.13 does
   implement Windows transparency (`platform_impl/windows/window.rs:1231-1244`, an empty
   `DwmEnableBlurBehindWindow` region) — and Slint already asks for it by default
   (`winitwindowadapter.rs:662`) — so the blocker is the **surface**, not the window.
3. **Upstream's example cannot be adopted as written.** Its drag is `WindowMoveArea`, and the
   token `MoveArea` appears in **no file of any `*slint*1.17.1` crate** (checked across
   `i-slint-core`, `i-slint-compiler`, `slint`, `i-slint-backend-winit` and friends, with
   `no_frame`/`resize_border_width` at `i-slint-core/src/items.rs:1262-1263` as the positive
   control). 1.17.1 is still the newest stable release on crates.io, so `WindowMoveArea` is
   master-only — unreleased, not merely unused here. The same `WindowItem` has no transparency
   property either.
4. **DWM will round this window.** `DwmSetWindowAttribute(DWMWA_WINDOW_CORNER_PREFERENCE /* 33 */,
   DWMWCP_ROUND /* 2 */)` returns `S_OK`, reads back `2` through `DwmGetWindowAttribute`, and the
   corner pixel stops being the app's `#2B2B2B` and becomes the desktop's. That is the only
   mechanism measured to actually work here, and it needs no alpha at all: DWM clips and
   antialiases, and keeps its own shadow.

## Options

### A. Ask DWM to round the window (recommended)

`DWMWA_WINDOW_CORNER_PREFERENCE` toggled `ROUND` when the window is normal and `DONOTROUND` when
it is maximized, from the bridge, through the port.

The cost is the honest one, and it is the reason this is a note and not a commit: **a bridge may
not name Win32** (`arch.rs`, `bridge-names-no-ffi`), so this needs `Command` + `Event` vocabulary
in `api`, a function in `platform`, and the `Win32_Graphics_Dwm` feature on the root's `windows`
entry — which no member can add for itself. By AGENTS.md's own test that is a design change, not
a tweak. It is also the smallest such change on the table: one trait method on `WindowBackend`,
mirroring `set_topmost` (`platform/src/windows/topmost.rs:67`) including its read-back-before-
verdict discipline, and nothing persisted — the bit is derived from window state every run, so
`Session` does not gain a field and no settings key appears.

Limits to accept: Windows 11 only (R11's support floor is Windows 10, so on 10 the call fails and
the note stays square — a degraded look, not a broken app); the radius is the OS's (~8 px at
100 %), not ours; and it is one more thing the eye-pass list must confirm rather than assert.

### B. Upstream's way: transparent window, drawn radius

Markup-only, no new `Command`, identical on every Windows version, radius ours to choose (the
example uses 12 px), and it squares off on maximize with one binding. On any other configuration
this would be the recommendation.

Measured dead here (finding 2). Adopting it means adopting `renderer-femtovg` — OpenGL and glutin
back into a binary whose cold-start budget the root `Cargo.toml` just spent a paragraph
defending, and whose manifest story already excludes a second toolkit's baggage. It also means
re-earning the drop-target and dialog behaviour that ADR-0005 and S9 measured on the software
renderer. Not a corner change; a renderer change.

### C. `SetWindowRgn` with a rounded rectangle

Works on 10 and 11 and needs no alpha. Rejected on quality: a region is a hard polygon, so the
corners come out aliased — visibly worse than DWM's antialiased arc — and the region must be
recomputed on every resize and DPI change. It buys nothing over A that justifies shipping jagged
corners.

## Recommendation

**A**, and record B's measurement so nobody re-walks it.

The reasoning is not that DWM is clever — it is that DWM is the component already responsible for
the shape of a window on the OS this product targets, that it was measured to work here, and that
the alternative requires changing which renderer the product draws with to satisfy a corner.
A costs the seam a `Command` and an `Event`, which is what the seam is *for*: the bridge wanted
an OS behaviour and asked for it (AGENTS.md: "Add the event instead"). B costs the seam nothing
and costs the product its cold-start budget, and on today's configuration it does not even work.

Say plainly what A is not: it is not "the transparent-background trick". If the human reading this
wants the transparent-window route specifically — because a future radius, a real shadow, or a
non-rectangular note is on the table — then the renderer question is the one being decided, and it
deserves its own note.

## What was built (same day, after the recommendation was accepted)

The vertical slice, which is what made this a design change rather than a tweak:

| Seam | What it gained |
|---|---|
| root `Cargo.toml` | `Win32_Graphics_Dwm` on the shared `windows` entry |
| `platform` | `windows/corners.rs`: one attribute call, one read-back, `// SAFETY:` on both blocks; `WindowBackend::set_corner_rounding(handle, round) -> PlatformResult<()>`; the seam added to `all_handle_seams`, so a bad handle is refused by the same test that refuses it for the other four |
| `api` | `Command::SetCornerRounding(bool)` (10 → 11) and `Event::CornerRoundingFailed { reason }` (14 → 15); `Engine::apply_corner_rounding`, the sole emitter |
| `bridge-slint` | `surface::ask_corners`, the policy, called from the 8 ms wake that already reads the window's fingerprint and from the caption's own toggle; two Pump bits (`corners_asked`, `corners_refused`); the refusal arm in `drain` |
| `bridge-gpui` | one `describe` arm (see below) |

Three things the shape turned out to hinge on, none of them visible in the table:

- **The read-back exists, the wait does not.** `set_topmost` polls for 25 ms because
  `SWP_ASYNCWINDOWPOS` posts a reband that can never land. This call is neither, so a second read
  is enough and a spin would be a loop copied for the look of diligence. Recorded on the function
  because the next person to mirror `topmost.rs` will mirror the wrong half of it.
- **The policy lives in the bridge and has to be polled, not hooked.** Win11 maximises through
  paths no callback sees - `Win+Up`, a snap layout, a drag to a screen edge - so the ask rides the
  wake that already measures the window, deduped by `corners_asked`. Hooking only the caption
  button would have been correct for our own clicks and wrong for every other way to maximise.
- **The frozen bridge grew an arm, and that is the freeze working.** `bridge-gpui::describe` is
  deliberately wildcard-free so an unhandled `Event` is a compile error rather than an undelivered
  fact; adding the corner arm is what that control was built to ask for. `notes-gpui` sends no
  corner ask (ADR-0006), so the arm is unreachable - the test list covers it anyway, because an
  unreachable arm is also where a copy typo hides.

Measured on the live product, 2026-09-14, after the build (`DwmGetWindowAttribute` from outside
the process, plus a screen capture of the window's own corner):

| moment | preference reported | corner pixels |
|---|---|---|
| launched, normal | `2` ROUND | a 13×13 antialiased arc: the bar colour regresses leftward one column per row |
| `SW_MAXIMIZE` (no caption click - the OS path) | `1` DONOTROUND | square |
| `SW_RESTORE` | `2` ROUND | the arc again |

and across two full cycles the app printed exactly **five** corner lines for five state changes,
which is the dedupe: the wake ran hundreds of times in between and asked nothing.

`cargo xtask smoke --binary=slint` passed on these bytes, and the same run now drives a real
maximise through its M9 fixed-point leg - drift `(0,0,0,0)` - so the attribute call did not
disturb the restore rect it sits beside.

**What is still not proven.** No Windows 10 machine was touched: the `E_INVALIDARG`-then-quiet
behaviour comes from the documented support floor and from the mock's refusal path, not from
hardware. The 150 %-scale drag evidence is still unit-test-grade (the record in
`2026-09-14-title-band-drag.md` says so). And "the corners look right" is an eye-pass, not a
test - the arc above is a pixel mask read by a script, which can see a corner was clipped and
cannot see whether it matches the strip's 6 px menu radius well enough to please anybody.

## Consequences

- `Command` went 10 → 11 and `Event` 14 → 15, each with its pinned count test, `rebuild()`
  and `variant()` arms (`command.rs:154-227`, `event.rs:565-651`); `host_mock.rs` gained a
  `WindowBackend` method, which is the point — the mock now has to answer the question too.
- `crates/platform` becomes the first crate to call `dwmapi`. The unsafe ledger grows by one
  function with one `// SAFETY:` block, which `check_unsafe` counts.
- The bridge owns the *policy* (round normal, square maximized) and the port owns the *act*, which
  keeps platform's "decides nothing" rule intact.
- R11's "rounded corners" item gains a machine-visible answer and a human eye-pass, and its
  "shadow" item is answered by DWM rather than by us — worth saying out loud, because R11 was
  written expecting us to draw both.
- If Slint ever gains a released `WindowMoveArea` (i.e. a 1.18+ that ships it), the drag stops
  being ours. That is a separate question from this one, and it should stay separate: A does not
  foreclose it, and neither would B.

## Reopening conditions

Re-open in favour of **B** if any of:
- a Slint release makes a transparent `Window` work with `renderer-software` on Windows (i.e.
  softbuffer grows an alpha path, or Slint gains a property that names it); or
- the product genuinely wants a non-rectangular window, a custom radius, or its own shadow — at
  which point the renderer change is being paid for a feature, not a corner; or
- Windows 10 is dropped as a support floor and DWM's fixed radius is judged too small next to
  the gpui bridge's look.

Re-open against **A** if:
- the `Command`/`Event` pair turns out to be a wedge for per-window OS attributes we do not want
  to keep growing (in which case the honest shape may be one `Command` that carries a window
  *appearance* struct, and that is an architecture conversation, not a corner one); or
- DWM's rounding proves to disagree with `resize-border-width: 8px` or the drop-target's hit
  testing at the corners — both measured against square corners so far.
