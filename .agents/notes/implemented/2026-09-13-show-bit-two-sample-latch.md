---
title: The show bit's two-sample latch
status: implemented
id: 2026-09-13-show-bit-two-sample-latch
created: 2026-09-13
updated: 2026-09-13
relates: [§4.1, §5.4, §5.5]
decision: null
---

Read this next to `crates/api/src/engine.rs` — the comment at the write site is the authority
on what the code claims. This note is the provenance: why a persisted boolean needs to be
asked twice, and the part of the story that has to stay embarrassing so nobody re-tells it as
a bug report.

## Question

`2026-09-12-maximized-persistence` shipped the first writer `session.maximized` ever had — a
measurement taken through the geometry seam, machine-proven by the smoke harness's M9 leg. That
left one design question open at the moment it landed: **the bit's entire history on the way to
disk is a single answer to `GetWindowPlacement`, taken at one instant.** Does one answer deserve
to become a fact that the next launch places the window from?

The options on the table were: keep single-sample authority; require the answer to repeat; wait a
settle delay before measuring; or let the bridge report the show state as intent (option A of the
earlier note, a `Command`, re-litigated here as a way to dodge the timing question). Only the
second was taken.

## What is built (`69de6c5e`)

One constant, one function, one write. The commit touched three files — `engine.rs`,
`crates/api/tests/geometry.rs`, `crates/api/tests/support/host_mock.rs` — 359 insertions, 27
deletions, no new `Command`, no new `Event`, no clock.

- **The rule.** `const SHOW_CONFIRMATIONS: u32 = 2` (`engine.rs:1271`, reasoning at `engine.rs:1267-1270`).
  A change to the bit must be seen by two consecutive answering measures before it is applied.
- **The sole writer.** `Engine::note_show_sample` (`engine.rs:1750-1764`) holds the only
  assignment to `session.maximized` in production code (`:1760`) — the field's sole writer is
  also the only place its confirmation rule lives. Its two call sites are the two arms of the
  one show match inside the MAJOR-5 move-in-flight guard (`:1663-1675`, arms at `:1665` and
  `:1668`, guard at `:1600-1608`), so a read-back taken while an async move is still landing
  never becomes a sample at all.
- **An agreeing sample settles instantly.** `:1756-1758` returns before any counting matters:
  there is no change to confirm, so nothing is outstanding and the write may retire. Without it,
  a healthy maximised launch would re-arm the session write forever on a bit that already says
  what the window says.
- **A contradicting sample starts a streak; the second one applies the change.** `:1751-1755`
  counts consecutive agreeing answers (`Option<(bool, u32)>`, not "the last value"), `:1759-1762`
  applies at two. A contradicting answer restarts the count at one. **Symmetric by construction**
  — one transient `Normal` does not clear a maximised window's bit either, which is the
  higher-stakes direction, because losing a state the user left is exactly as bad as inventing one.
- **The terminal flush keeps single-sample authority.** `terminal == true` applies on one sample
  (`:1759`, flag documented at `:1571-1578`): the unregister arm (`:597`), the shutdown drain and
  the abort flush. This is not a new judgement — it is the one `Engine::final_flush` already makes
  about the move-in-flight guard ("at shutdown there IS no later tick", `engine.rs:1254-1257`),
  extended to the show bit. The window's last honest read-back is the state the user actually left.
- **The streak resets at registration** — and that is a fact about *that window* (`:544-553`).
  On a recreate the count starts from nothing rather than carrying over whatever the window that
  died was answering, because a new window is where a first sample is least trustworthy. First
  registration only, per the same guard that stops a restore becoming a theft.
- **`Unknown` neither confirms nor breaks.** It never reaches `note_show_sample` at all: the
  match at `:1663-1675` drops it at `:1674`. It is the absence of a sample, so it does not
  confirm a change and does not reset the streak — a window minimised on one tick is the same
  window on the next. The write stays armed meanwhile, because `Measurement::show` is false and
  only a complete measure retires `pending.session` (`:195-197`, `:1905`), so the streak does
  get its second sample.

