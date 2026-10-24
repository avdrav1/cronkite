//! Brick's menus (MAC-4, WIN-3). On macOS and Windows the menu bar has File, Edit, Sheet,
//! Format, and Data, each item with its key from [`crate::keys`], running the window's
//! actions ([`add_window_actions`]) and the app's. Linux has the header bar's buttons
//! instead, with Data in its main menu; its recent files menu and File → Open Recent are
//! both [`fill_recent`].

use crate::grid_view::{FontToggle, GridView};
use crate::keys::{self, Command};
use crate::state;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::rc::Rc;
use suite_file::recent;
use suite_ui::chrome::MENU_BAR;
use suite_ui::keys::Primary;

thread_local! {
    /// File → Open Recent (macOS, Windows), filled again whenever the list may have changed.
    static RECENT: gio::Menu = gio::Menu::new();
}

/// Into `menu`: the recent files, read now, then Clear Recent Files.
pub fn fill_recent(menu: &gio::Menu) {
    let files = gio::Menu::new();
    for file in state::recent().files() {
        // A path that isn't UTF-8 can't travel in the action's string.
        let Some(path) = file.to_str() else { continue };
        let (name, dir) = recent::label(file);
        let item = gio::MenuItem::new(Some(&format!("{name}  ({dir})")), None);
        item.set_action_and_target_value(Some("app.open-recent"), Some(&path.to_variant()));
        files.append_item(&item);
    }
    if files.n_items() == 0 {
        files.append(Some("No recent files"), Some("app.none"));
    }
    menu.append_section(None, &files);
    let clear = gio::Menu::new();
    clear.append(Some("Clear Recent Files"), Some("app.clear-recent"));
    menu.append_section(None, &clear);
}

/// File → Open Recent shows the list as it is now (macOS, Windows; Linux reads it as its
/// menu opens).
pub fn refresh_recent() {
    if MENU_BAR {
        RECENT.with(|menu| {
            menu.remove_all();
            fill_recent(menu);
        });
    }
}

/// An item running `action`, showing `accel`.
fn item(label: &str, action: &str, accel: Option<String>) -> gio::MenuItem {
    let item = gio::MenuItem::new(Some(label), Some(action));
    if let Some(accel) = accel {
        item.set_attribute_value("accel", Some(&accel.to_variant()));
    }
    item
}

/// An item running `action`, showing the window key that does `command`.
fn command(label: &str, action: &str, command: Command) -> gio::MenuItem {
    item(label, action, keys::accel_for(Primary::HERE, command))
}

/// A menu of `sections`.
fn menu(sections: &[Vec<gio::MenuItem>]) -> gio::Menu {
    let menu = gio::Menu::new();
    for items in sections {
        let section = gio::Menu::new();
        for item in items {
            section.append_item(item);
        }
        menu.append_section(None, &section);
    }
    menu
}

/// The window keys' number formats, by their names in [`keys::WINDOW`], in Format → Number
/// Format's order.
const FORMATS: [&str; 5] = ["General", "Number", "Currency", "Date", "Percent"];

/// The code of the number format `name` (one of [`FORMATS`]).
pub fn format_code(name: &str) -> Option<&'static str> {
    keys::WINDOW.iter().find_map(|b| match b.command {
        Command::Format(code) if b.name == name => Some(code),
        _ => None,
    })
}

