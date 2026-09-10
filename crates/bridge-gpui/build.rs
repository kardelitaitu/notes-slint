//! Embeds `app.manifest` in the shipped executable, dependency-free.
//!
//! `winres` and `embed-manifest` are not in `[workspace.dependencies]`, and a
//! build-time crate for one linker flag is not worth the graph: link.exe already
//! does this with `/MANIFEST:EMBED` plus one `/MANIFESTINPUT`, merging the inputs
//! itself. Why the manifest is not optional is written at the top of app.manifest.
//!
//! There is a second manifest in the dependency graph: gpui embeds its own,
//! `PerMonitorV2`-only copy through its `windows-manifest` default feature. This
//! crate turns that feature off (see Cargo.toml) and so owns the single
//! authoritative manifest. Otherwise two files would disagree about longPathAware
//! and nothing in the source tree would say which one won.

fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=build.rs");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        // A no-op, not an error: DPI awareness and long-path behaviour are Win32
        // concepts, and the workspace must still configure on the mac and Linux
        // slices. Warn out loud so nobody assumes the embed also happened there.
        println!(
            "cargo:warning=notes-gpui: no Win32 manifest embedded for target-os {target_os}; \
            dpi awareness is a Windows-only contract, so none is needed here."
        );
        return;
    }

    let manifest = std::path::Path::new(
        &std::env::var("CARGO_MANIFEST_DIR").expect("cargo always sets CARGO_MANIFEST_DIR"),
    )
    .join("app.manifest");
    if !manifest.is_file() {
        panic!(
            "{} is missing: it is the DPI and long-path contract of this binary",
            manifest.display()
        );
    }

    // link-arg-bins, not link-arg: the flag belongs to the executable, not to every
    // artifact this package produces.
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
        manifest.display()
    );
}
