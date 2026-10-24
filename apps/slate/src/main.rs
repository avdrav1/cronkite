//! Slate: a fast, lightweight Linux-native word processor. SL0 is a read-only viewer
//! (docs/specs/slate.md, docs/adr/0003-slate-document-model.md).
//!
//! `slate FILE…` opens each file in its own window; without a file a start window offers
//! Open. Ctrl+O picks a file through the desktop's file dialog, and files dropped on any
//! window open too. A file is read off the UI thread while its window says "Opening…"; one
//! that can't be read gets an alert naming it and the reason, and its window closes.
//! Documents show as one pageless column (ADR 0005), drawn by Slate's own flow (ADR 0007),
//! whose width the menu's View → Text Width sets for every window at once, remembered in
//! the user's settings. Ctrl+S and Ctrl+Shift+S save (`saving`); closing a window or
//! quitting with changes not saved asks first.
//!
//! `slate FILE --bench-flow` measures the flow on FILE in a 1600x1000 window: re-wrapping
//! after width changes and the frames of scrolling (SLATE-11's acceptance test);
//! `--bench-typing` measures typing in it (SLATE-6's), `--bench-find` finding in it
//! (SLATE-16's). Each prints a JSON line.

// On Windows a GUI program: no console window opens with it (WIN-3).
#![cfg_attr(windows, windows_subsystem = "windows")]

mod bench;
mod find_bar;
mod fonts;
mod format_bar;
mod keys;
mod open;
mod recovery;
mod saving;
mod settings;
mod view;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use keys::Does;
use settings::{Saver, Settings, TextWidth};
use std::cell::RefCell;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use suite_core::AppKind;
use suite_file::canonical;
use suite_ui::keys::Primary;
use view::Command;

const APP: AppKind = AppKind::Slate;
const APP_ID: &str = APP.app_id();
const USAGE: &str =
    "usage: slate [FILE.txt|FILE.md|FILE.docx]... [--version] [--bench-flow|--bench-typing|--bench-find]";
const START_WINDOW: &str = "start";

