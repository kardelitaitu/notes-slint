//! THE TITLE BAR (ADR-0003) - its LEFT and CENTRE regions, and nothing else.
//!
//! ADR-0003 divides one 34px band into three regions: LEFT (pin toggle; the hamburger joins
//! in C3c), CENTRE (document title + the dirty dot of section 4.4), RIGHT (minimize /
//! maximize / close). The RIGHT region and the drag are NOT written here: C2 made
//! TitleBar::window_options() the base of this window's options, and the kit's own TitleBar
//! draws the Windows control area and owns the drag, so hand-rolling either would be a second
//! implementation of a thing already bought (ADR-0002). What this module supplies is the kit
//! TitleBar's CHILDREN - the kit renders children into the bar region - plus the two
//! decisions those regions express, kept as pure functions:
//!
//!   * title_words - what the centre says, and what the OS title is built from.
//!   * dot_state   - what the dot means, as a value rather than as a colour, so the
//!     precedence (section 4.4: a FAILED save outranks a merely dirty buffer) is testable.
//!
//! # What is deliberately NOT here
//!
//! The hamburger popup and the tooltip. Both want the kit's overlay machinery, and
//! .agents/notes/proposed/2026-09-12-titlebar-root-overlay-question.md settled against
//! mounting Root to get it: Root::render activates a window-scoped TextSelection scope every
//! frame, which is the second-owner-of-selection shape menu.rs:1-28 records as the way D51
//! and D52 get quietly broken (the editor's IME marked range is the first owner). That note's
//! option C is what ships in this slice - pin, title, dot - and the popup and the hover
//! reveal are the next one, hand-rolled inline in this layout rather than floated. Stated
//! rather than hidden, because it is a divergence from the ADR's drawing: the ADR permits
//! centre truncation only because the tooltip exists, so until C3c lands the full path is
//! reachable in the status line and in the OS title, not on hover.
//!
//! Also absent: Theme, Root, a kit Input, and any call into the port. The bar reads the two
//! booleans and the title it was handed. pinned arrives from Event::Pinned (the engine's
//! answer, 60e536c5) and a click only ever ASKS for the other state.

use std::path::Path;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Selectable as _, Sizable as _, TitleBar};
use gpui_kit::{
    AnyElement, IntoElement, ParentElement, Pixels, SharedString, Styled, div, px, rgb,
};

/// The app name: the third rung of ADR-0003's centre ladder and the word the OS title ends
/// with. Taken from the package name so there is never a second copy to forget.
pub(crate) const APP_NAME: &str = env!("CARGO_PKG_NAME");

/// Width of the button slot on the left AND of the matching slot on the right of the centre
/// region. The two exist together so the title is centred in the whole band rather than in
/// what the pin button leaves over: the kit lays children out justify_between, and that reads
/// as centring only when the two ends are the same fixed width.
const SLOT_WIDTH: Pixels = px(30.);

/// Section 4.4's two colours, on the dark background the window already paints (0x1f1f1f).
/// Literals rather than cx.theme() lookups on purpose: a themed bar would be one more
/// argument for the Theme state this window is component-free without.
const AMBER: u32 = 0xe5_a63b;
const RED: u32 = 0xe0_6c75;
/// The unlit colour, for the glyph the port did NOT confirm.
const MUTED: u32 = 0x9a_9a9a;

/// The three things the dot can mean. None is a real variant, not a fall-through: the clean
/// case must paint nothing, because a dot that is always there cannot carry section 4.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dot {
    /// Clean buffer: no dot at all.
    None,
    /// Unsaved edits, autosave healthy or disarmed.
    Dirty,
    /// Unsaved edits AND the last save failed - section 4.4's amber, the one state that must
    /// not be visually reducible to "dirty".
    SaveFailed,
}

