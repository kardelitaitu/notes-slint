#![doc = r#"
# notes-platform

OS primitives that **take a window handle and decide nothing**.

Everything in this crate has the same shape: a bare `isize` window handle goes in,
one Win32 call happens, and either a value or a `PlatformError` comes out. It is a
seam, not a policy engine. What it deliberately does *not* own:

- **Decisions.** Which monitor a window belongs to, whether a stored rect is now
  off-screen, whether to clamp, snap, centre, enforce a minimum size, keep a window
  inside a work area, or retry. All of that is policy and lives above this crate.
  The only arithmetic in here is `FrameRect::scaled`, and that is a unit conversion
  rather than a judgement.
- **Window lifecycle.** Nothing creates, shows, focuses, owns or destroys a window.
  The bridge creates the window and hands its handle down.
- **Handle types.** Handles cross this boundary as `isize` (HWND-as-int) so that
  `api` can consume these types without depending on the `windows` crate. No
  windows-rs type appears in any signature `api` has to name.
- **Storage, serde, settings, notes-core.** Geometry *storage* is core work and
  geometry *application* is this crate's; neither knows the other exists, and `api`
  is the only place their outputs are joined.
- **Client-area geometry.** A client-to-frame conversion needs `GetClientRect` plus
  `ClientToScreen` on a live window, so it cannot be tested without one; it is left
  to the bridge and said so here. `SetWindowPos` takes frame-space coordinates, the
  same space `GetWindowRect` reports, so placement itself needs no conversion.
- **DPI awareness.** This crate never queries a window's DPI (that would need a
  manifest and a second `windows` feature). Where a scale matters it is a parameter
  the caller supplies.

## Layout

- `geometry` - `FrameRect`, the crate's one rectangle type, in one documented unit.
- `PlatformError` / `PlatformResult` - the error contract. Every fallible function
  returns it, and nothing in this crate panics.
- `WindowBackend` - the trait `api` consumes.
- `HostFacts` - the handle-free machine facts (the process ANSI code page), on the
  same seam rules; platform-only facts enter through `api`, never through the
  bridge, which may not import this crate at all.
- `windows` - the Win32 implementation (`windows::topmost`, `windows::monitors`):
  the only module that allows `unsafe`, and every unsafe block carries a
  `// SAFETY:` comment naming the call and the invariant that makes it valid.

Non-Windows hosts still compile `geometry`, the error and the trait, so the seam
stays type-checkable and its pure tests run anywhere; the Win32 module is gated
behind `#[cfg(windows)]`.
"#]

pub mod geometry;

#[cfg(windows)]
pub mod windows;

pub use geometry::FrameRect;

/// The result of every fallible function in this crate.
pub type PlatformResult<T> = Result<T, PlatformError>;

/// Why a platform seam refused to act.
///
/// Deliberately small: this crate takes no action worth describing in more detail,
/// and it never panics, so a caller branches on exactly these three cases.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    /// The handle was null, or `IsWindow` refused it at check time: the value never
    /// named a window on this station.
    ///
    /// This is *not* the stale-handle signal. A handle that was live when the guard
    /// ran and whose window was destroyed a moment later is refused by Win32, and it
    /// arrives as [`PlatformError::Win32`] instead: all this crate can assert about a
    /// handle is what one call, at one instant, reported about it.
    #[error("invalid window handle")]
    InvalidHandle,
    /// A Win32 call was made and refused. `api` names the call; `message` is the OS
    /// text and code, passed through untranslated - kept even when the API only
    /// answered FALSE, so a vanished monitor stays distinguishable from a monitor
    /// that was never there ([`PlatformError::NoMonitor`]).
    ///
    /// This is also what a caller gets for a handle that went stale between the guard
    /// and the call: the guard cannot hold a window open.
    #[error("Win32 {api} failed: {message}")]
    Win32 {
        /// The Win32 entry point that failed, e.g. `"SetWindowPos"`.
        api: &'static str,
        /// The OS message and error code, as windows-rs reported them.
        message: String,
    },
    /// The monitor lookup returned no monitor at all: a null handle for the request,
    /// not a monitor that stopped existing mid-call (that is
    /// [`PlatformError::Win32`], with the OS code intact).
    #[error("no monitor for handle")]
    NoMonitor,
}

/// The window-handle seams that `api` consumes.
///
/// Object-safe and `Send`: a bridge registers one implementation (`windows::Backend`
/// on Windows) and `api` holds it as a `Box<dyn WindowBackend>`. Every `handle` is an
/// HWND-as-int, and every method returns an error instead of panicking. Nothing here
/// decides *whether* to call it - that is `api` and `core`.
pub trait WindowBackend: Send {
    /// Raises the window above every other window (`on`) or puts it back.
    /// Moves nothing and resizes nothing.
    fn set_topmost(&mut self, handle: isize, on: bool) -> PlatformResult<()>;

