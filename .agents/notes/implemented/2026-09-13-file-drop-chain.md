---
title: File drop onto the window, the whole chain
status: implemented
id: 2026-09-13-file-drop-chain
created: 2026-09-13
updated: 2026-09-13
relates: [§4.4, §5.4, §5.5]
decision: docs/decisions/0004-the-port-grows-a-second-synchronous-thread-affine-surface.md
---

Read this next to `crates/platform/src/windows/file_drop.rs`, `crates/api/src/file_drop.rs` and
`crates/api/src/engine.rs` — the comments at those three sites are the authority on what the code
claims. This note is the provenance, and the part that has to stay embarrassing is the pricing: the
plan called this feature **"nearly free to implement"** (`docs/features.md` §4.4), and on **both**
toolkits in this repo that was false. It was false in two different, instructive ways, so both are
recorded.

## Question

§4.4 promised "drag a file onto the window to open it". How much of the stack does that actually
touch — and can either of our two toolkits deliver the paths to the engine at all?

## Options

### A. Use what the toolkit already forwards

Both toolkits are handed a shell drop by their event source. GPUI converts it into an *internal
drag*; Slint's winit backend does not handle it at all. Choosing A on either toolkit ships a
feature that silently does nothing: the window shows the copy cursor, and the file never opens.

### B. Own the OS half in `platform`, and cross the port by call

Register our own `IDropTarget`, buffer the paths, let the engine pull them from its own timeout
arm. Costs one new synchronous port call, which rule 4 of `crates/api` had never allowed.

### C. Do not ship it

Leave the §4.4 bullet as intent. Cheap, and wrong for the fastest input path the product has.

## Recommendation

**B — and stop calling the feature cheap.** §4.4 now says what it costs. The engine half genuinely
was cheap (`Command::Open` already existed, and still serves a drop unchanged); the two toolkits
were the whole bill, and each billed in a different currency. Any future doc that prices an
OS-facing feature by counting our own files should be read against this note first.

## What is built

B, **on Slint**. Five rungs, bottom up:

- **OS half — `crates/platform/src/windows/file_drop.rs`.** A COM drop target declared with
  windows-rs' `#[implement(IDropTarget)]` (`:72-73`); the vtable impl is on the generated
  identity `FileDropTarget_Impl`, not on the wrapper (`:92-96`). `DragEnter`/`DragOver`/`Drop`
  answer `DROPEFFECT_COPY` and nothing else (`:81-90`) — a translation, not a decision. Paths
  are read off `CF_HDROP`, copied into owned `PathBuf`s inside the call, and parked in
  `static BUFFER: Mutex<Vec<PathBuf>>` (`:38-42`): `Backend` is a per-call unit struct, so the
  process is the only place a drop can persist. `DragLeave` clears nothing (`:116-121`) — a
  cancelled drag must not throw away a file somebody already dropped. The lock is
  poison-recovering (`:49-55`): a panic elsewhere must not cost the user a drop. `arm` calls
  `OleInitialize` (tolerating `RPC_E_CHANGED_MODE`), revokes first so a re-arm is a reload and
  not a permanent `DRAGDROP_E_ALREADYREGISTERED`, registers, and then **leaks** the target with
  `mem::forget` because OLE holds the only strong reference until `RevokeDragDrop`
  (`:188-244`, SAFETY comment at each step).
- **The door — `crates/api/src/file_drop.rs:57`:**
  `arm_file_drop(WindowHandle) -> Result<DropGuard, DropArmError>`. The port's only synchronous
  `Result`-returning call, and it is **thread-affine**: call it on the thread that pumps the
  window, drop the guard there too (`:22-45`). `DropGuard` (`:86-101`) *is* the registration's
  lifetime — not `Clone`, unbuildable outside the module, and its disarm error is swallowed
  because a destructor has nowhere to return an `Err`. Three `DropArmError` arms because three
  failures need three different sentences (`:103-110`). Naming this exception is what ADR-0004 is
  for.
- **Engine — `crates/api/src/engine.rs`.** Arrival is a pull, not a push:
  `const DROP_POLL: Duration = Duration::from_millis(120)` (`:66-81`) on a **second clock**,
  `next_drop`, kept deliberately apart from the autosave `deadline` (`:314-327`) — merge them
  and every 120 ms poll re-arms the 750 ms flush, so autosave never fires on the one machine shape
  where files get dropped. Answered first in the timeout arm (`:590-602`), drained by
  `poll_drops` (`:652-671`), which advances the clock before anything can early-return (no
  spin), refuses to act while the shutdown drain runs (`:624-631`, `:654`), and hands every path
  to **the same `Command::Open` handler the menu uses** (`:669`, arm at `:814`). It is neither
  queued nor waited on, so rule 4's reentrancy trap is not entered: `take_dropped_paths` drains,
  so the list cannot feed itself.
