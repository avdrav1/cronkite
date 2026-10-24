//! Window chrome and menus (MAC-4, WIN-3): GNOME's on Linux, the Mac's on macOS, Windows'
//! on Windows.
//!
//! On Linux a window has a header bar with its buttons, and at its end the main menu
//! ([`main_menu`]) with "Update Automatically" and About ([`app_section`]); Slate's menus
//! are in that menu too, and Etch shows its menu bar in the window, with Help holding
//! the same two items ([`help_menu`]). On macOS the app's menus are in the menu bar
//! ([`menu_bar`]), after the suite's app menu, which GTK builds there (About, Update
//! Automatically, Services, Hide ⌘H, Hide Others ⌥⌘H, Show All, Quit ⌘Q, whose About,
//! Update Automatically, and Quit run `app.about`, `app.update-automatically`, and
//! `app.quit`; `macos_app_menu`) and before a Window menu (Minimize ⌘M, Zoom, then the windows,
//! which AppKit lists). On Windows each window shows the same menus in a menu bar row
//! under its title bar (GTK's `GtkApplicationWindow` puts the app's menu bar there),
//! followed by Help with Update Automatically and About, and File ends with Exit
//! ([`exit_section`]). Windows keep the system's own title bar on both ([`init`] asks GTK
//! for it on Windows), and the buttons a header bar would hold go in a toolbar row at the
//! top of the window ([`Toolbar`]).
//!
//! Each app builds its menus once, as `gio::Menu`s, and shows the same ones everywhere.

use gtk::prelude::*;
use gtk::{gio, glib};
use suite_core::AppKind;

/// Whether this build puts the app's menus in a menu bar and keeps the system's title
/// bars: on macOS (the system's menu bar) and Windows (a row in each window).
pub const MENU_BAR: bool = cfg!(any(target_os = "macos", windows));

/// Ask GTK for the system's title bars where windows keep them: on Windows GTK draws its
/// own title bar (client-side decorations) unless `GTK_CSD` is `0`, even on a window
/// without a header bar. Leaves a `GTK_CSD` the environment sets alone. Call in `main`
/// before GTK starts and before any thread does; elsewhere it does nothing.
pub fn init() {
    if let Some(value) = csd(cfg!(windows), std::env::var_os("GTK_CSD").is_some()) {
        // Before any other thread runs (the caller's contract), so nothing reads the
        // environment meanwhile.
        std::env::set_var("GTK_CSD", value);
    }
}

/// The `GTK_CSD` to set on Windows (`windows`) when the environment has none (`set`).
fn csd(windows: bool, set: bool) -> Option<&'static str> {
    (windows && !set).then_some("0")
}

/// The `gio::Menu` attribute that makes a submenu macOS's Window menu (GTK's macOS
/// backend hands it to `NSApp.windowsMenu`, where AppKit lists the windows).
const MACOS_SPECIAL: &str = "gtk-macos-special";

/// "Update Automatically" (ADR 0012): whether it's on as the app starts, and what keeps a
/// change across runs (the app's `suite_update::automatic` and `suite_update::set_automatic`;
/// this crate doesn't depend on `suite-update`).
#[derive(Clone, Copy, Debug)]
pub struct AutoUpdate {
    pub on: bool,
    pub keep: fn(bool) -> std::io::Result<()>,
}

/// The label of the check item that turns updating on and off.
pub const UPDATE_AUTOMATICALLY: &str = "Update Automatically";

/// Install the app actions every app has, at startup: `app.about` (the About dialog, the
/// app menu's first item on macOS, Help's on Windows), `app.update-automatically` (the
/// check item beside it, `update`'s state, each change kept), and the Window menu's
/// `app.minimize` (⌘M on macOS) and `app.zoom`, which act on the active window. On Windows
/// also the windows' conventions there (`windows_conventions`).
pub fn install(app: &gtk::Application, kind: AppKind, version: &'static str, update: AutoUpdate) {
    let about_entry = gio::ActionEntry::builder("about")
        .activate(move |app: &gtk::Application, _, _| {
            about(app.active_window().as_ref(), kind, version);
        })
        .build();
    let minimize = gio::ActionEntry::builder("minimize")
        .activate(|app: &gtk::Application, _, _| {
            if let Some(window) = app.active_window() {
                window.minimize();
            }
        })
        .build();
    let zoom = gio::ActionEntry::builder("zoom")
        .activate(|app: &gtk::Application, _, _| {
            if let Some(window) = app.active_window() {
                if window.is_maximized() {
                    window.unmaximize();
                } else {
                    window.maximize();
                }
            }
        })
        .build();
    app.add_action_entries([about_entry, minimize, zoom]);
    app.add_action(&update_action(update));
    if cfg!(target_os = "macos") {
        app.set_accels_for_action("app.minimize", &["<Meta>m"]);
    }
    if cfg!(windows) {
        windows_conventions(app, kind);
    }
}

