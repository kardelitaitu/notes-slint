//! A stand-in for [`notes_platform::mock`], which does not exist yet.
//!
//! This file is the debt D47 accepted, dated: crates/platform keeps its Mock inside
//! a [`#[cfg(test)] mod`] (crates/platform/src/lib.rs, line 176), so no other
//! crate's tests can see it, and Slice B would otherwise serialise behind the
//! platform worker landing [`pub mod mock`]. When that ships, deleting THIS file
//! and the three [`mod host_mock;`] include lines is the whole migration - nothing
//! here redefines a trait, invents a geometry type, or encodes a product rule.
//!
//! One type serves both seams, exactly as platform's own Mock does, so a test hands
//! the same recorder to [`start_with_host`] twice. Calls are logged through an
//! [`Arc<Mutex<..>>`] because the engine owns the boxes and usually runs on
//! another thread: the [`Send`] bound on the traits rules out [`Rc<RefCell<..>>`].

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use notes_platform::{
    FrameRect, HostFacts, PinOutcome, Placement, PlatformError, PlatformResult, ShowState,
    WindowBackend,
};

/// One recorded call, in the order it happened.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    SetFrame {
        handle: isize,
        rect: FrameRect,
        scale: f32,
    },
    Topmost {
        handle: isize,
        on: bool,
    },
    /// One corner-shape ask. Recorded for the same reason `TakeDropped` is: the
    /// event stream is silent on a successful apply, so the call list is the only
    /// proof the seam was driven at all - and with what value.
    CornerRounding {
        handle: isize,
        round: bool,
    },
    RestoreRect {
        handle: isize,
    },
    SetRestoreFrame {
        handle: isize,
        rect: FrameRect,
    },
    FrameRect {
        handle: isize,
    },
    WorkArea {
        rect: FrameRect,
    },
    PrimaryWorkArea,
    Codepage,
    /// One ask for the paths somebody dropped on the window. Recorded because
    /// the COUNT is the assertion: a poll that never calls it and a poll that calls
    /// it and finds nothing look identical from the events, and the difference is
    /// the whole difference between "drained" and "never asked".
    TakeDropped,
}

/// What the fake host answers, and where it refuses.
#[derive(Debug, Clone)]
pub struct Answers {
    /// The work area [`work_area_for_rect`] reports, with its monitor id. A plain
    /// 1920x1080 minus a 48px taskbar: big enough that a default session rect needs
    /// no clamping, so a test that is not about the clamp moves the window for
    /// exactly one reason.
    pub work_area: FrameRect,
    pub monitor_id: u32,
    /// The rect [`restore_frame_rect`] reports as its `Placement::restore_rect` -
    /// the value D48 persists.
    pub restore: Option<FrameRect>,
    /// The show state [`restore_frame_rect`] reports as its `Placement::show` - the
    /// answer the measured `maximized` bit is stored from. `Unknown` by default:
    /// that is what "this fake has no view" honestly means, and it is also what
    /// keeps every fixture written before the show bit existed byte-identical, because
    /// the port stores nothing on an `Unknown`. A test that is ABOUT the bit answers
    /// `Maximized` or `Normal` here.
    pub restore_show: ShowState,
    /// [`ansi_codepage`]: a stand-in for a measurement, never a guess.
    pub codepage: u16,
    /// Each [`Some`] is the OS message that call answers with.
    pub fail_move: Option<String>,
    pub fail_topmost: Option<String>,
    /// What `set_corner_rounding` answers with [`Some`]: the Windows 10 case, where
    /// the attribute does not exist. None (accepted) by default, because the OS this
    /// product is developed on is the one that knows it.
    pub fail_corner_rounding: Option<String>,
    pub fail_restore: Option<String>,
    /// What [`set_restore_frame_rect`] answers with [`Some`]. Separate from
    /// `fail_restore` because the READ and the WRITE are two calls: refusing one
    /// must not silently refuse the other.
    pub fail_restore_set: Option<String>,
    /// Model the `SetWindowPlacement` -> `GetWindowPlacement` ROUND TRIP: a
    /// written `rcNormalPosition` is what the next read reports. OFF by default,
    /// because a fake that answers one question forever is what every fixture
    /// written before the maximised ratchet was found depends on - and a stateful
    /// placement would silently rewrite what those tests measure. ON, it is the
    /// only way to say "a cycle stores the rect it started with" without a window.
    pub placement_round_trip: bool,
    pub fail_work_area: Option<String>,
    /// MAJOR: how long `set_frame_rect` BLOCKS before answering, in
    /// milliseconds. Zero by default. This is the hang reproducer: a window op
    /// from another thread SENDS to the window's owner and waits for it to
    /// pump, so a blocked move is exactly what the real seam does to an engine
    /// whose owner is parked.
    pub block_move_ms: u64,
    /// The DPI scale of the monitor owning the queried rect. The platform
    /// seam landed (scale_for_rect); the port's scale_factor refresh is a
    /// follow-up once the verbatim signature is handed over - the mock
    /// carries the answer so that follow-up is testable when it lands.
    pub scale: f32,
    /// The FILE-DROP queue: exactly what the next [`take_dropped_paths`] hands
    /// over, and it DRAINS. Empty by default, which is the trait's own answer, so
    /// every fixture written before the drop poll existed behaves unchanged - a
    /// host that never has anything is indistinguishable from a host with no drop
    /// support, and no existing assertion shifts by a byte.
    pub drops: Vec<PathBuf>,
}

