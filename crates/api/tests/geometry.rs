//! The geometry join, proved against a recording host (D33/D40/D46/D48).
//!
//! These tests own the claims the live bridge run could only observe by
//! accident: the PORT moves the window, in frame pixels, exactly once, on the
//! FIRST registration and never on a later one; a maximized session is never
//! moved but still gets its pin; a refused move is reported, never silenced;
//! and what is persisted is the rect the host REPORTS, not the one the port
//! asked for. Every platform call is made by [`host_mock::Host`] - see that
//! file's header for what it stands in for - so nothing here touches Win32 and
//! every assertion is a recorded call or a byte on disk.
//!
//! The state dir starts EMPTY or with a session.json written by core's own
//! writer: there is no other way to control what the engine restores from,
//! because [`Gateway::start_with_host`] reads the session from disk exactly
//! as a real launch would.

#[path = "support/host_mock.rs"]
mod host_mock;

use std::path::Path;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use notes_core::session::{read_session, write_session};
use notes_platform::FrameRect;

use host_mock::{Answers, Host};
use notes_api::{Command, Event, Gateway, Rect, Session, Settings, StateDir, WindowHandle};

// thiserror is a dependency of notes-api, not of this test target; naming it
// keeps the unused-crate-dependencies lint honest about that.
use thiserror as _;

/// Long enough that a loaded machine cannot flake; short enough that a hung
/// engine fails the run instead of hanging it forever.
const ANSWER: Duration = Duration::from_secs(5);

/// Starts an engine on `dir` with the fake host answering `answers`, and
/// hands the SAME host back for assertions: the engine owns one clone, the
/// test the other, and the call log is shared through the Arc inside.
fn start_with(dir: &Path, answers: Answers) -> (Gateway, Receiver<Event>, Host) {
    let host = Host::with_answers(answers);
    let (gateway, rx) = Gateway::start_with_host(
        StateDir(dir.to_path_buf()),
        Settings::default(),
        Some(Box::new(host.clone())),
        Some(Box::new(host.clone())),
    );
    (gateway, rx, host)
}

/// Waits until the engine thread has made at least `at_least` recorded calls:
/// registration is asynchronous, and the success path emits nothing (silence
/// IS the success signal), so the call log is the only receipt there is.
fn wait_for_calls(host: &Host, at_least: usize, what: &str) {
    let deadline = Instant::now() + ANSWER;
    while host.calls().len() < at_least {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// D40, in one assertion set: one move, frame pixels, scale 1.0, the CLAMPED
/// rect. The input rect hangs 780 px off a 1920 work area - 20 visible pixels,
/// under the port's MIN_VISIBLE of 32 - so the clamp has real work to do, and
/// only the position may change. The expected value comes from core's own rule
/// ([`Rect::clamped_to`]) because the clamp is core's judgement: what this
/// test proves is that the port PASSES the clamped rect through, unchanged, at
/// scale 1.0 - not that it re-implements the clamp.
#[test]
fn the_first_registration_moves_the_window_once_at_the_clamped_frame_rect() {
    let dir = tempfile::tempdir().expect("tempdir");
    let saved = Rect::new(1900, 100, 800, 600);
    write_session(
        dir.path(),
        &Session {
            rect: saved,
            ..Session::default()
        },
    )
    .expect("write the session fixture");
    let (gateway, _rx, host) = start_with(dir.path(), Answers::default());

    gateway
        .send(Command::RegisterWindow {
            handle: WindowHandle(0x100),
        })
        .expect("queued");
    wait_for_calls(&host, 1, "the registration's platform calls");
    gateway.close().expect("shutdown joins the engine");

    let moves = host.moves();
    assert_eq!(
        moves.len(),
        1,
        "exactly one move, on the first registration only"
    );
    let (handle, rect, scale) = moves[0];
    assert_eq!(handle, 0x100);
    let clamped = saved.clamped_to(Rect::new(0, 0, 1920, 1032), 32);
    assert_ne!(
        clamped, saved,
        "the fixture really does not fit the work area"
    );
    assert_eq!(
        rect,
        FrameRect::new(clamped.x, clamped.y, clamped.w, clamped.h),
        "the port moves to the CLAMPED rect, in frame pixels, no rescale"
    );
    assert_eq!(
        scale, 1.0,
        "the rect is already in this window's frame space"
    );
}

/// A maximized session is not yanked out of maximization to be moved - but the
/// pin still applies, because it is orthogonal to placement, and the restore
/// rect is still measured (D48): the NORMAL position is the number worth
/// persisting even while the window is maximized.
#[test]
fn a_maximized_session_is_never_moved_but_still_pinned() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_session(
        dir.path(),
        &Session {
            maximized: true,
            pinned: true,
            ..Session::default()
        },
    )
    .expect("write the session fixture");
    let (gateway, _rx, host) = start_with(dir.path(), Answers::default());

    gateway
        .send(Command::RegisterWindow {
            handle: WindowHandle(0x100),
        })
        .expect("queued");
    // close() joins the engine thread: after it, no further platform call can
    // happen, so an empty move log is a proven zero, not an unlucky poll.
    gateway.close().expect("shutdown joins the engine");

    assert!(host.moves().is_empty(), "a maximized window is not moved");
    assert_eq!(
        host.topmost(),
        vec![(0x100, true)],
        "the pin is not part of the move question"
    );
}

/// A second registration is a recreate, not a restore: the user may have moved
/// the window by hand in between, and the port must never yank it back.
#[test]
fn a_re_registration_never_moves_the_window_a_second_time() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_session(dir.path(), &Session::default()).expect("write the session fixture");
    let (gateway, _rx, host) = start_with(dir.path(), Answers::default());

    gateway
        .send(Command::RegisterWindow {
            handle: WindowHandle(0x100),
        })
        .expect("queued");
    wait_for_calls(&host, 1, "the first registration's move");
    assert_eq!(host.moves().len(), 1, "the first registration moved once");

    gateway
        .send(Command::RegisterWindow {
            handle: WindowHandle(0x100),
        })
        .expect("queued");
    gateway.close().expect("shutdown joins the engine");
    assert_eq!(
        host.moves().len(),
        1,
        "the second registration moved nothing"
    );
}