/// `update-automatically`: a boolean state starting at `update.on`; activating it toggles
/// it (GIO's default for a boolean action without a parameter), and a change takes only
/// once `update.keep` has written it.
fn update_action(update: AutoUpdate) -> gio::SimpleAction {
    let action =
        gio::SimpleAction::new_stateful("update-automatically", None, &update.on.to_variant());
    action.connect_change_state(move |action, value| {
        let Some(on) = value.and_then(|v| v.get::<bool>()) else {
            return;
        };
        match (update.keep)(on) {
            Ok(()) => action.set_state(&on.to_variant()),
            Err(e) => tracing::warn!("update: cannot keep {UPDATE_AUTOMATICALLY}: {e}"),
        }
    });
    action
}

/// macOS: the suite's app menu (`data/app-menu.ui`: About `<App>`, Update Automatically,
/// Services, Hide, Hide Others, Show All, Quit), with "Update Automatically" a check item
/// for `app.update-automatically`. GTK 4's macOS backend builds the app menu from the
/// resource `/org/gtk/libgtk/ui/gtkapplication-quartz.ui` and has no API to add to it, so
/// `build.rs` compiles the suite's menu to that path and this registers it ahead of GTK's
/// own: GLib looks a path up in the resource registered last first. GTK's resources are
/// registered in `gtk_init`, or, compiled in as static resources, at the first lookup,
/// each then going ahead of those registered before; GTK reads the menu at the
/// application's startup. So this initialises GTK and looks the path up before
/// registering; call it after the environment GTK reads is set, before the application
/// runs ([`crate::launch::run`] does). GTK keeps the item in its menu as it rebuilds the
/// menu bar, its check mark following the action's state.
#[cfg(target_os = "macos")]
pub(crate) fn macos_app_menu() {
    const PATH: &str = "/org/gtk/libgtk/ui/gtkapplication-quartz.ui";
    static MENU: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/app-menu.gresource"));
    if let Err(e) = gtk::init() {
        tracing::warn!("app menu: {e}");
        return;
    }
    let lookup = || gio::resources_lookup_data(PATH, gio::ResourceLookupFlags::NONE);
    let _ = lookup();
    match gio::Resource::from_data(&glib::Bytes::from_static(MENU)) {
        Ok(resource) => gio::resources_register(&resource),
        Err(e) => tracing::warn!("app menu: {e}"),
    }
    let ours = lookup().is_ok_and(|menu| menu.windows(20).any(|w| w == b"update-automatically"));
    if !ours {
        tracing::warn!("app menu: GTK's own is still the one found, without Update Automatically");
    }
}

/// Windows: every app window shows the menu bar in a row of its own under the title bar
/// (`GtkApplicationWindow` doesn't unless asked), and a window without a title (GTK's
/// alerts) takes the app's name, which Windows shows in its title bar and taskbar (GDK
/// would show ".").
fn windows_conventions(app: &gtk::Application, kind: AppKind) {
    app.connect_window_added(|_, window| {
        if let Some(window) = window.downcast_ref::<gtk::ApplicationWindow>() {
            window.set_show_menubar(true);
        }
    });
    gtk::Window::toplevels().connect_items_changed(move |model, position, _removed, added| {
        for i in position..position + added {
            if let Some(window) = model.item(i).and_downcast::<gtk::Window>() {
                window.connect_realize(move |window| {
                    if window.title().is_none_or(|t| t.is_empty()) {
                        window.set_title(Some(kind.display_name()));
                    }
                });
            }
        }
    });
}

/// The menu bar of `menus`, each a submenu with its label; after them the Window menu on
/// macOS and the Help menu ([`help_menu`]) on Windows.
pub fn menu_bar(kind: AppKind, menus: &[(&str, &gio::Menu)]) -> gio::Menu {
    let bar = gio::Menu::new();
    for (label, menu) in menus {
        bar.append_submenu(Some(label), *menu);
    }
    if cfg!(target_os = "macos") {
        bar.append_item(&window_menu());
    } else if cfg!(windows) {
        bar.append_item(&help_menu(kind));
    }
    bar
}