/// The page's own font, for what it shows outside the document: Liberation Sans, else the
/// system's sans, at 11 pt. A document's text is drawn in its own fonts, from its style
/// sheet (ADR 0008; Markdown and plain text in Slate's built-in Liberation Sans 11 pt). A
/// page break's dashed line has room above and below it (`flow::PAGE_BREAK_HEIGHT` in all).
const CSS: &str = "
slatepage { font-family: \"Liberation Sans\", sans-serif; font-size: 11pt; }
separator.page-break {
  min-height: 0;
  background: none;
  border-top: 2px dashed #9a9996;
  margin: 18px 0;
}
";

thread_local! {
    /// The windows showing a file, by the file's canonical path.
    static OPEN: RefCell<Vec<(PathBuf, glib::WeakRef<gtk::ApplicationWindow>)>> =
        const { RefCell::new(Vec::new()) };
    /// The pages shown, which follow the text width setting.
    static PAGES: RefCell<Vec<glib::WeakRef<view::Page>>> = const { RefCell::new(Vec::new()) };
    static SAVER: RefCell<Saver> = RefCell::new(Saver::default());
    /// Watches the settings file for changes made by other Slate processes.
    static SETTINGS_MONITOR: RefCell<Option<gio::FileMonitor>> = const { RefCell::new(None) };
    /// The menus, built on first use.
    static MENUS: Menus = Menus::new();
}

fn main() -> glib::ExitCode {
    // Before anything prints: on Windows the terminal Slate was started from (WIN-3).
    suite_platform::console::attach();
    let (mut files, mut bench) = (Vec::new(), None);
    for arg in std::env::args_os().skip(1) {
        match arg.to_str() {
            Some("--version") => {
                println!("slate {}", env!("CARGO_PKG_VERSION"));
                return glib::ExitCode::SUCCESS;
            }
            Some("--help" | "-h") => {
                println!("{USAGE}");
                return glib::ExitCode::SUCCESS;
            }
            Some("--bench-flow") => bench = Some(bench::Mode::Flow),
            Some("--bench-typing") => bench = Some(bench::Mode::Typing),
            Some("--bench-find") => bench = Some(bench::Mode::Find),
            Some(a) if a.starts_with('-') => {
                eprintln!("slate: unknown option {a}\n{USAGE}");
                return glib::ExitCode::from(2);
            }
            _ => files.push(PathBuf::from(arg)),
        }
    }
    if bench.is_some() && files.len() != 1 {
        eprintln!("slate: a benchmark needs one file\n{USAGE}");
        return glib::ExitCode::from(2);
    }
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    tracing::info!("application starting");
    if let Err(e) = suite_platform::current() {
        eprintln!("slate: {e}");
        return glib::ExitCode::FAILURE;
    }
    eprintln!("{}", suite_ui::startup_message(APP));
    // This run's part in applying updates (ADR 0012); before the bundle's environment.
    let updates = suite_update::launch(APP, env!("CARGO_PKG_VERSION"));
    // "Updated to Slate X.Y.Z" in the first window, at the first start after an update.
    suite_ui::notice::queue(suite_update::notice(APP, env!("CARGO_PKG_VERSION")));
    // In a macOS app bundle, GTK and its libraries find their files there (MAC-7).
    match suite_platform::bundle::init(APP) {
        Ok(Some(res)) => tracing::info!("running from the bundle {}", res.display()),
        Ok(None) => {}
        Err(e) => tracing::warn!("bundle: {e}"),
    }
    suite_ui::renderer::init();
    suite_ui::chrome::init();
    suite_ui::fonts::init();

    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE | gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    // macOS and Windows: the system's light or dark mode (GTK doesn't follow the Mac's).
    #[cfg(any(target_os = "macos", windows))]
    app.connect_startup(|_| suite_platform::appearance::follow(suite_ui::appearance::set_dark));
    // Check for an update once the first window shows (ADR 0012).
    app.connect_window_added(|_, window| {
        window.connect_map(|_| suite_update::start(APP, env!("CARGO_PKG_VERSION")));
    });
    app.connect_startup(|app| {
        let css = gtk::CssProvider::new();
        css.load_from_string(CSS);
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let open = gio::ActionEntry::builder("open")
            .activate(|app: &gtk::Application, _, _| choose_and_open(app))
            .build();
        let new = gio::ActionEntry::builder("new")
            .activate(|app: &gtk::Application, _, _| open_window(app, None, None, None))
            .build();
        let close = gio::ActionEntry::builder("close")
            .activate(|app: &gtk::Application, _, _| {
                if let Some(w) = app.active_window() {
                    w.close();
                }
            })
            .build();
        // Quit closes each window, so a window with changes asks first.
        let quit = gio::ActionEntry::builder("quit")
            .activate(|app: &gtk::Application, _, _| {
                for window in app.windows() {
                    window.close();
                }
            })
            .build();
        // View → Text Width: a radio choice for every window at once, remembered.
        let text_width = gio::ActionEntry::builder("text-width")
            .parameter_type(Some(glib::VariantTy::STRING))
            .state(Settings::load().text_width.id().to_variant())
            .change_state(|_: &gtk::Application, action, value| {
                let Some(width) = value.and_then(|v| v.str()).and_then(TextWidth::from_id) else {
                    return;
                };
                if apply_text_width(action, width) {
                    SAVER.with_borrow_mut(|s| s.save(Settings { text_width: width }));
                }
            })
            .build();
        app.add_action_entries([new, open, close, quit, text_width]);
        let update = suite_ui::chrome::AutoUpdate {
            on: suite_update::automatic(APP),
            keep: |on| suite_update::set_automatic(APP, on),
        };
        suite_ui::chrome::install(app, APP, env!("CARGO_PKG_VERSION"), update);
        if suite_ui::chrome::MENU_BAR {
            app.set_menubar(Some(&MENUS.with(Menus::bar)));
        }
        for (action, accels) in keys::accels(Primary::HERE) {
            app.set_accels_for_action(
                action,
                &accels.iter().map(String::as_str).collect::<Vec<_>>(),
            );
        }
        watch_settings(app);
        // Package copies of Word documents left by killed Slate processes.
        std::thread::spawn(saving::Original::sweep);
    });
    // Changes still being written finish before Slate exits.
    app.connect_shutdown(|_| SAVER.with_borrow_mut(Saver::finish));
    // Files, from the command line or the system (Finder), each open in their own window;
    // a benchmark runs on the first (`suite_ui::launch`).
    let bench = std::cell::Cell::new(bench);
    app.connect_open(move |app, files, _| {
        let parent = app.active_window();
        for file in files {
            match (bench.take(), file.path()) {
                (Some(bench), Some(path)) => open_window(app, Some(&path), Some(bench), None),
                _ => open_file(app, file, parent.as_ref()),
            }
        }
    });
    app.connect_activate(|app| {
        suite_ui::launch::no_file(app, suite_platform::launch::when_finished, start_window)
    });
    let code = suite_ui::launch::run(&app, &files);
    updates.exit();
    code
}

/// The window already showing `path`, if any.
fn window_for(path: &Path) -> Option<gtk::ApplicationWindow> {
    OPEN.with_borrow_mut(|open| {
        open.retain(|(_, w)| w.upgrade().is_some());
        open.iter()
            .find(|(p, _)| p == path)
            .and_then(|(_, w)| w.upgrade())
    })
}

/// `window` shows the file at `path` now (Save As, or a new document's first save).
fn moved(window: &gtk::ApplicationWindow, path: &Path) {
    OPEN.with_borrow_mut(|open| {
        match open
            .iter_mut()
            .find(|(_, w)| w.upgrade().as_ref() == Some(window))
        {
            Some((p, _)) => *p = path.to_owned(),
            None => open.push((path.to_owned(), window.downgrade())),
        }
    });
}

/// Slate's menus (MAC-4), built once for every window: File, Edit, Format, and View. Linux
/// shows them in the header bar's menu (`primary`, with Quit), macOS and Windows in the
/// menu bar (Windows' File ending with Exit).
struct Menus {
    file: gio::Menu,
    edit: gio::Menu,
    format: gio::Menu,
    view: gio::Menu,
    primary: gio::Menu,
}

impl Menus {
    fn new() -> Self {
        let file = gio::Menu::new();
        file.append_item(&item("New", "app.new", Does::Action("app.new")));
        file.append(Some("Open…"), Some("app.open"));
        file.append_item(&item("Save", "win.save", Does::Action("win.save")));
        file.append_item(&item(
            "Save As…",
            "win.save-as",
            Does::Action("win.save-as"),
        ));
        file.append(Some("Close Window"), Some("app.close"));
        if let Some(exit) = suite_ui::chrome::exit_section() {
            file.append_section(None, &exit);
        }
        // View → Text Width: a radio item per width, checked by `app.text-width`'s state.
        let widths = gio::Menu::new();
        for width in TextWidth::ALL {
            let item = gio::MenuItem::new(Some(width.label()), None);
            item.set_action_and_target_value(
                Some("app.text-width"),
                Some(&width.id().to_variant()),
            );
            widths.append_item(&item);
        }
        let view = gio::Menu::new();
        view.append_submenu(Some("Text Width"), &widths);
        let (edit, format) = (edit_menu(), format_menu());
        // The header bar's menu (Linux): every menu a section, then Update Automatically
        // and About Slate, then Quit.
        let quit = gio::Menu::new();
        quit.append(Some("Quit"), Some("app.quit"));
        let primary = gio::Menu::new();
        primary.append_section(None, &file);
        primary.append_section(Some("View"), &view);
        primary.append_section(Some("Edit"), &edit);
        primary.append_section(Some("Format"), &format);
        primary.append_section(None, &suite_ui::chrome::app_section(APP));
        primary.append_section(None, &quit);
        Menus {
            file,
            edit,
            format,
            view,
            primary,
        }
    }

    /// The menu bar (macOS, Windows): File, Edit, Format, View, and the Window menu (macOS,
    /// whose app menu has Quit) or the Help menu (Windows).
    fn bar(&self) -> gio::Menu {
        suite_ui::chrome::menu_bar(
            APP,
            &[
                ("File", &self.file),
                ("Edit", &self.edit),
                ("Format", &self.format),
                ("View", &self.view),
            ],
        )
    }
}

/// A window's header bar (Linux; macOS and Windows windows keep their own title bar):
/// Open, the title, and the menu.
fn header(title: &str) -> gtk::HeaderBar {
    let header = gtk::HeaderBar::new();
    let open = gtk::Button::from_icon_name("document-open-symbolic");
    open.set_tooltip_text(Some(&keys::tip("Open…", Does::Action("app.open"))));
    open.set_action_name(Some("app.open"));
    open.update_property(&[gtk::accessible::Property::Label("Open")]);
    header.pack_start(&open);
    // The primary menu: F10 opens it.
    header.pack_end(&suite_ui::chrome::main_menu(
        &MENUS.with(|m| m.primary.clone()),
    ));
    let label = gtk::Label::new(Some(title));
    label.add_css_class("title");
    header.set_title_widget(Some(&label));
    header
}

/// `window`'s chrome: the header bar titled `title` on Linux; nothing on macOS and
/// Windows, whose title bar shows the window's title and whose menu bar has the menus.
fn put_header(window: &gtk::ApplicationWindow, title: &str) {
    if !suite_ui::chrome::MENU_BAR {
        window.set_titlebar(Some(&header(title)));
    }
}

/// A menu item for `action` with the key that does `does` shown beside it (the page
/// handles most keys: `keys`).
fn item(label: &str, action: &str, does: Does) -> gio::MenuItem {
    let item = gio::MenuItem::new(Some(label), Some(action));
    if let Some(accel) = keys::accel(does) {
        item.set_attribute_value("accel", Some(&accel.to_variant()));
    }
    item
}

/// A menu item for `action`, the page's `command`, with its key shown beside it.
fn command(label: &str, action: &str, command: Command) -> gio::MenuItem {
    item(label, action, Does::Command(command))
}

fn edit_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append_item(&command("Undo", "win.undo", Command::Undo));
    menu.append_item(&command("Redo", "win.redo", Command::Redo));
    menu.append_item(&command("Cut", "win.cut", Command::Cut));
    menu.append_item(&command("Copy", "win.copy", Command::Copy));
    menu.append_item(&command("Paste", "win.paste", Command::Paste));
    menu.append_item(&command("Select All", "win.select-all", Command::SelectAll));
    menu.append_item(&item("Find…", "win.find", Does::Action("win.find")));
    menu.append_item(&item(
        "Replace…",
        "win.replace",
        Does::Action("win.replace"),
    ));
    menu
}