impl Default for Answers {
    fn default() -> Self {
        Self {
            work_area: FrameRect::new(0, 0, 1920, 1032),
            monitor_id: 1,
            restore: None,
            restore_show: ShowState::Unknown,
            codepage: 1252,
            fail_move: None,
            fail_topmost: None,
            fail_corner_rounding: None,
            fail_restore: None,
            fail_restore_set: None,
            placement_round_trip: false,
            fail_work_area: None,
            block_move_ms: 0,
            scale: 1.0,
            drops: Vec::new(),
        }
    }
}

// No Default derive: a Host is built through new/with_answers, and deriving
// Default would need Mutable: Default for a value nobody can use.
#[derive(Debug, Clone)]
pub struct Host {
    shared: Arc<Mutable>,
}

#[derive(Debug)]
struct Mutable {
    calls: Mutex<Vec<Call>>,
    answers: Mutex<Answers>,
}

impl Host {
    pub fn with_answers(answers: Answers) -> Self {
        Self {
            shared: Arc::new(Mutable {
                calls: Mutex::new(Vec::new()),
                answers: Mutex::new(answers),
            }),
        }
    }

    pub fn calls(&self) -> Vec<Call> {
        lock(&self.shared.calls).clone()
    }

    /// THE WORLD CHANGES UNDER THE WINDOW: replaces every answer (work area,
    /// monitor, restore rect, codepage, refusals, block). A fake that answers
    /// the same question forever cannot prove a REFRESH - which is exactly how
    /// the frozen-at-launch session facts (MAJOR 2/5) stayed invisible: every
    /// test saw one immutable world. Tests that need the world to move call
    /// this between flushes.
    pub fn set_answers(&self, answers: Answers) {
        *lock(&self.shared.answers) = answers;
    }

    /// Moves ONLY the show answer, leaving every other answer exactly where it
    /// was. The fixtures that are ABOUT the bit need this: the claim under test
    /// is that one transient `SW_SHOWMAXIMIZED` sample must never reach the
    /// file, and that is only provable if the rect the fake reports does not
    /// change at the same moment - a test that swapped the whole world could not
    /// tell a latch failure from a rect move. [`Self::set_answers`] replaces
    /// everything; this replaces the one question the sample answers.
    pub fn set_restore_show(&self, show: ShowState) {
        let mut answers = self.answers();
        answers.restore_show = show;
    }

