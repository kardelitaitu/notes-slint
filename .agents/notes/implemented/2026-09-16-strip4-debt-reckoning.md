---
title: The STRIP-4 debt reckoning - two rows paid in code, the panic hook now proven by a child
status: implemented
id: 2026-09-16-strip4-debt-reckoning
created: 2026-09-16
updated: 2026-09-16
relates: [§5.5, §9]
decision: null
---

Three rows went into STRIP-4 as debts, and `2fc924f3` paid them in the only currency a commit holds:
code. The question this record settles is narrower and less comfortable - **which of those three had
ever been observed doing it?** A row can be written, compile, pass its unit test, and still be a
claim nobody watched land. This is the audit of that, at `db2e9d2e`, and the one row it closed.

## The ledger

| Row | Verdict | The evidence, by name |
|---|---|---|
| Geometry settle | **PAID, and paid+proven** | `crates/bridge-slint/src/product.rs:103-107` carries the header, `:108-109` the two consts, `:192-198` the pure decision, `:528-590` the watch inside the wake. Proven twice over: the unit tier (`a_drag_that_never_stops_still_tells_the_port`, `:817-836`) pins the arithmetic AND the two numbers, and `2fc924f3` quotes the live run - two sends in one episode, next startup reading back where the last left the window (`2026-09-14-strip-program`, "Geometry: an episode, not a count"). |
| Honest shutdown | **PAID, and paid+proven** | `product.rs:624-738`, the six-step order stated at `:632-636` and the join answered at `:681-737`: `Ok` says the session write ran (`:686`), `QueueClosed` says the thread joined (`:690`), `Abandoned` waits again for the port to close its channel and leaves nonzero if it never does (`:693-725`), `Panicked` says so in words that cannot be misread as "saved" (`:726-735`). Proven by the machine leg that reads it: `crates/xtask/src/smoke.rs:4075-4084`, `PRODUCT_CLOSE_NEEDLES`, which owes `close: requested #1, granted` and `shutdown: joined cleanly` on the LIVE app stderr - a force-killed run cannot produce the second line and cannot PASS whatever code it ends with (`2026-09-14-strip-program.md:140-143`). |
| The panic hook | **CODE-PAID, PROOF-OWED - now code-paid AND child-proven** | The hook was real and always ran first: `product.rs:261-283`, taking ownership at `:266`, installing at `:267`, chaining std at `:282`; the one line it speaks is built by the pure `panic_note` at `:205-210`. What it never had was a witnessed use, and the strip record said so in as many words: "no live panic was provoked" (`2026-09-14-strip-program:111`). `ebbb755c` provokes one. |

## Row three, the proof that was missing

A panic hook is the one piece of a `windows_subsystem` binary whose entire job is to speak at the
moment the program dies. `report()` is its only voice, so an unobserved hook is an unmeasured voice:
the unit test named the format and nothing named the wiring. Three pieces, one commit:

- **The gate** (`product.rs:291-293`): `if std::env::var_os("NOTES_PANIC_PROBE").is_some() { panic!(...) }`,
  sited between the hook at `:283` and `state_dir()` - so the provoked child never touches a session
  file, never starts the port, never asks for a window. **One order, and the refusal is in the comment
  beside it**: a mid-loop act needs a timer, a timer races the event pump, and the result is a flaky
  test wearing a proof's name. There is deliberately no second variant.
- **The child** (`crates/bridge-slint/tests/panic_hook.rs`, 169 lines): the real
  `env!("CARGO_BIN_EXE_notes-slint")`, its own piped stderr, a deadline that kills instead of waiting
  (shape taken from `crates/core/tests/crash_safety.rs:45-103`). `a_real_child_that_really_panics...`
  (`:113-152`) asserts the pair and the chain: a NONZERO exit (`:124-127`, `101` printed as detail only -
  `panic = "abort"` is one profile key away and would still voice both lines, so the number is not the
  gate), a line STARTING `notes-gpui: panic:` carrying the probe token (`:130-132`), and std's own
  `panicked at` still present (`:143`) - our line beside std's is what proves the default hook was
  chained rather than replaced, which is precisely the half a unit test cannot reach. The control,
  `the_probe_is_inert_when_the_variable_is_absent` (`:154-168`), runs the same binary with the variable
  ABSENT for 1.5 s and asserts the token did NOT appear.
