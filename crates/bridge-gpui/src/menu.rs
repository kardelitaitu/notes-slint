//! THE M2 MENU - and the reason it is built like this is one function's signature.
//!
//! The brief allowed this slice only if the menu does NOT need `Root`, because `Root`
//! coordinates window-scoped text selection through the same machinery that owns our
//! IME marked range, and a second owner of selection is how D51 and D52 get quietly
//! broken. The answer from source, three facts deep:
//!
//!   * `App::set_menus(&self, menus)` (gpui-pre-0.3.4 src/app.rs:2424) takes `&self` on
//!     the APP. No `Window`, no entity, no view, therefore nothing to wrap.
//!   * The Windows platform's implementation is five lines that store the menus into a
//!     `RefCell` (gpui-pre-windows-0.3.4 src/platform.rs:746-748). There is no
//!     `CreateMenu`, no `TrackPopupMenuEx`, no window of its own - it does not read the
//!     overlay stack because it does not touch the window at all.
//!   * Both file prompts are on `App` too, and they answer through an awaitable
//!     `oneshot::Receiver`, not a view event: `prompt_for_paths(&self, options)`
//!     (src/app.rs:1582) and `prompt_for_new_path(&self, directory, suggested_name)`
//!     (src/app.rs:1595). So Open and Save As need no dialog layer either.
//!
//! **Root is not required for any part of this.** What the same reading costs: a menu bar
//! that is stored and never drawn on Windows. So every item here has a key equivalent, and
//! the chords are the feature on this platform - which is the trade the brief said it would
//! rather ship. The menu structure is still built, because it is the shape macOS draws and
//! because keeping the two in one function means the day the bar appears the behaviour does
//! not have to be discovered again.
//!
//! What is deliberately NOT here: `Theme`, `Root`, a kit `Input`, a popup or tooltip
//! system, and any change to `editor.rs`. One window, one editor, one pump, 13 Events and
//! 10 Commands.

use gpui_kit::{KeyBinding, Menu, MenuItem, SharedString, actions};
use notes_api::RecentEntry;

// The three commands the menu owns. `ToggleAutosave` is the only one of the four M2 items
// with no reply from the port (nothing in `Event` reports the setting back), so its check
// mark is the value the PORT last stated - seeded from `InitialState::autosave_enabled` at
// startup - and the reason a first-launch check mark is evidence rather than decoration.
actions!(
    notes_menu,
    [OpenFile, SaveAsFile, ToggleAutosave, ClearRecents]
);

// One action per recent slot. A menu item needs a `Box<dyn Action>` (app_menu.rs:92), and
// gpui's action identity is a TYPE, not a value, so a list of ten entries is ten unit
// actions rather than one parameterised one. Slot `i` means "the entry at index i of the
// list `recents_items` was given", which is why the mapping below is a pure function with
// tests: the alignment between a label and a slot is the only place this feature can put
// the wrong file in the user's hands.
actions!(
    notes_recent,
    [
        Recent0, Recent1, Recent2, Recent3, Recent4, Recent5, Recent6, Recent7, Recent8, Recent9
    ]
);

/// The port keeps at most ten (README: "recent files (up to 10)"), and the menu shows at
/// most ten - the cap is applied HERE so the slot actions above cannot be indexed past a
/// struct that exists.
pub(crate) const MAX_RECENTS: usize = 10;

/// Which menu item a recent entry at `index` becomes, labelled `label`. Ten arms because
/// a gpui action's identity is its TYPE (`MenuItem::action` takes `impl Action`, so one
/// generic parameter per call site - a table of them cannot close over a shared
/// constructor). `None` past the cap is a bug guard, not a policy.
fn slot_item(index: usize, label: SharedString) -> Option<MenuItem> {
    match index {
        0 => Some(MenuItem::action(label, Recent0)),
        1 => Some(MenuItem::action(label, Recent1)),
        2 => Some(MenuItem::action(label, Recent2)),
        3 => Some(MenuItem::action(label, Recent3)),
        4 => Some(MenuItem::action(label, Recent4)),
        5 => Some(MenuItem::action(label, Recent5)),
        6 => Some(MenuItem::action(label, Recent6)),
        7 => Some(MenuItem::action(label, Recent7)),
        8 => Some(MenuItem::action(label, Recent8)),
        9 => Some(MenuItem::action(label, Recent9)),
        _ => None,
    }
}