    /// Moves ONLY the drop queue, leaving every other answer where it was - the
    /// same reason [`Self::set_restore_show`] moves one question: a test about the
    /// drain must not move the geometry, the codepage or a refusal at the same
    /// moment, or it cannot tell which answer it observed. REPLACES the queue, so
    /// `set_drops(vec![])` is how a test says "nothing more is coming".
    ///
    /// ALLOWED-as-unused on purpose, and so is [`Self::drop_takes`]: the engine's own
    /// tests drive a local fixture instead (they need the queue and the counter as
    /// shared cells the box cannot hand back), and the integration suite that takes
    /// the drop door is the next slice. `dead_code` is per TARGET and this file is
    /// compiled into three of them, so an accessory one target does not call is a
    /// gate error. notes-platform's `took_over()` carries the same allow for the same
    /// reason: the consumer is not here YET, which is not the same as dead.
    #[allow(dead_code)]
    pub fn set_drops(&self, paths: Vec<PathBuf>) {
        self.answers().drops = paths;
    }

    /// How many times the engine ASKED. The count is the point: "the second tick
    /// emitted no `Loaded`" is also what a poll that never ran looks like, and only
    /// these two facts together separate "drained" from "never asked".
    #[allow(dead_code)]
    pub fn drop_takes(&self) -> usize {
        self.calls()
            .into_iter()
            .filter(|c| matches!(c, Call::TakeDropped))
            .count()
    }

    /// Every [`set_frame_rect`]: (handle, rect, scale). The assertions in
    /// tests/geometry.rs are about this list - its length, its rect, and the scale.
    pub fn moves(&self) -> Vec<(isize, FrameRect, f32)> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::SetFrame {
                    handle,
                    rect,
                    scale,
                } => Some((handle, rect, scale)),
                _ => None,
            })
            .collect()
    }

    pub fn topmost(&self) -> Vec<(isize, bool)> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Topmost { handle, on } => Some((handle, on)),
                _ => None,
            })
            .collect()
    }

    /// Every [`set_restore_frame_rect`]: (handle, rect). This is the port pushing
    /// a frame-space number BACK through the same door it read it from. The
    /// maximised launch has no [`set_frame_rect`] to assert on, so this list is
    /// the whole receipt that the loop closed in frame space.
    pub fn restore_sets(&self) -> Vec<(isize, FrameRect)> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::SetRestoreFrame { handle, rect } => Some((handle, rect)),
                _ => None,
            })
            .collect()
    }

    pub fn restore_reads(&self) -> usize {
        self.calls()
            .into_iter()
            .filter(|c| matches!(c, Call::RestoreRect { .. }))
            .count()
    }

    fn record(&self, call: Call) {
        lock(&self.shared.calls).push(call);
    }

    fn answers(&self) -> MutexGuard<'_, Answers> {
        lock(&self.shared.answers)
    }
}