## What it costs the user, visibly

A mid-session maximise reaches `session.json` after **two flush ticks** — `AUTOSAVE_IDLE` is
750 ms (`engine.rs:64`; the tests use the same figure as `AUTOSAVE_TICK`, `geometry.rs:67`), so
**roughly 1.5 s**, and a maximise that is quit inside one tick persists immediately through the
terminal path instead. Two consequences for anyone probing this from outside:

- A poll that reads `session.json` sooner than ~1.5 s after a real maximise can legitimately see
  `maximized: false`. That is the rule working. Write the probe to wait for two ticks, or quit.
- Any future feature that needs "the moment the user maximised" must read the bridge (the Watch
  fingerprint, `crates/bridge-gpui/src/main.rs:453-474`), not `session.json`. The file is now
  deliberately a half-second-plus behind the window.

## How we got here

Told straight, because the tempting version is a good story about catching a Windows bug.

An investigation was opened to chase a **phantom `maximized: true` observed at ~1.92 s** on a
window that had never been maximised. If real, it was exactly the failure the write path permits:
one transient answer, persisted, and the next launch born from a state nobody chose.

Two independent attempts to reproduce the premise found nothing. An **OSD probe exercising five
different Win32 orderings** (the show/position call sequences a window actually goes through)
measured `showCmd == 3` — `SW_MAXIMIZE` — **never**. An **external 25 ms poll of the live app's
registered hwnd**, reading window placement from outside the process while it ran, also found
`showCmd == 3` **never**.