    /// The window frame rectangle, in the unit `FrameRect` documents.
    fn frame_rect(&self, handle: isize) -> PlatformResult<FrameRect>;

    /// The window's RESTORE frame rect: `GetWindowPlacement`'s
    /// `rcNormalPosition` - the rect the USER sees as "the size I left it",
    /// even while the window is maximized. Same unit as [`FrameRect`] (physical
    /// frame pixels). `GetWindowRect` on a maximized window returns a rect that
    /// EXCEEDS the monitor by the invisible borders, so storing it would persist
    /// a lie; this is the only correct source for the persisted rect.
    ///
    /// The normal position is returned regardless of the current show state:
    /// whether the window is maximized right now is what `WNDPLACEMENT`'s
    /// `showCmd` and `flags` fields report, and interpreting them - "restore it
    /// maximized?", "un-maximize first?" - is a decision above this crate.
    fn restore_frame_rect(&self, handle: isize) -> PlatformResult<FrameRect>;

    /// Places and sizes the window at `r`.
    ///
    /// `scale` is the single unit conversion this seam performs: it is applied to `r`
    /// (see `FrameRect::scaled`) before the coordinates reach Win32, and what it
    /// means is the caller's business - pass `1.0` for "already in this window's
    /// space". There is no move-only or size-only variant, because choosing to keep
    /// one dimension is a decision: read `frame_rect` and copy the part to preserve.
    /// Z-order and activation are never touched.
    fn set_frame_rect(&mut self, handle: isize, r: FrameRect, scale: f32) -> PlatformResult<()>;

    /// The primary monitor's *work* area: its bounds minus whatever a reserved edge
    /// (the taskbar) occupies, so a window placed inside it stays usable. Not the
    /// screen resolution.
    fn primary_work_area(&self) -> PlatformResult<FrameRect>;
}

/// Machine facts that need no window handle. Same rule as [`WindowBackend`]:
/// call out, value back, decide nothing, no policy, no caching.
///
/// Why this seam exists and why it is not the bridge's job: the product opens
/// ordinary text files too, and core can only write back a code page it can
/// decode (CP1252 by hand, anything else refused, never guessed). The bridge
/// may import `notes-api` plus its toolkit and nothing else, so "the bridge
/// will supply GetACP" is unimplementable. The rule this seam establishes:
/// **platform-only facts enter through api, never through the bridge** - as an
/// action where possible, as data only when a UI must render it.
pub trait HostFacts: Send {
    /// `GetACP()`: the ANSI code page of this process's logon session. A
    /// measured value, not a constant: it is what the host answers, and a host
    /// answering something this build cannot decode is exactly the fact the
    /// caller needs to see.
    fn ansi_codepage(&self) -> u16;

    /// The work area of the monitor a frame rect BELONGS to: the monitor whose
    /// surface overlaps the rect the most; when nothing overlaps - the saved
    /// monitor is gone and no window exists yet, the handle-less restore case -
    /// the monitor NEAREST to the rect, so the caller clamps. Never blindly the
    /// primary: a rect living fully on a live secondary has zero overlap with
    /// the primary and would be yanked across screens. Returns the chosen
    /// monitor's id beside the work area so the caller can persist it
    /// (Session.monitor_id exists for this).
    ///
    /// The id is the numeric suffix of Win32's display-device name
    /// (\\.\\DISPLAY<n>): a session ordinal, NOT a hardware identity - unplug
    /// and replug can renumber, so treat it as a hint and recover placement
    /// from the rect itself. Overlap ties are resolved by MonitorFromRect's own
    /// ordering and are not guaranteed stable; any monitor the rect touches
    /// yields a safe clamp, which is why this crate does not arbitrate ties.
    /// Overlap is measured against the monitor's full surface (MonitorFromRect's
    /// documented rule): a rect lying wholly inside a reserved edge (the
    /// taskbar) still resolves to the monitor that edge belongs to, and that
    /// monitor's work area is what comes back.
    fn work_area_for_rect(&self, rect: FrameRect) -> PlatformResult<(FrameRect, u32)>;
}
#[cfg(test)]
mod tests {
    use super::{FrameRect, HostFacts, PlatformError, PlatformResult, WindowBackend};

    /// A stand-in for `api`: it implements the seam with no window, no desktop and
    /// no `windows` dependency, which is the point of the shape - bare `isize`
    /// handles in, typed errors out, object-safe.
    #[derive(Debug, Default)]
    struct Mock {
        calls: Vec<String>,
    }

