---
id: 0005
title: Arming the drop target takes the toolkit's own target, and says whose it took
status: accepted
date: 2026-09-13
deciders: []
supersedes: null
superseded_by: null
relates: [whitepaper §5.4, whitepaper §4.4, ADR-0004]
---

## Context

ADR-0004 admitted a second synchronous, thread-affine surface to the port and fixed the rule that
the arm must run on the thread that pumps the window. It did not answer two questions that only a
live window could answer, and the live window answered both of them differently from the guess.

**Both toolkits already own the window's drop target.** winit registers an `IDropTarget` per
window — its Windows handler emits `WindowEvent::DroppedFile`
(`winit-0.30.13/src/platform_impl/windows/drop_handler.rs:140-146`) — and Slint's winit backend
never handles that event: the window-event match in `i-slint-backend-winit-1.17.1/
winitwindowadapter.rs` ends in `_ => {}` (`:1537`), so the payload is discarded one layer above
the OS. gpui-pre registers its own target too (`gpui-pre-windows-0.3.4/src/window.rs:1094`
`#[implement(IDropTarget)]`, registered at `:1469-1471`) and routes the drop into its own input
pipeline (`:1146`, `:1184`, `:1200`), where the payload sits in `active_drag`, a
`pub(crate)` field (`gpui-pre-0.3.4/src/app.rs:689`) whose only public neighbours,
`has_active_drag` and `active_drag_cursor_style` (`app.rs:2524-2530`), carry no path. Neither
toolkit hands the dropped file to a bridge. So the arming ADR-0004 approved is not adding a second
receiver to a silent window: it is **displacing the one the toolkit installed**.

**Which thread that is was measured, and it reversed the gate.** The first version of the apartment
predicate inside `arm` refused the apartment a real run reported. The measured value is **3**, and
the `windows` 0.61.3 headers number the enum `APTTYPE_STA = 0, APTTYPE_MTA = 1, APTTYPE_NA = 2,
APTTYPE_MAINSTA = 3` (`Win32/System/Com/mod.rs:933-937`; the gate pins the value in a test,
`crates/platform/src/windows/file_drop.rs:566`). 3 is the process's **primary STA**, and it exists
because the toolkit called `OleInitialize` on its own event-loop thread — which is the very thread
a bridge arms from. The thread that pumps the window is therefore not merely admissible, it is the
best apartment a drop target can have, and the original gate refused precisely the good case
(`crates/platform/src/windows/file_drop.rs:260-270`).

The same measurement settles the initialisation count. Our `OleInitialize` joins the toolkit's
apartment and answers `S_FALSE` with near-certainty, so the count we would release on the way out
is **the toolkit's**; `windows` types the call so `S_OK` and `S_FALSE` are indistinguishable
once severity is dropped, meaning a recorded bool could only repay a case this file cannot see
(`:233-243`). The count is deliberately never paid back — which is what makes the takeover below
safe to attempt at all.

## Decision

**The arm takes the window, proves the thread, and reports whose target it took.**

1. **`RevokeDragDrop` before `RegisterDragDrop`, unconditionally**
   (`crates/platform/src/windows/file_drop.rs:326`). This is the toolkit covenant, stated as a
   covenant and not as cleanup: **we unregister, we never free.** The pre-emptive revoke does not
   test whether the window was ours first, because a revoke guarded by ownership could never have
   revoked the toolkit — it would have guarded exactly the case the feature exists to handle
   (`:294-304`).
2. **The exit half still revokes only what we recorded.** `ARMED` gains a key only on a
   registration this module actually made (`:373-374`) and remains the disarm gate
   (`:438-462`), so the takeover is one-directional: we may displace a target we did not create,
   and we may never revoke a target we did not register.
3. **The thread is proven, not trusted.** After `OleInitialize`, `arm` calls
   `CoGetApartmentType` and applies `admits_a_drop_target` (`:271-293`, list at `:387`):
   **both STAs are admitted** (`APTTYPE_STA` 0 and `APTTYPE_MAINSTA` 3); **MTA (1) and NA (2)
   are refused**, as is `APTTYPE_CURRENT` (-1) and any value a future `windows` release adds —
   the safe default is to refuse and be told. The refusal prints **apartment and qualifier
   together** (`:286-292`), because the headline alone misleads: `IMPLICIT_MTA` is exactly what
   a "just initialise it" fix cannot see, and an `NA_ON_STA` qualifier says the *call* came from a
   neutral context, not that the thread is dispatchable.