/// The menus: File, Edit, Sheet, Format, and Data.
pub fn menus() -> [(&'static str, gio::Menu); 5] {
    use Command as C;
    let open_recent = RECENT.with(|recent| {
        recent.remove_all();
        fill_recent(recent);
        gio::MenuItem::new_submenu(Some("Open Recent"), recent)
    });
    let file = menu(&[
        vec![
            command("New", "app.new", C::New),
            command("Open…", "app.open", C::Open),
            open_recent,
        ],
        vec![
            command("Close Window", "app.close", C::Close),
            command("Save", "win.save", C::Save),
            command("Save As…", "win.save-as", C::SaveAs),
        ],
    ]);
    if let Some(exit) = suite_ui::chrome::exit_section() {
        file.append_section(None, &exit);
    }
    let grid =
        |label: &str, action: &str| item(label, action, keys::grid_accel(Primary::HERE, label));
    let edit = menu(&[
        vec![
            command("Undo", "win.undo", C::Undo),
            command("Redo", "win.redo", C::Redo),
        ],
        vec![
            grid("Cut", "win.cut"),
            grid("Copy", "win.copy"),
            grid("Paste", "win.paste"),
        ],
        vec![
            command("Find…", "win.find", C::Find),
            command("Replace…", "win.replace", C::Replace),
            command("Go To…", "win.go-to", C::GoTo),
        ],
    ]);
    let sheet = menu(&[
        vec![
            command("New Sheet", "win.new-sheet", C::NewSheet),
            command("Next Sheet", "win.next-sheet", C::Sheet(1)),
            command("Previous Sheet", "win.previous-sheet", C::Sheet(-1)),
        ],
        vec![
            item("Insert Rows Above", "win.insert-above", None),
            item("Insert Rows Below", "win.insert-below", None),
            item("Delete Rows", "win.delete-rows", None),
        ],
        vec![
            item("Insert Columns Left", "win.insert-left", None),
            item("Insert Columns Right", "win.insert-right", None),
            item("Delete Columns", "win.delete-cols", None),
        ],
    ]);
    let formats = gio::Menu::new();
    for name in FORMATS {
        let Some(code) = format_code(name) else {
            continue;
        };
        let format = command(name, "win.number-format", C::Format(code));
        format.set_action_and_target_value(Some("win.number-format"), Some(&code.to_variant()));
        formats.append_item(&format);
    }
    formats.append_item(&command("Custom…", "win.custom-format", C::CustomFormat));
    let font = |label: &str, action: &str, toggle| command(label, action, C::Font(toggle));
    let format = menu(&[
        vec![
            font("Bold", "win.bold", FontToggle::Bold),
            font("Italic", "win.italic", FontToggle::Italic),
            font("Underline", "win.underline", FontToggle::Underline),
            font("Strikethrough", "win.strikethrough", FontToggle::Strike),
        ],
        vec![
            gio::MenuItem::new_submenu(Some("Number Format"), &formats),
            item("Clear Formatting", "win.clear-formatting", None),
        ],
        vec![
            item("Merge Cells", "win.merge", None),
            item("Unmerge Cells", "win.unmerge", None),
        ],
    ]);
    let data = data();
    [
        ("File", file),
        ("Edit", edit),
        ("Sheet", sheet),
        ("Format", format),
        ("Data", data),
    ]
}

/// Data: sorting, then First Row Is Header (APP-10), a check item on `win.header-row`.
pub fn data() -> gio::Menu {
    menu(&[
        vec![
            item("Sort Ascending", "win.sort-asc", None),
            item("Sort Descending", "win.sort-desc", None),
        ],
        vec![item("First Row Is Header", "win.header-row", None)],
    ])
}

/// Linux's main menu in a document's window: Data, then the app's section (About, Update
/// Automatically).
pub fn main_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let section = gio::Menu::new();
    section.append_submenu(Some("Data"), &data());
    menu.append_section(None, &section);
    menu.append_section(None, &suite_ui::chrome::app_section(crate::APP));
    menu
}

/// The menu bar: the menus, then macOS's Window menu or Windows' Help menu.
pub fn bar() -> gio::Menu {
    let menus = menus();
    let refs: Vec<(&str, &gio::Menu)> = menus.iter().map(|(l, m)| (*l, m)).collect();
    suite_ui::chrome::menu_bar(crate::APP, &refs)
}

/// Add the action `name` to `window`: `activate` runs with its target, a string where
/// `parameter` says so.
fn add(
    window: &gtk::ApplicationWindow,
    name: &str,
    parameter: Option<&glib::VariantTy>,
    activate: impl Fn(Option<&glib::Variant>) + 'static,
) {
    let action = gio::SimpleAction::new(name, parameter);
    action.connect_activate(move |_, target| activate(target));
    window.add_action(&action);
}