    impl WindowBackend for Mock {
        fn set_topmost(&mut self, handle: isize, on: bool) -> PlatformResult<()> {
            self.calls.push(format!("topmost {handle:#x} {on}"));
            Ok(())
        }

        fn frame_rect(&self, _handle: isize) -> PlatformResult<FrameRect> {
            Ok(FrameRect::new(1, 2, 3, 4))
        }

        fn restore_frame_rect(&self, _handle: isize) -> PlatformResult<FrameRect> {
            // Not recorded: the trait method takes &self (a rect read is a query),
            // so the mock cannot push into its call log through it.
            Ok(FrameRect::new(5, 6, 7, 8))
        }

        fn set_frame_rect(
            &mut self,
            handle: isize,
            r: FrameRect,
            scale: f32,
        ) -> PlatformResult<()> {
            self.calls
                .push(format!("set_frame_rect {handle:#x} {r:?} {scale}"));
            Ok(())
        }

        fn primary_work_area(&self) -> PlatformResult<FrameRect> {
            Err(PlatformError::NoMonitor)
        }
    }

    impl HostFacts for Mock {
        fn ansi_codepage(&self) -> u16 {
            // A mock, not a measurement: it stands in for whatever the host answers.
            1252
        }

        fn work_area_for_rect(&self, rect: FrameRect) -> PlatformResult<(FrameRect, u32)> {
            // A fixed two-monitor layout: a rect at or past x=1920 belongs to the
            // mock secondary, everything else to the mock primary.
            if rect.x >= 1920 {
                Ok((FrameRect::new(1920, 0, 1920, 1040), 2))
            } else {
                Ok((FrameRect::new(0, 0, 1920, 1040), 1))
            }
        }
    }

    #[test]
    fn a_consumer_can_hold_the_seam_as_a_boxed_trait_object() {
        let mut backend: Box<dyn WindowBackend> = Box::new(Mock::default());
        assert!(backend.set_topmost(0x1234, true).is_ok());
        assert!(matches!(
            backend.frame_rect(0x1234),
            Ok(rect) if rect == FrameRect::new(1, 2, 3, 4)
        ));
        assert!(
            backend
                .set_frame_rect(0x1234, FrameRect::new(0, 0, 800, 600), 1.5)
                .is_ok()
        );
        assert!(matches!(
            backend.primary_work_area(),
            Err(PlatformError::NoMonitor)
        ));
    }

    #[test]
    fn the_seam_forwards_a_bare_handle_and_reports_typed_errors() {
        let mut backend = Mock::default();
        assert!(
            backend
                .set_frame_rect(0x10, FrameRect::new(0, 0, 10, 10), 1.0)
                .is_ok()
        );
        assert_eq!(
            backend.calls,
            ["set_frame_rect 0x10 FrameRect { x: 0, y: 0, w: 10, h: 10 } 1"]
        );
        assert!(matches!(
            backend.primary_work_area(),
            Err(PlatformError::NoMonitor)
        ));
    }

    #[test]
    fn the_mock_serves_both_seams_without_a_window() {
        let backend: Box<dyn WindowBackend> = Box::new(Mock::default());
        assert!(matches!(
            backend.restore_frame_rect(0x1234),
            Ok(rect) if rect == FrameRect::new(5, 6, 7, 8)
        ));
        let facts: Box<dyn HostFacts> = Box::new(Mock::default());
        // The mock's answer is a stand-in constant; the real one is measured in
        // windows/mod.rs's test.
        assert_eq!(facts.ansi_codepage(), 1252);
    }

    #[test]
    fn the_mock_answers_the_nearest_monitor_question_without_a_display() {
        let facts: Box<dyn HostFacts> = Box::new(Mock::default());
        let (work, id) = facts
            .work_area_for_rect(FrameRect::new(0, 0, 100, 100))
            .expect("mock answers");
        assert_eq!((work, id), (FrameRect::new(0, 0, 1920, 1040), 1));
        let (_, id) = facts
            .work_area_for_rect(FrameRect::new(5000, 0, 100, 100))
            .expect("mock answers");
        assert_eq!(
            id, 2,
            "a rect past the mock primary belongs to the mock secondary"
        );
    }

    #[test]
    fn the_error_texts_are_part_of_the_public_contract() {
        assert_eq!(
            PlatformError::InvalidHandle.to_string(),
            "invalid window handle"
        );
        assert_eq!(
            PlatformError::NoMonitor.to_string(),
            "no monitor for handle"
        );
        let error = PlatformError::Win32 {
            api: "SetWindowPos",
            message: "The handle is invalid. (0x80070006)".to_string(),
        };
        assert_eq!(
            error.to_string(),
            "Win32 SetWindowPos failed: The handle is invalid. (0x80070006)"
        );
    }
}