/// Format: bold, italic, underline, strikethrough, alignment, the paragraph's style, and
/// lists; the window's page does them.
fn format_menu() -> gio::Menu {
    use document_model::Align;
    let menu = gio::Menu::new();
    menu.append_item(&command("Bold", "win.bold", Command::Bold));
    menu.append_item(&command("Italic", "win.italic", Command::Italic));
    menu.append_item(&command("Underline", "win.underline", Command::Underline));
    menu.append(Some("Strikethrough"), Some("win.strike"));
    let aligns = gio::Menu::new();
    aligns.append_item(&command(
        "Align Left",
        "win.align-left",
        Command::Align(Align::Start),
    ));
    aligns.append_item(&command(
        "Centre",
        "win.align-center",
        Command::Align(Align::Center),
    ));
    aligns.append_item(&command(
        "Align Right",
        "win.align-right",
        Command::Align(Align::End),
    ));
    aligns.append_item(&command(
        "Justify",
        "win.justify",
        Command::Align(Align::Justify),
    ));
    menu.append_submenu(Some("Alignment"), &aligns);
    let styles = gio::Menu::new();
    for level in 0..=6u8 {
        let label = if level == 0 {
            "Normal Text".to_owned()
        } else {
            format!("Heading {level}")
        };
        let item = command(&label, "win.style", Command::Style(level));
        item.set_action_and_target_value(Some("win.style"), Some(&level.to_variant()));
        styles.append_item(&item);
    }
    menu.append_submenu(Some("Paragraph Style"), &styles);
    menu.append_item(&command("Bulleted List", "win.bullets", Command::Bullets));
    menu.append_item(&command("Numbered List", "win.numbers", Command::Numbers));
    menu
}