/// Silence is forbidden: a refused move arrives as
/// [`Event::GeometryNotRestored`], carrying the rect the port asked for and
/// the platform's own sentence, untranslated.
#[test]
fn a_refused_move_is_reported_not_swallowed() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_session(dir.path(), &Session::default()).expect("write the session fixture");
    let (gateway, rx, host) = start_with(
        dir.path(),
        Answers {
            fail_move: Some("access is denied.".to_string()),
            ..Answers::default()
        },
    );

    gateway
        .send(Command::RegisterWindow {
            handle: WindowHandle(0x100),
        })
        .expect("queued");
    let event = rx
        .recv_timeout(ANSWER)
        .expect("the refusal must be emitted, never silenced");
    match event {
        Event::GeometryNotRestored { rect, reason } => {
            assert!(
                reason.contains("access is denied"),
                "the platform's own words travel through: {reason}"
            );
            assert_eq!(
                rect,
                Rect::new(120, 90, 800, 600),
                "the rect the port asked for, nameable in the copy"
            );
        }
        other => panic!("expected GeometryNotRestored, got {other:?}"),
    }
    assert_eq!(host.moves().len(), 1, "the move WAS attempted, once");
    gateway.close().ok();
}

/// THE FRESH-INSTALL BUG the live run proved on disk: nothing marked the
/// session dirty at startup, so a user who never dragged the window never got
/// a session.json at all - the headline feature silently depended on the user
/// having moved the window once. Registration is the moment that changes. And
/// what lands in the file is the rect the host REPORTS (the fake's
/// `restore` answer, D48), not the rect the session asked with: a clamp or a
/// chrome correction that was not measured would re-diverge on every launch.
#[test]
fn a_fresh_install_writes_the_session_with_the_rect_the_host_reports() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(
        !dir.path().join("session.json").exists(),
        "fixture: this must start with an empty state dir"
    );
    let reported = FrameRect::new(11, 22, 333, 222);
    let (gateway, _rx, host) = start_with(
        dir.path(),
        Answers {
            restore: Some(reported),
            ..Answers::default()
        },
    );

    gateway
        .send(Command::RegisterWindow {
            handle: WindowHandle(0x100),
        })
        .expect("queued");
    wait_for_calls(&host, 1, "the registration's first platform call");
    // The move is ASYNC: the measured rect only becomes trustworthy after the
    // move-in-flight guard expires (MAJOR 5), so the write that carries the
    // measured rect is the one queued after that window - still the existing
    // single tick, no second timer.
    std::thread::sleep(Duration::from_millis(750 * 2 + 100));
    gateway.send(Command::SetPinned(true)).expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        if let Ok(session) = read_session(dir.path()) {
            if session.rect == Rect::new(11, 22, 333, 222) {
                break;
            }
        }
        assert!(Instant::now() < deadline, "the measured rect never landed");
        std::thread::sleep(Duration::from_millis(20));
    }
    gateway
        .close()
        .expect("shutdown flushes the queued session write");

    let persisted = read_session(dir.path())
        .expect("a fresh install must have written session.json on registration");
    assert!(
        host.restore_reads() >= 1,
        "the persisted rect came from restore_frame_rect, not from the input"
    );
    assert_ne!(
        Session::default().rect,
        persisted.rect,
        "the stored rect is not the one the session asked with"
    );
    assert_eq!(
        persisted.rect,
        Rect {
            x: 11,
            y: 22,
            w: 333,
            h: 222
        },
        "the MEASURED restore rect the fake host reported, not the requested one"
    );
}

