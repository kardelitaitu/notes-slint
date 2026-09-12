---
title: Autosave arms on an explicit save the port cannot express
status: proposed
id: 2026-09-12-arm-on-explicit-save-path
created: 2026-09-12
updated: 2026-09-12
relates: [whitepaper §4.2, whitepaper §4.4, whitepaper §4.5, whitepaper §5.4, whitepaper §10.3]
decision: null
---

## Question

ADR-0001 disarms autosave on a foreign file "until the user performs one explicit save
(`Ctrl+S` or the Save menu item)". There is no Save command — not in the menu, not in the
port. So is the `ctrl-s -> SaveAsFile` keybinding the rule, or is a rule missing? The report
that prompted this note said `Engine::flush` has no arm route. Reading the code, it has one;
it just cannot be the **first** arming. That difference decides which fix is small.

## What the code does today

Every line below was read this session. `crates/bridge-gpui/src/main.rs` is being edited
right now, so its numbers may drift; the function names are the stable citation.

**The keybinding is what the report said.** `menu.rs:139`: `"ctrl-s" =>
KeyBinding::new(chord, SaveAsFile, None)`. `menu.rs:118` labels that row `"save as"`,
`menu.rs:206` builds the item `Save As...\t` + key, and `menu.rs:341` asserts the
string. The File menu is Open, Save As, Recents, separator, Auto-save (`menu.rs:203-212`) —
**no Save item**, and `actions!` (`menu.rs:37-40`) declares only
`OpenFile, SaveAsFile, ToggleAutosave, ClearRecents`. The handler
(`main.rs:1832-1834`) calls `prompt_save_as`, which seeds the dialog with the current
directory AND file name (`main.rs:1975-1989`), opens `cx.prompt_for_new_path`
(`main.rs:1999`) and sends `Command::SaveAs` (`main.rs:2025-2032`). Chords are the
menu here because Windows stores the bar and never draws it (`menu.rs:19-24`).

**There is no explicit-save command in the port.** `command.rs:43-133` is the whole
vocabulary: Open, SaveAs, Flush, SetAutosave, SetPinned, ClearRecents, Shutdown,
RegisterWindow, GeometryChanged, UnregisterWindow. The bridge says it outright: "there is no
manual Save command" (`main.rs:890-892`). And `command.rs:66` describes `Flush` as
"A snapshot of the buffer: `Ctrl+S` and every autosave trigger", while
`command.rs:73-75` adds that on a path "the user triggered by hand it must not go silent" —
the port's own comment names a hand-triggered case that `Flush`'s payload
(`text, revision, epoch`, `command.rs:88-92`) cannot represent. **That is the missing
rule, recorded in the file that lacks it.**

**Arming, exactly.** `Document::mark_saved` (`document.rs:144-154`, the flag at
`:153`) is the only setter of `armed = true` outside the two constructors
(`document.rs:74` untitled, `:92` `.notes`). Two callers: `Document::save_as`
(`document.rs:174-177`) and `Engine::flush`.

- **CORRECTION 1:** `Engine::flush` **does** arm — `engine.rs:1047` calls
  `mark_saved` on write success.
- **CORRECTION 2, the real defect:** it cannot be reached as a *first* arming. `flush`
  consults `should_flush` first (`engine.rs:926-934`), which delegates to
  `should_autosave`'s fixed order — AutosaveDisabled, ReadOnly, Oversize,
  **ForeignFileNotArmed**, Clean (`document.rs:182-199`, armed test at `:192-194`,
  delegation at `:207-212`) — and returns before any write. So the gate is **check order**,
  not a missing line, and for a foreign document the only arming act in the build is
  `Command::SaveAs` (`engine.rs:798-866`; asserted at `session.rs:392` and
  `session.rs:433`).
- A third site exists: a path-less buffer's flush gives itself a file by calling
  `doc.save_as(&scratch)` (`engine.rs:992`), armed immediately (`engine.rs:1013-1015`).
  Correct — the app made that file — but it means "SaveAs arms" is doubling as plumbing.

**The double gate in the bridge.** `flush_due` refuses to send any `Flush` for a loaded,
un-armed document (`main.rs:1711-1728`, clause at `:1721`), and `final_flush` skips with
"the document is not armed for autosave - the text is NOT saved" (`main.rs:1774-1781`).
Arming is read back from the port, never inferred (`Wire::adopted`,
`main.rs:260-262`; called at `:827` on Loaded and `:858` on Rebound).

**What Ctrl+S on a foreign file actually costs.** Not the wrong file. Accepting the dialog's
default writes the same bytes to the same path in **that target's** encoding, because
`save_as` re-detects the target (`engine.rs:822-826`). The cost is the interruption, the
generation bump (`engine.rs:842`), the `Rebound` event, and a recents push
(`engine.rs:858`) that makes an ordinary save look like a freshly chosen location.

**The UI already tells the user to do something that does not exist.** The skip line for this
state is "a file this app did not create: **save it once** and it keeps saving"
(`main.rs:1217-1219`). The only answer available is a rename dialog. ADR-0001 requirement 1
forbids silence; this is the other failure — an instruction with no referent.

**Against intent.** §4.4 assigns **Save** = `Ctrl+S` and **Save As** = `Ctrl+Shift+S`
(`docs/features.md:76-77`). The bridge has neither the Save item nor the `Ctrl+Shift+S`
chord. README promises "No save dialog" (`README.md:23`) and a menu of "Open, Save, Save
As" (`README.md:26`). §4.2 restates the arming rule in the same words as the ADR
(`docs/features.md:49-53`).