/// macOS's Window menu: Minimize, Zoom, and the windows.
fn window_menu() -> gio::MenuItem {
    let menu = gio::Menu::new();
    menu.append(Some("Minimize"), Some("app.minimize"));
    menu.append(Some("Zoom"), Some("app.zoom"));
    let item = gio::MenuItem::new_submenu(Some("Window"), &menu);
    item.set_attribute_value(MACOS_SPECIAL, Some(&"window-submenu".to_variant()));
    item
}

/// The app's own items: "Update Automatically" (a check item, `app.update-automatically`'s
/// state) and "About `<App>`". The main menu's last section on Linux, Help on Windows (and
/// Etch's on Linux); on macOS GTK's app menu has them (`macos_app_menu`).
pub fn app_section(kind: AppKind) -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some(UPDATE_AUTOMATICALLY), Some("app.update-automatically"));
    menu.append(
        Some(&format!("About {}", kind.display_name())),
        Some("app.about"),
    );
    menu
}

/// The Help menu: [`app_section`]. Windows' menu bar ends with it, and so does Etch's on
/// Linux, whose windows show their menu bar.
pub fn help_menu(kind: AppKind) -> gio::MenuItem {
    gio::MenuItem::new_submenu(Some("Help"), &app_section(kind))
}

/// A Linux header bar's main menu: the button at its end (`open-menu-symbolic`, named
/// "Main Menu"; F10 opens it) showing `menu`, which ends with [`app_section`].
pub fn main_menu(menu: &impl IsA<gio::MenuModel>) -> gtk::MenuButton {
    let button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(menu)
        .tooltip_text("Main Menu")
        .primary(true)
        .build();
    button.update_property(&[gtk::accessible::Property::Label("Main Menu")]);
    button
}

/// The section File ends with on Windows: Exit, which runs `app.quit` (each app's closes
/// every window, asking about unsaved changes). `None` elsewhere: macOS has Quit in the
/// app menu, and Linux keeps each app's own.
pub fn exit_section() -> Option<gio::Menu> {
    cfg!(windows).then(|| {
        let menu = gio::Menu::new();
        menu.append(Some("Exit"), Some("app.quit"));
        menu
    })
}

/// What the About dialog says the app is.
pub fn summary(kind: AppKind) -> &'static str {
    match kind {
        AppKind::Brick => "A fast, lightweight spreadsheet: open, edit, sort, filter, and save huge CSV files and Excel workbooks.",
        AppKind::Slate => "A pageless word processor for Word documents, Markdown, and text.",
        AppKind::Frame => "Open, edit, show, and save PowerPoint presentations, with one inspector and an outline.",
        AppKind::Etch => "A fast, simple image editor: closer to Paint than to Photoshop.",
    }
}

/// The app's web page: `https://brick.cronkite.cc`.
pub fn website(kind: AppKind) -> String {
    format!("https://{}.cronkite.cc", kind.command())
}

/// The licence the About dialog shows (the workspace's `MIT OR Apache-2.0`).
pub fn licence(kind: AppKind) -> String {
    format!(
        "{} is free software, licensed under either the MIT license or the Apache License, \
         Version 2.0, at your option.",
        kind.display_name()
    )
}

/// The app's icon, as packaged for Linux (`packaging/<app id>.svg`).
fn icon_svg(kind: AppKind) -> &'static [u8] {
    match kind {
        AppKind::Brick => include_bytes!("../../../packaging/dev.cronkite.Brick.svg"),
        AppKind::Slate => include_bytes!("../../../packaging/dev.cronkite.Slate.svg"),
        AppKind::Frame => include_bytes!("../../../packaging/dev.cronkite.Frame.svg"),
        AppKind::Etch => include_bytes!("../../../packaging/dev.cronkite.Etch.svg"),
    }
}

/// The app's About dialog: its icon, name, version, what it is, its licence, and its web
/// page.
pub fn about(parent: Option<&gtk::Window>, kind: AppKind, version: &str) {
    let dialog = gtk::AboutDialog::builder()
        .program_name(kind.display_name())
        .version(version)
        .comments(summary(kind))
        .website(website(kind))
        .website_label(website(kind).trim_start_matches("https://"))
        .license_type(gtk::License::Custom)
        .license(licence(kind))
        .wrap_license(true)
        .modal(true)
        .build();
    // The icon theme's icon where it has the app's (installed on Linux), else the packaged
    // one decoded here (macOS has no icon theme with it); none if neither can be had.
    let themed = gtk::gdk::Display::default()
        .is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(kind.app_id()));
    if themed {
        dialog.set_logo_icon_name(Some(kind.app_id()));
    } else if let Ok(texture) =
        gtk::gdk::Texture::from_bytes(&glib::Bytes::from_static(icon_svg(kind)))
    {
        dialog.set_logo(Some(&texture));
    }
    dialog.set_transient_for(parent);
    dialog.present();
}

