# Window chrome: frameless title bars, dragging, corners

What a `no-frame: true` `Window` can and cannot do, and the traps in doing it yourself.
Upstream's worked example is
[`examples/custom-titlebar`](https://github.com/slint-ui/slint/tree/master/examples/custom-titlebar)
— read it for the *shape* of the problem, and check every line against your pin before copying it:
as of Slint **1.17.1** (2026-09-14) two of its three headline mechanisms are not in a release yet.

## What `no-frame` actually removes

On Windows the toolkit keeps the standard styles and makes the whole window the client area.
Measured on a live 1.17.1 winit window: `GWL_STYLE = 0x16CF0000` (`WS_OVERLAPPEDWINDOW |
WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN`, `WS_POPUP` clear, **`WS_MAXIMIZEBOX` set**), and
`DWMWA_EXTENDED_FRAME_BOUNDS` equals `GetWindowRect`.

Two consequences people get wrong:

- **Win+arrow, snap layouts and the maximize box still work.** The common worry that an
  undecorated window loses `WS_MAXIMIZEBOX` is not what `no-frame` does.
- **There is no non-client strip left for the OS to grab.** Nothing drags, resizes or rounds the
  window for you; every one of those is your code, and the frame you draw is the frame you must
  implement behaviour for.

`resize-border-width` is the one chrome property the backend honours (winit; not the browser
preview, where all window-management features do nothing).

## Dragging: there is no `WindowMoveArea` before 1.18

`WindowMoveArea` exists only on upstream `master`. It is **not** in 1.17.1 — the token `MoveArea`
appears in no file of any `*slint*1.17.1` crate (`i-slint-core`, `i-slint-compiler`, `slint`,
`i-slint-backend-winit`, verified against a positive control: `no_frame` and `resize_border_width`
are right there in `i-slint-core/src/items.rs`). Neither does the Rust `Window` API offer a door:
1.17.1's `WindowItem` is `width, height, safe_area_insets, virtual_keyboard_*, background, title,
no_frame, resize_border_width, always_on_top, full_screen, minimized, maximized, icon,
default_font_*` — no drag/move member, and `Window` has no `start_system_move()`.

So the drag is yours, and the shape that works is a callback carrying **movement between frames**
rather than a position: markup never learns where the window is, so it cannot misplace it.

```slint
// the band, reporting deltas only
TouchArea {
    // 1.17.x gotcha: TouchArea has NO `dragged` callback; `pressed` is a property.
    property <length> last-x, last-y;
    moved => { root.drag-delta(self.mouse-x - self.last-x, self.mouse-y - self.last-y); }
    changed pressed => { if !self.pressed { root.drag-ended(); } }
    // seed last-x/last-y in `clicked`, and remember whether anything moved
}
```

```rust
// host side: one conversion, in physical pixels
let here = window.position();                       // PHYSICAL, frame-inclusive
let (x, y) = (here.x + (dx * window.scale_factor()).round() as i32,
              here.y + (dy * window.scale_factor()).round() as i32);
window.set_position(slint::PhysicalPosition::new(x, y));   // physical in => no second conversion
```

The three traps in those six lines:

1. **Units.** `Window::position()` and `size()` return **physical** pixels (`api.rs:556-576`);
   every `length` in your markup is **logical**. Adding a logical delta to a physical position and
   writing a `LogicalPosition` back is correct at scale 1.0 and moves the window by
   `delta × scale` at 1.5 — the window outruns or lags the cursor, and no test at 100 % DPI sees it.
2. **Maximised windows report a position that is not a position.** A maximised window's
   `position()` is the invisible-border offset (`≈ -8,-8`), so "read, add, write" is arithmetically
   perfect and destroys the user's restore rectangle when the host persists the result. Guard on
   `window.is_maximized()` and refuse the drag — then tell the user, on the surface they are
   reading, not only in a log.
3. **Per-frame flood.** A real drag fires a callback per mouse-move frame. Print once per gesture,
   and ask the persistence layer to store a rect on the *release*, not per frame.

Double-click to maximize is the same band's `double-clicked` →
`window.set_maximized(!window.is_maximized())`.

## Rounded corners

Do not assume `background: transparent` gives you an alpha window. On Windows with
**`renderer-software`** it does not: the surface is presented through `softbuffer`, whose Windows
path is an opaque `BitBlt`, and `i-slint-backend-winit` consults `window_attributes.transparent` in
exactly one place — `renderer/femtovg.rs`. Measured 2026-09-14 on 1.17.1 +
`renderer-software` + Windows 11: a `no-frame` window with `background: transparent` reads
`#000000` in the region that should show the desktop, while an opaque `#101010` control reads
`#101010` in the same spot, and the antialiased edge of a drawn `border-radius` still renders
(`#E10000`). Slint computes the alpha; the last step drops it. winit *does* implement Windows
transparency (`platform_impl/windows/window.rs:1231-1244`) and Slint already requests
`with_transparent(true)` by default, so this is a renderer limitation, not a windowing one — and
it is why upstream's example, which is drawn with a GL-capable renderer, gets away with it.

Options, in order of how boring they are:

- **Ask the OS.** On Windows 11, `DwmSetWindowAttribute(DWMWA_WINDOW_CORNER_PREFERENCE /* 33 */,
  DWMWCP_ROUND /* 2 */)` rounds and antialiases the corners and needs no alpha at all;
  `DWMWCP_DONOTROUND` (1) when maximized matches native behaviour. It is
  Windows-11-only (on 10 the call fails; the window stays square) and the radius is the OS's.
  Do **not** assume this also buys the shadow people pair with the corners: on the machine this was
  written on, no shadow was measurable around a `no-frame` window *or* around an ordinary decorated
  control window, so the attribute's shadow behaviour is unverified, not verified-absent. Measure it
  against a decorated window in the same capture before repeating the claim.
- **Draw it, with a real alpha surface** — i.e. a GL/skia renderer. Costs cold start and every
  behaviour you measured on the software path.
- **`SetWindowRgn`** — works back to Windows 10, but the corners are unantialiased and the region
  must be recomputed on resize and DPI change.