/// The window's Edit and Format actions, done by `page`.
fn add_page_actions(window: &gtk::ApplicationWindow, page: &view::Page) {
    use document_model::Align;
    let simple = |name: &str, command: Command| {
        let page = page.downgrade();
        gio::ActionEntry::builder(name)
            .activate(move |_: &gtk::ApplicationWindow, _, _| {
                if let Some(page) = page.upgrade() {
                    page.command(command);
                }
            })
            .build()
    };
    let style = {
        let page = page.downgrade();
        gio::ActionEntry::builder("style")
            .parameter_type(Some(glib::VariantTy::BYTE))
            .activate(move |_: &gtk::ApplicationWindow, _, level| {
                if let (Some(page), Some(level)) =
                    (page.upgrade(), level.and_then(|v| v.get::<u8>()))
                {
                    page.command(Command::Style(level));
                }
            })
            .build()
    };
    window.add_action_entries([
        simple("undo", Command::Undo),
        simple("redo", Command::Redo),
        simple("bold", Command::Bold),
        simple("italic", Command::Italic),
        simple("underline", Command::Underline),
        simple("strike", Command::Strike),
        simple("align-left", Command::Align(Align::Start)),
        simple("align-center", Command::Align(Align::Center)),
        simple("align-right", Command::Align(Align::End)),
        simple("justify", Command::Align(Align::Justify)),
        simple("bullets", Command::Bullets),
        simple("numbers", Command::Numbers),
        simple("cut", Command::Cut),
        simple("copy", Command::Copy),
        simple("paste", Command::Paste),
        simple("select-all", Command::SelectAll),
        style,
    ]);
}