/// Where a window's buttons go: its header bar on Linux; on macOS and Windows a toolbar
/// row at the top of the window, under the system's title bar (and Windows' menu bar).
#[derive(Clone, Debug)]
pub enum Toolbar {
    Header(gtk::HeaderBar),
    Row(gtk::ActionBar),
}

impl Default for Toolbar {
    fn default() -> Self {
        Self::new()
    }
}

impl Toolbar {
    /// This platform's, empty.
    pub fn new() -> Self {
        if MENU_BAR {
            Toolbar::Row(gtk::ActionBar::new())
        } else {
            Toolbar::Header(gtk::HeaderBar::new())
        }
    }

    /// `widget` at the start (left).
    pub fn pack_start(&self, widget: &impl IsA<gtk::Widget>) {
        match self {
            Toolbar::Header(bar) => bar.pack_start(widget),
            Toolbar::Row(bar) => bar.pack_start(widget),
        }
    }

    /// `widget` at the end (right).
    pub fn pack_end(&self, widget: &impl IsA<gtk::Widget>) {
        match self {
            Toolbar::Header(bar) => bar.pack_end(widget),
            Toolbar::Row(bar) => bar.pack_end(widget),
        }
    }

    /// Put it on `window`: a header bar becomes the window's title bar. A row is returned,
    /// for the caller to put at the top of the window's content.
    pub fn put_on(&self, window: &impl IsA<gtk::Window>) -> Option<gtk::ActionBar> {
        match self {
            Toolbar::Header(bar) => {
                window.set_titlebar(Some(bar));
                None
            }
            Toolbar::Row(bar) => Some(bar.clone()),
        }
    }
}

/// The labels of `menu`'s items, submenus' and sections' items too, depth first, each with
/// its action and the accelerator it shows (its `accel` attribute): for the apps' tests of
/// their menus.
pub fn items(menu: &impl IsA<gio::MenuModel>) -> Vec<(String, Option<String>, Option<String>)> {
    let mut out = Vec::new();
    collect(menu.upcast_ref(), &mut out);
    out
}

