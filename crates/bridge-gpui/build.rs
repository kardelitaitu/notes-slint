//! NO LONGER EMBEDS ANYTHING, ON PURPOSE - and `app.manifest` stays in the tree.
//!
//! This build script used to add `/MANIFEST:EMBED` plus `/MANIFESTINPUT=app.manifest` to the
//! executable's link line, dependency-free, so that the binary could declare `longPathAware` and
//! `PerMonitorV2` while gpui's own manifest feature was switched off in `[workspace.dependencies]`.
//!
//! The kit generation took that choice away. `gpui-pre-platform` force-enables `windows-manifest` in
//! its OWN dependency declaration; cargo features are additive, so no line in this workspace can turn
//! it off, and gpui's RT_MANIFEST (type 1032/ID 1... the resource the linker calls ID 24) is now
//! always present in this executable. With our embed still in place all three spike variants died the
//! same way:
//!
//!     rust-lld: error: duplicate resource: type MANIFEST (ID 24)
//!
//! so ours went first, because a bridge that does not link has no opinion about anything else.
//!
//! WHAT THAT COSTS, DATED AND TEMPORARY (not an oversight): until `cargo xtask manifest` lands - the
//! other lane is writing it now, and it replaces the embedded resource POST-LINK and verifies by
//! reading our marker back out of the exe - this binary carries GPUI's manifest. A full audit of the
//! two files says exactly one behaviour is lost: `longPathAware`, which is load-bearing for a
//! USER-CHOSEN path over ~222 characters because core's temp sibling adds up to 42 more. Our assembly
//! identity goes with it, and nothing reads that. `PerMonitorV2` is declared identically on both
//! sides, so no coordinate assumption in the geometry path moves by one pixel.
//!
//! `app.manifest` is deliberately NOT deleted: it is the input the post-link step writes back, and the
//! contract it states (DPI awareness, long paths) is still this binary's contract.

fn main() {
    // Nothing to do at build time. The rerun lines are gone with the work: nothing here reads a file.
    // Kept as a script rather than deleted because this file is where the reason belongs, and because
    // removing a build script silently is how the next reader puts the embed back.
}
