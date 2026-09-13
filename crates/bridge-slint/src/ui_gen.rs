// THE SLINT ENTRY, verbatim from probe.rs (STRIP-2a). The five lines that used to sit
// above it, moved here so the reason travels with the code:
// The UI lives in ui/main.slint, imported rather than inlined: that file is one of
// xtask smoke's freshness roots, so editing the markup behind a built binary makes the
// binary stale in the guard's eyes instead of invisible to it. 1.17 finding 4: the
// `export` line takes NO trailing semicolon, and relying on an implicit re-export of
// the last import is deprecated - so it is spelled.

//! STRIP-2a: THE GENERATED UI, in its own module so BOTH roots can declare it.
//!
//! The `slint::slint!` macro below is verbatim from probe.rs. It expands to items in whatever
//! module encloses it, which is why a module file works: the crate root declares
//! `mod ui_gen; use ui_gen::Spike;`, and every existing reference to `crate::Spike` - the probe's
//! acts and plumbing.rs's import - keeps resolving without a single call site changing. The
//! markup paths inside ("../ui/main.slint") are relative to THIS file, which sits in the same
//! directory the macro sat in before, so they are unchanged.
slint::slint! {
    import { Spike } from "../ui/main.slint";
    export { Spike }
}