**A §4.2 promise B and C would lean on does not exist.** `features.md:47-48` requires
detecting an external change (mtime + hash) before overwriting — "warn and keep both, do not
silently clobber". Nothing in `crates/core/src/save.rs` compares the target's current bytes
or mtime with what was loaded: `atomic_write` checks the path-policy verdict
(`save.rs:168-182`), reparse points (`:183`), the read-only bit (`:189-193`), encodes
before touching the filesystem (`save.rs:113-116`), and stops. Do-no-harm today is
**format** preservation, not **ownership** preservation.

**Testability.** `crates/api/tests/roundtrip_port.rs` (manifest `count: 30` — 25 foreign,
5 `.notes`) already states this note's coincidence in its own header: `:20-30` says the
drive "performs the one explicit save the decision names as the arming act -
`Command::SaveAs` at the file's OWN path, which is what the live bridge puts behind
`ctrl-s`", asserts the pre-arming flush writes nothing (`:772-791`), and then judges the
arming write **byte-identical** (`:792-803`), including UTF-16, BOM-only, lone CR, CP1252
and zero-byte shapes (`:1022-1036`). So B and C are checkable headlessly today.

## Options

### A. Correct-by-design: a foreign file is only ever saved to a named target

Keep the behaviour, fix the docs. Caveat README:23 to "no *save changes* prompt", admit that
ADR-0001's `Ctrl+S`-or-Save-item sentence describes §4.4 intent and not the build, and
rewrite `main.rs:1217-1219` to say "choose a location once". Costs: the headline promise
gains an exception class; §4.4's Save row stays unbuilt; ADR-0001 stays true only because
requirement 4 (Save As arms) is the live route.

### B. Arm on any explicit Save of a path-bound document

Give `Flush` a trigger field (or add `Command::Save`), let a hand-triggered write past
`document.rs:192-194`, and the existing `engine.rs:1047` does the arming. Strongest
reading of README:23 and of the ADR's letter. Risk: a silent overwrite of foreign bytes with
no collision check, because the guard in `features.md:47-48` is unbuilt.

### C. Split: Ctrl+S saves in place, the dialog stays for Save As

Same port change as B, narrower promise. `Ctrl+S` writes the bound path with the encoding
and EOL already detected from **that file** (the `self.detected` `flush` already uses,
`engine.rs:1040-1042`), with **no** generation bump and **no** recents push; Save As keeps
`prompt_for_new_path` and moves to `Ctrl+Shift+S`, which is where §4.4 already says it
lives.

## Recommendation

**C, landed together with §4.2's external-edit check — not without it.**

C is the smallest change that makes the **ADR** true instead of making the **keybinding** true:
the arming code already exists at `engine.rs:1047`, so C adds an input to a gate, not a
policy, and `document.rs:182-199` keeps one fixed order with one new reason. It restores
§4.4 (`features.md:76-77` already specifies both chords) instead of re-documenting the
accident, so README:23 and README:26 need no caveats and `main.rs:1217-1219` becomes
literal. Not B: B's extra reach is exactly the overwrite ADR-0001 exists to make deliberate,
paid for before the thing that makes it safe exists. Not A: A spends the docs to protect a
shape that `menu.rs:19-24` shows was chosen because Windows draws no menu bar — an
accident of platform chrome is a weak reason to redefine a headline feature.

Stated as a rule, because it is the part that gets forgotten: **C's in-place write must not
ship before something detects that the file changed on disk since `Event::Loaded`.** If
that cannot land in M2, take A for now — with the skip line corrected to name the dialog —
rather than shipping B's blind overwrite to keep a chord.

## Consequences

- B and C both change the `api` vocabulary, so no bridge-only fix exists (AGENTS.md: a
  bridge may not reach around the port). `command.rs:139-178`'s exhaustive `rebuild`
  match means a new variant cannot arrive silently — the compiler asks first.
- Under B/C the "was this asked for by hand" input belongs in **core**, next to the skip
  order. Do not put the exception in the bridge: `main.rs:1721` and `main.rs:1774-1781`
  already gate on `armed`, and a third gate in a second crate is how one gets fixed and
  the other does not.
- C makes recents honest again (a save is not a choice) and leaves `self.epoch` alone for a
  document that did not change identity, so `flush_due`'s echo stays valid across a
  `Ctrl+S`.
- C turns `roundtrip_port.rs:20-30`'s "one honest deviation" into the real product path:
  Open -> Save -> Flush, byte-identity judgement unchanged. A new dirty-target case joins it.
- ADR-0001 is **not** superseded here. C implements its wording; A would edit
  `README.md` and `docs/features.md` (intent), never the ADR, which is append-only.
- What this does NOT fix: the bar is still undrawn on Windows (`menu.rs:19-24`), so a new
  chord must also gain a `SHORTCUTS` row at `menu.rs:116-131` and a `binding_for` arm
  (`menu.rs:136-155`) — `menu.rs:300-321` fails the build if either is forgotten, which is
  the safety net this note relies on.

## Reopening conditions

- Move to **A** if external-change detection cannot be built and a real user clobbers a
  version-controlled or synced file via `Ctrl+S` — then the dialog is the only guard and
  §4.4 should be amended instead. R6 (`docs/risks.md:25`) is that scenario named.
- Move to **B** (in-place always, no dialog) once §4.2's warn-and-keep-both exists and the
  corpus at `roundtrip_port.rs:792-803` still passes byte-exactly on all 30 fixtures plus a
  dirty-target case.
- Reopen wholesale if a second bridge lands: the split between "the chord is the menu" (a
  Windows-GPUI fact) and "an explicit save arms" (a product rule) is only visible here because
  `menu.rs:19-24` wrote it down.
- Reopen if `Event::AutosaveSkipped` gains a setting readback: `main.rs:890-894` infers
  the toggle from `Saved` **only because** no manual Save exists. B or C breaks that
  inference, and ADR-0001 requirement 3 arrives whether or not anyone schedules it.
