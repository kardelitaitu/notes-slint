---
title: Architecture
type: planning
owns: ['§5']
status: living
updated: 2026-09-14
---

# Architecture

Owns **§5** of this project's plan. Part of the set indexed by
[whitepaper.md](../whitepaper.md) — section numbers are **global across the set**, so a
reference like `§4.5` resolves from any file. Each numbered section has exactly one owner;
the validator fails if one appears in two files.

## 5. Architecture

The requirement is *"core engine in Rust, UI in GPUI."* Made concrete, that is a stack in
which the UI toolkit itself is a swappable detail, behind a bridge: the engine talks to
exactly one module, and `core` depends on nothing in this repo:

```
┌───────────────────────────────────────────────────┐
│  bridge-*       one adapter per UI toolkit        │
│  bridge-gpui    views, keybindings, render loop,  │
│                 and THE TEXT BUFFER               │
│                 may import ONLY api               │
│  bridge-tauri   ...only if a second one is        │
│                 ever actually built               │
├───────────────────────────────────────────────────┤
│                                                   │
│  api            the port. one surface, both       │
│                 directions. no logic of its own,  │
│                 no UI types, and it does not know │
│                 that any bridge exists.           │
├───────────────────────────────────────────────────┤
│  core           document model, save engine,      │
│                 session state, config             │
│                 pure Rust: no UI, no OS, no       │
│                 unsafe, no gpui                   │
│                                                   │
│  platform       window geometry, topmost,         │
│                 monitor/DPI, file dialogs         │
│                 trait + per-OS backends           │
└───────────────────────────────────────────────────┘
   bridge -> api -> { core, platform }      core is independent of platform
```

**`core` and `platform` do not know about each other**, and neither knows the UI exists.
`api` is the only place their outputs are joined. That asymmetry is what makes the
gateway a single one — see §5.4.

**`core` is headless and fully unit-testable.** No GPUI import, no `windows` crate, no
`unsafe`. Save engine, atomic write, dirty tracking, and state serialisation are all
testable without ever opening a window. This is the main structural reason to insist on
the split, and it is the reason cross-platform later is a refactor of `platform` rather
than a rewrite.

**`platform` is where the OS-specific ugliness is quarantined.** A trait
(`WindowBackend`?) with a Windows implementation first. macOS/Linux are additional
implementations of the same trait later — nothing above `platform` changes.

**The bridge is thin.** GPUI views and the event loop. Business logic living here is a bug.

### State & file layout (draft)

```
%APPDATA%\notes-gpui\
  settings.toml      # deliberate prefs: autosave on/off, interval, recent-files list (≤10)
  session.json       # restoration state: window rect, monitor id, scale, maximized,
                     #   pinned, last-open file path
  notes\             # default save location for new files, if the user lets us pick one
```

`session.json` is written by the core engine, not by the windowing layer, so it can be
tested without a display.

**Portable builds change this path.** A portable app must leave nothing behind in
`%APPDATA%` — that is the entire point of shipping one. So state resolution is a rule,
not a constant:

```
if  <exe dir>\data\ exists   →  use <exe dir>\data\      (portable mode)
else                         →  use %APPDATA%\notes-gpui  (installed mode)
```

Same rule on every OS. This belongs in `core` as a single `resolve_state_dir()` so the
behaviour is testable and identical everywhere. Cheap to add now; painful to retrofit
once users have notes in a location we chose for them. See §7.

### 5.1 Repository layout

One binary, five crates, mirroring the layering above. Everything else in the tree
exists to serve the five packaging artifacts or the do-no-harm rule.

