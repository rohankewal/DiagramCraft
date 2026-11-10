//! The macOS menu bar.
//!
//! On macOS the menu bar belongs to the system, not to the window, so the
//! commands go into a real `NSMenu` at the top of the screen rather than into
//! a row drawn inside the window. The structure is the same one
//! [`crate::menu`] describes; this module only translates it.
//!
//! Two things are translated rather than copied across:
//!
//! **The application menu.** macOS puts *About* and *Quit* in the menu named
//! after the application, so those two are lifted out of Help and File and the
//! rules they leave behind are tidied away.
//!
//! **Accelerators.** Only shortcuts holding the command key are registered.
//! A menu accelerator is matched before the key reaches the window, so
//! claiming a bare `T` or `Delete` would make it impossible to type a `T` into
//! a label or to backspace inside one — see [`Shortcut::is_claimable`].
//!
//! Because the system owns those shortcuts, the window layer stops handling
//! command keys itself on macOS. Exactly one of the two dispatches any given
//! shortcut, which is why neither can fire twice.

use crate::menu::{ItemKind, Menu, MenuItem, Shortcut, ShortcutKey};
use crate::ui::UiAction;
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{AboutMetadata, CheckMenuItem, PredefinedMenuItem, Submenu};
use std::collections::HashMap;

/// One native item, kept so its state can be updated without rebuilding the
/// menu underneath the user's pointer.
enum NativeItem {
    Command(muda::MenuItem),
    Check(CheckMenuItem),
}

impl NativeItem {
    fn id(&self) -> String {
        match self {
            NativeItem::Command(item) => item.id().0.clone(),
            NativeItem::Check(item) => item.id().0.clone(),
        }
    }

    fn set_enabled(&self, enabled: bool) {
        match self {
            NativeItem::Command(item) => item.set_enabled(enabled),
            NativeItem::Check(item) => item.set_enabled(enabled),
        }
    }
}

/// The system menu bar, and the map from its item ids back to commands.
pub struct NativeMenus {
    /// Held so the menu outlives its installation in the application.
    _menu: muda::Menu,
    items: Vec<NativeItem>,
    actions: HashMap<String, UiAction>,
    /// The state the native items were last set from, so an unchanged frame
    /// does no work.
    last: Vec<Menu>,
}

impl NativeMenus {
    /// Builds the menu bar and installs it as the application's.
    ///
    /// Must be called on the main thread once the application exists, which
    /// is why the window layer does it from `resumed`.
    pub fn install(menus: &[Menu]) -> Result<Self, muda::Error> {
        let bar = muda::Menu::new();
        bar.append(&app_submenu()?)?;

        let mut items = Vec::new();
        let mut actions = HashMap::new();
        // The application menu's own command, which is not rebuilt by `sync`
        // because nothing about it changes.
        actions.insert(SETTINGS_ID.to_string(), UiAction::OpenSettings);

        for (menu_index, menu) in menus.iter().enumerate() {
            let submenu = Submenu::new(&menu.title, true);
            for (item_index, item) in native_items(menu).iter().enumerate() {
                if item.is_separator() {
                    submenu.append(&PredefinedMenuItem::separator())?;
                    continue;
                }

                let id = format!("{menu_index}.{item_index}");
                let accelerator = accelerator_for(&item.shortcut);
                let native = match item.kind {
                    ItemKind::Check(checked) => NativeItem::Check(CheckMenuItem::with_id(
                        &id,
                        &item.label,
                        item.enabled,
                        checked,
                        accelerator,
                    )),
                    _ => NativeItem::Command(muda::MenuItem::with_id(
                        &id,
                        &item.label,
                        item.enabled,
                        accelerator,
                    )),
                };

                match &native {
                    NativeItem::Command(i) => submenu.append(i)?,
                    NativeItem::Check(i) => submenu.append(i)?,
                }
                actions.insert(id, item.action.clone());
                items.push(native);
            }
            bar.append(&submenu)?;
        }

        bar.init_for_nsapp();

        Ok(NativeMenus {
            _menu: bar,
            items,
            actions,
            last: menus.to_vec(),
        })
    }

    /// Brings the native items in line with the current state.
    ///
    /// Only the enabled flag, the check mark, and the action behind each item
    /// can change; the labels and the structure are fixed. Rebuilding the menu
    /// instead would drop it while it was open.
    pub fn sync(&mut self, menus: &[Menu]) {
        if self.last == menus {
            return;
        }

        let mut next = self.items.iter();
        for menu in menus {
            for item in native_items(menu) {
                if item.is_separator() {
                    continue;
                }
                let Some(native) = next.next() else { break };
                native.set_enabled(item.enabled);
                if let (NativeItem::Check(check), ItemKind::Check(checked)) =
                    (native, &item.kind)
                {
                    check.set_checked(*checked);
                }
                self.actions.insert(native.id(), item.action.clone());
            }
        }

        self.last = menus.to_vec();
    }

    /// The command a menu item id stands for.
    pub fn action(&self, id: &str) -> Option<UiAction> {
        self.actions.get(id).cloned()
    }
}