/// MAJOR 2/5: the stored monitor facts are REFRESHED, not frozen at launch.
/// The world changes under the window (the mock is told to move the monitor);
/// the NEXT flush must name the new monitor in session.json. Red-before: the
/// old code set monitor_id once at registration and never again, so the file
/// named the launch-time monitor forever. Also the other side of the contract:
/// an UNCHANGED world must not turn the idle tick into a disk write - the
/// refresh is change-gated and only runs inside an already-pending write.
#[test]
fn the_flush_refreshes_monitor_facts_when_the_world_changes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (gateway, _rx, host) = start_with(
        dir.path(),
        Answers {
            monitor_id: 1,
            ..Answers::default()
        },
    );

    // First: register on monitor 1 and queue a change so the tick flushes.
    gateway
        .send(Command::RegisterWindow {
            handle: WindowHandle(0x100),
        })
        .expect("queued");
    gateway.send(Command::SetPinned(true)).expect("queued");

    // The FILE is the signal: poll it (bounded) until the first flush has
    // landed with the launch-time monitor.
    let deadline = Instant::now() + ANSWER;
    let first = loop {
        if let Ok(session) = read_session(dir.path()) {
            if session.monitor_id == 1 && session.pinned {
                break session;
            }
        }
        assert!(Instant::now() < deadline, "the first flush never landed");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(first.monitor_id, 1);

    // IDLE WORLD, NO DISK WRITE: two tick periods with nothing changed must
    // leave the file byte-identical - a refresh that made every 750 ms tick
    // rewrite the file would be a battery-life bug in a notepad.
    let before_idle = std::fs::read(dir.path().join("session.json")).expect("read");
    std::thread::sleep(Duration::from_millis(750 * 2 + 100));
    assert_eq!(
        std::fs::read(dir.path().join("session.json")).expect("read again"),
        before_idle,
        "an unchanged world must not rewrite the session"
    );

    // Next, THE WORLD CHANGES: the window now lives on monitor 2 at 175%, a
    // different work area, and the host reports a different restore rect.
    host.set_answers(Answers {
        monitor_id: 2,
        work_area: FrameRect::new(1920, 0, 1920, 1040),
        restore: Some(FrameRect::new(2000, 100, 800, 600)),
        scale: 1.75,
        ..Answers::default()
    });
    // Finally: flush again - close() makes the final drain deterministic.
    gateway.send(Command::SetPinned(false)).expect("queued");
    gateway.close().expect("shutdown joins the engine");

    let second = read_session(dir.path()).expect("the second flush wrote the session");
    assert_eq!(
        second.monitor_id, 2,
        "the file must name the NEW monitor, not the launch-time one"
    );
    assert_eq!(
        second.scale_factor, 1.75,
        "the file must name the NEW scale, not the launch-time one"
    );
    assert!(
        !second.pinned,
        "and the changed pin bit, from the same flush"
    );
    // Both flushes MEASURED (the restore rect was read at each one), which is
    // what feeds the refresh.
    assert!(
        host.restore_reads() >= 2,
        "each flush must have measured the restore rect"
    );
}