```
notes-gpui/
├── README.md                  # human-facing intro
├── AGENTS.md                  # conventions + invariants for anyone working here
├── whitepaper.md              # this file — exactly one, living, never forked
│
├── .agents/                   # agent-facing project memory (§5.6)
│   ├── skills/
│   │   └── doc-management/
│   │       ├── SKILL.md       #   where every document goes, and why
│   │       ├── templates/note.md
│   │       └── scripts/check-docs.ps1   #   those rules, enforced
│   └── notes/
│       ├── proposed/          #   ideas on the table
│       ├── implemented/       #   built — code exists and works
│       ├── rejected/          #   decided against, with the reason
│       └── archived/          #   superseded or obsolete
│
├── Cargo.toml                 # workspace + [profile.release] lto/strip — cold-start budget
├── Cargo.lock
├── rust-toolchain.toml        # pin it; GPUI API churn is a known risk (R1)
├── .cargo/config.toml
├── .github/workflows/
│   ├── ci.yml                 #   fmt, clippy, test, the xtask gates, docs — Windows only
│   └── (release.yml)          #   NOT BUILT. Nothing builds the 5 artifacts on tag yet (§7).
│
├── crates/
│   ├── core/                  # PURE RUST. No gpui, no windows, no unsafe.
│   │   ├── src/               #   ten modules, none of them allowed to see a window
│   │   │   ├── lib.rs         #   the purity invariant, in the file the linter reads
│   │   │   ├── document.rs    #   buffer, revision counter, dirty tracking
│   │   │   ├── format.rs      #   .notes: frontmatter parse/serialise (§4.5)
│   │   │   ├── encoding.rs    #   UTF-8/16, BOM, EOL — detect once, write back same
│   │   │   ├── geometry.rs    #   Rect in FRAME pixels — client space drifts 8/19 px (§12.2)
│   │   │   ├── save.rs        #   atomic: temp → fsync → rename
│   │   │   ├── session.rs     #   rect, monitor id, scale, maximized, pinned, last file
│   │   │   ├── recent.rs      #   MRU ≤10, canonical-path dedupe, missing-file state
│   │   │   ├── settings.rs    #   deliberate prefs
│   │   │   ├── path_policy.rs #   name-only verdicts — no fs, no canonicalisation, no timeout
│   │   │   └── paths.rs       #   resolve_state_dir() — portable vs installed
│   │   └── tests/
│   │       ├── roundtrip.rs   #   load→save byte-identical (R10)
│   │       └── fixtures/      #   CRLF / LF / BOM / UTF-16LE / empty / no-trailing-newline
│   │
│   ├── platform/              # OS seams. Trait + cfg-gated backend modules.
│   │   ├── src/
│   │   │   ├── lib.rs         #   WindowBackend trait, geometry types
│   │   │   ├── geometry.rs    #   rect/monitor/scale — ONE documented unit (§4.1)
│   │   │   └── windows/       #   #[cfg(windows)]
│   │   │       ├── mod.rs     #   the one module tree allowed unsafe; HWND in, answer out
│   │   │       ├── topmost.rs #   SetWindowPos + SWP_NOACTIVATE (§4.3)
│   │   │       ├── monitors.rs#   EnumDisplayMonitors, per-monitor DPI (R5)
│   │   │       ├── corners.rs #   DWM corner attribute — one call, it reports the OS answer
│   │   │       ├── file_drop.rs#   shell paths in; decides nothing about what they are
│   │   │       └── paths.rs   #   volume identity of an OPEN handle — immune to a rename
│   │   └── Cargo.toml
│   │
│   ├── api/                   # THE GATEWAY (§5.4). Commands in, Events out.
│   │   └── src/               #   the vocabulary is FROZEN; the lifecycle is what a bridge wires
│   │       ├── lib.rs         #   Gateway: start(), send(Command), events()
│   │       ├── gateway.rs     #   the port's only handle — two channels, one engine thread
│   │       ├── command.rs     #   UI → engine, one variant per user intent
│   │       ├── event.rs       #   engine → UI, incl. async failures (§5.4)
│   │       ├── engine.rs      #   the worker loop; owns core + platform handles
│   │       ├── file_drop.rs   #   arm_file_drop(): the one synchronous call, rule 4
│   │       └── dto.rs         #   types the UI may see — re-exported, never leaked
│   │
│   ├── bridge-gpui/           # THE FIRST BRIDGE (§5.5). gpui-kit (ADR-0002). FROZEN (ADR-0006).
│   │   ├── src/               #   flat — these four files are the whole crate, no views/ subtree
│   │   │   ├── main.rs        #   #![windows_subsystem = "windows"] — no console window
│   │   │   ├── editor.rs      #   OWNS THE TEXT BUFFER — model, paint, focus, the IME seam
│   │   │   ├── menu.rs        #   the chord table — the bar is stored, never drawn (Windows)
│   │   │   ├── titlebar.rs    #   custom chrome — ADR-0003, LEFT + CENTRE regions only
│   │   ├── build.rs           #   a no-op now: gpui's own RT_MANIFEST took the embed
│   │   ├── app.manifest       #   DPI + long paths; the input the post-link step writes back
│   │   └── tests/             #   editor_roundtrip, ime_seam, paint_cost
│   │
│   ├── bridge-slint/          # THE SHIPPED BRIDGE (§5.5). Slint; the product is notes-slint.exe.
│   │   ├── src/               #   two [[bin]] roots — the four modules below compile into BOTH
│   │   │   ├── product.rs     #   bin notes-slint — a root that owns no policy
│   │   │   ├── probe.rs       #   bin notes-slint-probe — the instrument that earned it
│   │   │   ├── surface.rs     #   the pure decisions both roots must not disagree about
│   │   │   ├── plumbing.rs    #   needle prefix, send(), the title strip and unsaved dot
│   │   │   ├── title_contract.rs#   the title bar's text half, with no toolkit in it
│   │   │   └── ui_gen.rs      #   the slint! entry — imports ../ui/main.slint
│   │   ├── ui/                #   main.slint, chrome.slint, theme.slint, icons/*.svg
│   │   └── tests/             #   editor_roundtrip, panic_hook
│   │
│   ├── xtask/                 # REPO AUTOMATION (§5.2). Builds no product; imports no UI.
│   │   ├── src/               #   one module per gate; main.rs holds the roster
│   │   │   ├── main.rs        #   subcommand dispatch + the usage text
│   │   │   ├── arch.rs        #   check-arch — the layering gate, rules and their limits (§5.2)
│   │   │   ├── check.rs       #   check — the AGENTS.md gate as one command
│   │   │   ├── check_ci.rs    #   check-ci — ci.yml must run exactly that roster
│   │   │   ├── check_unsafe.rs#   check-unsafe — the ledger keeping unsafe in platform
│   │   │   ├── deps.rs        #   check-deps — member manifests match the workspace template
│   │   │   ├── fixtures.rs    #   fixtures verify — the R10 corpus is byte-exact vs manifest
│   │   │   ├── identity.rs    #   proves the checker binary is the code it claims to be
│   │   │   ├── manifest.rs    #   manifest — app.manifest into the exe post-link, then read back
│   │   │   ├── metadata.rs    #   cargo metadata, shared; never `cargo tree -i`
│   │   │   └── smoke.rs       #   smoke — drives a real window, greps its needles (§9)
│   │   └── Cargo.toml
│   │
│   └── (bridge-tauri/)        # NOT BUILT. A seam, not a task — see §5.5.
│
├── assets/                    # icons/ holds one .svg; .ico/.icns/.png are NOT BUILT (§7)
├── (packaging/)               # NOT BUILT. §7's five artifacts are a plan, not a tree.
│   ├── windows/               #   WiX .wxs or Inno .iss — NOT MSIX (§7)
│   ├── linux/                 #   deb control, .desktop, AppImage recipe
│   └── macos/                 #   Entitlements.plist, dmg settings, notarise script (§7.1)
├── docs/                      # INTENT: planning & developer documentation
│   ├── README.md              #   the docs map, and the tense rule (§5.6)
│   ├── dev/                   #   how to build/test/package - written once code exists
│   └── decisions/             #   ADRs, one per settled decision (§5.6)
└── (scripts/)                 # NOT BUILT. Its duties landed as cargo xtask subcommands (§5.2)
```