Then the observation itself evaporated. The "true" had come from the slint spike's own JSON
reader, which sliced **40 bytes from the `"maximized"` key** and asked whether that window
contained `true`. On a pretty-printed `session.json` those 40 bytes are `: false,` + newline +
`"pinned"` — so a file holding `maximized: false, pinned: true` answered **true**. It was a pin
indicator wearing a maximise label (RC2, recorded at `crates/bridge-slint/src/main.rs:104-129`),
and the phantom was its shadow. The claim was retracted in `49e2e715` (the write-site comment)
and `44917d19` (the red-proof's doc comment). `smoke.rs` was never exposed: it matches the
**whole literal** `"maximized": true` (`crates/xtask/src/smoke.rs:2246`), which cannot straddle
two keys. The phantom exercised none of the machinery it was credited with exercising — not the
write site, not the MAJOR-5 guard, not the `Unknown` branch.

So: **nothing was ever observed going wrong.** The latch is explicitly *not* a fix for an
incident, and ships as **defense-in-depth whose only proof is a mock**. What it defends against is
a property of the seam rather than an event — `GetWindowPlacement` answers what the window says
at the instant it is asked, that one answer is the only input the field has ever had, and a wrong
answer is unrecoverable downstream because the next launch places the window from the file.
Requiring a wrong answer to repeat itself is the cheapest rule that closes that, and the mock
proves the port obeys it.

**The authority on what the code does NOT claim is the comment at the write site** —
`crates/api/src/engine.rs:1621-1650` ("WHY TWO SAMPLES — AND WHAT IS NOT THE REASON") and the
function doc at `:1714-1749`. Read it before describing this latch as a bug fix anywhere: in the
commit message, in a changelog, in a reply to someone about to re-report the 1.92 s phantom as
live. If a future reader needs one sentence, it is theirs: *no OS or bridge has ever been
observed giving the single wrong answer; the claim is about what the port does with one sample.*

## Recommendation

**Keep the rule exactly as it stands, and keep the accounting honest.** Concretely, for whoever
touches this next:

1. **Do not raise `SHOW_CONFIRMATIONS`.** Three samples buys nothing the second did not already
   buy — the exposure is "a single answer", not "a short run of answers" — and it costs 2.25 s of
   lag on the state the user is most likely to quit inside. The terminal path already covers that
   case with authority, which is the reason the count can stay small.
2. **Do not lower it to 1.** That is this commit deleted, and the symmetry argument (one transient
   `Normal` steals a real maximise) goes with it.
3. **Do not add a second writer for the bit.** One writer is what makes the two-sample rule a
   whole answer. A `Command::SetMaximized`, or a bridge patching the file, reopens the design
   question `2026-09-12-maximized-persistence` settled by choosing measurement over intent.
4. **Do not "fix" the comment by removing the retraction.** The `1.92 s` paragraph earns its
   keep: it is the reason nobody has to prove Windows is innocent, and the reason the latch will
   not be cited as an OS bug six months from now.
5. If the settle-timing worry ever becomes real, the fix is a **settle delay on the measure**, not
   a bigger count: they are different claims (one is about when we look, one about what we
   believe), and only the first was ever in dispute here.

## Consequences

- **M9 was not touched.** The smoke harness's maximised leg (`crates/xtask/src/smoke.rs:2270`)
  passed **unchanged** over the latch — no assertion re-tuned, no sleep extended to accommodate
  the new rule — because its verdict is the restore rect's zero drift across a real
  maximise → close → relaunch, and the close is a terminal flush, which keeps one-sample
  authority. `eb4a3812` (the quit-time measure arms its own write) is what makes that path fire
  at all; without it, the latch would have had a terminal case that never got to run.
- **The write can stay armed one tick longer.** An unconfirmed change keeps `pending.session`
  set (`engine.rs:1905`), the same door `98a2fc42` opened for an unanswered show half. No new
  event, no per-tick flood: MAJOR 6 is untouched.
- **Tests pinning it** — three, all in `crates/api/tests/geometry.rs`, all red against the code
  as it stood before `69de6c5e` and green with it:
  - `a_single_transient_maximised_sample_never_persists_the_bit` (`:419`) — the headline,
    **mock-proven on purpose**: the fake answers Maximized once and Unknown ever after, the rect
    is pinned at rest (800x600@120,90) so nothing but the latch explains a `true` in the file,
    and `host.restore_reads() >= 2` asserts a second sample was actually taken and refused.
  - `a_single_transient_normal_sample_never_clears_a_stored_maximised_bit` (`:479`) — the
    symmetric direction, the one that loses a state the user left, asserted again after the quit.
  - `two_consecutive_maximised_samples_persist_the_bit_without_any_quit` (`:530`) — the positive
    half: no quit, no guard lift, no last-chance flush; the bit arrives because two measures
    agreed, and survives a seam that goes mute afterwards.
  The mock grew `Answers.restore_show` (`host_mock.rs:69`), `set_restore_show` (`:165`) and
  `restore_reads` (`:210`) for these — the counting is what lets a test assert "the second
  sample happened", which the pre-existing answers could not distinguish.
- **Suite state at the commit:** 137 `api` tests green. The arithmetic corroborates the run:
  138 `#[test]` in `crates/api` less the one documented `#[ignore]` at
  `crates/api/tests/debounce.rs:69` (no document debounce yet — a different finding, unchanged
  by this commit).

## Reopening conditions

- **A real transient sample is measured on a real window** — `showCmd == 3` seen where it should
  not be, from the OSD probe or an external poll — then the latch stops being defense-in-depth and
  becomes a fix. Record the incident, and re-open whether the count is right for the observed
  duration.
- **A bridge starts answering `Unknown` for a steady maximised window** (a toolkit that reports
  no show state from a timer): the streak can then never reach two mid-session, and the
  silence-is-not-an-answer rule needs a timeout beside it.
- **`AUTOSAVE_IDLE` changes** — the ~1.5 s figure above and every sleep budget in
  `geometry.rs:450`, `:507`, `:577` move together, and M9's timing assumptions with them.
- **Multi-window (§10's "one window in v1" is relaxed):** `show_streak` is engine-global, so one
  window's answer would confirm another's bit. The latch becomes per-handle the day the second
  window exists.
- **A Maximize menu item or shortcut appears:** that *is* user intent, it earns a `Command`, and
  the intent path should bypass the latch rather than race it — see the same reopening condition
  in `2026-09-12-maximized-persistence`.