/// The window shown without a file: what Slate opens, and Open.
fn start_window(app: &gtk::Application) {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Slate")
        .default_width(720)
        .default_height(520)
        .name(START_WINDOW)
        .build();
    put_header(&window, "Slate");
    let heading = gtk::Label::new(Some("Open a document"));
    heading.add_css_class("title-2");
    let hint = gtk::Label::new(Some(
        "Slate shows .txt, .md, and .docx files. Drop one here, choose Open, or start a new \
         one.",
    ));
    hint.add_css_class("dim-label");
    let open = gtk::Button::with_label("Open…");
    open.set_action_name(Some("app.open"));
    open.add_css_class("suggested-action");
    let new = gtk::Button::with_label("New Document");
    new.set_action_name(Some("app.new"));
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    buttons.set_halign(gtk::Align::Center);
    buttons.append(&open);
    buttons.append(&new);
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .build();
    content.append(&heading);
    content.append(&hint);
    content.append(&buttons);
    // After a crash: the unsaved edits it left, each to restore or discard.
    let restore = recovery::start_section(app).map(|(section, first)| {
        content.append(&section);
        first
    });
    window.set_child(Some(&suite_ui::notice::host(&content)));
    accept_drops(app, &window);
    window.present();
    if let Some(first) = restore {
        first.grab_focus();
    }
}

/// Close the start window once a document shows, unless it still lists unsaved edits
/// from a crash to restore.
fn close_start_window(app: &gtk::Application) {
    if recovery::offers_left() {
        return;
    }
    for w in app.windows() {
        if w.widget_name() == START_WINDOW {
            w.close();
        }
    }
}