### 5.2 Rules that keep this honest

**1. The layering must be machine-enforced, or it is a suggestion.** A `core` that
accidentally imports GPUI is one convenience commit away, and then `core` is no longer
headless or testable and the cross-platform plan becomes fiction. Make CI fail on it:

```
cargo tree -p core -i gpui          # must be empty — core is pure
cargo tree -p core -i windows       # must be empty — core has no OS
cargo tree -p core -i platform      # must be empty — core is independent of platform
cargo tree -p api  -i gpui          # must be empty — the port is UI-agnostic
cargo tree -p bridge-gpui -i core      # must be empty: a bridge may only see api
cargo tree -p bridge-gpui -i platform  # must be empty: ditto
```

The last two are the interesting ones: they make *"one gateway"* a fact rather than a
convention. A bridge physically cannot reach around `api` without failing the build.

**2. `platform` backends are cfg-gated modules, not separate crates.** `platform-windows`
/ `platform-macos` / `platform-linux` as three crates sounds tidier, and costs a separate
workspace build per OS plus duplicated trait plumbing. One crate with
`#[cfg(windows)] mod win;` is simpler, and is revisitable if compile times ever complain.

**3. The M0 spike does not live in this tree.** Do it in a throwaway directory. Spike code
is the most dangerous kind of code — it works, so people keep building on it instead of
deleting it. The spike's only output is a yes/no answer plus a note on which GPUI version
and which dialog crate worked. The real tree is then started clean.

