//! SLINT SPIKE, slice 0 - the skeleton, and NOTHING more.
//!
//! This crate exists so the workspace, the layering gate and CI know a second bridge
//! is possible. It is deliberately inert: no `.slint` file, no window, no event loop,
//! no compilation macro. Slice 1 (72b) starts from here and writes the UI. What must
//! survive that work is the reason this file exists: the bridge owns the text editor
//! and the window, and notes-api never sees a keystroke (AGENTS.md, architecture
//! invariants). A Slint bridge that reaches around the port is exactly what
//! `cargo xtask check-arch` now polices for BOTH bridges on day one, rather than
//! after there is UI to unpick.
//!
//! `fn main()` is empty on purpose. The two constants below are the only code, and
//! they exist for a lint rather than for a feature: the workspace denies an unused
//! crate dependency (`unused_crate_dependencies = "warn"`, and CI adds `-D warnings`),
//! so a skeleton that declares notes-api and slint without naming either would be red
//! on the day it lands. Naming each in a `size_of!` is a compile-time fact, links
//! nothing, is not UI code, and goes away the moment slice 1 has real uses for both.

/// The port this bridge may talk to - the ONLY repo crate in its graph.
const _: u64 = std::mem::size_of::<notes_api::Command>() as u64;
/// Its own toolkit. `SharedString` is the type a Slint callback hands over and the
/// one a `TextInput` binds, so slice 1 will not need a different name to prove the
/// edge is real.
const _: u64 = std::mem::size_of::<slint::SharedString>() as u64;

fn main() {}