/// The id of the Settings item in the application menu.
const SETTINGS_ID: &str = "app.settings";

/// The application menu, which macOS names after the application.
fn app_submenu() -> Result<Submenu, muda::Error> {
    let about = AboutMetadata {
        name: Some("DiagramCraft".into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        comments: Some("Open-source diagramming, rebuilt in pure Rust.".into()),
        ..Default::default()
    };

    let menu = Submenu::new("DiagramCraft", true);
    menu.append_items(&[
        &PredefinedMenuItem::about(Some("About DiagramCraft"), Some(about)),
        &PredefinedMenuItem::separator(),
        // Every other application on the platform keeps this here, under this
        // name and this shortcut, so this one does too.
        &muda::MenuItem::with_id(
            SETTINGS_ID,
            "Settings…",
            true,
            accelerator_for("⌘,"),
        ),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::hide(None),
        &PredefinedMenuItem::hide_others(None),
        &PredefinedMenuItem::show_all(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::quit(None),
    ])?;
    Ok(menu)
}

/// A menu's items as the native bar should show them: without the two the
/// application menu already carries, and without the rules those leave behind.
pub fn native_items(menu: &Menu) -> Vec<MenuItem> {
    let kept = menu
        .items
        .iter()
        .filter(|item| {
            !matches!(
                item.action,
                UiAction::Quit | UiAction::ShowAbout | UiAction::OpenSettings
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    tidy_separators(kept)
}

/// Drops leading, trailing, and doubled rules from a list of items.
pub fn tidy_separators(items: Vec<MenuItem>) -> Vec<MenuItem> {
    let mut out: Vec<MenuItem> = Vec::with_capacity(items.len());
    for item in items {
        if item.is_separator() {
            // Nothing to separate from yet, or the last thing was a rule too.
            if out.last().is_none_or(MenuItem::is_separator) {
                continue;
            }
        }
        out.push(item);
    }
    while out.last().is_some_and(MenuItem::is_separator) {
        out.pop();
    }
    out
}

/// The accelerator a shortcut string names, if a menu bar may claim it.
pub fn accelerator_for(shortcut: &str) -> Option<Accelerator> {
    let shortcut = Shortcut::parse(shortcut)?;
    if !shortcut.is_claimable() {
        return None;
    }

    let mut modifiers = Modifiers::empty();
    if shortcut.command {
        modifiers |= Modifiers::META;
    }
    if shortcut.shift {
        modifiers |= Modifiers::SHIFT;
    }
    if shortcut.alt {
        modifiers |= Modifiers::ALT;
    }

    Some(Accelerator::new(Some(modifiers), code_for(shortcut.key)?))
}

fn code_for(key: ShortcutKey) -> Option<Code> {
    Some(match key {
        ShortcutKey::Delete => Code::Backspace,
        ShortcutKey::Enter => Code::Enter,
        ShortcutKey::Escape => Code::Escape,
        ShortcutKey::Char(c) => match c.to_ascii_uppercase() {
            'A' => Code::KeyA,
            'B' => Code::KeyB,
            'C' => Code::KeyC,
            'D' => Code::KeyD,
            'E' => Code::KeyE,
            'F' => Code::KeyF,
            'G' => Code::KeyG,
            'H' => Code::KeyH,
            'I' => Code::KeyI,
            'J' => Code::KeyJ,
            'K' => Code::KeyK,
            'L' => Code::KeyL,
            'M' => Code::KeyM,
            'N' => Code::KeyN,
            'O' => Code::KeyO,
            'P' => Code::KeyP,
            'Q' => Code::KeyQ,
            'R' => Code::KeyR,
            'S' => Code::KeyS,
            'T' => Code::KeyT,
            'U' => Code::KeyU,
            'V' => Code::KeyV,
            'W' => Code::KeyW,
            'X' => Code::KeyX,
            'Y' => Code::KeyY,
            'Z' => Code::KeyZ,
            '0' => Code::Digit0,
            '1' => Code::Digit1,
            '2' => Code::Digit2,
            '3' => Code::Digit3,
            '4' => Code::Digit4,
            '5' => Code::Digit5,
            '6' => Code::Digit6,
            '7' => Code::Digit7,
            '8' => Code::Digit8,
            '9' => Code::Digit9,
            '+' | '=' => Code::Equal,
            '-' => Code::Minus,
            ',' => Code::Comma,
            '.' => Code::Period,
            '[' => Code::BracketLeft,
            ']' => Code::BracketRight,
            '/' => Code::Slash,
            _ => return None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Editor;
    use crate::menu::{self, ViewState};

    fn menus() -> Vec<Menu> {
        let mut e = Editor::new();
        e.resize_viewport(900.0, 700.0);
        menu::menus(&e, &ViewState::default())
    }

    #[test]
    fn quit_about_and_settings_move_to_the_application_menu() {
        for menu in menus() {
            for item in native_items(&menu) {
                for lifted in [UiAction::Quit, UiAction::ShowAbout, UiAction::OpenSettings] {
                    assert_ne!(
                        item.action, lifted,
                        "{} still has {lifted:?}",
                        menu.title
                    );
                }
            }
        }
    }

    #[test]
    fn lifting_those_two_out_leaves_no_dangling_rules() {
        for menu in menus() {
            let items = native_items(&menu);
            assert!(!items.is_empty(), "{} emptied out", menu.title);
            assert!(!items.first().unwrap().is_separator(), "{}", menu.title);
            assert!(!items.last().unwrap().is_separator(), "{}", menu.title);
            for pair in items.windows(2) {
                assert!(
                    !(pair[0].is_separator() && pair[1].is_separator()),
                    "{} has a doubled rule",
                    menu.title
                );
            }
        }
    }

    #[test]
    fn every_other_command_survives_the_translation() {
        for menu in menus() {
            let before: Vec<&UiAction> = menu
                .items
                .iter()
                .filter(|i| !i.is_separator())
                .map(|i| &i.action)
                .filter(|a| {
                    !matches!(
                        a,
                        UiAction::Quit | UiAction::ShowAbout | UiAction::OpenSettings
                    )
                })
                .collect();
            let after: Vec<UiAction> = native_items(&menu)
                .into_iter()
                .filter(|i| !i.is_separator())
                .map(|i| i.action)
                .collect();
            assert_eq!(
                before.len(),
                after.len(),
                "{} lost a command",
                menu.title
            );
        }
    }

    #[test]
    fn tidying_handles_the_degenerate_lists() {
        let rule = || MenuItem {
            label: String::new(),
            shortcut: String::new(),
            action: UiAction::NoOp,
            kind: ItemKind::Separator,
            enabled: false,
        };
        assert!(tidy_separators(vec![]).is_empty());
        assert!(tidy_separators(vec![rule(), rule()]).is_empty());

        let command = MenuItem {
            label: "Save".into(),
            shortcut: String::new(),
            action: UiAction::Save,
            kind: ItemKind::Command,
            enabled: true,
        };
        let tidied = tidy_separators(vec![rule(), command.clone(), rule(), rule(), command, rule()]);
        assert_eq!(tidied.len(), 3, "a rule should survive between two commands");
        assert!(!tidied[0].is_separator());
        assert!(tidied[1].is_separator());
        assert!(!tidied[2].is_separator());
    }

    #[test]
    fn command_shortcuts_become_accelerators() {
        assert!(accelerator_for("⌘S").is_some());
        assert!(accelerator_for("Shift ⌘Z").is_some());
        assert!(accelerator_for("⌘1").is_some());
        assert!(accelerator_for("Shift ⌘]").is_some());
        assert!(accelerator_for("⌘+").is_some());
        assert!(accelerator_for("⌘-").is_some());
    }

    #[test]
    fn bare_keys_are_never_claimed() {
        // The regression this guards: claiming `T` natively would stop the
        // letter ever reaching an open label.
        for bare in ["T", "C", "?", "Del", "Enter", "Esc", ""] {
            assert!(
                accelerator_for(bare).is_none(),
                "{bare:?} must not become an accelerator"
            );
        }
    }

    #[test]
    fn every_claimable_shortcut_in_the_menus_converts() {
        for menu in menus() {
            for item in native_items(&menu) {
                let Some(shortcut) = Shortcut::parse(&item.shortcut) else {
                    continue;
                };
                if !shortcut.is_claimable() {
                    continue;
                }
                assert!(
                    accelerator_for(&item.shortcut).is_some(),
                    "{} > {} has {:?}, which has no key code",
                    menu.title,
                    item.label,
                    item.shortcut
                );
            }
        }
    }

    #[test]
    fn item_ids_are_unique_across_the_bar() {
        let menus = menus();
        let mut seen: Vec<String> = Vec::new();
        for (menu_index, menu) in menus.iter().enumerate() {
            for (item_index, item) in native_items(menu).iter().enumerate() {
                if item.is_separator() {
                    continue;
                }
                let id = format!("{menu_index}.{item_index}");
                assert!(!seen.contains(&id), "{id} is used twice");
                seen.push(id);
            }
        }
        assert!(seen.len() > 50, "only {} items", seen.len());
    }

    #[test]
    fn the_item_order_is_stable_as_state_changes() {
        // `sync` walks the native items in build order, so the sequence of
        // non-separator items must not depend on editor state.
        let order = |e: &Editor| {
            menu::menus(e, &ViewState::default())
                .iter()
                .flat_map(native_items)
                .filter(|i| !i.is_separator())
                .map(|i| i.label.clone())
                .collect::<Vec<_>>()
        };

        let mut empty = Editor::new();
        empty.resize_viewport(900.0, 700.0);
        let mut busy = Editor::new();
        busy.resize_viewport(900.0, 700.0);
        busy.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(0.0, 0.0, 10.0, 10.0),
        );
        busy.add_page();
        busy.select_all();

        assert_eq!(order(&empty), order(&busy));
    }
}