/// File → Open: the desktop's file dialog (through the portal when one runs), filtered to
/// the documents Slate reads.
fn choose_and_open(app: &gtk::Application) {
    let documents = gtk::FileFilter::new();
    documents.set_name(Some("Documents (.txt, .md, .docx)"));
    for mime in [
        "text/plain",
        "text/markdown",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ] {
        documents.add_mime_type(mime);
    }
    for suffix in ["txt", "text", "md", "markdown", "docx"] {
        documents.add_suffix(suffix);
    }
    let all = gtk::FileFilter::new();
    all.set_name(Some("All files"));
    all.add_pattern("*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&documents);
    filters.append(&all);
    let dialog = gtk::FileDialog::builder()
        .title("Open")
        .modal(true)
        .filters(&filters)
        .default_filter(&documents)
        .build();
    let (app, parent) = (app.clone(), app.active_window());
    glib::spawn_future_local(async move {
        match dialog.open_future(parent.as_ref()).await {
            Ok(file) => open_file(&app, &file, parent.as_ref()),
            Err(e) if e.matches(gtk::DialogError::Dismissed) => {}
            Err(e) => alert(
                parent.as_ref(),
                "Cannot show the file dialog",
                &e.to_string(),
            ),
        }
    });
}

fn open_file(app: &gtk::Application, file: &gio::File, parent: Option<&gtk::Window>) {
    match file.path() {
        Some(path) => open_window(app, Some(&path), None, None),
        None => alert(
            parent,
            &format!("Cannot open {}", file.uri()),
            "Only local files can be opened.",
        ),
    }
}

/// Files dropped on `window` open as Open does, each in its own window. They open once the
/// drop is done: opening one may close the start window, which may be the drop's target.
fn accept_drops(app: &gtk::Application, window: &gtk::ApplicationWindow) {
    let target = suite_ui::launch::file_drop_target();
    target.connect_drop({
        let (app, window) = (app.clone(), window.downgrade());
        move |_, value, _, _| {
            let Ok(files) = value.get::<gdk::FileList>() else {
                return false;
            };
            let files = files.files();
            if files.is_empty() {
                return false;
            }
            let (app, window) = (app.clone(), window.clone());
            glib::idle_add_local_once(move || {
                let parent = window.upgrade().map(|w| w.upcast::<gtk::Window>());
                for file in files {
                    open_file(&app, &file, parent.as_ref());
                }
            });
            true
        }
    });
    window.add_controller(target);
}

fn alert(parent: Option<&gtk::Window>, message: &str, detail: &str) {
    gtk::AlertDialog::builder()
        .message(message)
        .detail(detail)
        .modal(true)
        .build()
        .show(parent);
}

/// Show `path` in a window: its own if it is open, else a new one that says "Opening…"
/// while the file is read in the background; with no path, a new window with a new
/// document, titled Untitled (SLATE-18). `bench`: run that benchmark on it. `restore`: put
/// back the unsaved edits of that crashed session (SLATE-17) without asking; otherwise a
/// crashed session's edits to the file are offered back.
fn open_window(
    app: &gtk::Application,
    path: Option<&Path>,
    bench: Option<bench::Mode>,
    restore: Option<recovery::Crashed>,
) {
    let path = path.map(canonical);
    if let Some(window) = path.as_deref().and_then(window_for) {
        window.present();
        return;
    }
    let name = path.as_deref().map_or_else(
        || saving::UNTITLED.to_owned(),
        |p| suite_file::document_name(p).into_owned(),
    );
    let (width, height) = if bench.is_some() {
        bench::WINDOW
    } else {
        (900, 860)
    };
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(&name)
        .default_width(width)
        .default_height(height)
        .build();
    put_header(&window, &name);

    let spinner = gtk::Spinner::builder().spinning(true).build();
    let opening = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .build();
    opening.append(&spinner);
    opening.append(&gtk::Label::new(Some("Opening…")));
    let stack = gtk::Stack::builder().vexpand(true).build();
    stack.add_named(&opening, Some("opening"));

    let status = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .margin_start(12)
        .margin_end(12)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    status.add_css_class("caption");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&stack);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&status);
    window.set_child(Some(&suite_ui::notice::host(&content)));
    accept_drops(app, &window);
    if let Some(path) = &path {
        OPEN.with_borrow_mut(|open| open.push((path.clone(), window.downgrade())));
    }
    window.present();

    let restoring = restore.is_some();
    let reading = gio::spawn_blocking({
        let path = path.clone();
        move || {
            let mut opened = match &path {
                Some(path) => open::read(path),
                None => open::blank(),
            }?;
            let crashed = match (restore, &path) {
                (Some(c), _) => Some((c, false)),
                (None, Some(path)) if bench.is_none() => {
                    recovery::crashed_for(path).map(|c| (c, true))
                }
                _ => None,
            };
            let recovered =
                crashed.and_then(|(c, ask)| recovery::Recovered::read(c, ask, &mut opened));
            Ok::<_, open::OpenError>((opened, recovered))
        }
    });
    let app = app.clone();
    glib::spawn_future_local(async move {
        let reason = match reading.await {
            Ok(Ok((opened, recovered))) => {
                let page = show(&app, &stack, &status, path.as_deref(), &name, opened, bench);
                if let Some(recovered) = recovered {
                    recovered.offer(&window, &page);
                }
                close_start_window(&app);
                return;
            }
            Ok(Err(e)) => e.to_string(),
            Err(_) => "Reading it failed unexpectedly.".to_owned(),
        };
        tracing::warn!("cannot open {name}: {reason}");
        if bench.is_some() {
            std::process::exit(1);
        }
        let detail = if restoring {
            format!("{reason} The unsaved edits are kept for later.")
        } else {
            reason
        };
        let dialog = gtk::AlertDialog::builder()
            .message(format!("Cannot open “{name}”"))
            .detail(&detail)
            .buttons(["Close"])
            .default_button(0)
            .cancel_button(0)
            .modal(true)
            .build();
        let _ = dialog.choose_future(Some(&window)).await;
        window.close();
    });
}