fn collect(menu: &gio::MenuModel, out: &mut Vec<(String, Option<String>, Option<String>)>) {
    let text = |i: i32, name: &str| {
        menu.item_attribute_value(i, name, Some(glib::VariantTy::STRING))
            .and_then(|v| v.get::<String>())
    };
    for i in 0..menu.n_items() {
        if let Some(label) = text(i, "label") {
            out.push((label, text(i, "action"), text(i, "accel")));
        }
        for link in ["section", "submenu"] {
            if let Some(inner) = menu.item_link(i, link) {
                collect(&inner, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APPS: [AppKind; 4] = [
        AppKind::Brick,
        AppKind::Slate,
        AppKind::Frame,
        AppKind::Etch,
    ];

    #[test]
    fn the_menu_bar_has_the_menus_then_window_on_macos_and_help_on_windows() {
        let file = gio::Menu::new();
        file.append(Some("Open…"), Some("app.open"));
        let edit = gio::Menu::new();
        let bar = menu_bar(AppKind::Brick, &[("File", &file), ("Edit", &edit)]);
        let label = |i| {
            bar.item_attribute_value(i, "label", Some(glib::VariantTy::STRING))
                .and_then(|v| v.get::<String>())
        };
        assert_eq!(label(0).as_deref(), Some("File"));
        assert_eq!(label(1).as_deref(), Some("Edit"));
        let special = (bar.n_items() > 2)
            .then(|| bar.item_attribute_value(2, MACOS_SPECIAL, Some(glib::VariantTy::STRING)))
            .flatten()
            .and_then(|v| v.get::<String>());
        let item = |l: &str, a: Option<&str>| (l.to_owned(), a.map(str::to_owned), None);
        let mut want = vec![
            item("File", None),
            item("Open…", Some("app.open")),
            item("Edit", None),
        ];
        if cfg!(target_os = "macos") {
            assert_eq!(bar.n_items(), 3);
            assert_eq!(label(2).as_deref(), Some("Window"));
            assert_eq!(special.as_deref(), Some("window-submenu"));
            want.extend([
                item("Window", None),
                item("Minimize", Some("app.minimize")),
                item("Zoom", Some("app.zoom")),
            ]);
        } else if cfg!(windows) {
            assert_eq!(bar.n_items(), 3);
            assert_eq!(label(2).as_deref(), Some("Help"));
            assert_eq!(special, None);
            want.extend([
                item("Help", None),
                item("Update Automatically", Some("app.update-automatically")),
                item("About Brick", Some("app.about")),
            ]);
        } else {
            assert_eq!(bar.n_items(), 2, "Linux has no Window or Help menu");
        }
        assert_eq!(items(&bar), want);
    }

    #[test]
    fn update_automatically_sits_before_about() {
        let item = |l: &str, a: &str| (l.to_owned(), Some(a.to_owned()), None);
        assert_eq!(
            items(&app_section(AppKind::Etch)),
            [
                item("Update Automatically", "app.update-automatically"),
                item("About Etch", "app.about"),
            ]
        );
        let help = gio::Menu::new();
        help.append_item(&help_menu(AppKind::Slate));
        assert_eq!(items(&help)[0], ("Help".to_owned(), None, None));
        assert_eq!(items(&help)[2], item("About Slate", "app.about"));
    }

    /// The settings file the test's `keep` writes, as `suite_update::set_automatic` does.
    fn settings() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("suite-ui-update-{}", std::process::id()))
    }

    fn read_setting() -> bool {
        std::fs::read_to_string(settings()).map_or(true, |s| s.trim() != "automatic=false")
    }

    fn keep_setting(on: bool) -> std::io::Result<()> {
        std::fs::write(settings(), format!("automatic={on}\n"))
    }

    fn refuse(_: bool) -> std::io::Result<()> {
        Err(std::io::Error::other("read-only"))
    }

    #[test]
    fn update_automatically_toggles_keeps_each_change_and_starts_from_what_was_kept() {
        let _ = std::fs::remove_file(settings());
        let state = |a: &gio::SimpleAction| a.state().and_then(|s| s.get::<bool>());
        let update = || AutoUpdate {
            on: read_setting(),
            keep: keep_setting,
        };
        let action = update_action(update());
        assert_eq!(state(&action), Some(true), "on by default");
        action.activate(None);
        assert_eq!(state(&action), Some(false));
        assert_eq!(
            std::fs::read_to_string(settings()).unwrap(),
            "automatic=false\n"
        );
        // The next run's action starts off.
        let next = update_action(update());
        assert_eq!(state(&next), Some(false));
        next.activate(None);
        assert_eq!(state(&next), Some(true));
        assert!(read_setting());
        let _ = std::fs::remove_file(settings());
        // A change that can't be kept doesn't take.
        let stuck = update_action(AutoUpdate {
            on: true,
            keep: refuse,
        });
        stuck.activate(None);
        assert_eq!(state(&stuck), Some(true));
    }

    #[test]
    fn file_ends_with_exit_on_windows_only() {
        let exit = exit_section().map(|exit| items(&exit));
        let want =
            cfg!(windows).then(|| vec![("Exit".to_owned(), Some("app.quit".to_owned()), None)]);
        assert_eq!(exit, want);
    }

    #[test]
    fn windows_asks_for_the_systems_title_bars_unless_the_environment_chooses() {
        assert_eq!(csd(true, false), Some("0"));
        assert_eq!(csd(true, true), None);
        assert_eq!(csd(false, false), None);
        assert_eq!(csd(false, true), None);
    }

    #[test]
    fn each_app_says_what_it_is_and_where_it_lives() {
        for kind in APPS {
            assert!(!summary(kind).is_empty());
            assert!(licence(kind).starts_with(kind.display_name()));
        }
        assert_eq!(website(AppKind::Brick), "https://brick.cronkite.cc");
        assert_eq!(website(AppKind::Etch), "https://etch.cronkite.cc");
    }

    /// macOS and Windows have no icon theme with the apps' icons, so the About dialog
    /// decodes them (Homebrew's and MSYS2's gdk-pixbuf have an SVG loader; Ubuntu's runner
    /// image hasn't).
    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn each_apps_icon_decodes_for_the_about_dialog() {
        for kind in APPS {
            let texture = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_static(icon_svg(kind)))
                .unwrap_or_else(|e| panic!("{kind:?}: {e}"));
            assert_eq!((texture.width(), texture.height()), (128, 128), "{kind:?}");
        }
    }
}
