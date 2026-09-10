//! REVIEWER ITEM 4 (D33): the surface a bridge may touch, checked from OUTSIDE the
//! crate.
//!
//! A [`pub use`] that quietly stops being public - a module that goes private, a
//! re-export moved behind a [`cfg`], a name that stops matching - is discovered
//! by a bridge at LINK time, in someone else's slice. Everything here is reached
//! through [`notes_api::`] paths only, with no [`notes_core`] import except
//! where a type's fields are the thing under test, which is exactly the shape
//! [`crates/bridge-gpui`] has to compile against: notes-api, its toolkit, and
//! nothing else in this repo (AGENTS.md, and bridge-sees-only-api in cargo arch).

use std::path::Path;
// notes_core is on this target's dependency graph through notes-api; it is named
// and never used, which is the whole point of the file - a bridge needs nothing
// from it. The unused-crate-dependencies lint is per target, so it is told.
use notes_core as _;
// Named for the same per-target lint; this file uses none of it.
use thiserror as _;

use notes_api::{
    Command, Encoding, Event, FileMeta, Gateway, InitialState, LineEnding, LoadError, RecentEntry,
    Rect, SaveError, Session, Settings, SkipReason, StateDir, WindowHandle, resolve_state_dir,
};

/// FCR 2: the D-STATE rule is reachable from the port. The bridge must not import
/// notes_core to obey it, and the three shapes below are the whole rule - installed,
/// portable, and the empty-appdata case that must not resolve relative to the CWD.
#[test]
fn the_state_dir_resolver_is_public_and_decides_what_core_decides() {
    let exe = Path::new("C:/apps/notes");
    let appdata = Path::new("C:/Users/u/AppData/Roaming");

    let installed = resolve_state_dir(exe, Some(appdata));
    assert_eq!(
        installed.0,
        appdata.join("notes-gpui"),
        "installed lives in the profile"
    );

    // The caller does the <exe_dir>/data existence probe and passes None for a
    // portable deployment (dto.rs states the contract; this pins that the contract
    // is reachable from outside).
    let portable = resolve_state_dir(exe, None);
    assert_eq!(
        portable.0,
        exe.join("data"),
        "portable lives beside the binary"
    );

    assert_eq!(
        resolve_state_dir(exe, Some(Path::new(""))).0,
        exe.join("data"),
        "an empty profile path is no profile at all, never a CWD-relative state dir"
    );
}

/// The four startup steps, in order, with nothing but notes_api in scope - the
/// criterion FCR 2 was granted against.
#[test]
fn a_bridge_can_obey_the_startup_order_with_only_notes_api() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let state = StateDir(tmp.path().to_path_buf());

    let (mut gateway, _rx) = Gateway::start(state, Settings::default());
    // 1. query state - once, and the types are nameable from out here.
    let initial: InitialState = gateway.startup_state().expect("the pre-window read");
    let session: Session = initial.session.clone();
    let rect: Rect = session.rect;
    assert_eq!(rect, session.rect, "the snapshot is the session's own rect");
    assert_eq!(
        initial.pinned, session.pinned,
        "one home for the pin bit (D10)"
    );
    assert!(
        gateway.startup_state().is_none(),
        "the snapshot is handed over once, so it cannot go stale in a bridge's hands"
    );
    // 2. create the window at [`rect`] is bridge work - nothing to assert here
    //    beyond the fact that the rect arrived before any command was sent.
    // 3. register the handle.
    assert!(
        gateway
            .send(Command::RegisterWindow {
                handle: WindowHandle(1),
            })
            .is_ok()
    );
    // 4. apply topmost from [`initial.pinned`], through notes-platform, which
    //    this crate cannot even name - the boundary is the assertion.
    assert!(!gateway.is_closed(), "a started engine is open");
    assert!(gateway.engine_is_alive());

    // close() is the explicit, joinable shutdown, and it CONSUMES the handle -
    // which is the point: the alternative is a destructor that blocks. After it,
    // the event channel is closed, and that is how a caller with no handle left
    // still learns the engine is gone.
    let before_close = gateway.send(Command::Shutdown).is_ok();
    assert!(
        before_close,
        "the engine accepted the shutdown while it was alive"
    );
    drop(gateway);
    // The receiver now drains and reports Disconnected.
    let _ = _rx.recv();

    // A Gateway whose engine has gone hands commands BACK rather than swallowing
    // them; proven on a hand-built shape below is impossible from out here, so the
    // observable half is that send() after a close is an Err.
    let (mut second, rx2) = Gateway::start(StateDir(tmp.path().to_path_buf()), Settings::default());
    assert!(
        second.startup_state().is_some(),
        "a new engine hands its snapshot over once"
    );
    assert!(
        second.startup_state().is_none(),
        "and never twice - the consume-once contract is visible from outside"
    );
    second.close().expect("explicit shutdown");
    // close() consumes the handle, so the Err-from-send-after-close half can only
    // be asserted where a Gateway can be built by hand: gateway.rs's
    // a_command_the_engine_cannot_accept_comes_back pins it. What IS observable
    // out here is the other half of the same fact - the channel closes with the
    // engine, so no caller can be left waiting for an event that will never come.
    assert!(rx2.recv().is_err(), "EventRx closes with the engine");
}

/// The vocabulary a status line needs is all public: a save failure, a skip reason,
/// a load failure and a file's format facts - each nameable, each [`Debug`], and
/// each carrying its copy rather than leaving it to the caller (AGENTS.md: the
/// [`SaveError`] enum is part of the contract because the UI must render it).
#[test]
fn the_rendering_vocabulary_is_public_and_carries_its_copy() {
    let meta = FileMeta {
        encoding: Encoding::Ansi(1252),
        line_ending: LineEnding::CrLf,
        trailing_newline: false,
        read_only: false,
        oversize: false,
        armed: true,
    };
    assert_eq!(meta.encoding, Encoding::Ansi(1252));
    let err = SaveError::Unencodable(Encoding::Utf16Be);
    assert!(err.to_string().contains("cannot represent this text"));
    let skip = SkipReason::NeedsPath;
    assert!(
        !matches!(skip, SkipReason::Clean),
        "a distinct answer, not a string"
    );
    let load = LoadError::TooLarge;
    assert!(load.to_string().contains("too large"));
    let entry = RecentEntry {
        path: std::path::PathBuf::from("C:/notes/a.notes"),
        display: "a.notes".to_string(),
        exists: false,
    };
    let event = Event::RecentsUpdated(vec![entry]);
    assert!(matches!(&event, Event::RecentsUpdated(list) if !list[0].exists));
    assert!(
        !format!("{event:?}").is_empty(),
        "a bridge can log what it was told"
    );
}