/// THE RECENT LIST AS MENU ITEMS: the label is `RecentEntry::display` VERBATIM, which is
/// the OUTPUT of `crates/core/src/recent.rs:237 visible_label` (applied at :221, then
/// hard-capped), so the RTL and zero-width characters are already escaped. A bridge that
/// labelled from `path` instead would be re-inventing a rule core owns.
pub(crate) fn recents_items(entries: &[RecentEntry]) -> Vec<MenuItem> {
    let mut items = Vec::new();
    for (index, entry) in entries.iter().take(MAX_RECENTS).enumerate() {
        let label = format!("{}. {}", index + 1, entry.display).into();
        // The label is built FIRST and then handed to the slot, so the number a user reads
        // and the index the action carries are the same expression.
        let Some(item) = slot_item(index, label) else {
            break;
        };
        items.push(item.disabled(!entry.exists));
    }
    items
}

/// Which entry a slot action names. `None` means the list was shorter than the action, which
/// happens when the menu was rebuilt between the click and the dispatch - a missing file is
/// reported, never guessed at.
pub(crate) fn entry_for(entries: &[RecentEntry], index: usize) -> Option<&RecentEntry> {
    entries.get(index)
}

/// THE KEY LIST, AS DATA. On Windows `set_menus` stores the bar and never draws it
/// (gpui-pre-windows src/platform.rs:746), so these chords ARE the menu on this platform,
/// and a command only its author can find is not a command. One table feeds three
/// consumers - `key_bindings`, `legend`, and the accelerator text inside `build_menus` - so
/// the sentence a user reads cannot disagree with the key that works. That is why it is a
/// list and not four hand-written strings.
///
/// Chosen against the keymap the app already owns (ctrl-c/x/v, the arrows, home/end, the
/// page keys - all editor actions): every chord below is unclaimed. Alt+digit carries the
/// recents because ten slots need ten keys and that is the convention a notepad user
/// expects; the tenth is Alt+0 because Alt+10 is not a chord.
pub(crate) const SHORTCUTS: &[(&str, &str, &str)] = &[
    ("ctrl-o", "Ctrl+O", "open a file"),
    ("ctrl-s", "Ctrl+S", "save as"),
    ("ctrl-t", "Ctrl+T", "toggle autosave"),
    ("ctrl-shift-r", "Ctrl+Shift+R", "clear recent files"),
    ("alt-1", "Alt+1", "recent 1"),
    ("alt-2", "Alt+2", "recent 2"),
    ("alt-3", "Alt+3", "recent 3"),
    ("alt-4", "Alt+4", "recent 4"),
    ("alt-5", "Alt+5", "recent 5"),
    ("alt-6", "Alt+6", "recent 6"),
    ("alt-7", "Alt+7", "recent 7"),
    ("alt-8", "Alt+8", "recent 8"),
    ("alt-9", "Alt+9", "recent 9"),
    ("alt-0", "Alt+0", "recent 10"),
];

/// One chord to its binding. A match, because a `KeyBinding` needs the action TYPE and a
/// const table cannot hold types; `None` means the table gained a row nobody bound, which
/// the test below turns into a failure instead of a silently dead key.
fn binding_for(chord: &str) -> Option<KeyBinding> {
    let binding = match chord {
        "ctrl-o" => KeyBinding::new(chord, OpenFile, None),
        "ctrl-s" => KeyBinding::new(chord, SaveAsFile, None),
        "ctrl-t" => KeyBinding::new(chord, ToggleAutosave, None),
        "ctrl-shift-r" => KeyBinding::new(chord, ClearRecents, None),
        "alt-1" => KeyBinding::new(chord, Recent0, None),
        "alt-2" => KeyBinding::new(chord, Recent1, None),
        "alt-3" => KeyBinding::new(chord, Recent2, None),
        "alt-4" => KeyBinding::new(chord, Recent3, None),
        "alt-5" => KeyBinding::new(chord, Recent4, None),
        "alt-6" => KeyBinding::new(chord, Recent5, None),
        "alt-7" => KeyBinding::new(chord, Recent6, None),
        "alt-8" => KeyBinding::new(chord, Recent7, None),
        "alt-9" => KeyBinding::new(chord, Recent8, None),
        "alt-0" => KeyBinding::new(chord, Recent9, None),
        _ => return None,
    };
    Some(binding)
}

