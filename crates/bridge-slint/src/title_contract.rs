//! THE TEXT HALF OF THE TITLE BAR, with no toolkit in it.
//!
//! Ported from `crates/bridge-gpui/src/titlebar.rs` - `title_words` (:124) and
//! `window_title` (:142) - because both are pure functions over a path and a loaded
//! flag, and the rule they encode is a PRODUCT rule, not a GPUI rule: which words a
//! user sees when nothing is saved yet, and the refusal to print a file name the port
//! never announced. Carrying them here means a second bridge cannot quietly invent a
//! second opinion about the same window.
//!
//! TWO THINGS ARE DELIBERATELY DIFFERENT FROM THE ORIGINAL, both about purity: the
//! return type is `String`, not `SharedString` - a contract that names a toolkit
//! cannot be tested by the crate that has not chosen one yet, and every Slint mount
//! takes `Into<SharedString>` (`Window::set_title`, a `string` property), so the
//! conversion happens at the seam that needs it, not here. And `APP_NAME` stays
//! `env!("CARGO_PKG_NAME")`, exactly as the original (:48) does, which for this crate
//! means "notes-bridge-slint". That is a user-visible string: S4 should pick the
//! product name ONCE rather than let two bridges disagree. Flagged, not fixed.
#![allow(dead_code)] // mounted in S4; the contract is testable before the widget is.

use std::path::Path;

/// What the app calls itself in the OS title. See the note in the header.
pub(crate) const APP_NAME: &str = env!("CARGO_PKG_NAME");

/// The word for "no document yet". Identical to bridge-gpui's, because "Untitled"
/// vs "new note" is exactly the drift nobody catches in review.
pub(crate) const UNTITLED: &str = "Untitled";

/// THE CENTRE REGION'S WORDS: the file's REAL name with its REAL extension
/// (ADR-0003: the undecided .notes-vs-.md question changes what shows here, not this
/// rule), else `Untitled`.
///
/// `loaded` is not decoration. A path the port never announced is not a document, and
/// the bridge refuses to name one. Everything reaching the mount arrives on
/// `Event::Loaded` or `Event::Rebound`, so the guard costs nothing - and it is the
/// reason a field set from a REQUEST instead of an ANSWER cannot put a file name on
/// screen before the engine has agreed that file exists.
pub(crate) fn title_words(path: Option<&Path>, loaded: bool) -> String {
    if loaded {
        if let Some(p) = path {
            return match p.file_name().map(|n| n.to_string_lossy().into_owned()) {
                Some(name) => name,
                // A path with no final component (a drive root, a trailing separator)
                // still deserves a name that is findable in Explorer.
                None => p.to_string_lossy().into_owned(),
            };
        }
    }
    UNTITLED.to_string()
}

/// THE OS TITLE, in ADR-0003's "name dash app" form with the ADR's em dash (U+2014,
/// not a hyphen). Nothing in the strip renders this - the centre region uses
/// `title_words` - which is exactly why it is built FROM `title_words` instead of
/// recomputed: two formulas drift, one does not. Alt+Tab, the taskbar previews and
/// accessibility tooling are what read the result.
pub(crate) fn window_title(path: Option<&Path>, loaded: bool) -> String {
    format!("{} \u{2014} {}", title_words(path, loaded), APP_NAME)
}

#[cfg(test)]
mod tests {
    use super::{APP_NAME, UNTITLED, title_words, window_title};
    use std::path::Path;

    fn p(s: &str) -> &Path {
        Path::new(s)
    }

    #[test]
    fn nothing_is_loaded_and_nothing_is_named() {
        assert_eq!(title_words(None, false), UNTITLED);
        assert_eq!(title_words(None, true), UNTITLED);
    }

    #[test]
    fn a_path_the_port_never_announced_is_still_untitled() {
        // THE GUARD, and the whole reason `loaded` is a parameter instead of a
        // `Option` alone: a name that came from a request must not reach the screen.
        assert_eq!(title_words(Some(p("C:/notes/todo.notes")), false), UNTITLED);
        assert_eq!(
            window_title(Some(p("C:/notes/todo.notes")), false),
            format!("{UNTITLED} \u{2014} {APP_NAME}")
        );
    }

    #[test]
    fn a_loaded_path_shows_its_real_name_with_its_real_extension() {
        assert_eq!(
            title_words(Some(p("C:/notes/todo.notes")), true),
            "todo.notes"
        );
        assert_eq!(title_words(Some(p("C:/notes/todo.md")), true), "todo.md");
    }

    #[test]
    fn only_the_final_component_shows_and_the_whole_path_is_the_fallback() {
        assert_eq!(title_words(Some(p("meeting notes")), true), "meeting notes");
        // A drive root has no file_name(); the long form still beats "Untitled".
        assert_eq!(title_words(Some(p("C:/")), true), "C:/");
    }

    #[test]
    fn spaces_parens_and_leading_dots_are_kept_verbatim() {
        assert_eq!(
            title_words(Some(p("C:/x/my final draft v2 (1).notes")), true),
            "my final draft v2 (1).notes"
        );
        assert_eq!(title_words(Some(p("C:/x/.hidden")), true), ".hidden");
    }

    #[test]
    fn non_latin_names_survive_intact() {
        // CJK in the name and in the directories above it. The lossy conversion is
        // only for paths that are not valid UTF-8; a file name is a user's words.
        assert_eq!(title_words(Some(p("/tmp/ノート.md")), true), "ノート.md");
        assert_eq!(
            title_words(Some(p("C:/用户/笔记/会议记录.notes")), true),
            "会议记录.notes"
        );
        assert_eq!(
            window_title(Some(p("C:/x/会议记录.notes")), true),
            format!("会议记录.notes \u{2014} {APP_NAME}")
        );
    }

    #[test]
    fn the_os_title_is_the_centre_words_plus_the_app_never_a_second_opinion() {
        for (path, loaded) in [
            (Some(p("C:/notes/a.notes")), true),
            (Some(p("C:/notes/a.notes")), false),
            (None, true),
            (None, false),
        ] {
            assert_eq!(
                window_title(path, loaded),
                format!("{} \u{2014} {APP_NAME}", title_words(path, loaded))
            );
        }
    }

    #[test]
    fn the_separator_is_an_em_dash_not_a_hyphen() {
        // A screen reader speaks this string. ADR-0003 chose the em dash on purpose,
        // so a future typo-fix to " - " would be a product change, not a tidy-up.
        let title = window_title(Some(p("a.notes")), true);
        assert!(title.contains('\u{2014}'), "{title:?}");
        assert!(!title.contains(" - "), "{title:?}");
    }
}