fn lock<T>(cell: &Mutex<T>) -> MutexGuard<'_, T> {
    cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl WindowBackend for Host {
    fn set_topmost(&mut self, handle: isize, on: bool) -> PinOutcome {
        self.record(Call::Topmost { handle, on });
        match self.answers().fail_topmost.clone() {
            Some(why) => PinOutcome::Failed(PlatformError::Win32 {
                api: "SetWindowPos",
                message: why,
            }),
            None => PinOutcome::Applied,
        }
    }

    fn frame_rect(&self, handle: isize) -> PlatformResult<FrameRect> {
        // D48 persists the RESTORE rect, so the port never calls this. It is
        // answered anyway: a mock with a hole in it turns a typo into a panic.
        self.record(Call::FrameRect { handle });
        Ok(self.answers().work_area)
    }

    fn set_corner_rounding(&mut self, handle: isize, round: bool) -> PlatformResult<()> {
        self.record(Call::CornerRounding { handle, round });
        match self.answers().fail_corner_rounding.clone() {
            Some(why) => Err(PlatformError::Win32 {
                api: "DwmSetWindowAttribute",
                message: why,
            }),
            None => Ok(()),
        }
    }

    fn restore_frame_rect(&self, handle: isize) -> PlatformResult<Placement> {
        self.record(Call::RestoreRect { handle });
        let answers = self.answers();
        if let Some(message) = answers.fail_restore.clone() {
            return Err(PlatformError::Win32 {
                api: "GetWindowPlacement",
                message,
            });
        }
        Ok(Placement {
            restore_rect: answers.restore.unwrap_or(answers.work_area),
            show: answers.restore_show,
        })
    }

    fn set_restore_frame_rect(&mut self, handle: isize, rect: FrameRect) -> PlatformResult<()> {
        self.record(Call::SetRestoreFrame { handle, rect });
        let mut answers = self.answers();
        if let Some(message) = answers.fail_restore_set.clone() {
            return Err(PlatformError::Win32 {
                api: "SetWindowPlacement",
                message,
            });
        }
        if answers.placement_round_trip {
            // THE ROUND TRIP: what SetWindowPlacement wrote IS what the next
            // GetWindowPlacement reports. Gated (see the field) because a fake
            // that answers one number forever is what the older fixtures lean on.
            answers.restore = Some(rect);
        }
        Ok(())
    }

    fn set_frame_rect(&mut self, handle: isize, r: FrameRect, scale: f32) -> PlatformResult<()> {
        self.record(Call::SetFrame {
            handle,
            rect: r,
            scale,
        });
        let block = self.answers().block_move_ms;
        if block > 0 {
            std::thread::sleep(std::time::Duration::from_millis(block));
        }
        match self.answers().fail_move.clone() {
            Some(message) => Err(PlatformError::Win32 {
                api: "SetWindowPos",
                message,
            }),
            None => Ok(()),
        }
    }

    fn primary_work_area(&self) -> PlatformResult<FrameRect> {
        self.record(Call::PrimaryWorkArea);
        Ok(self.answers().work_area)
    }

    /// THE OVERRIDE, and the reason it exists: the trait's own default answers
    /// `Vec::new()` (S1), so without this the engine's drop poll could never be
    /// driven from a test at all - a suite that passes on a host which structurally
    /// cannot produce a drop proves nothing. This answers what the fixture was told
    /// and DRAINS it, which is the real seam's contract: `take_buffered` is
    /// `mem::take`, so a second call sees nothing unless something arrived between
    /// the two. The record happens either way, including on the empty answer,
    /// because the ASK is the observable.
    fn take_dropped_paths(&mut self) -> Vec<PathBuf> {
        self.record(Call::TakeDropped);
        // The guard is the mutable borrow: `Answers::drops` is reached through
        // `DerefMut`, which is how the round-trip setter above writes too.
        let mut answers = self.answers();
        std::mem::take(&mut answers.drops)
    }
}

impl HostFacts for Host {
    fn ansi_codepage(&self) -> u16 {
        self.record(Call::Codepage);
        self.answers().codepage
    }

    fn scale_for_rect(&self, _rect: FrameRect) -> PlatformResult<f32> {
        let scale = self.answers().scale;
        Ok(scale)
    }

    fn work_area_for_rect(&self, rect: FrameRect) -> PlatformResult<(FrameRect, u32)> {
        self.record(Call::WorkArea { rect });
        let answers = self.answers();
        if let Some(message) = answers.fail_work_area.clone() {
            return Err(PlatformError::Win32 {
                api: "MonitorFromRect",
                message,
            });
        }
        // The real seam picks the monitor with the most overlap (nearest when
        // nothing overlaps); one configured answer is enough to prove the port
        // asked, passed the rect it stored, and clamped with what came back.
        Ok((answers.work_area, answers.monitor_id))
    }
}
