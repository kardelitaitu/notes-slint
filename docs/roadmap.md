---
title: Roadmap
type: planning
owns: ['§9, §12']
status: living
updated: 2026-09-11
---

# Roadmap

Owns **§9, §12** of this project's plan. Part of the set indexed by
[whitepaper.md](../whitepaper.md) — section numbers are **global across the set**, so a
reference like `§4.5` resolves from any file. Each numbered section has exactly one owner;
the validator fails if one appears in two files.

## 9. Milestones

**M0 — Spike (do this first, timebox it).**
Plain `cargo` project, GPUI dependency, window opens on Windows. While there: confirm we
can obtain the raw window handle, set topmost, set an explicit position, and invoke a
native file dialog. **Exit criterion: yes/no on GPUI viability.** If no, we stop and pick
a different UI layer while the cost of changing our mind is still zero. Everything
downstream depends on this.

**M1 — Core engine, headless.**
`core` + `api` crates: document model, open/save/save-as with path rebinding, encoding
+ line-ending detection and preservation, atomic save engine, dirty tracking, recent-files
MRU, session/config serialisation, `resolve_state_dir()`, and the `Command`/`Event` gateway
(§5.4). Zero UI. Fully unit-tested, plus a headless "play a session" test driven through `api`.

**M2 — First usable UI (`bridge-gpui`).**
`bridge-gpui`: editor view, hamburger menu, Open / Save / Save As, native file dialogs,
recent files, drag-and-drop, and reopen-last-file on launch. The bridge imports only `api`,
and the §5.2 checks start running here so the boundary cannot drift later. Follow the
startup order in §5.5. **Exit criterion: you can use it as a notepad.** The chrome decision
(§10.2) is already made — ADR-0003, enabled by ADR-0002 — so M2 opens by re-running the six
M0 checks against gpui-kit (R14), then builds the title bar and runs R11's verification
checklist.

**M3 — Window persistence.**
`platform` trait + Windows backend. Restore, validate against monitors, clamp off-screen,
handle maximised + DPI. Test on 100/150/200% scaling and with a monitor unplugged.

**M4 — Autosave & pin.**
Debounce, periodic flush, blur/close/quit flush, external-change detection, and the
byte-identical round-trip test suite (§4.5). Pin button + shortcut + persisted state.

**M5 — Polish & package (Windows).**
Cold-start time check, icon, crash handling, settings UI, `.notes` file association. Then
the two Windows artifacts: portable `.exe` and MSI installer. **Ship both from CI on every
tag from here on** — packaging that is only done at release time is always broken at
release time.

**M6 — Cross-platform seams.**
Audit `platform` for Windows leakage; macOS backend; Linux decision from §10.

**M7 — mac-install.**
Apple Developer account + certificate + notarisation pipeline (§7.1), universal binary,
`.dmg`. Longest lead time of any milestone and the least code in it — start the account
decision during M5.

**M8 — linux-install + linux-portable.**
`.deb` with proper desktop integration; AppImage with bundled GPUI system deps and a
stated glibc floor (§7.4). Test matrix: one old LTS, one current release.

M1–M4 are roughly independent and could interleave, but M2 is the one that turns this
from a library into an app — worth reaching early. M0 gates all of them. M7's
*administrative* work gates nothing and should start early, because it is the only
milestone with a lead time we don't control.


---

## 12. M0 spike results

Run 2026-09-10 against **gpui 0.2.2** on Windows 11 (10.0.26200), rustc 1.98.0. Built in a
throwaway directory outside this repo, per §5.2 rule 3 — only its findings are recorded here.

### 12.1 Verdict

**GPUI is viable. Proceed to M1.** Every blocking question came back yes. R1, R2 and R3 are
resolved; nothing found suggests re-choosing the UI layer.

| | Question | Result |
|---|---|---|
| Q1 | Plain `cargo` project depending on gpui compiles and links on Windows | **PASS** — 441 crates, ~5 min cold, 1m10s incremental |
| Q2 | A window opens and enters the render loop | **PASS** |
| Q3 | Raw HWND reachable | **PASS** |
| Q4 | Topmost set **and cleared** | **PASS** — `WS_EX_TOPMOST` verified on, then off |
| Q5 | Explicit position/size honoured | **PASS with a finding that matters** — §12.2 |
| Q6 | Native file dialogs | **PASS at the API level** — exist and link; *presenting one still needs a human* |

### 12.2 The one real finding: two coordinate spaces

Requested `(120, 90)` sized `480x320`. Observed:

```
client area    480 x 320        <- exactly what was asked for
frame rect     origin (112, 71), size 496 x 359
chrome deltas   left 8   top 19   right 8   bottom 20
```

**gpui `Bounds` is client space. Win32 `GetWindowRect` is frame space.** They differ by the
window chrome, which is *not symmetric* — 19px above, 20px below, 8px each side, because
Win11 keeps invisible resize borders.

This is exactly the failure §4.1 predicted, now with a measured size. **Persist the frame
rect and restore it as gpui client bounds and the window drifts 8px left and 19px up on
every launch** — a tenth of the screen inside ten sessions, while the app confidently
reports that it remembered. It would have surfaced in M3 as an intermittent, hard-to-
attribute bug. Found on day one, for free.

**Consequences:**

- `platform/geometry.rs` must declare **one** persisted coordinate space and convert at
  both ends. Its "ONE documented unit" comment is now load-bearing, not tidy.