- **The drift guard** (`product.rs:763-796`, `the_panic_hook_is_installed_before_the_port_and_chains_the_default`):
  the closure itself cannot be called from a test, so the test forbids the ORDER from moving -
  `take_hook` < `set_hook` < `Gateway::start`, `default_hook(info)` present inside that window, and no
  `process::exit` in it, because an exit there ends the run and takes the trace line with it. And the
  unprintable arm finally has an owner (`:798-815`): `(unprintable panic payload)` is what a payload
  that is neither a `String` nor a `&str` gets, and until now renaming it in the closure would have
  gone red nowhere.

**Measured, this box, `2026-09-16`**: `cargo test -q -p notes-bridge-slint` -> 49 + 65 unit + 4 + 2
integration, all passing; the child's own line, printed by the test:

```text
panic_hook: exit exit code: 101; the child said: notes-gpui: panic: NOTES_PANIC_PROBE=startup:
provoked before the port was asked for a snapshot at crates\bridge-slint\src\product.rs:292
```

## Wording fix, on the row two shorthand

Row two travels in shorthand as one word - "unbounded" - and one word is how a policy gets tidied
away. The
sentence in the source is not a gap in a rule, it is the rule: `product.rs:103-107` takes gpui's two
numbers as inherited (`:104` names `main.rs:380` and `:385`), then says what is **deliberately NOT**
inherited - the probe's cap of two sends, "a measurement budget for one act walk", closing at `:107`,
"Unbounded here, on purpose." So read the row as:

> the probe's 2-send cap retired - unbounded ON PURPOSE

and not as "no cap yet". The difference is what a future reader does when the line looks untidy: the
first wording says a drag that never stops must still keep telling the port where the window ACTUALLY
is, and re-adding a cap is the bug; the second invites exactly that.

## Limits of what was just proved

- The panic was provoked **PRE-WINDOW**, in the first breath of `main`, on a child whose stderr this
  test pipes. That is what makes it deterministic, and it is also the whole limit: no hook line has
  ever been witnessed on a desktop run with a window mapped, a person typing and a pump running.
  The chained-default claim is proven for the startup position only.
- **The control proves the gate, nothing more.** A `notes-slint.exe` with no order given is a GUI
  binary on a station that may not be able to host a window - and this box currently has pump trouble
  - so a clean no-op is not on offer and none was claimed. The assert is about the token.
- **The `Abandoned` and `Panicked` ENGINE arms are still unproven.** `product.rs:693-735` has been
  read, never seen: `Ok` is the arm every real run so far has taken, and the two alarming arms need
  an engine that stalls or unwinds on the way out. Named here as PRICED FUTURE DEBT, on the same door
  this row just used: a second probe order, mid-shutdown, that stalls the engine between `Flush` and
  `Shutdown` (and its sibling that panics it). It is a second order and not a variant of the first -
  the same flaky-timer objection applies to any act timed inside the pump, and the shutdown arms need
  a wedge the harness itself controls rather than a race it hopes to win.

## Recommendation

Keep the probe. It is nine lines, one env var, and it is the only reason the hook's claim is now
"observed" instead of "written" - and the two orders it forecloses are worth naming out loud, because
they are the two a future author will reach for: **no mid-loop panic act** (a timer racing the pump is
a flaky proof, refused at `product.rs:288-290`), and **no exact exit code asserted** (the gate is
nonzero; `101` belongs to the unwind profile, not to the hook). Reopen the row-three claim for a
desktop run when a hook line can be provoked in front of a window without a timer; reopen row two's
`Abandoned`/`Panicked` arms when the harness can stall the engine on command.