**4. `api` knows nothing about bridges.** It must compile with zero UI crates present, and
there is no `#[cfg(feature = "gpui")]` anywhere in it. If `api` starts branching on which
bridge is attached, the seam is gone.

**5. `api` contains no business logic.** It translates and routes. The moment a rule like
*"don't autosave foreign files without an explicit save"* lives in `api/engine.rs`
instead of `core/autosave.rs`, the gateway has become a fourth domain layer and the
testability of `core` is quietly compromised. Review for this.

### 5.3 Two structural notes

- **DPI awareness does travel in the exe's application manifest (R5) — the plan's error
  was the WHEN, never the fact.** Per-monitor v2 must be declared there, not called at
  runtime: a runtime call is too late once the process is already DPI-scaled, and with
  nothing declared Windows virtualises every coordinate it reports (M0's D1/D9). No build
  script puts it in. gpui's own `windows-manifest` feature is in its DEFAULT list and is
  not subtractable from outside, so the linker always embeds *a* manifest — GPUI's,
  which drops `longPathAware` and the `system` DPI fallback. `cargo xtask manifest`
  therefore replaces that resource AFTER the link, from `crates/bridge-gpui/app.manifest`,
  and then proves itself by reading the exe back for its markers rather than trusting its
  own run: 0 = replaced and verified, 20 = no `mt.exe` (no Windows SDK), 21 = the
  read-back refused to judge, 22 = the work could not run. There is deliberately no
  "already fine, did nothing" path. The icon and the version info — the other half of the
  old sentence — are still owed to M5a (§7), where `winres` is one door under discussion
  and not a dependency this repo has.
- **The shipped binary's name is load-bearing.** Portable mode resolves state relative to
  the exe's directory (§5), so renaming the binary relocates the user's notes. Pin it
  early and keep it stable across all five artifacts.

### 5.4 The API gateway

`crates/api` is the single surface the UI is allowed to speak to. One crate, one module
tree, both directions. It is the right call for three reasons, and it forces one design
decision that is easy to get wrong.

**Why it's worth a crate.**

- **The UI becomes replaceable.** R1–R3 say GPUI on Windows is an unproven bet for us. If
  the *only* contract between our logic and our UI is `api`, then losing that bet means
  rewriting the bridge, not the project. That insurance is nearly free to take now and very
  expensive to buy later.
- **The whole app is testable headlessly.** Feed `Command`s, assert `Event`s. No window, no
  GPUI, no display server — a "play a session" integration test runs in CI on all three
  OSes. And because `core` is pure and fast, **tests use the real engine; no mocks.**
- **One place to look.** Every capability the UI can reach is one file. Feature planning
  becomes "what does `Command` need?" instead of spelunking four crates.

**The decision it forces: autosave breaks the request/response model.**

A gateway that is just methods returning `Result` cannot express this app. Autosave
happens seconds after any UI call, on a worker thread, and *can fail* — read-only file,
permission denied, disk full, OneDrive lock. There is no caller waiting for that `Err`.
So the gateway must be **command/event**, not call/return:

```rust
// UI → engine. One variant per user intent. Fire-and-forget.
enum Command {
    Open(PathBuf),
    SaveAs { path: PathBuf, text: String, revision: u64 },
    Flush { text: String, revision: u64, epoch: u64 },  // Ctrl+S and every autosave trigger
    SetAutosave(bool),
    SetPinned(bool),
    ClearRecents,
    Shutdown,
}

// engine → UI. Delivered on the UI thread. Includes things with no caller.
enum Event {
    Loaded { path: PathBuf, text: String, meta: FileMeta },
    Saved  { path: PathBuf, revision: u64 },
    SaveFailed { path: PathBuf, revision: u64, reason: SaveError },  // ← the async one
    ExternalChange { path: PathBuf },
    AutosaveSkipped { reason: SkipReason },   // disarmed foreign file - ADR-0001
    RecentsUpdated(Vec<RecentEntry>),
}
```

`FileMeta` is where §4.5's do-no-harm data surfaces: encoding, line ending, trailing
newline, read-only flag, and the size-guard verdict. The UI needs read-only and
oversize to render honestly, and `status.rs` needs `SaveFailed` to go amber (§4.4).

**The thread boundary lives here, and it is the sharp edge.** `engine.rs` owns a worker
thread; GPUI runs its logic on its own thread. Events must be marshalled onto the GPUI
thread before touching any view. Two rules: **never block on a channel inside a GPUI
frame**, and **never call into `api` from the worker thread**. Get either wrong and you
get a deadlock that only appears under load. This is the main cost of the design, and it
is paid once.

**The second decision: who owns the live text buffer?**

- **(i) `core` owns it** — every keystroke crosses the gateway. Purest, but chatty, and
  typing latency now depends on a thread hop.
- **(ii) the bridge owns it** — GPUI's text buffer is the live document; `core` owns the *file*
  and receives a snapshot on `Flush`. Gateway stays low-traffic, edits never cross it.

Recommendation: **(ii)**. It keeps keystrokes off the channel and keeps the gateway small.
The cost is that dirty state is split — the bridge knows the buffer changed, `api` knows what
was last written — reconciled by the `revision` counter on `Flush`. Open decision §10.4.

**What it costs, honestly.** Indirection: every feature touches two enums before it
touches logic. Drift risk: `Command`/`Event` lag behind what the UI actually needs, and
someone reaches around the gateway to fix it — which is exactly what the `cargo tree -p
app -i core` check in §5.2 exists to prevent. And a real temptation to let logic pile into
`engine.rs`, hence rule 4.

### 5.5 Bridges: one port, many UIs

A **bridge** is an adapter that owns a UI toolkit end to end and speaks to nothing else in
this repo except `api`. **Two are built; one of them is past tense.** `bridge-slint` ships
the product — `notes-slint.exe`, beside the instrumented probe that earned it — while
`bridge-gpui` is FROZEN (ADR-0006): still built, still cited as evidence, no new needles.
`bridge-tauri`, `bridge-egui`, or `bridge-objc` would come later only if a toolkit ever
had to be replaced. One adapter per toolkit is what keeps the toolkit a swappable detail.
`api` is the port, bridges are the adapters, and the dependency arrow only ever points one
way: bridge -> api. The port never learns that any bridge exists (rule 4, §5.2).

**The toolkit `bridge-gpui` speaks is `gpui-kit`, not bare `gpui`** — ADR-0002, taken with
the §10.2 chrome resolution (ADR-0003). The kit wraps GPUI plus a component layer, and that
layer's `TitleBar` is what turns custom chrome from a subsystem into a dependency. No rule
above bends: the kit *is* the bridge's toolkit, the import edge is still exactly one, and
§5.2's `cargo tree` checks are unchanged. What does bite: M0 tested plain `gpui 0.2.2`,
the kit resolves `gpui-pre` — see R14 and the amendment at §12.4.

**This works only because the UI surface is small — and it is worth seeing why.** The entire
vocabulary a bridge must carry is: text plus file metadata down, and edit / menu / pin
intents up. That is a genuinely narrow interface. A richer app could not abstract its UI
this way. This one can, because the product is a buffer and a menu.

**What cannot be abstracted: the text editor.** Cursor, selection, IME, undo, soft wrap,
scrolling and multi-byte input are not a widget you can hide behind a trait. GPUI ships a
text buffer; Tauri inherits one from the browser; egui is weak here. If we tried to abstract
the editor we would be writing a UI toolkit, and would finish it sometime around 2029. So:

> **The bridge owns the editor. `api` never sees a keystroke** — only `Flush { text }`.

Note the dependency this creates: it makes **§10.4 resolving to "the UI owns the buffer"**
effectively mandatory. If `core` held the live buffer, every keystroke would cross the port,
and the port would have to abstract text editing — the one thing above that is impossible.
The bridge idea and the buffer-ownership decision stand or fall together.

**The hard part: whoever owns the window owns the window.** `platform` has real window
duties (topmost, position, restore) but it does not create the window — the toolkit does.
And what a toolkit allows varies per bridge: GPUI creates an HWND we can hand to
`SetWindowPos`, while Tauri owns its window *and* exposes always-on-top itself, making a
`platform` call both impossible and wrong. So the division has to be:

- `platform` provides **OS primitives that take a window handle**, and decides nothing.
- The **bridge owns the window** and its lifecycle.
- `api` **routes**, holding the handle the bridge registered.

Which yields a startup order that must be respected, and is easy to get wrong:

```
1. bridge  -> api      query saved session   (rect, scale, maximized, pinned)
2. bridge             create the window AT that rect   <- only the bridge can do this
3. bridge  -> api      register the window handle
4. api     -> platform apply topmost(handle)
```

Step 1 is a synchronous query — the one pragmatic exception to the pure command/event rule,
since the window does not exist yet and there is nowhere to deliver an event. Step 2 is why
geometry *restore* is bridge work while geometry *storage* is `core` work. This is the part
of the design a second bridge will most likely force us to revisit.

**Do not build the abstraction yet.** That is the trap inside the idea. A `trait Bridge`
designed while GPUI is the only implementation will quietly encode GPUI's shape — its
entity types, its frame loop, its thread model — into whatever you labelled "generic", and
the second bridge will then cost more than having no seam at all. The cheap seam is the one
we already have: `api` containing no UI types, enforced by CI. That is all the abstraction
this phase needs. Let a real second implementation **extract** the interface from
comparison, rather than guessing it now from a sample of one.

**A reality check on `bridge-tauri` specifically.** Naming it changes §2. Tauri means a web
frontend, a JS toolchain in the build, and a webview with a separate renderer process —
which is the architecture §2 pitches against ("no Electron-sized footprint, no renderer
process", cold start under ~200ms). Tauri is far lighter than Electron, but it still ships a
renderer and a bundle. Keep it as insurance against R1–R3; if it ever becomes the actual
plan, §2 needs rewriting, not just a new crate.

### 5.6 Documentation and project memory

The code will accumulate decisions faster than anyone remembers them, so the docs have a
type system of their own — location carries meaning, and a validator enforces it.

**There are two trees, split by tense rather than by topic:**

| Tree | Holds | Tense | When it disagrees with the code |
|---|---|---|---|
| `docs/` + `whitepaper.md` | **planning and developer documentation** — what we intend to build, and how to work on it | future / present | **the code wins** — edit the doc |
| `.agents/notes/` | **decision and implementation records** — whether a thing was decided, built, refused, or retired | past | the record stands; a newer note supersedes it |

| Kind | Lives in | Character |
|---|---|---|
| The founding sketch | `whitepaper.md` | one, living, never forked |
| How to build, test, package the code | `docs/dev/` | present tense — **not writable before the code exists** |
| Settled architecture decisions | `docs/decisions/` | ADRs — **append-only** |
| Ideas under discussion | `.agents/notes/proposed/` | working memory, moves as it matures |
| Built, and how it actually works | `.agents/notes/implemented/` | living until removed |
| Decided against, with the reason | `.agents/notes/rejected/` | near-frozen |
| Superseded or obsolete | `.agents/notes/archived/` | frozen |
| Conventions for whoever works here next | `AGENTS.md` | living |
| Reusable agent capability | `.agents/skills/<name>/` | living |

**A plan is not a dev doc.** `docs/dev/` describes code that exists; `whitepaper.md`
describes intent. Promote a plan into `docs/dev/` when it is implemented, not when it is
agreed — which is why that folder currently holds only its map and a schedule of which file
unlocks at which milestone. Writing it early produces a confident description of something
that may never be built.

**The distinction that earns its keep is note vs ADR.** A note is disposable: it argues,
it changes status, it can be deleted. An ADR is a court record: immutable, and edited only
by superseding it with a numbered successor. Confusing them is how repositories end up with
a permanent "decision" file full of live speculation, and a speculating note nobody dares to
edit. When a note gets a real decision, it graduates into an ADR and the note archives.

**Statuses are directories, not tags**, so the state of an idea is visible in the file tree
and `git log --stat` reads like a decision history. The cost is that moving a note is a
three-part edit (file, `status:`, and `whitepaper.md` §10 if the product changed), and a
half-done move leaves a note with a stale status that someone will cite as fact — which is
why the validator checks folder/status agreement specifically.

> `.agents/skills/doc-management/scripts/check-docs.ps1` validates placement, filenames,
> frontmatter, duplicate ids, status/folder agreement, and every `§` cross-reference. It
> runs in `ci.yml`. The full map is in that skill's `SKILL.md`.

**One acknowledged gap:** the four statuses have no state for *decided but not built*, which
is where most of §10 "Resolved" currently sits. `implemented` is defined strictly as "the
code exists and works", and accepted-but-unbuilt decisions stay in `proposed/` with a
`DECIDED:` line, mirrored in §10. If that gets awkward, add a fifth status by proposal —
do not quietly redefine `implemented`, because a folder whose meaning drifts is worse than
no folder at all.