/// Make `width` `app.text-width`'s state and show every document at it; false if it
/// already was.
fn apply_text_width(action: &gio::SimpleAction, width: TextWidth) -> bool {
    let state = width.id().to_variant();
    if action.state().as_ref() == Some(&state) {
        return false;
    }
    action.set_state(&state);
    PAGES.with_borrow_mut(|pages| {
        pages.retain(|p| p.upgrade().is_some());
        for page in pages.iter().filter_map(glib::WeakRef::upgrade) {
            page.set_text_width(width);
        }
    });
    true
}

/// Follow text width changes another Slate process saves (each `slate` command is one), so
/// every window follows the setting. The file is read off the UI thread.
fn watch_settings(app: &gtk::Application) {
    let path = settings::path();
    // A file in a directory that doesn't exist yet is only polled for; make the directory.
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let monitor = match gio::File::for_path(&path)
        .monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
    {
        Ok(monitor) => monitor,
        Err(e) => {
            tracing::warn!("cannot watch {}: {e}", path.display());
            return;
        }
    };
    let app = app.downgrade();
    monitor.connect_changed(move |_, _, _, event| {
        use gio::FileMonitorEvent as E;
        if !matches!(
            event,
            E::ChangesDoneHint | E::Created | E::MovedIn | E::Renamed
        ) {
            return;
        }
        let app = app.clone();
        glib::spawn_future_local(async move {
            let Ok(settings) = gio::spawn_blocking(Settings::load).await else {
                return;
            };
            let action = app
                .upgrade()
                .and_then(|app| app.lookup_action("text-width"))
                .and_downcast::<gio::SimpleAction>();
            if let Some(action) = action {
                apply_text_width(&action, settings.text_width);
            }
        });
    });
    SETTINGS_MONITOR.set(Some(monitor));
}

/// The text width setting: `app.text-width`'s state.
fn text_width(app: &gtk::Application) -> TextWidth {
    app.action_state("text-width")
        .and_then(|v| v.str().and_then(TextWidth::from_id))
        .unwrap_or_default()
}

/// Replace "Opening…" with the document of `path` (`None`: a new document), shown as
/// `name`; its page. `bench`: run that benchmark on it.
fn show(
    app: &gtk::Application,
    stack: &gtk::Stack,
    status: &gtk::Label,
    path: Option<&Path>,
    name: &str,
    opened: open::Opened,
    bench: Option<bench::Mode>,
) -> view::Page {
    let words = opened.words;
    status.set_text(&format!(
        "{} · {} word{}",
        opened.doc.format.name(),
        group_digits(words),
        if words == 1 { "" } else { "s" }
    ));
    let setting = if bench.is_some() {
        TextWidth::Full
    } else {
        text_width(app)
    };
    let plain = matches!(opened.doc.format, document_model::Format::Text(_));
    let page = view::Page::new(opened.doc, opened.textures, &opened.fonts, setting);
    PAGES.with_borrow_mut(|pages| pages.push(page.downgrade()));
    page.update_property(&[gtk::accessible::Property::Label(name)]);
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&page)
        .build();
    // The format bar over the page, for documents with formatting (SLATE-21).
    let document = gtk::Box::new(gtk::Orientation::Vertical, 0);
    if !plain {
        let bar = format_bar::FormatBar::new(&page);
        // A window narrower than the bar scrolls it rather than growing to fit it.
        let holder = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .child(bar.root())
            .build();
        document.append(&holder);
        document.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        // The bar lives as long as its widgets.
        let root = bar.root().clone();
        root.connect_destroy(move |_| {
            let _ = &bar;
        });
    }
    document.append(&scrolled);
    stack.add_named(&document, Some("document"));
    stack.set_visible_child_name("document");
    if let Some(window) = stack.root().and_downcast::<gtk::ApplicationWindow>() {
        add_page_actions(&window, &page);
        if let Some(content) = stack.parent().and_downcast::<gtk::Box>() {
            find_bar::attach(&window, &content, &page);
        }
        saving::Saver::attach(
            &window,
            &page,
            status,
            path.map(Path::to_owned),
            opened.target,
            opened.identity,
        );
    }
    page.grab_focus();
    if let Some(mode) = bench {
        bench::run(app, &page, words, mode);
    }
    page
}

/// `1234567` as `1,234,567`.
fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
