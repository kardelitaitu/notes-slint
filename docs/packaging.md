---
title: Packaging and distribution
type: planning
owns: ['§7']
status: living
updated: 2026-09-10
---

# Packaging and distribution

Owns **§7** of this project's plan. Part of the set indexed by
[whitepaper.md](../whitepaper.md) — section numbers are **global across the set**, so a
reference like `§4.5` resolves from any file. Each numbered section has exactly one owner;
the validator fails if one appears in two files.

## 7. Packaging & distribution

Five artifacts on the final roadmap. They are not five equal amounts of work, and two of
them have external prerequisites that cost money and lead time.

| Artifact | Format | Notes |
|---|---|---|
| **win-install** | MSI (WiX) or Inno Setup | **Not MSIX.** Package identity gives filesystem virtualisation and uninstall-on-update that fight a portable-state app and a tray/hotkey app. Classic installer. |
| **win-portable** | Single `.exe` | Zero install, state lives beside the exe (§5). Also the best artifact for testing and for people who just want it to work. |
| **linux-install** | `.deb` (+ `.rpm` if demand) | Needs a real `.desktop` file, icon, and MIME hints or it won't integrate with the DE. |
| **linux-portable** | AppImage | Self-contained, but GPUI pulls in display-server, fontconfig and graphics libs — bundling those correctly is the fiddly part. |
| **mac-install** | `.dmg` wrapping a `.app` | See the signing gate below. No mac-portable requested; a `.app` in a `.dmg` is already effectively portable, which is presumably why. |

### 7.1 The macOS signing gate — start early

This is the item most likely to embarrass us, because it is **not a code problem and has
a lead time measured in weeks**:

- Apple **notarisation** is required for an app downloaded from the internet to open at
  all. Unsigned or un-notarised → Gatekeeper blocks it with a message most users cannot
  work around.
- Notarisation requires an **Apple Developer Program account (US$99/yr)** and code
  signing with a Developer ID certificate + notarisation via `notarytool`.
- CI needs the certificate and an API key as secrets, and the build must be signed
  *before* notarisation and re-stapled *after*.

**Action:** someone needs to own the Apple Developer account decision now, not at M6.

### 7.2 Windows signing — smaller, but real

An unsigned exe triggers SmartScreen's "Windows protected your PC" blue screen. It's
dismissible, but for a note app you're asking people to run all day, it's a trust hit.
A standard OV certificate removes most of it; an EV certificate removes the warning
immediately but costs more and is increasingly being replaced by reputation accrual.
Decision deferred to §10 — but it's a budget line, not an engineering task.

### 7.3 Linux: sandboxing conflicts with our own features

If we ever ship via **Flatpak**, be aware it directly undermines the product:

- Global hotkeys and system-wide tray icons do not work from inside the sandbox.
- Window positioning and always-on-top are already restricted on Wayland (§6); a sandbox
  adds another layer of "the compositor decides."

A native `.deb`/`.rpm` or an AppImage does not have this problem. Recommendation:
**treat Flatpak as opt-in later, and say plainly that the pinned/hotkey experience is
degraded in the sandboxed build** rather than shipping a broken-feeling app.

### 7.4 Build & release mechanics

- **Build natively per OS in CI** (a GitHub Actions matrix on `windows-latest`,
  `ubuntu-latest`, `macos-latest`). Cross-compiling a GPUI app — especially *to* macOS,
  which needs an SDK and a universal binary — is more pain than the three free runners
  are worth.
- **macOS: ship a universal binary** (`arm64` + `x86_64`) unless we decide otherwise;
  lipo-ing two target builds is straightforward and avoids asking users their architecture.
- **Linux glibc floor:** an AppImage built on a current distro won't run on an older one.
  Build on an old baseline (or use `zig cc` to target an explicit glibc version) and
  state the minimum supported glibc.
- **Versioning:** one semver string driving git tag, binary metadata, installer version,
  and the in-app about box. Set this up in M5, not at first release.
- **Auto-update is not in the five artifacts** but users will expect it. Open decision §10.