/// WHAT THE DOT MEANS, with the precedence section 4.4 requires: a failed save outranks
/// plain dirtiness, because dirty already says "you have unsaved text" and the amber exists
/// to add "and the app could not write it". A save_failed over a clean buffer is the
/// impossible-but-cheap case; the failure is still the more urgent word, so it wins.
fn dot_state(save_failed: bool, dirty: bool) -> Dot {
    if save_failed {
        Dot::SaveFailed
    } else if dirty {
        Dot::Dirty
    } else {
        Dot::None
    }
}

impl Dot {
    /// The colour to paint, or None for "paint nothing". Kept apart from the variant so the
    /// tests assert meaning and only this line owns the hex digits.
    fn color(self) -> Option<u32> {
        match self {
            Dot::None => None,
            Dot::Dirty => Some(RED),
            Dot::SaveFailed => Some(AMBER),
        }
    }
}

/// THE PIN GLYPH: Pin when the port confirmed the window is above everything, PinOff when it
/// confirmed it is not. The glyph changes as well as the colour because colour alone is the
/// one signal this app cannot own - a grey-on-dark bar is exactly where a tint distinction is
/// the first thing a user loses.
fn pin_icon(pinned: bool) -> IconName {
    if pinned {
        IconName::Pin
    } else {
        IconName::PinOff
    }
}

/// THE CENTRE REGION'S WORDS: the file's REAL name with its REAL extension (ADR-0003: the
/// undecided .notes-vs-.md question of section 10.1 changes what shows here, not this rule),
/// else Untitled. loaded is not decoration: a path the port never announced is not a document,
/// and the bridge refuses to name one. Everything in Wire::path arrives on Loaded or Rebound
/// today, so the guard costs nothing and is the reason a future field set from a REQUEST
/// instead of an ANSWER cannot put a file name on screen.
pub(crate) fn title_words(path: Option<&Path>, loaded: bool) -> SharedString {
    if loaded {
        if let Some(p) = path {
            return match p.file_name().map(|n| n.to_string_lossy().into_owned()) {
                Some(name) => SharedString::from(name),
                // A path with no final component (a drive root, a trailing separator) still
                // deserves to be named by something findable in Explorer.
                None => SharedString::from(p.to_string_lossy().into_owned()),
            };
        }
    }
    SharedString::from("Untitled")
}

/// THE OS TITLE, in ADR-0003's "name dash app" form with the ADR's em dash. Nothing in the
/// band renders this string; Alt+Tab, the taskbar previews and accessibility tooling read it,
/// so it carries the same words the centre region does rather than a second opinion. Written
/// once here so the title and any later copy cannot drift.
pub(crate) fn window_title(path: Option<&Path>, loaded: bool) -> SharedString {
    SharedString::from(format!(
        "{} \u{2014} {}",
        title_words(path, loaded),
        APP_NAME
    ))
}

/// THE DOT AS AN ELEMENT. An Option is not an element, so the clean case contributes an
/// empty div rather than nothing: the precedence stays in the pure dot_state, and the paint
/// order of the centre region does not depend on which arm was taken.
fn dot_element(dot: Dot) -> AnyElement {
    match dot.color() {
        Some(color) => div()
            .flex_none()
            .size(px(7.))
            .mr(px(6.))
            .rounded_full()
            .bg(rgb(color))
            .into_any_element(),
        None => div().flex_none().into_any_element(),
    }
}

/// One fixed-width slot at either end of the centre region, so the kit's justify_between
/// reads as centring rather than as a left shift. The right-hand slot is empty on purpose:
/// the caption buttons are painted by the kit OUTSIDE the children row, and their width is
/// the bar's own, so nothing here reserves room for them.
fn slot(child: impl IntoElement) -> gpui_kit::Div {
    div()
        .flex_none()
        .w(SLOT_WIDTH)
        .flex()
        .items_center()
        .justify_center()
        .child(child)
}