/// MAJOR 3: the handle is a VALUE, not a lease. After UnregisterWindow, no
/// host call may happen (nothing moves - a RECYCLED handle would pass IsWindow
/// and name a stranger) and a post-unregister GeometryChanged is a no-op (no
/// stranger's rect lands in session.rect). The fake's restore answer is set
/// equal to the session's rect, so the persisted value proves which path
/// wrote it: anything other than `before` means the GeometryChanged leaked.
#[test]
fn after_unregister_nothing_moves_and_no_rect_is_written() {
    let dir = tempfile::tempdir().expect("tempdir");
    let before = Rect::new(50, 60, 700, 500);
    write_session(
        dir.path(),
        &Session {
            rect: before,
            ..Session::default()
        },
    )
    .expect("write the session fixture");
    let (gateway, _rx, host) = start_with(
        dir.path(),
        Answers {
            restore: Some(FrameRect::new(50, 60, 700, 500)),
            ..Answers::default()
        },
    );

    gateway
        .send(Command::RegisterWindow {
            handle: WindowHandle(0x100),
        })
        .expect("queued");
    wait_for_calls(&host, 1, "the registration's move");
    let moves_at_registration = host.moves().len();

    gateway.send(Command::UnregisterWindow).expect("queued");
    // A geometry update for a window the port no longer holds.
    gateway.send(Command::GeometryChanged).expect("queued");
    // Re-registering would be a legitimate NEW window; deliberately not sent.
    gateway.close().expect("shutdown joins the engine");

    assert_eq!(
        host.moves().len(),
        moves_at_registration,
        "nothing may move after UnregisterWindow"
    );
    let persisted = read_session(dir.path()).expect("session readable");
    assert_eq!(
        persisted.rect, before,
        "the post-unregister GeometryChanged must not write session.rect"
    );
}

/// MAJOR 4: the shutdown drain re-runs every queued command, so a
/// RegisterWindow behind Shutdown used to run SetWindowPos again at exit - in
/// the exact state where the UI thread already holds the (now bounded) join.
/// The drain stores the handle as state but performs no host work.
#[test]
fn the_shutdown_drain_does_not_replay_the_restore() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_session(dir.path(), &Session::default()).expect("write the session fixture");
    let (gateway, _rx, host) = start_with(dir.path(), Answers::default());

    // First registration moves once (the legitimate restore).
    gateway
        .send(Command::RegisterWindow {
            handle: WindowHandle(0x100),
        })
        .expect("queued");
    wait_for_calls(&host, 1, "the first move");

    // Shutdown first, a second RegisterWindow BEHIND it: the drain must store
    // the handle and skip the move.
    gateway.send(Command::Shutdown).expect("queued");
    gateway
        .send(Command::RegisterWindow {
            handle: WindowHandle(0x200),
        })
        .expect("queued behind shutdown");
    gateway.close().expect("shutdown joins the engine");

    assert_eq!(
        host.moves().len(),
        1,
        "the drain replayed the restore: a second move happened at exit"
    );
}

/// MAJOR 5: an async move returns BEFORE it lands, so the flush that runs
/// inside two idle periods of issuing one measures the PRE-MOVE position - and
/// persisting that wrote a rect the window was only PASSING through. The guard
/// refuses to store a rect measured inside that window: the file keeps the
/// last good rect, and the landed rect persists on the next GeometryChanged
/// once the guard expires.
#[test]
fn a_measure_taken_before_a_move_lands_is_never_persisted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pre_move = Rect::new(120, 90, 800, 600);
    write_session(
        dir.path(),
        &Session {
            rect: pre_move,
            ..Session::default()
        },
    )
    .expect("write the session fixture");
    // The host answers the PRE-MOVE normal position first: until the move
    // lands, that is what GetWindowPlacement reports.
    let (gateway, _rx, host) = start_with(
        dir.path(),
        Answers {
            restore: Some(FrameRect::new(120, 90, 800, 600)),
            ..Answers::default()
        },
    );

    // Register: the move is issued and stamped. The queued session write
    // flushes inside the guard window - with the pre-move rect in the air.
    gateway
        .send(Command::RegisterWindow {
            handle: WindowHandle(0x100),
        })
        .expect("queued");
    wait_for_calls(&host, 1, "the registration's move");

    // The tick (or close) flushes while the move is still in flight.
    gateway.send(Command::SetPinned(true)).expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        if let Ok(session) = read_session(dir.path()) {
            if session.pinned {
                break;
            }
        }
        assert!(Instant::now() < deadline, "the guarded flush never landed");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        read_session(dir.path()).expect("read").rect,
        pre_move,
        "the pre-move measure must NOT be persisted over the stored rect"
    );

    // The move lands, the guard expires, and the host now reports the landed
    // position: the next flush persists THAT (a pin toggle queues the write).
    host.set_answers(Answers {
        restore: Some(FrameRect::new(200, 150, 800, 600)),
        ..Answers::default()
    });
    std::thread::sleep(Duration::from_millis(750 * 2 + 100));
    gateway.send(Command::SetPinned(false)).expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        if let Ok(session) = read_session(dir.path()) {
            if session.rect == (Rect::new(200, 150, 800, 600)) {
                break;
            }
        }
        assert!(Instant::now() < deadline, "the landed rect never persisted");
        std::thread::sleep(Duration::from_millis(20));
    }
    gateway.close().expect("shutdown joins the engine");
}