/// The window's actions the menu bar's items run (macOS, Windows; on Linux the main menu's
/// Data shows its own). `run` does a window command as its key does.
pub fn add_window_actions(
    window: &gtk::ApplicationWindow,
    grid: &GridView,
    run: Rc<dyn Fn(Command) -> bool>,
) {
    use Command as C;
    for (name, command) in [
        ("save", C::Save),
        ("save-as", C::SaveAs),
        ("undo", C::Undo),
        ("redo", C::Redo),
        ("find", C::Find),
        ("go-to", C::GoTo),
        ("new-sheet", C::NewSheet),
        ("next-sheet", C::Sheet(1)),
        ("previous-sheet", C::Sheet(-1)),
        ("bold", C::Font(FontToggle::Bold)),
        ("italic", C::Font(FontToggle::Italic)),
        ("underline", C::Font(FontToggle::Underline)),
        ("strikethrough", C::Font(FontToggle::Strike)),
        ("custom-format", C::CustomFormat),
    ] {
        let run = run.clone();
        add(window, name, None, move |_| {
            run(command);
        });
    }
    // Format → Number Format's items: the format's code.
    add(
        window,
        "number-format",
        Some(glib::VariantTy::STRING),
        move |target| {
            let code = target.and_then(|t| t.str());
            let known = FORMATS
                .iter()
                .filter_map(|name| format_code(name))
                .find(|c| Some(*c) == code);
            if let Some(code) = known {
                run(C::Format(code));
            }
        },
    );
    // The grid's own, as its context menu has them.
    for name in [
        "insert-above",
        "insert-below",
        "delete-rows",
        "insert-left",
        "insert-right",
        "delete-cols",
        "sort-asc",
        "sort-desc",
        "merge",
        "unmerge",
        "clear-formatting",
    ] {
        let (grid, target) = (grid.downgrade(), format!("grid.{name}"));
        add(window, name, None, move |_| {
            if let Some(grid) = grid.upgrade() {
                let _ = grid.activate_action(&target, None);
            }
        });
    }
    // Cut, Copy, and Paste: the focused text's (a cell being edited, the formula bar, the
    // find bar), else the grid's selection.
    for name in ["cut", "copy", "paste"] {
        let (weak, grid) = (window.downgrade(), grid.downgrade());
        add(window, name, None, move |_| {
            let (Some(window), Some(grid)) = (weak.upgrade(), grid.upgrade()) else {
                return;
            };
            let text = GtkWindowExt::focus(&window).is_some_and(|f| {
                f.activate_action(&format!("clipboard.{name}"), None)
                    .is_ok()
            });
            if !text {
                match name {
                    "paste" => grid.paste_clipboard(),
                    _ => grid.copy_selection(name == "cut"),
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use suite_ui::chrome::items;

    #[test]
    fn every_item_shows_its_key_from_the_tables() {
        let bar = bar();
        let all = items(&bar);
        let accel = |label: &str| {
            all.iter()
                .find(|(l, _, _)| l == label)
                .unwrap_or_else(|| panic!("no {label}"))
                .2
                .clone()
        };
        let p = Primary::HERE;
        assert_eq!(accel("Save"), Some(p.accel("<Primary>s")));
        assert_eq!(accel("Save As…"), Some(p.accel("<Primary><Shift>s")));
        assert_eq!(accel("Redo"), Some(p.accel("<Primary><Shift>z")));
        assert_eq!(accel("Copy"), Some(p.accel("<Primary>c")));
        assert_eq!(accel("Bold"), Some(p.accel("<Primary>b")));
        assert_eq!(accel("Strikethrough"), Some(p.accel("<Primary>5")));
        assert_eq!(accel("Custom…"), Some(p.accel("<Primary>1")));
        assert_eq!(accel("Next Sheet"), Some("<Control>Page_Down".to_owned()));
        assert_eq!(accel("Currency"), Some("<Control><Shift>dollar".to_owned()));
        assert_eq!(
            accel("Replace…"),
            Some(p.pick("<Control>h", "<Alt><Meta>f").to_owned())
        );
        assert_eq!(
            accel("Close Window"),
            (p == Primary::Command).then(|| "<Meta>w".to_owned())
        );
        for name in FORMATS {
            assert!(format_code(name).is_some(), "{name}");
        }
        assert_eq!(format_code("Percent"), Some("0.00%"));
        let top: Vec<_> = (0..bar.n_items())
            .filter_map(|i| {
                bar.item_attribute_value(i, "label", Some(gtk::glib::VariantTy::STRING))
                    .and_then(|v| v.get::<String>())
            })
            .collect();
        let mut want = vec!["File", "Edit", "Sheet", "Format", "Data"];
        if cfg!(target_os = "macos") {
            want.push("Window");
        } else if cfg!(windows) {
            want.push("Help");
            assert!(all.contains(&("Exit".to_owned(), Some("app.quit".to_owned()), None)));
        }
        assert_eq!(top, want);
    }

    #[test]
    fn data_has_first_row_is_header() {
        let want = (
            "First Row Is Header".to_owned(),
            Some("win.header-row".to_owned()),
            None,
        );
        assert!(items(&bar()).contains(&want));
        // Linux's main menu has Data too, before About.
        let main = items(&main_menu());
        let at = |label: &str| main.iter().position(|(l, _, _)| l == label).unwrap();
        assert!(main.contains(&want));
        assert!(at("First Row Is Header") < at("About Brick"));
    }
}
