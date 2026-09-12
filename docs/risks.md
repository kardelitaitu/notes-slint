---
title: Technical risks
type: planning
owns: ['§8']
status: living
updated: 2026-09-11
---

# Technical risks

Owns **§8** of this project's plan. Part of the set indexed by
[whitepaper.md](../whitepaper.md) — section numbers are **global across the set**, so a
reference like `§4.5` resolves from any file. Each numbered section has exactly one owner;
the validator fails if one appears in two files.

## 8. Technical risks

| # | Risk | Impact | Mitigation |
|---|---|---|---|
| R1 | **GPUI as a standalone crate.** It grew up inside Zed's monorepo; building a non-Zed GPUI app outside that repo has historically been rough — unpublished deps, missing docs, API churn. | Could invalidate the whole UI choice | **Spike before anything else.** Prove a hello-world GPUI window builds on Windows from a plain `cargo` project. |
| R2 | **GPUI Windows maturity.** Zed's Windows support is comparatively recent; the renderer path differs from macOS Metal. | Rendering bugs, perf, missing features | Same spike. Pin an exact GPUI version, not a moving git rev. |
| R3 | **GPUI may not expose topmost or the raw HWND.** | Pin is a core feature | Verify in spike. Fallback: create/own the window ourselves via the `windows` crate and hand the handle to GPUI. |
| R4 | **Wayland limits (§6).** | Feature parity promise | Decide scope now, document it, don't discover it in year two. |
| R5 | **DPI awareness.** Windows per-monitor v2 DPI is easy to get subtly wrong — blurry or mis-sized window on a mixed-DPI desktop. | Restored window looks broken | Set DPI awareness explicitly at startup; test on 100/150/200% before calling M3 done. |
| R6 | **Autosave + external editor / sync folder conflict.** A OneDrive/Dropbox-synced notes dir will fight us. | Data loss | Detect external modification before write; keep a rolling backup of the last N versions cheaply. |
| R7 | **macOS notarisation lead time (§7.1).** Needs a paid Developer account, a certificate, and CI secrets — none of it code. | mac-install slips, or ships blocked by Gatekeeper | Decide who owns the Apple account now; wire signing into CI at M7, not at release day. |
| R8 | **GPUI drags in heavy system deps on Linux.** Display-server libs, fontconfig, GPU drivers. | AppImage bundling breaks on some distros | Test the AppImage on an old LTS and a current release, not just the build machine. |
| R9 | **No native file dialog in GPUI.** Open/Save As need the OS dialog. | Core menu items blocked | Verify in the M0 spike. Likely `rfd`, or Win32 `IFileOpenDialog` behind the `platform` trait. |
| R10 | **Silent corruption of foreign files.** Autosaving a file whose encoding or line endings we normalised quietly rewrites someone's repo config. | Trust damage disproportionate to effort | §4.5 do-no-harm rules, plus round-trip tests: load → save → byte-identical for a corpus of CRLF/LF/BOM/UTF-16 fixtures. |
| R11 | **Custom chrome: we draw the title bar** (decided — §10.2 resolved by ADR-0003). We own window drag, resize borders, snap layouts, double-click-maximise, and rounded corners — purchased from the gpui-kit `TitleBar` (ADR-0002) rather than built, but still owed a verification pass. | Feels broken if verification is skipped; snap layouts and caption accessibility are the usual casualties | M2 acceptance checklist against the component: drag, double-click-maximise, Win+arrow snapping, snap-layout flyout, accessibility exposure, rounded corners, DPI 100/150/200%, remote desktop, IME. |
| R12 | **A bridge interface designed from one implementation.** A `trait Bridge` guessed while GPUI is the only adapter encodes GPUI's shape into what we call generic. | The second bridge costs more than having no seam at all | Build only `bridge-gpui`. The seam is "api has no UI types", enforced by CI. Extract the trait when a second bridge is actually funded (§5.5). |
| R13 | **`platform` and the bridge both claim the window.** Topmost and positioning need a window handle, but the toolkit creates the window — and what each toolkit permits differs (§5.5). | Startup-order bugs, deadlocks, or unportable code that assumed GPUI's HWND | `platform` primitives take a handle and decide nothing; the bridge owns the window; api routes. Honour the 4-step startup order in §5.5. |
| R14 | **gpui-kit is not the crate M0 tested.** M0 verified plain `gpui =0.2.2`; `gpui-kit` 0.6.1 resolves `gpui-pre ^0.3.1` instead (ADR-0002). Every §12.3 API fact — HWND, topmost, dialogs, `WindowBounds`, decorations — is now a claim about a forked codebase. | The chrome decision's enabling premise could be stale before M2 ends | M2's opening task: re-run the six M0 checks against the kit, timeboxed, before building on them. Pin `gpui-kit = "=0.6.1"` exactly; check §12.3's facts against the pinned version's source, not memory. |

