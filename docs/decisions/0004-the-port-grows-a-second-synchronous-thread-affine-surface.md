---
id: 0004
title: The port grows a second synchronous, thread-affine surface
status: accepted
date: 2026-09-13
deciders: []
supersedes: null
superseded_by: null
relates: [whitepaper §5.4, whitepaper §4.4]
---

## Context

§5.4 fixed the shape of the port: Commands in, Events out, and **nothing that answers
synchronously**. Rule 4 of `crates/api` exists because autosave fires seconds after any UI
call, on another thread, and can fail with no caller waiting — so a failure crosses the port as
`Event::SaveFailed`, and "never add a synchronous `Result`-returning save path just because it
is convenient" is in AGENTS.md as well as the crate docs.

The one thing that would not fit that shape is `drop-a-file-onto-window` (§4.4). Windows calls
an `IDropTarget` on the thread that owns the window, **inside the drag loop the shell runs for
the sender** (`crates/platform/src/windows/file_drop.rs:8-14`). `OleInitialize` builds the COM
apartment of whichever thread calls it, and `RegisterDragDrop` binds the target to the window's
owner. So arming is not a request that can be queued: routed through the command queue, it would
run on the engine thread, which pumps nothing, and the callbacks would wait for a pump inside
somebody else's drag loop — Explorer freezes mid-drag, across the desktop, with no error returned
to this process (`crates/api/src/file_drop.rs:37-45`).

The pre-existing synchronous surface is `Gateway::query_session`, the §5.5 step-1 read
(`crates/api/src/gateway.rs:407`). This is the second, and it is the first one whose thread
matters as well as its timing.

## Decision

**The port gains a second synchronous surface, bounded to one call with one kind of answer:
`arm_file_drop(WindowHandle) -> Result<DropGuard, DropArmError>`**
(`crates/api/src/file_drop.rs:57`). It is declared by name as the exception in the crate docs
(`crates/api/src/lib.rs:73-85`, `crates/api/src/gateway.rs:1-4`), and bounded by four rules:

1. **It answers about a registration, never about a document.** No `Event`, no `Command`, no
   `PathBuf` in its signature (`crates/api/src/file_drop.rs:11-18`), so it cannot decide what a
   dropped file means even by accident. Rule 1 still holds; the rule still forbids a *second*
   such call that answers about a document (`crates/api/src/lib.rs:84-85`).
2. **It crosses by call, never by channel** — touches no `Sender`, no `Receiver`, no engine
   state, so it cannot block a frame behind a queue.
3. **It is thread-affine, and the bridge owns that.** Arm from the thread that pumps the window;
   drop the `DropGuard` on the same thread (`crates/api/src/file_drop.rs:30-35`, `api/src/lib.rs:73-85`).
4. **Arrival stays asynchronous.** What actually dropped travels the ordinary route: the platform
   buffers paths (`platform/src/windows/file_drop.rs:42`), the engine asks for them on a poll
   (`DROP_POLL`, 120 ms, `api/src/engine.rs:81`) and hands each one to the existing
   `Command::Open` arm (`api/src/engine.rs:652-671`).

## Consequences

**Becomes possible.** A drop is bit-identical to a menu Open: same unsaved-buffer policy, same
ADR-0001 autosave arming, same recents, same epoch/generation bookkeeping
(`crates/bridge-slint/src/main.rs:193-201`). No new `Command` variant, no new `Event`, no new
policy — §4.4 needed none.

**Becomes harder, and is named so it does not quietly spread.**

- Rule 4 now has a named exception, and exceptions attract imitators. The test of a candidate is
  in this document: *is it synchronous because the OS binds it to the calling thread, and does its
  answer concern a registration rather than a document?* A `Result`-returning save, open, or
  rename fails both halves and stays forbidden.
- `DropGuard` carries an `isize`; Rust will let it move between threads. The thread contract is
  bridge discipline, not a type invariant (`api/src/file_drop.rs:32-35`), and getting it wrong
  is not a degraded window — it is another process hanging. A second bridge must re-read this
  before it arms.
- `mem::forget` on the registered target is a deliberate leak (`platform/src/windows/file_drop.rs:234-243`):
  OLE holds the only strong reference until `RevokeDragDrop`, so a Rust-owned box would free the
  vtable under the OS. `unsafe` in `platform`, with the invariant named in the comment.
- The engine now keeps **two clocks** on one wake-up (`api/src/engine.rs:314-327`): merging
  `next_drop` into `deadline` would re-arm the 750 ms flush every 120 ms and autosave would never
  fire on exactly the machines where files get dropped. That is the first real cost of the poll
  design, and it is load-bearing.
- A poll is a latency choice with no owner: 120 ms is named, not derived (`api/src/engine.rs:74-80`).
- A toolkit that will not let us reach the window's drop target cannot use this surface at all,
  which is why the second bridge armed it and the first one has not
  (`.agents/notes/implemented/2026-09-13-file-drop-chain.md`).

**Does not change.** ADR-0001 (autosave arms on an explicit save) is untouched: a drop is an
Open, and an Open arms nothing.