/// THE STARTUP ANNOUNCE: a launch whose state holds recents must announce the
/// list BEFORE any command, or the menu renders empty until something changes
/// (silently wrong on every launch, and invisible to any test that triggers a
/// change first). The entries are core's stored facts with core's rendered
/// labels; the empty case stays silent because the bridge's default IS empty,
/// which is why the reentrancy no-event contract survives.
#[test]
fn the_first_event_names_the_persisted_recents_before_any_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Two persisted recents, written by core's own writer - exactly what a
    // second launch finds on disk.
    let settings = Settings {
        recents: vec![
            notes_core::recent::RecentEntry {
                path: Path::new("C:/notes/one.notes").to_path_buf(),
                display: "one.notes".to_string(),
                exists: false,
            },
            notes_core::recent::RecentEntry {
                path: Path::new("C:/notes/two.notes").to_path_buf(),
                display: "two.notes".to_string(),
                exists: false,
            },
        ],
        ..Settings::default()
    };
    notes_core::settings::write_settings(&StateDir(dir.path().to_path_buf()), &settings)
        .expect("seed settings.toml");

    let (gateway, rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        Settings::default(),
        None,
        None,
    );
    // NO COMMAND HAS BEEN SENT. The first thing the port says must be the
    // list, with the stored paths in order and labels rendered.
    let first = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the recents announce must precede every command");
    match first {
        Event::RecentsUpdated(entries) => {
            assert_eq!(entries.len(), 2, "the stored list, verbatim in count");
            assert_eq!(
                entries[0].path,
                Path::new("C:/notes/one.notes"),
                "most-recent-first order as persisted"
            );
            assert!(
                entries.iter().all(|e| !e.display.is_empty()),
                "labels are rendered, not blank"
            );
            assert!(
                entries.iter().all(|e| !e.exists),
                "exists is the stored fact - the files were never created"
            );
        }
        other => panic!("expected RecentsUpdated as the FIRST event, got {other:?}"),
    }
    gateway.close().expect("shutdown joins the engine");
}

/// The settings contract's middle case, through the port's OWN seam: the
/// settings left the codepage unset, and the host's ANSI code page fills the
/// gap - here the fake's CP932, which must turn a CP1252 byte stream into a
/// refusal, not a guess (D27). No backend is supplied, because this test does
/// not register a window.
#[test]
fn the_hosts_ansi_codepage_fills_the_gap_the_settings_left() {
    let bytes: Vec<u8> = [b"caf".as_slice(), &[0xE9u8], b" notes"].concat();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("fr.notes");
    std::fs::write(&path, &bytes).expect("write the fixture");

    let host = Host::with_answers(Answers {
        codepage: 932,
        ..Answers::default()
    });
    let (gateway, rx) = Gateway::start_with_host(
        StateDir(dir.path().to_path_buf()),
        Settings::default(),
        None,
        Some(Box::new(host)),
    );
    gateway
        .send(Command::Open { path: path.clone() })
        .expect("queued");
    let event = rx.recv_timeout(ANSWER).expect("an answer to Open");
    assert!(
        matches!(&event, Event::LoadFailed { path: p, .. } if p == &path),
        "CP932 cannot hold an unpaired 0xE9: {event:?}"
    );
    gateway.close().ok();
}