- Recommend persisting **frame** coordinates — that is what "where the window is" means to
  a user, and what §4.1 monitor-clamping must compare against — converting to client space
  only at `open_window`.
- Chrome deltas must be measured per window, never assumed constant: they vary with DPI,
  theme, and whether custom chrome (§10.2) is in effect.
- A regression test must assert a **restore → read-back round trip is stable across two
  launches**. Single-launch correctness cannot detect drift.

### 12.3 API facts worth keeping

- **No topmost API in gpui** — confirmed by source inspection. Pin is our Win32 code
  against the handle, which is exactly the split §5.5 assumed (`platform` primitives take a
  handle, the bridge owns the window). The design held.
- **`WindowBounds` is already `Windowed | Maximized | Fullscreen`**, so §4.1's "persist
  restored bounds *and* a separate maximised flag" maps onto the API 1:1. No workaround.
- **`window_decorations` supports server or client decorations** — the §10.2 custom-chrome
  option is available, not forced. R11 stays a decision rather than a constraint.
- **Dialogs are built in** (`PathPromptOptions`, `prompt_for_paths`, `prompt_for_new_path`):
  no `rfd`, no second event loop owning a window. R9 resolved.
- **`Context::spawn` takes `(WeakEntity<T>, &mut AsyncApp)` and `AsyncApp` has no `quit()`.
  The obvious "quit after the first frame" route does not exist; find another before M2.
- **The Windows backend is native**, not a shim: `directx_renderer.rs`, `direct_write.rs`,
  `directx_devices.rs`, `vsync.rs`, `display.rs` and more. That is what de-risks R2.

### 12.4 Watch items

- Pin **`gpui = "=0.2.2"`** and record why. 0.1.x is yanked and the API moved under it —
  R2's mitigation, now actionable. *(Amended 2026-09-11: the bridge depends on
  **`gpui-kit = "=0.6.1"`** per ADR-0002, which resolves `gpui-pre ^0.3.1`, not this gpui.
  The pin guidance now applies to the kit, recording its resolved `gpui-pre` version
  alongside; everything in §12.3 gets re-verified under R14 before M2 builds on it.)*
- A transitive dep (`proc-macro-error2 v2.0.1`) emits *future-incompat:*; harmless today,
  check on every toolchain bump.
- 441 crates x 3 OSes is a slow CI matrix. Add registry and `target/` caching to `ci.yml`
  at M1 or every run pays for it.
- **Two checks remain undone:** presenting a dialog for real, and a human looking at whether
  the painted frame is correct. Both are minutes, and both belong before M2 is called done.

### 12.5 Re-verification against gpui-kit (2026-09-12)

R14's opening task: the six M0 checks re-run against the crate the bridge actually depends
on (ADR-0002), not the gpui 0.2.2 that §12 verified. Same method as §12 — a throwaway
plain-cargo project outside this repo, one binary printing one PASS/FAIL line per check to
stderr — built against **`gpui-kit =0.6.1`**, which resolves **`gpui-pre 0.3.4`** (via
`gpui-base`/`gpui-component`/`gpui-kit-assets` 0.6.1), rustc/cargo 1.98.0, Windows 11 (10.0.26200).
The spike source lives in that throwaway directory by design and is not committed; per
§5.2 rule 3 only its findings are recorded here.

| | Question (as §12.1) | Result |
|---|---|---|
| Q1 | Plain `cargo` project compiles and links on Windows | **PASS** — full gpui-kit/gpui-pre tree builds and links; final incremental link 2.2 s |
| Q2 | A window opens and enters the render loop | **PASS** |
| Q3 | Raw HWND reachable | **PASS** — `HasWindowHandle` on `gpui::Window` (delegates to the platform window), hwnd `0x7e0cc0` |
| Q4 | Topmost set **and cleared** | **PASS** — `WS_EX_TOPMOST` verified on (exstyle `0x240108`), then off (`0x240100`) |
| Q5 | Explicit position/size honoured | **PASS** — creation bounds honoured; `SetWindowPos (260,180)` → `GetWindowRect (260,180)` |
| Q6 | Native file dialogs | **PASS at the API level** — `cx.prompt_for_paths` accepted, oneshot receiver returned; presenting still needs a human |

Every check passed, so at kit 0.6.1 / fork 0.3.4 R14's stale-premise risk does not
materialise, and the chrome work can build on §12.3 as written. Three facts sharpened:

- **§12.2's coordinate-space finding reproduces exactly.** The same client request
  `(120,90) 480x320` produced the same frame rect `(112,71) 496x359` — chrome deltas
  8/19/8/20 are a fork property too, so `platform/geometry.rs`'s one-coordinate-space
  rule carries over unchanged.
- **The §12.3 API facts hold under the fork**, with one migration note: `Application::new`
  is gone; the entry point is `gpui_kit::platform::application()` (picks the platform
  backend) plus `gpui_kit::init(cx)` before first render. HWND reachability, `WindowBounds`,
  built-in dialogs, and the native DirectX backend (`gpui-pre-windows`: `directx_renderer.rs`,
  `direct_write.rs`, `directx_devices.rs`, `vsync.rs`) all verified in the pinned source.
- **§12.4's two undone checks remain undone** — presenting a dialog for real, and a human
  eyeballing the painted frame, are still owed before M2 is called done.