/// What the app actually registers. Called once, in the run closure.
pub(crate) fn key_bindings() -> Vec<KeyBinding> {
    SHORTCUTS
        .iter()
        .filter_map(|(chord, ..)| binding_for(chord))
        .collect()
}

/// The human-facing spelling of a chord, so a menu label can name the key without
/// restating it.
pub(crate) fn display_of(chord: &str) -> Option<&'static str> {
    SHORTCUTS
        .iter()
        .find(|(binding, ..)| *binding == chord)
        .map(|(_, display, _)| *display)
}

/// THE LEGEND, generated from the same table. This is what the status line says on the
/// first frame, because on this platform there is nothing else that can tell a user the
/// commands exist.
pub(crate) fn legend() -> SharedString {
    let words: Vec<String> = SHORTCUTS
        .iter()
        .filter(|(chord, ..)| binding_for(chord).is_some())
        .map(|(_, display, what)| format!("{display} {what}"))
        .collect();
    words.join("  |  ").into()
}

/// The whole bar. `autosave_on` is the PORT's last stated value, and the check mark is the
/// only place that number becomes visible.
pub(crate) fn build_menus(autosave_on: bool, entries: &[RecentEntry]) -> Vec<Menu> {
    // Accelerator text is READ FROM THE TABLE, so a chord can only be changed in one
    // place. A menu label naming a key the keymap does not bind is worse than no label, and
    // a hand-copied string is how that happens.
    let key = |chord: &str| display_of(chord).unwrap_or("");
    let mut recents = Menu::new("Open Recent");
    recents.items = recents_items(entries);
    if !recents.items.is_empty() {
        recents.items.push(MenuItem::separator());
    }
    recents.items.push(MenuItem::action(
        format!("Clear Recent Files\t{}", key("ctrl-shift-r")),
        ClearRecents,
    ));

    let mut file = Menu::new("File");
    file.items = vec![
        MenuItem::action(format!("Open...\t{}", key("ctrl-o")), OpenFile),
        MenuItem::action(format!("Save As...\t{}", key("ctrl-s")), SaveAsFile),
        MenuItem::submenu(recents),
        MenuItem::separator(),
        MenuItem::action(format!("Auto-save\t{}", key("ctrl-t")), ToggleAutosave)
            .checked(autosave_on),
    ];
    vec![file]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(name: &str, exists: bool) -> RecentEntry {
        RecentEntry {
            path: PathBuf::from(format!("C:\\notes\\{name}")),
            display: name.to_string(),
            exists,
        }
    }

    /// The label is the port's string, unedited, and the number is the slot. If this drifts
    /// the menu offers "3. shopping.notes" and opens the fifth file.
    #[test]
    fn a_recent_is_labelled_by_the_ports_own_string_at_its_own_slot() {
        let entries = [entry("alpha.notes", true), entry("beta.notes", true)];
        let items = recents_items(&entries);
        assert_eq!(items.len(), 2);
        let names: Vec<String> = items
            .iter()
            .map(|item| match item {
                MenuItem::Action { name, .. } => name.to_string(),
                _ => panic!("a recent entry must be an action"),
            })
            .collect();
        assert_eq!(names, vec!["1. alpha.notes", "2. beta.notes"]);
    }

    /// A path that no longer exists is not removed - the port keeps it and says `false` -
    /// and must not be clickable. Deleting it would make the list lie about what the user
    /// opened; enabling it would hand the click to `LoadFailed`.
    #[test]
    fn a_missing_file_is_shown_greyed_and_never_forgotten() {
        let entries = [entry("here.notes", true), entry("gone.notes", false)];
        let items = recents_items(&entries);
        let flags: Vec<bool> = items
            .iter()
            .map(|item| match item {
                MenuItem::Action { disabled, .. } => *disabled,
                _ => panic!("a recent entry must be an action"),
            })
            .collect();
        assert_eq!(flags, vec![false, true], "only the missing one is disabled");
    }

    /// Ten is the cap the product states, and ten is how many slot actions exist. An
    /// eleventh entry would otherwise be labelled "11." and bound to no action at all.
    #[test]
    fn the_list_stops_where_the_slots_end() {
        let entries: Vec<RecentEntry> = (0..14)
            .map(|i| entry(&format!("{i}.notes"), true))
            .collect();
        let items = recents_items(&entries);
        assert_eq!(items.len(), MAX_RECENTS, "ten, and never more");
        assert!(
            slot_item(MAX_RECENTS, "x".into()).is_none(),
            "there is no eleventh slot"
        );
        assert!(slot_item(0, "x".into()).is_some());
    }

    /// The empty list is a menu that still has Clear, and no dangling separator.
    #[test]
    fn an_empty_recent_list_does_not_draw_a_separator_into_nothing() {
        let menus = build_menus(true, &[]);
        let recents = &menus[0].items[2];
        let MenuItem::Submenu(sub) = recents else {
            panic!("the third File item is the recent submenu")
        };
        assert_eq!(sub.items.len(), 1, "just Clear Recent Files");
        let MenuItem::Action { name, .. } = &sub.items[0] else {
            panic!("the only item is the clear action")
        };
        assert_eq!(
            name.as_ref(),
            "Clear Recent Files\tCtrl+Shift+R",
            "the clear item names its own key, from the table"
        );
    }

    /// THE DRIFT TEST, and the reason the table exists. Every row must have a binding, or
    /// the legend advertises a key that does nothing - the exact failure a hand-maintained
    /// list of strings produces the day someone adds a command and forgets the keymap.
    #[test]
    fn every_chord_in_the_list_is_bound_and_named() {
        let bindings = key_bindings();
        assert_eq!(
            bindings.len(),
            SHORTCUTS.len(),
            "one binding per row, nothing dropped by a missing match arm",
        );
        for (_, display, what) in SHORTCUTS {
            let words = legend().to_string();
            assert!(
                words.contains(display) && words.contains(what),
                "{display} is in the table but not on the user's screen: {words}"
            );
        }
        assert_eq!(display_of("ctrl-o"), Some("Ctrl+O"));
        assert_eq!(
            display_of("ctrl-z"),
            None,
            "a chord nobody listed stays unlabelled"
        );
    }

    /// The labels the bar carries name the SAME keys the bindings do, in the same place.
    #[test]
    fn the_menu_names_the_keys_that_work() {
        let menus = build_menus(true, &[]);
        let names: Vec<String> = menus[0]
            .items
            .iter()
            .map(|item| match item {
                MenuItem::Action { name, .. } => name.to_string(),
                MenuItem::Submenu(menu) => menu.name.to_string(),
                MenuItem::Separator => "--".to_string(),
                _ => "?".to_string(),
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "Open...\tCtrl+O",
                "Save As...\tCtrl+S",
                "Open Recent",
                "--",
                "Auto-save\tCtrl+T",
            ]
        );
    }

    /// The check mark is the ONLY visible readback of the setting, and it must come from
    /// what was passed in - the port's value - not from a literal. Toggling Autosave off
    /// with `autosave_on` still true would be the bridge asserting a state it does not have.
    #[test]
    fn the_autosave_check_reflects_the_value_it_was_given() {
        let on = build_menus(true, &[]);
        let off = build_menus(false, &[]);
        let check = |menus: &Vec<Menu>| match &menus[0].items[4] {
            MenuItem::Action { checked, .. } => *checked,
            _ => panic!("the fifth File item is the autosave toggle"),
        };
        assert!(check(&on), "on is checked");
        assert!(!check(&off), "off is not");
    }
}