/// THE HINT MUST NEVER WIN THE WRITE (the drift bug): the bridge's
/// GeometryChanged carries ITS OWN space (gpui's window_bounds, client-ish),
/// and the platform dossier showed a move-then-quit persisting exactly that
/// hint: 390,278,1010,698 frame went in, 398,297,1002,678 client came out -
/// the chrome applied, walking the window down-right on every cycle. The rule
/// now: while a move is in flight the flush writes NOTHING (the measured
/// read-back is stale and the stored rect may be the hint), so the file keeps
/// the previous persisted value until an honest measure can win; and once the
/// guard expires, the measure replaces the hint before the write.
#[test]
fn the_hint_never_wins_the_write_and_the_measure_replaces_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let previous = Rect::new(50, 50, 700, 500);
    write_session(
        dir.path(),
        &Session {
            rect: previous,
            ..Session::default()
        },
    )
    .expect("write the session fixture");
    // The MEASURED answer differs from the hint by a chrome-sized offset.
    let measured = Rect::new(200, 150, 800, 600);
    let hint = Rect::new(120, 110, 800, 600);
    let (gateway, _rx, host) = start_with(
        dir.path(),
        Answers {
            restore: Some(FrameRect::new(200, 150, 800, 600)),
            ..Answers::default()
        },
    );

    // Register (issues + stamps the async move), then the bridge reports its
    // hint, then the user quits IMMEDIATELY - the drain flushes inside the
    // move-in-flight window, exactly the live sequence that drifted.
    gateway
        .send(Command::RegisterWindow {
            handle: WindowHandle(0x100),
        })
        .expect("queued");
    wait_for_calls(&host, 1, "the registration's move");
    gateway.send(Command::GeometryChanged).expect("queued");
    gateway.close().expect("the immediate quit joins");

    let persisted = read_session(dir.path()).expect("the session persisted");
    assert_ne!(
        persisted.rect, hint,
        "the bridge's hint must never be persisted into the frame-space field"
    );
    assert_eq!(
        persisted.rect, measured,
        "even an immediate quit measures: the MEASURED frame rect wins the drain\n         write (the guard lifts at shutdown because there is no later tick)"
    );

    // A LATER launch (guard long expired): the measure is honest and wins.
    let (second, _rx, _host2) = start_with(
        dir.path(),
        Answers {
            restore: Some(FrameRect::new(200, 150, 800, 600)),
            ..Answers::default()
        },
    );
    second
        .send(Command::RegisterWindow {
            handle: WindowHandle(0x100),
        })
        .expect("queued");
    wait_for_calls(&_host2, 1, "the relaunch's move");
    std::thread::sleep(Duration::from_millis(750 * 2 + 100));
    second.send(Command::SetPinned(true)).expect("queued");
    let deadline = Instant::now() + ANSWER;
    loop {
        if let Ok(session) = read_session(dir.path()) {
            if session.rect == measured {
                break;
            }
        }
        assert!(Instant::now() < deadline, "the measured rect never won");
        std::thread::sleep(Duration::from_millis(20));
    }
    second.close().expect("shutdown joins");
}

/// THE ACCEPTANCE THE DRIFT BUG BOUGHT: a GeometryChanged whose measure fails
/// (here: the flush ticks run while a move is in flight, so the only rect the
/// engine could write is one nobody measured) leaves session.json BYTE-IDENTICAL.
/// A stale rect is recoverable; a systematically offset one drifts.
#[test]
fn a_deferred_write_leaves_session_json_byte_identical() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_session(
        dir.path(),
        &Session {
            rect: Rect::new(50, 50, 700, 500),
            ..Session::default()
        },
    )
    .expect("write the session fixture");
    let before = std::fs::read(dir.path().join("session.json")).expect("read the seed");
    let (gateway, _rx, host) = start_with(dir.path(), Answers::default());

    // Register (issues + stamps the async move), then the trigger. Every flush
    // inside the move-in-flight window must be a full defer.
    gateway
        .send(Command::RegisterWindow {
            handle: WindowHandle(0x100),
        })
        .expect("queued");
    wait_for_calls(&host, 1, "the registration's move");
    gateway.send(Command::GeometryChanged).expect("queued");
    gateway.send(Command::SetPinned(true)).expect("queued");
    std::thread::sleep(Duration::from_millis(750 + 200));

    // Not one byte moved while the measure could not be trusted.
    let after = std::fs::read(dir.path().join("session.json")).expect("read again");
    assert_eq!(
        before, after,
        "an unmeasurable tick must leave the persisted file untouched"
    );
    gateway.close().expect("shutdown joins");
}