4. **`took_over` travels as a return value, never as process state.** It is computed by
   `took_over_from(preexisting, ours)` (`:402`) from the revoke's answer plus an `ARMED` read
   taken **before** the revoke (`:319-327`), and leaves `platform` as `DropArm { took_over }`
   (`:375`, field at `:420`) → the port's `DropGuard::took_over()` → the bridge's report line.
   Displacing our own target on a re-arm is `false` (replacement, not theft); displacing the
   toolkit's handler is `true`. A race — `DRAGDROP_E_ALREADYREGISTERED` after an unconditional
   revoke — is retried once, recorded as a takeover, and a second refusal is returned as an error
   rather than looped away (`:342-368`). Nothing caches the flag in a static; the statics are the
   path buffer and the `ARMED` set.

## Consequences

**Positive.** Both bridges arm now: `bridge-slint` at both `Command::RegisterWindow` sites
(`crates/bridge-slint/src/main.rs:696`, `:923`) and `bridge-gpui` once its handle is read
(`crates/bridge-gpui/src/main.rs:1671`, since `d2a98b76`). A drop is still exactly an `Open`,
so no policy moved and ADR-0001 is untouched. And the run says whose window it changed — "took the
toolkit drop target; our copy cursor is the law now" (`bridge-slint/src/main.rs:230-237`), "took
gpui's own drop target; its FileDropEvent had no subscriber here, so nothing was given up"
(`bridge-gpui/src/main.rs:1997-2004`) — while reporting "quiet - nothing was registered" rather
than inventing a takeover it cannot see.

**Negative, and named as cost.**

- **The toolkit's handler object stays allocated and simply stops being called.** We revoke, we do
  not free: winit's `FileDropHandler` and gpui's target remain live, held by nobody, along with
  whatever state the toolkit keyed to them. Nothing reclaims them; nothing may.
- **Every `DROPEFFECT` cursor over the window is now ours.** `accept_copy` answers
  `DROPEFFECT_COPY` and nothing else (`crates/platform/src/windows/file_drop.rs:81-90`), so the
  copy/move/link affordance the toolkit would have shown is gone, and any gesture a toolkit builds
  on its own target — including one invented later — is dead while this arm stands.
- **An initialisation count the process carries on our behalf.** One `OleInitialize` we did not
  create and deliberately do not unwind (`:233-243`).
- **gpui logs a stray `RevokeDragDrop` error at shutdown.** Its teardown revokes as well
  (`gpui-pre-windows-0.3.4/src/window.rs:599`, failure surfaced by `.log_err()`); ours ran
  first and succeeded, so the toolkit's second revoke is the one that fails. The line belongs to
  the toolkit, it is expected after a successful arm, and it is documented at our own disarm
  (`crates/bridge-gpui/src/main.rs:1945-1950`, `:2018-2032`) so nobody chases it as an app bug.
- **The admissible list is a policy, not a proof.** It admits what dispatch works in today, so a
  future OLE or `windows` change surfaces as a refusal an operator reads — the intended failure
  mode, but still a hand-maintained one.

## Reopen condition

**This takeover becomes a bug the day the toolkit delivers what it owes.** For Slint the line is
drawn in the covenant comment (`crates/platform/src/windows/file_drop.rs:306-313`): winit's
payload is discarded today only because the winit backend has no `DroppedFile` arm, and **Slint
upstream issue #1967** — forwarding `DroppedFile` to a custom window — turns this arm from the
only way to obtain the paths into **stealing a delivery we owe**. If #1967 lands, the slint arm is
withdrawn and the toolkit's own event becomes the source of drops; re-read that paragraph before
keeping this one. For gpui the condition is behavioural rather than upstream: **arming must become
opt-in the day this bridge grows a drop-to-tab or drop-on-chrome gesture of its own, or the day
gpui exposes the dropped paths to a window subscriber** — a public payload, or an
`on_file_drop`-style handler. At that point two targets are competing for one gesture and the
toolkit's claim wins by default: the call needs a setting in front of it, and the needle that
reports the takeover is the line that gets gated
(`crates/bridge-gpui/src/main.rs:1952-1957`).

## Relation to ADR-0004

**ADR-0004 stands unchanged.** The exception is still exactly one call, still synchronous, still
thread-affine, still answering about a registration rather than about a document. This record
amends one of its consequences — the thread rule — with what the measurement added: arm **from the
apartment the toolkit armed first**, the primary STA its own `OleInitialize` created on the
event-loop thread, rather than from any thread that happens to pump the window. The predicate, the
unconditional pre-emptive revoke, the un-repaid init count, and `took_over` as a reported return
value are that amendment.