/// THE BAR: the kit's TitleBar, carrying this slice's two regions as its children.
///
/// dirty is the wire's own arithmetic (seen_edits != flushed_edits), save_failed is the
/// port's last SaveFailed unanswered by a Saved, and pinned is the port's Event::Pinned.
/// on_pin is handed the state to ASK for (!pinned) because the closure cannot see the wire
/// and the bridge keeps no second copy of the bit.
pub(crate) fn bar(
    title: SharedString,
    dirty: bool,
    save_failed: bool,
    pinned: bool,
    on_pin: Rc<dyn Fn(bool)>,
) -> impl IntoElement {
    // A click handler outlives this frame, so it owns its own handle on the ask.
    let toggle = on_pin;
    TitleBar::new()
        .child(slot(
            Button::new("title-bar-pin")
                .icon(pin_icon(pinned))
                .selected(pinned)
                .small()
                .ghost()
                .text_color(rgb(if pinned { AMBER } else { MUTED }))
                .on_click(move |_, _, _| toggle(!pinned)),
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .child(dot_element(dot_state(save_failed, dirty)))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(13.))
                        .child(title),
                ),
        )
        .child(slot(div()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_the_port_announced_names_the_centre() {
        assert_eq!(
            title_words(Some(Path::new("C:/notes/scratch.notes")), true),
            SharedString::from("scratch.notes"),
            "file name, never the directory it lives in"
        );
    }

    /// ADR-0003's accepted consequence: the title shows the real extension. Section 10.1 is
    /// undecided about the format; the rule is not.
    #[test]
    fn the_extension_belongs_to_the_file_not_to_a_guess() {
        for name in ["a.md", "b.txt", "c.notes", "d"] {
            assert_eq!(
                title_words(Some(Path::new(name)), true),
                SharedString::from(name)
            );
        }
    }

    #[test]
    fn nothing_to_name_says_untitled() {
        assert_eq!(title_words(None, false), SharedString::from("Untitled"));
        assert_eq!(title_words(None, true), SharedString::from("Untitled"));
    }

    /// THE GUARD, and the reason loaded is a parameter rather than a comment: a name the port
    /// never announced is not a document. Today Wire::path is only ever set from an answer, so
    /// this arm is unreachable - and stays correct if that stops being true.
    #[test]
    fn a_path_the_port_never_announced_names_nothing() {
        assert_eq!(
            title_words(Some(Path::new("C:/notes/ghost.notes")), false),
            SharedString::from("Untitled"),
        );
    }

    #[test]
    fn the_os_title_is_the_adrs_form() {
        assert_eq!(
            window_title(Some(Path::new("/tmp/y/y.notes")), true),
            SharedString::from(format!("y.notes \u{2014} {APP_NAME}")),
        );
        assert_eq!(
            window_title(None, false),
            SharedString::from(format!("Untitled \u{2014} {APP_NAME}")),
        );
    }

    #[test]
    fn a_failed_save_outranks_plain_dirtiness() {
        assert_eq!(dot_state(true, true), Dot::SaveFailed);
        assert_eq!(dot_state(true, false), Dot::SaveFailed);
    }

    #[test]
    fn dirty_without_a_failure_is_the_ordinary_dot() {
        assert_eq!(dot_state(false, true), Dot::Dirty);
    }

    #[test]
    fn clean_paints_nothing_at_all() {
        let dot = dot_state(false, false);
        assert_eq!(dot, Dot::None);
        assert_eq!(dot.color(), None, "the dot is not a permanent fixture");
    }

    /// The two meanings must not collapse into one colour, or section 4.4's amber is
    /// decoration.
    #[test]
    fn the_failure_colour_is_not_the_dirty_colour() {
        assert_eq!(Dot::SaveFailed.color(), Some(AMBER));
        assert_eq!(Dot::Dirty.color(), Some(RED));
        assert_ne!(Dot::SaveFailed.color(), Dot::Dirty.color());
    }

    /// Both pin states are readable with no colour at all, which is why the glyph changes
    /// alongside the tint.
    #[test]
    fn the_pin_glyph_changes_with_the_confirmed_state() {
        assert_eq!(pin_icon(true), IconName::Pin);
        assert_eq!(pin_icon(false), IconName::PinOff);
        assert_ne!(pin_icon(true), pin_icon(false));
    }
}
