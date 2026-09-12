# dev

**Developer documentation — how to build, test, and change this code.** Human-facing,
present-tense, and describing what exists rather than what is planned.

> M0 answered yes and the workspace builds: two files from the contract below now
> exist — [`getting-started.md`](getting-started.md) and
> [`testing.md`](testing.md). The rest of the table still describes *when* each
> file gets written, not a request to fill it in speculatively.

## Intended contents, and the milestone that unlocks each

| File | Covers | Write it when |
|---|---|---|
| `getting-started.md` | Toolchain, `rust-toolchain.toml`, first build, first run, GPUI version pin | **Written** — M0 answered yes |
| `architecture.md` | The four crates, `api` port, bridges, startup order, threading | M2, once the layering is real rather than designed |
| `invariants.md` | The `cargo tree` checks and *why* each exists, long-form | M1 (the commands themselves live in `AGENTS.md`) |
| `testing.md` | Round-trip fixtures, headless session tests through `api`, `cargo test` | **Written** — M1 landed |
| `packaging.md` | The five artifacts, CI matrix, signing/notarisation setup | M5, gated by the §10.8 account decision |
| `windows.md` | DPI awareness manifest, `SetWindowPos` behaviours, per-monitor scaling gotchas | M3 |

## House rules

- **Commands go in `AGENTS.md`, prose goes here.** One canonical list of "how to verify", or
  the two copies will disagree. Link, don't duplicate.
- If anything here contradicts the code, **the code wins and this file is a bug.** Fix the
  doc in the same commit that changed the behaviour.
- Do not restate the whitepaper. It holds the *why* and the *plan*; this folder holds the
  *how*, current tense. A paragraph explaining motivation here is usually a paragraph that
  belongs in `../decisions/`.
- One topic per file. `windows.md` growing a packaging section means packaging deserves its
  own file, not a heading.
- Keep `getting-started.md` runnable by a newcomer with no context. If it needs a
  predecessor doc to make sense, it is in the wrong place.