- **Bridge — `crates/bridge-slint/src/main.rs`.** `arm_drop_target` (`:181-231`) holds the
  guard in an `Rc<RefCell<Option<DropGuard>>>` (`:667-670`), refuses a second arm on an
  already-named hwnd — the first guard's `Drop` would revoke the registration the second arm just
  made (`:207-216`) — and reports both outcomes rather than failing quietly. It is called at
  **both** `Command::RegisterWindow` sites: after the first `show` (`:684-696`) and from the
  tick callback (`:914-923`), which is the one that fires in practice because the hwnd often does
  not exist until the loop has spun. Arming lives in the bridge, never in the engine's
  `RegisterWindow` arm, whose thread pumps nothing; doing it there freezes Explorer in the
  *sender's* drag loop (`:183-191`). Disarm is explicit on the main thread after `ui.run()`
  returns (`:1513-1521`), so revocation happens while the apartment that registered is alive.
- **Policy: none new.** A drop **is** an `Open` — same unsaved-buffer guard, same recents, same
  epoch/generation bookkeeping, same autosave arming. **ADR-0001 is unchanged**: dropping a foreign
  file arms nothing, exactly as `Ctrl+O` arms nothing (`bridge-slint/src/main.rs:193-201`). No
  new `Command`, no new `Event`, no settings key.

## Why "nearly free" was false, toolkit by toolkit

- **gpui-pre 0.3.4.** The window adapter *does* receive the drop, and converts
  `FileDropEvent::Entered` into `cx.active_drag = Some(AnyDrag { value: Arc::new(paths), view:
  cx.new(|_| paths), .. })` (`gpui-pre-0.3.4/src/window.rs:5276-5288`), then rewrites `Pending`
  as a `MouseMove` and `Submit` as a plain `MouseUp` (`:5295-5312`). That is the machinery
  for *internal* drags: the paths land parked in a drag this app is not running, on an entity it
  never renders and never reads, and the drop's own moment arrives as a mouse release. Nothing
  downstream can reach them without building a drag view layer for a notepad that has nothing to
  drag. Not free — a slice of its own.
- **Slint 1.17.1 winit backend.** winit emits the event (`winit-0.30.13/src/event.rs:180`
  `DroppedFile(PathBuf)`, produced on Windows at
  `winit-0.30.13/src/platform_impl/windows/drop_handler.rs:140-146`). The Slint adapter's window
  event match has **no arm for it** and the fallthrough is `_ => {}`
  (`i-slint-backend-winit-1.17.1/winitwindowadapter.rs:1537`). The path is dropped one layer above
  the OS that went to the trouble of handing it over — and Slint exposes no API to reach it, which
  is why the work moved down into `platform` instead of staying in the bridge.

## State of the two bridges

**`bridge-gpui` does NOT arm a drop target.** The S7 slice of the spike plan was not run:
`arm_file_drop` is imported nowhere in that crate (only `crates/bridge-slint/src/main.rs:35`
imports it), and gpui has no `platform`-level takeover behind it, for the reason above. Dropping a
file on the gpui build still does nothing. The `platform`/`api`/`engine` rungs are ready for
whoever runs that slice: one call on the thread that pumps the window, one guard held next to it.

## Consequences

- Rule 4 of the port now carries a named exception, with the reason in ADR-0004 — so the rule
  stayed honest, and became arguable. The two tests for a future candidate are in that record.
- `platform` gained its first leaked COM object and its first `OleInitialize`, so it now has
  opinions about which thread calls it.
- The engine carries a second clock. That is a trap with a comment on it (`engine.rs:314-327`),
  not a type — the merge that breaks autosave still compiles.
- Arming is headless-safe and ungated: an `IDropTarget` nobody drags onto costs one registration
  and says nothing (`bridge-slint/src/main.rs:203-205`).
- The promise is met **on Windows only**. `take_dropped_paths` and `arm_file_drop` are the two
  seams macOS and Linux fill at M6–M8; nothing above `platform` changes when they do.
- Two bridges, one behaviour: until S7 runs, the same feature works on one build and not the other.
  That is the divergence a user reports as a bug, and it is why this section exists.

## Reopening conditions

- If Slint ships a public drop API that hands paths to the app outside the window-event match, the
  `platform` drop target becomes redundant — though the thread-affine arm stays ours.
- If gpui remains the shipped toolkit, S7 must run; do not close this note's gap by deleting the
  §4.4 bullet.
- If the 120 ms poll is ever folded into the autosave tick "to save a timer", read
  `engine.rs:314-327` first and expect autosave to stop firing.
