//! Application shell: windows, menus, dialogs, OS integration. Toolkit: GTK4 (ADR 0001).
//!
//! Usage: `brick [FILE.csv] [--version] [--bench-scroll FRAMES | --bench-jump ROW [--jumps N] | --bench-save EDITS | --bench-find TEXT]`
//!
//! Without a file it shows a start window; Ctrl+O or an Open button picks one through the
//! desktop's file dialog (xdg-desktop-portal via `gtk::FileDialog`, APP-1), and Ctrl+N or
//! New starts an untitled table (SAVE-2). Each file gets its own window. Ctrl+Shift+S
//! saves as another file, delimiter, or encoding.
//!
//! Benchmark modes print JSON and exit; the acceptance tests run them.
//! - `--bench-scroll` (GRID-1) waits for indexing, jumps to the middle of the file, scrolls
//!   37 px per frame (a fast flick) for FRAMES frames, and reports frame times and memory.
//! - `--bench-jump` (GRID-2) moves the vertical adjustment the way a scrollbar drag does:
//!   first to ROW, then to N-1 random rows, and reports how long each took to paint.
//! - `--bench-status` (APP-2) prints the status bar's row text (`STATUS …`) on every status
//!   update while the file indexes, and quits once indexing is complete.
//! - `--bench-save` (SAVE-3) pastes EDITS cells, saves, scrolls while the save runs, and
//!   reports the save time, progress updates, main-loop lateness, and frame times. It only
//!   saves files under the temp directory.
//! - `--bench-find TEXT` (SRCH-2) types TEXT into the find bar a character at a time and
//!   prints each keystroke, search start, and result (`FIND …`).

// On Windows a GUI program: no console window opens with it (WIN-3).
#![cfg_attr(windows, windows_subsystem = "windows")]

mod crash;
mod editor;
mod format_bar;
mod formula_bar;
mod grid_view;
mod keys;
mod looks;
mod malformed;
mod menus;
mod recovery;
mod sheets;
mod state;
mod status;
#[cfg(not(windows))]
mod theme;

use csv_engine::{Charset, Encoding, IndexError};
use data_model::{
    CsvTable, DelimiterChoice, Origin, RereadError, SaveProgress, SaveStats, Step, StylePatch,
    Workbook,
};
use grid_view::{GridView, ROW_H};
use gtk::{glib, prelude::*};
use status::FindNote;
use std::cell::{Cell, RefCell};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use suite_core::AppKind;
use suite_file::{canonical, recent};
use suite_ui::chrome::Toolbar;
use suite_ui::keys::Primary;
use xlsx::{ImportError, Imported};

const APP_ID: &str = APP.app_id();
const APP: AppKind = AppKind::Brick;

#[derive(Clone)]
enum Bench {
    Scroll { frames: u32 },
    Jump { row: u64, jumps: u32 },
    Open,
    Status,
    Save { edits: u32 },
    Find { text: String },
}

struct Args {
    file: Option<PathBuf>,
    bench: Option<Bench>,
}

fn parse_args() -> Result<Args, String> {
    let mut file = None;
    let mut bench = None;
    let mut jumps = 20;
    let mut it = std::env::args().skip(1);
    let number = |flag: &str, it: &mut dyn Iterator<Item = String>| -> Result<u64, String> {
        let v = it.next().ok_or(format!("{flag} needs a number"))?;
        v.parse()
            .map_err(|_| format!("bad number for {flag}: {v:?}"))
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "--bench-scroll" => {
                bench = Some(Bench::Scroll {
                    frames: number(&a, &mut it)? as u32,
                })
            }
            "--bench-jump" => {
                bench = Some(Bench::Jump {
                    row: number(&a, &mut it)?,
                    jumps: 0,
                })
            }
            "--jumps" => jumps = number(&a, &mut it)?.max(1) as u32,
            // For the installer and release checks (PKG-1, PKG-3).
            "--version" => {
                println!("brick {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--bench-open" => bench = Some(Bench::Open),
            "--bench-status" => bench = Some(Bench::Status),
            "--bench-find" => {
                let text = it.next().ok_or("--bench-find needs text")?;
                bench = Some(Bench::Find { text })
            }
            "--bench-save" => {
                bench = Some(Bench::Save {
                    edits: number(&a, &mut it)?.max(1) as u32,
                })
            }
            _ if file.is_none() && !a.starts_with("--") => file = Some(PathBuf::from(a)),
            _ => return Err(format!("unexpected argument {a:?}")),
        }
    }
    if let Some(Bench::Jump { jumps: j, .. }) = &mut bench {
        *j = jumps;
    }
    if bench.is_some() && file.is_none() {
        return Err("benchmarks need a file".into());
    }
    Ok(Args { file, bench })
}

fn main() -> glib::ExitCode {
    // Before anything prints: on Windows the terminal Brick was started from (WIN-3).
    suite_platform::console::attach();
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("brick: {e}\nusage: brick [FILE.csv] [--version] [--bench-scroll FRAMES | --bench-jump ROW [--jumps N] | --bench-save EDITS | --bench-find TEXT]");
            return glib::ExitCode::from(2);
        }
    };
    // Logs and the startup line go to stderr: stdout carries the benchmark modes' reports.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    tracing::info!("application starting");
    eprintln!("{}", suite_ui::startup_message(APP));
    // This run's part in applying updates (ADR 0012); before the bundle's environment.
    let updates = suite_update::launch(APP, env!("CARGO_PKG_VERSION"));
    // "Updated to Brick X.Y.Z" in the first window, at the first start after an update.
    suite_ui::notice::queue(suite_update::notice(APP, env!("CARGO_PKG_VERSION")));
    // Before the file's indexing threads start: these may set the environment (macOS).
    // In a macOS app bundle, GTK and its libraries find their files there (MAC-7).
    match suite_platform::bundle::init(APP) {
        Ok(Some(res)) => tracing::info!("running from the bundle {}", res.display()),
        Ok(None) => {}
        Err(e) => tracing::warn!("bundle: {e}"),
    }
    suite_ui::renderer::init();
    suite_ui::chrome::init();
    suite_ui::fonts::init();
    // The save benchmark overwrites its file: only ever a scratch copy.
    if let (Some(Bench::Save { .. }), Some(file)) = (&args.bench, &args.file) {
        let canonical = |p: &Path| p.canonicalize().ok();
        let scratch = canonical(file)
            .zip(canonical(&std::env::temp_dir()))
            .is_some_and(|(f, tmp)| f.starts_with(tmp));
        if !scratch {
            eprintln!("brick: --bench-save only saves files under the temp directory");
            return glib::ExitCode::from(2);
        }
    }
    // A file named on the command line is mapped and indexing before the toolkit starts,
    // so its first rows show as early as possible (BENCH-1 times this path). One that
    // can't be opened is said so on the start window (a file manager's launch has no
    // terminal to print to); benchmarks just fail. The file then reaches `open`, as the
    // system's files do (`suite_ui::launch`).
    let mut failed = None;
    let opened = match &args.file {
        Some(path) => match open_session(path) {
            Ok(opened) => {
                // Benchmarks and tests open corpus files: they stay off the list.
                if args.bench.is_none() {
                    remember(path);
                }
                Some(opened)
            }
            Err(e) => {
                eprintln!("brick: cannot open {}: {e}", path.display());
                if args.bench.is_some() {
                    return glib::ExitCode::FAILURE;
                }
                failed = Some((format!("Cannot open {}", path.display()), e.to_string()));
                None
            }
        },
        None => None,
    };

    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE | gtk::gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    // macOS and Windows: the system's light or dark mode (GTK doesn't follow the Mac's).
    #[cfg(any(target_os = "macos", windows))]
    app.connect_startup(|_| suite_platform::appearance::follow(suite_ui::appearance::set_dark));
    // Check for an update once the first window shows (ADR 0012).
    app.connect_window_added(|_, window| {
        window.connect_map(|_| suite_update::start(APP, env!("CARGO_PKG_VERSION")));
    });
    app.connect_startup(|app| {
        // The settings portal (Linux; on macOS it finds none). Windows has no portal, and
        // asking would start a D-Bus session bus (DEC-11 R13).
        #[cfg(not(windows))]
        theme::follow_color_scheme();
        let open = gtk::gio::ActionEntry::builder("open")
            .activate(|app: &gtk::Application, _, _| choose_and_open(app))
            .build();
        let new = gtk::gio::ActionEntry::builder("new")
            .activate(|app: &gtk::Application, _, _| new_window(app))
            .build();
        let open_recent = gtk::gio::ActionEntry::builder("open-recent")
            .parameter_type(Some(glib::VariantTy::STRING))
            .activate(|app: &gtk::Application, _, param| {
                if let Some(path) = param.and_then(|p| p.str()) {
                    open_window(app, Path::new(path), app.active_window().as_ref());
                }
            })
            .build();
        let clear_recent = gtk::gio::ActionEntry::builder("clear-recent")
            .activate(|_: &gtk::Application, _, _| {
                if let Err(e) = state::recent().clear() {
                    eprintln!("brick: cannot clear recent files: {e}");
                }
                menus::refresh_recent();
            })
            .build();
        // The menu bar's Close Window and Quit (macOS), and Exit (Windows). Quit closes
        // each window, so a window with unsaved edits asks first.
        let close = gtk::gio::ActionEntry::builder("close")
            .activate(|app: &gtk::Application, _, _| {
                if let Some(w) = app.active_window() {
                    w.close();
                }
            })
            .build();
        let quit = gtk::gio::ActionEntry::builder("quit")
            .activate(|app: &gtk::Application, _, _| {
                for window in app.windows() {
                    window.close();
                }
            })
            .build();
        app.add_action_entries([open, new, open_recent, clear_recent, close, quit]);
        let update = suite_ui::chrome::AutoUpdate {
            on: suite_update::automatic(APP),
            keep: |on| suite_update::set_automatic(APP, on),
        };
        suite_ui::chrome::install(app, APP, env!("CARGO_PKG_VERSION"), update);
        if suite_ui::chrome::MENU_BAR {
            app.set_menubar(Some(&menus::bar()));
        }
        // Open and New, and Replace (each window's action): from `keys::WINDOW` (MAC-3);
        // on macOS Close Window and Quit too.
        for (action, accels) in keys::accels(Primary::HERE) {
            let accels: Vec<&str> = accels.iter().map(String::as_str).collect();
            app.set_accels_for_action(action, &accels);
        }
    });
    let (opened, failed) = (RefCell::new(opened), RefCell::new(failed));
    let bench = RefCell::new(args.bench);
    // The first `open` brings the command line's file, opened above; later ones, and the
    // system's (Finder), open each file in its own window.
    app.connect_open(move |app, files, _| {
        if let Some((session, book)) = opened.take() {
            build_window(app, &session, book, bench.take());
        } else if let Some((message, detail)) = failed.take() {
            let window = start_window(app);
            alert(Some(window.upcast_ref()), &message, &detail);
        } else {
            let parent = app.active_window();
            for file in files {
                open_file(app, file, parent.as_ref());
            }
        }
    });
    app.connect_activate(|app| {
        suite_ui::launch::no_file(app, suite_platform::launch::when_finished, |app| {
            start_window(app);
        })
    });
    let code = suite_ui::launch::run(&app, args.file.as_slice());
    updates.exit();
    code
}

/// Map `path` and start indexing it; an .xlsx file starts importing (XLSX-1), and the
/// window shows its sheets once that is done. Old Excel, password-protected, and
/// OpenDocument files are refused with what to do instead.
fn open_session(path: &Path) -> std::io::Result<(Rc<Session>, Workbook)> {
    let session = Rc::new(Session::new(Some(path.to_owned())));
    match xlsx::kind(path)? {
        xlsx::Kind::Text => {
            let table = session.open()?;
            malformed::refuse_binary(&table)?;
            let book = session.workbook(table);
            Ok((session, book))
        }
        xlsx::Kind::Xlsx => {
            session.kind.set(DocKind::Xlsx);
            session.import();
            Ok((session, Workbook::default()))
        }
        kind => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            xlsx::refusal(kind).unwrap_or("Brick can't open this kind of file."),
        )),
    }
}

/// The workbook an import read, its sheets' formula cells and styles known.
fn workbook_of(imported: Imported) -> Workbook {
    let sheets = imported
        .sheets
        .into_iter()
        .map(|s| {
            let mut table = s.table;
            table.set_formula_cells(s.formulas);
            (s.name, table, Origin::Xlsx(s.index), s.cached)
        })
        .collect();
    let mut book = Workbook::from_sheets(sheets, imported.names);
    book.set_styles(imported.styles);
    book
}

/// Show the workbook `session`'s import reads once it is done (XLSX-1); until then the
/// grid says "Opening…". A failed import says why and closes the window (Esc cancels it).
fn follow_import(
    app: &gtk::Application,
    window: &gtk::ApplicationWindow,
    grid: &GridView,
    session: &Rc<Session>,
) {
    let Some(job) = session.importing.borrow().clone() else {
        return;
    };
    grid.set_loading(true);
    let (app, window, grid, session) = (
        app.clone(),
        window.downgrade(),
        grid.downgrade(),
        session.clone(),
    );
    glib::spawn_future_local(async move {
        let result = job.finished().await;
        let (Some(window), Some(grid)) = (window.upgrade(), grid.upgrade()) else {
            return;
        };
        session.importing.take();
        match result {
            Ok(Ok(imported)) => {
                session.package.replace(Some(imported.package.clone()));
                grid.replace_workbook(workbook_of(imported));
                grid.set_loading(false);
            }
            Ok(Err(ImportError::Cancelled)) | Err(jobs::Stopped::Cancelled) => window.close(),
            failed => {
                let why = match failed {
                    Ok(Err(e)) => e.to_string(),
                    _ => "the import stopped unexpectedly".to_owned(),
                };
                let path = session.path.borrow().clone().unwrap_or_default();
                eprintln!("brick: cannot open {}: {why}", path.display());
                let _ = state::recent().remove(&path);
                let last = app.windows().len() == 1;
                window.close();
                let parent = if last {
                    Some(start_window(&app).upcast::<gtk::Window>())
                } else {
                    app.active_window()
                };
                alert(
                    parent.as_ref(),
                    &format!("Cannot open {}", path.display()),
                    &why,
                );
            }
        }
    });
}

thread_local! {
    /// Every window and the file it shows: opening a file again brings its window forward
    /// instead of a second window that would save over the first one's edits. Looked up by
    /// the session's current path, so it follows Save As.
    static OPEN_WINDOWS: RefCell<Vec<(Rc<Session>, glib::WeakRef<gtk::ApplicationWindow>)>> =
        const { RefCell::new(Vec::new()) };
}

/// The window already showing `path`, if any.
fn window_for(path: &Path) -> Option<gtk::ApplicationWindow> {
    let path = canonical(path);
    OPEN_WINDOWS.with_borrow_mut(|open| {
        open.retain(|(_, w)| w.upgrade().is_some());
        open.iter()
            .find(|(s, _)| s.path.borrow().as_deref().map(canonical) == Some(path.clone()))
            .and_then(|(_, w)| w.upgrade())
    })
}

/// Ctrl+N (SAVE-2): a new window with an untitled, empty table; type into its edge row
/// and column to fill it, and Save asks where to put it.
fn new_window(app: &gtk::Application) {
    let session = Rc::new(Session::new(None));
    let book = session.workbook(CsvTable::untitled());
    build_window(app, &session, book, None);
    close_start_window(app);
}

fn close_start_window(app: &gtk::Application) {
    for w in app.windows() {
        if w.widget_name() == START_WINDOW {
            w.close();
        }
    }
}

/// Shown when the app starts without a file: buttons that open a file or start a new one,
/// and the recent files (APP-4).
fn start_window(app: &gtk::Application) -> gtk::ApplicationWindow {
    let heading = gtk::Label::new(Some("Open a CSV file"));
    heading.add_css_class("title-2");
    let open = gtk::Button::with_label("Open…");
    open.add_css_class("suggested-action");
    open.add_css_class("pill");
    open.set_halign(gtk::Align::Center);
    open.set_action_name(Some("app.open"));
    let new = gtk::Button::with_label("New");
    new.add_css_class("pill");
    new.set_halign(gtk::Align::Center);
    new.set_action_name(Some("app.new"));
    let hint = gtk::Label::new(Some(&format!(
        "{}, {}, drop a file here, or run: brick FILE.csv",
        keys::label(keys::Command::Open),
        keys::label(keys::Command::New),
    )));
    hint.add_css_class("dim-label");
    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    page.append(&heading);
    page.append(&open);
    page.append(&new);
    // Unsaved changes from a crash (APP-7) come first: putting them back is likely next.
    let restore = crash::start_section(app).map(|(section, restore)| {
        page.append(&section);
        restore
    });
    let files = state::recent().files().to_vec();
    if !files.is_empty() {
        let title = gtk::Label::new(Some("Recent"));
        title.add_css_class("heading");
        title.set_margin_top(12);
        page.append(&title);
        for file in files {
            page.append(&recent_button(app, &file));
        }
    }
    page.append(&hint);
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Brick")
        .default_width(900)
        .default_height(600)
        .child(&suite_ui::notice::host(&page))
        .build();
    window.set_widget_name(START_WINDOW);
    accept_file_drops(app, &window);
    // Linux's header bar with the main menu; macOS and Windows keep their own title bar.
    if !suite_ui::chrome::MENU_BAR {
        let header = gtk::HeaderBar::new();
        header.pack_end(&main_menu());
        window.set_titlebar(Some(&header));
    }
    // Enter acts on it: Restore after a crash, else Open.
    let first = restore.unwrap_or(open);
    window.set_default_widget(Some(&first));
    GtkWindowExt::set_focus(&window, Some(&first));
    window.present();
    window
}

/// A recent file on the start window: its name and folder; it goes away if the file
/// can't be opened any more.
fn recent_button(app: &gtk::Application, file: &Path) -> gtk::Button {
    let (name, dir) = recent::label(file);
    let text = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    text.append(&gtk::Label::new(Some(&name)));
    let folder = gtk::Label::new(Some(&dir));
    folder.add_css_class("dim-label");
    text.append(&folder);
    let button = gtk::Button::builder()
        .child(&text)
        .tooltip_text(file.display().to_string())
        .css_classes(["flat"])
        .build();
    button.connect_clicked({
        let (app, file) = (app.clone(), file.to_owned());
        move |b| {
            let window = b.root().and_downcast::<gtk::Window>();
            if !open_window(&app, &file, window.as_ref()) {
                b.set_visible(false);
            }
        }
    });
    button
}

/// The recent files menu in a window's header bar (APP-4), read again each time it opens.
fn recent_menu() -> gtk::MenuButton {
    let button = gtk::MenuButton::builder()
        .icon_name("pan-down-symbolic")
        .tooltip_text("Recent files")
        .build();
    button.set_create_popup_func(|button| {
        let menu = gtk::gio::Menu::new();
        menus::fill_recent(&menu);
        button.set_menu_model(Some(&menu));
    });
    button
}

/// A Linux header bar's main menu: Update Automatically and About Brick.
fn main_menu() -> gtk::MenuButton {
    suite_ui::chrome::main_menu(&suite_ui::chrome::app_section(APP))
}

/// Put `path` at the top of the recent files; a failure only costs the list.
fn remember(path: &Path) {
    if let Err(e) = state::recent().add(path) {
        eprintln!("brick: cannot update recent files: {e}");
    }
    menus::refresh_recent();
}

const START_WINDOW: &str = "start";

/// "Spreadsheets" (CSV, text, and Excel workbooks) and "All files", the first as the
/// default.
fn file_filters() -> (gtk::gio::ListStore, gtk::FileFilter) {
    let csv = gtk::FileFilter::new();
    csv.set_name(Some("Spreadsheets"));
    for mime in [
        "text/csv",
        "text/tab-separated-values",
        "text/plain",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "application/vnd.ms-excel.sheet.macroEnabled.12",
    ] {
        csv.add_mime_type(mime);
    }
    for suffix in ["csv", "tsv", "tab", "psv", "txt", "xlsx", "xlsm"] {
        csv.add_suffix(suffix);
    }
    let all = gtk::FileFilter::new();
    all.set_name(Some("All files"));
    all.add_pattern("*");
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&csv);
    filters.append(&all);
    (filters, csv)
}

/// The file dialog (APP-1). `gtk::FileDialog` goes through xdg-desktop-portal's
/// FileChooser when one is running, so each desktop shows its own dialog (GTK falls back
/// to its built-in one without a portal).
fn choose_and_open(app: &gtk::Application) {
    let (filters, csv) = file_filters();
    let dialog = gtk::FileDialog::builder()
        .title("Open")
        .modal(true)
        .filters(&filters)
        .default_filter(&csv)
        .build();
    let (app, parent) = (app.clone(), app.active_window());
    glib::spawn_future_local(async move {
        match dialog.open_future(parent.as_ref()).await {
            Ok(file) => match file.path() {
                Some(path) => {
                    open_window(&app, &path, parent.as_ref());
                }
                None => alert(
                    parent.as_ref(),
                    &format!("Cannot open {}", file.uri()),
                    "Only local files can be opened.",
                ),
            },
            Err(e) if e.matches(gtk::DialogError::Dismissed) => {}
            Err(e) => alert(
                parent.as_ref(),
                "Cannot show the file dialog",
                &e.to_string(),
            ),
        }
    });
}

/// Drag and drop to open (APP-5): files dropped on a window open as Open does, each in
/// its own window, or brought forward if already open. GTK hands over a file list from
/// both Wayland and X11 drags (it reads `text/uri-list` too). The windows open after the
/// drop is done: opening one closes the start window, which may be the drop's target.
fn accept_file_drops(app: &gtk::Application, window: &gtk::ApplicationWindow) {
    let target = suite_ui::launch::file_drop_target();
    target.connect_drop({
        let (app, window) = (app.clone(), window.downgrade());
        move |_, value, _, _| {
            let Ok(files) = value.get::<gtk::gdk::FileList>() else {
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

/// Open `file` as [`open_window`] does; one that isn't local (no path) alerts instead.
fn open_file(app: &gtk::Application, file: &gtk::gio::File, parent: Option<&gtk::Window>) {
    match file.path() {
        Some(path) => {
            open_window(app, &path, parent);
        }
        None => alert(
            parent,
            &format!("Cannot open {}", file.uri()),
            "Only local files can be opened.",
        ),
    }
}

/// Show `path` in a window: its existing one, or a new one. The start window closes once
/// a file is open. A file that can't be opened leaves the recent files. Whether it opened.
fn open_window(app: &gtk::Application, path: &Path, parent: Option<&gtk::Window>) -> bool {
    if let Some(window) = window_for(path) {
        window.present();
        remember(path);
        return true;
    }
    match open_session(path) {
        Ok((session, book)) => {
            remember(path);
            build_window(app, &session, book, None);
            close_start_window(app);
            true
        }
        Err(e) => {
            let _ = state::recent().remove(path);
            alert(
                parent,
                &format!("Cannot open {}", path.display()),
                &e.to_string(),
            );
            false
        }
    }
}

fn alert(parent: Option<&gtk::Window>, message: &str, detail: &str) {
    gtk::AlertDialog::builder()
        .message(message)
        .detail(detail)
        .modal(true)
        .build()
        .show(parent);
}

/// The open file: where it is (`None` while untitled), how its delimiter and encoding are
/// chosen, and the index build in flight.
struct Session {
    path: RefCell<Option<PathBuf>>,
    choice: Cell<DelimiterChoice>,
    /// The encoding a Save As wrote (SAVE-2); `None` while detection decides.
    encoding: Cell<Option<Encoding>>,
    /// "First row is header" as the user set it (ENG-7); `None` while detection decides.
    /// Re-applied whenever the file is read again (delimiter change, save).
    header: Cell<Option<bool>>,
    /// The running index build; cancelled when a newer reading replaces it.
    indexing: RefCell<Option<jobs::Job<Result<(), IndexError>>>>,
    /// The file's size and modification time when last read (APP-7): a crash journal
    /// records them, and its edits only go back onto the same bytes.
    identity: Cell<Option<(u64, u64)>>,
    /// A crashed session's edits to put back once the file is indexed (APP-7).
    restore: RefCell<Option<crash::Pending>>,
    /// The reading (delimiter, encoding) the malformed-file check last ran on (APP-8).
    checked: Cell<Option<(u8, Encoding)>>,
    /// What the file is: CSV, or an Excel workbook (XLSX-1).
    kind: Cell<DocKind>,
    /// The .xlsx file as it was read, which a save patches (ADR 0010).
    package: RefCell<Option<Arc<xlsx::Package>>>,
    /// The import of an Excel workbook in flight (XLSX-1), and how far it got.
    importing: RefCell<Option<jobs::Job<Result<Imported, ImportError>>>>,
    import_progress: RefCell<Arc<xlsx::Progress>>,
}

/// What a session's file is (XLSX-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocKind {
    Csv,
    Xlsx,
}

impl Session {
    fn new(path: Option<PathBuf>) -> Self {
        Self {
            path: RefCell::new(path),
            choice: Cell::new(DelimiterChoice::Auto),
            encoding: Cell::new(None),
            header: Cell::new(None),
            indexing: RefCell::new(None),
            identity: Cell::new(None),
            restore: RefCell::new(None),
            checked: Cell::new(None),
            kind: Cell::new(DocKind::Csv),
            package: RefCell::new(None),
            importing: RefCell::new(None),
            import_progress: RefCell::new(Arc::default()),
        }
    }

    /// Import the session's .xlsx file on the job pool (XLSX-1), cancelling any earlier
    /// import; [`follow_import`] or a save's check shows the result.
    fn import(&self) -> jobs::Job<Result<Imported, ImportError>> {
        let path = self.path.borrow().clone().unwrap_or_default();
        self.identity.set(recovery::identity(&path));
        let progress = Arc::new(xlsx::Progress::default());
        self.import_progress.replace(progress.clone());
        let job = jobs::spawn(move |cancel| xlsx::import(&path, cancel.flag(), &progress));
        if let Some(old) = self.importing.replace(Some(job.clone())) {
            old.cancel();
        }
        job
    }

    /// The import's share done (0..=1), while one runs.
    fn import_share(&self) -> Option<f64> {
        self.importing.borrow().as_ref()?;
        let p = self.import_progress.borrow();
        let (done, total) = (
            p.rows.load(std::sync::atomic::Ordering::Relaxed),
            p.rows_total.load(std::sync::atomic::Ordering::Relaxed),
        );
        Some(done as f64 / total.max(1) as f64)
    }

    /// The window title's file name: the file's, or "Untitled".
    fn name(&self) -> String {
        self.path.borrow().as_deref().map_or_else(
            || "Untitled".to_owned(),
            |p| suite_file::document_name(p).into_owned(),
        )
    }

    /// Map the file with the current delimiter choice (detected on Auto, ENG-4) and
    /// encoding (detected unless a Save As chose it), and start indexing it.
    fn open(&self) -> std::io::Result<CsvTable> {
        let path = self
            .path
            .borrow()
            .clone()
            .ok_or_else(|| std::io::Error::other("an untitled table has no file to read"))?;
        let mut table = match self.encoding.get() {
            Some(encoding) => CsvTable::open_as(&path, self.choice.get(), encoding)?,
            None => CsvTable::open(&path, self.choice.get())?,
        };
        self.identity.set(recovery::identity(&path));
        self.apply_header(&mut table);
        self.index(&table);
        Ok(table)
    }

    /// A workbook of `table`, named after the file (Sheet1 while untitled).
    fn workbook(&self, table: CsvTable) -> Workbook {
        let path = self.path.borrow();
        let name = path
            .as_deref()
            .and_then(Path::file_stem)
            .map_or_else(|| "Sheet1".to_owned(), |s| s.to_string_lossy().into_owned());
        let origin = if path.is_some() {
            Origin::Csv
        } else {
            Origin::New
        };
        Workbook::single(name, table, origin)
    }

    /// The file is a macro workbook (.xlsm): saving it as .xlsm keeps the macros.
    fn is_xlsm(&self) -> bool {
        self.path
            .borrow()
            .as_deref()
            .and_then(Path::extension)
            .is_some_and(|e| e.eq_ignore_ascii_case("xlsm"))
    }

    /// How Ctrl+S writes the file: as it was read (XLSX-1).
    fn format(&self, grid: &GridView) -> Format {
        match self.kind.get() {
            DocKind::Xlsx => Format::Xlsx,
            DocKind::Csv => Format::Csv {
                delimiter: grid.delimiter(),
                encoding: grid.encoding(),
            },
        }
    }

    /// Ctrl+S has to ask where and how: an untitled table, or a CSV file that has more
    /// sheets now than a CSV file holds (XLSX-1), or formatting it can't keep (STYLE-6).
    fn needs_save_as(&self, grid: &GridView) -> bool {
        self.path.borrow().is_none()
            || (self.kind.get() == DocKind::Csv
                && (grid.sheet_count() > 1 || grid.has_formatting()))
    }

    /// The user's header choice wins over detection on a fresh reading of the file.
    fn apply_header(&self, table: &mut CsvTable) {
        if let Some(on) = self.header.get() {
            table.set_header(on);
        }
    }

    /// Index `table` on the job pool (ENG-8), cancelling any earlier build; rows become
    /// readable as it goes.
    fn index(&self, table: &CsvTable) {
        let index = table.index().clone();
        let job = jobs::spawn(move |cancel| index.build(cancel.flag()));
        if let Some(old) = self.indexing.replace(Some(job)) {
            old.cancel();
        }
    }
}

/// Data → Delimiter's items: label, `win.delimiter`'s target, and the choice.
const DELIMITERS: [(&str, &str, DelimiterChoice); 5] = [
    ("Auto", "auto", DelimiterChoice::Auto),
    ("Comma", "comma", DelimiterChoice::Fixed(b',')),
    ("Tab", "tab", DelimiterChoice::Fixed(b'\t')),
    ("Semicolon", "semicolon", DelimiterChoice::Fixed(b';')),
    ("Pipe", "pipe", DelimiterChoice::Fixed(b'|')),
];

/// `win.delimiter`'s target for `choice`.
fn delimiter_target(choice: DelimiterChoice) -> &'static str {
    DELIMITERS
        .iter()
        .find(|(_, _, c)| *c == choice)
        .map_or("auto", |(_, t, _)| t)
}

/// Data → Delimiter (ENG-5, APP-10), `win.delimiter`, its state the chosen item's target:
/// re-reads the open file with the chosen delimiter and re-indexes it in the background.
/// Refused, saying why, while edits are unsaved, since edits are tied to the columns the
/// old delimiter produced.
fn delimiter_action(
    session: &Rc<Session>,
    grid: &GridView,
    window: &gtk::ApplicationWindow,
) -> gtk::gio::SimpleAction {
    let action = gtk::gio::SimpleAction::new_stateful(
        "delimiter",
        Some(glib::VariantTy::STRING),
        &delimiter_target(session.choice.get()).to_variant(),
    );
    action.connect_activate({
        let (session, grid, window) = (session.clone(), grid.downgrade(), window.downgrade());
        move |_, target| {
            let target = target.and_then(|t| t.str());
            let (Some(grid), Some(&(_, _, choice))) = (
                grid.upgrade(),
                DELIMITERS.iter().find(|(_, t, _)| Some(*t) == target),
            ) else {
                return;
            };
            if choice == session.choice.get() {
                return;
            }
            match grid.reread(choice) {
                Ok(mut table) => {
                    session.apply_header(&mut table);
                    session.index(&table);
                    session.choice.set(choice);
                    // Other columns now: the old widths don't belong to them.
                    grid.reset_column_widths();
                    grid.replace_workbook(session.workbook(table));
                }
                Err(RereadError::UnsavedEdits(_)) => alert(
                    window.upgrade().as_ref().map(|w| w.upcast_ref()),
                    "Save your edits before changing the delimiter",
                    "Your edits belong to the columns the current delimiter makes.",
                ),
            }
        }
    });
    action
}

/// Data → First Row Is Header (ENG-7, APP-10), `win.header-row`: whether the first row is
/// column titles or data, its state the check mark. Starts as detected; flipping it is the
/// user's choice for this file from then on.
fn header_action(session: &Rc<Session>, grid: &GridView) -> gtk::gio::SimpleAction {
    let action =
        gtk::gio::SimpleAction::new_stateful("header-row", None, &grid.has_header().to_variant());
    action.connect_activate({
        let (session, grid) = (session.clone(), grid.downgrade());
        move |action, _| {
            let Some(grid) = grid.upgrade() else { return };
            let on = !grid.has_header();
            grid.set_header(on);
            session.header.set(Some(on));
            action.set_state(&on.to_variant());
        }
    });
    action
}

fn build_window(
    app: &gtk::Application,
    session: &Rc<Session>,
    book: Workbook,
    bench: Option<Bench>,
) {
    let vadj = gtk::Adjustment::new(0.0, 0.0, 0.0, ROW_H, 0.0, 0.0);
    let hadj = gtk::Adjustment::new(0.0, 0.0, 0.0, 30.0, 0.0, 0.0);
    let grid = GridView::new(book, &vadj, &hadj);

    let layout = gtk::Grid::new();
    // The format bar (STYLE-6), then the formula bar and name box (FML-9), over the grid.
    let format_bar = format_bar::FormatBar::new(&grid);
    layout.attach(format_bar.root(), 0, 0, 2, 1);
    let formula_bar = formula_bar::FormulaBar::new(&grid);
    layout.attach(formula_bar.root(), 0, 1, 2, 1);
    layout.attach(&grid, 0, 2, 1, 1);
    layout.attach(
        &gtk::Scrollbar::new(gtk::Orientation::Vertical, Some(&vadj)),
        1,
        2,
        1,
        1,
    );
    layout.attach(
        &gtk::Scrollbar::new(gtk::Orientation::Horizontal, Some(&hadj)),
        0,
        3,
        1,
        1,
    );
    let find = FindBar::new(&grid);
    layout.attach(&find.0.root, 0, 4, 2, 1);
    let tabs = sheets::SheetTabs::new(&grid);
    layout.attach(tabs.root(), 0, 5, 2, 1);
    let bar = StatusBar::new();
    layout.attach(&bar.root, 0, 6, 2, 1);

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(session.name())
        .default_width(1400)
        .default_height(900)
        .child(&suite_ui::notice::host(&layout))
        .build();
    OPEN_WINDOWS.with_borrow_mut(|open| open.push((session.clone(), window.downgrade())));
    // Crash recovery (APP-7), except in benchmarks, which measure the app alone: journal
    // the unsaved edits, and offer back a crashed session's for this file.
    let journal = bench.is_none().then(crash::Journal::new);
    if let Some(j) = &journal {
        window.connect_destroy({
            let j = j.clone();
            move |_| crash::closed(&j)
        });
        let path = session.path.borrow().clone();
        if session.restore.borrow().is_none() {
            if let Some(c) = path.as_deref().and_then(crash::crashed_for) {
                session.restore.replace(Some(crash::Pending {
                    crashed: c,
                    ask: true,
                }));
            }
        }
    }
    accept_file_drops(app, &window);
    follow_import(app, &window, &grid, session);
    // A change refused for its size (SHEET-2): say why.
    grid.connect_closure(
        "refused",
        false,
        glib::closure_local!(
            #[weak]
            window,
            move |_: GridView, title: String, why: String| {
                alert(Some(window.upcast_ref()), &title, &why);
            }
        ),
    );
    let delimiter = delimiter_action(session, &grid, &window);
    window.add_action(&delimiter);
    let save = Rc::new(RefCell::new(SaveState::Idle));
    // The window's buttons. Linux: the header bar with Open, Recent, New, and Save As, then
    // the main menu (Data, About). macOS and Windows have those commands in the menu bar,
    // and no toolbar row.
    if !suite_ui::chrome::MENU_BAR {
        let toolbar = Toolbar::new();
        let open = gtk::Button::from_icon_name("document-open-symbolic");
        open.set_tooltip_text(Some(&format!(
            "Open… ({})",
            keys::label(keys::Command::Open)
        )));
        open.set_action_name(Some("app.open"));
        let new = gtk::Button::from_icon_name("document-new-symbolic");
        new.set_tooltip_text(Some(&format!("New ({})", keys::label(keys::Command::New))));
        new.set_action_name(Some("app.new"));
        let save_as_button = gtk::Button::from_icon_name("document-save-as-symbolic");
        save_as_button.set_tooltip_text(Some(&format!(
            "Save As… ({})",
            keys::label(keys::Command::SaveAs)
        )));
        save_as_button.set_action_name(Some("win.save-as"));
        toolbar.pack_start(&open);
        toolbar.pack_start(&recent_menu());
        toolbar.pack_start(&new);
        toolbar.pack_start(&save_as_button);
        toolbar.pack_end(&suite_ui::chrome::main_menu(&menus::main_menu()));
        toolbar.put_on(&window);
    }
    let header_row = header_action(session, &grid);
    window.add_action(&header_row);
    // Another brick may have changed the recent files meanwhile (File → Open Recent).
    window.connect_is_active_notify(|window| {
        if window.is_active() {
            menus::refresh_recent();
        }
    });

    // Mouse wheel and touchpad.
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    scroll.connect_scroll({
        let (vadj, hadj) = (vadj.clone(), hadj.clone());
        move |c, dx, dy| {
            let step = if c.unit() == gtk::gdk::ScrollUnit::Wheel {
                ROW_H * 3.0
            } else {
                1.0
            };
            vadj.set_value(vadj.value() + dy * step);
            hadj.set_value(hadj.value() + dx * step);
            glib::Propagation::Stop
        }
    });
    grid.add_controller(scroll);

    bar.cancel.connect_clicked({
        let save = save.clone();
        move |_| {
            if let SaveState::Saving(r) = &*save.borrow() {
                r.job.cancel();
            }
        }
    });

    // What a window command does, for the window's keys and the menu bar's items (macOS,
    // Windows).
    // `false`: it's an accelerator's (Open, New, Replace, Close Window, Quit), not this.
    let run: Rc<dyn Fn(keys::Command) -> bool> = Rc::new({
        let (window, grid, save, session, find, tabs, formula_bar, format_bar) = (
            window.downgrade(),
            grid.downgrade(),
            save.clone(),
            session.clone(),
            find.clone(),
            tabs.clone(),
            formula_bar.clone(),
            format_bar.clone(),
        );
        move |command| {
            use keys::Command as C;
            let (Some(window), Some(grid)) = (window.upgrade(), grid.upgrade()) else {
                return false;
            };
            match command {
                // The name box (FML-9).
                C::GoTo => {
                    grid.finish_editing();
                    formula_bar.focus_name();
                }
                // A new sheet, as in Excel (SHEET-1).
                C::NewSheet => {
                    grid.add_sheet();
                    tabs.refresh();
                }
                // Formatting (STYLE-6), as in Excel.
                C::Font(toggle) => grid.toggle_font(toggle),
                C::CustomFormat => format_bar.open_custom(),
                C::Format(code) => grid.format_selection(&StylePatch {
                    num_fmt: Some(code.into()),
                    ..StylePatch::default()
                }),
                // The next or previous sheet (XLSX-1).
                C::Sheet(step) => {
                    grid.switch_to_neighbor(step);
                }
                C::Find => {
                    grid.finish_editing();
                    find.open(false);
                }
                C::Save | C::SaveAs => {
                    grid.finish_editing(); // save what was typed, as Calc does
                    if command == C::SaveAs || session.needs_save_as(&grid) {
                        save_as(&window, &grid, &session, &save, false);
                    } else {
                        start_save(&grid, &session, &save, None, false);
                    }
                }
                C::Undo => {
                    grid.undo();
                }
                C::Redo => {
                    grid.redo();
                }
                C::Open | C::New | C::Replace | C::Close | C::Quit => return false,
            }
            true
        }
    });

    // The window's keys (`keys::WINDOW`, MAC-3): Primary+S saves (asking where, for an
    // untitled table) and Primary+Shift+S saves as; Primary+Z undoes, Primary+Shift+Z (and
    // Ctrl+Y on Linux) redo (CMD-1); Esc cancels a running save (SAVE-3). Navigation keys
    // belong to the grid (GRID-3); the cell editor's entry keeps its own text undo, and the
    // grid lets Esc through when no editor is open.
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let (grid, save, session, run) =
            (grid.downgrade(), save.clone(), session.clone(), run.clone());
        move |_, key, _, mods| {
            let Some(grid) = grid.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if key == gtk::gdk::Key::Escape {
                if let Some(job) = session.importing.borrow().as_ref() {
                    job.cancel();
                    return glib::Propagation::Stop;
                }
                if let SaveState::Saving(r) = &*save.borrow() {
                    r.job.cancel();
                    return glib::Propagation::Stop;
                }
                if grid.cancel_job() {
                    return glib::Propagation::Stop;
                }
            }
            match keys::window_command(Primary::HERE, key, mods) {
                Some(command) if run(command) => glib::Propagation::Stop,
                _ => glib::Propagation::Proceed,
            }
        }
    });
    window.add_controller(keys);
    // Replace (Ctrl+H, ⌥⌘F on macOS) is the accelerator `win.replace`: GTK matches it by
    // keycode too, which Option+letter needs.
    let replace = gtk::gio::ActionEntry::builder("replace")
        .activate({
            let (grid, find) = (grid.downgrade(), find.clone());
            move |_: &gtk::ApplicationWindow, _, _| {
                if let Some(grid) = grid.upgrade() {
                    grid.finish_editing();
                    find.open(true);
                }
            }
        })
        .build();
    window.add_action_entries([replace]);
    menus::add_window_actions(&window, &grid, run);

    // Closing with unsaved edits asks first (APP-9): Save, Don't Save, or Cancel. Closing
    // while a save runs waits for it and closes once it succeeds; a failed or cancelled
    // save keeps the window and its edits.
    let discard = Rc::new(Cell::new(false));
    window.connect_close_request({
        let (grid, session, save) = (grid.downgrade(), session.clone(), save.clone());
        move |window| {
            let Some(grid) = grid.upgrade() else {
                return glib::Propagation::Proceed;
            };
            grid.finish_editing(); // what was typed counts as an edit
            if discard.get() {
                return glib::Propagation::Proceed;
            }
            if let SaveState::Saving(r) = &mut *save.borrow_mut() {
                r.close_after = true;
                return glib::Propagation::Stop;
            }
            let edits = grid.edit_count();
            if edits == 0 {
                return glib::Propagation::Proceed;
            }
            let dialog = gtk::AlertDialog::builder()
                .message(format!("Save changes to “{}”?", session.name()))
                .detail(format!(
                    "{} unsaved edit{} will be lost if you close without saving.",
                    status::group_digits(edits as u64),
                    if edits == 1 { "" } else { "s" }
                ))
                .buttons(["Cancel", "Don't Save", "Save"])
                .cancel_button(0)
                .default_button(2)
                .modal(true)
                .build();
            let (window, session, save, discard) = (
                window.clone(),
                session.clone(),
                save.clone(),
                discard.clone(),
            );
            glib::spawn_future_local(async move {
                match dialog.choose_future(Some(&window)).await {
                    Ok(1) => {
                        discard.set(true);
                        window.close();
                    }
                    Ok(2) if session.needs_save_as(&grid) => {
                        save_as(&window, &grid, &session, &save, true)
                    }
                    Ok(2) => start_save(&grid, &session, &save, None, true),
                    _ => {}
                }
            });
            glib::Propagation::Stop
        }
    });

    // Status: follow the indexer (grow the scroll range) and keep the status bar and title
    // current. Runs once now, so the bar is filled from the first frame, then every 100 ms.
    let started = Instant::now();
    let (mut was_complete, mut generation) = (false, 0);
    let status_bench = matches!(bench, Some(Bench::Status));
    let mut tick = {
        let (grid, window, session, app, save, find, journal, tabs) = (
            grid.downgrade(),
            window.downgrade(),
            session.clone(),
            app.clone(),
            save.clone(),
            find.clone(),
            journal.clone(),
            tabs.clone(),
        );
        move || {
            let (Some(grid), Some(window)) = (grid.upgrade(), window.upgrade()) else {
                return glib::ControlFlow::Break;
            };
            // While indexing, grow the scroll range; also once more for a table swapped in
            // since the last tick (a save or delimiter change), which may have finished
            // indexing in between.
            if grid.generation() != generation {
                generation = grid.generation();
                was_complete = false;
            }
            let complete = grid.is_complete();
            if !complete || !was_complete {
                grid.update_adjustments();
            }
            if complete && !was_complete {
                grid.infer_types();
                grid.scan_formulas();
                grid.apply_pending_filters(); // the filters a save had on (FILT-3)
            }
            if let Some(j) = &journal {
                crash::journal_tick(&grid, &session, j);
                crash::pending_tick(&window, &grid, &session);
            }
            if session.restore.borrow().is_none() && grid.active_origin() == Origin::Csv {
                malformed::tick(&window, &grid, &session);
            }
            if complete && !was_complete && std::env::var_os("BRICK_TIMINGS").is_some() {
                eprintln!(
                    "indexed {} rows ({:?} since start)",
                    grid.row_count(),
                    started.elapsed()
                );
            }
            // Detection runs again when the file is re-read (delimiter change, save).
            if header_row.state().and_then(|v| v.get::<bool>()) != Some(grid.has_header()) {
                header_row.set_state(&grid.has_header().to_variant());
            }
            // Only a CSV file has a delimiter to choose (XLSX-1), and not while it saves.
            delimiter.set_enabled(
                grid.active_origin() == Origin::Csv
                    && !matches!(*save.borrow(), SaveState::Saving(..)),
            );
            // A Save As can change the delimiter choice.
            let choice = delimiter_target(session.choice.get());
            if delimiter.state().as_ref().and_then(|v| v.str()) != Some(choice) {
                delimiter.set_state(&choice.to_variant());
            }
            let shown = bar.update(&grid, &session, &save.borrow());
            find.tick(&grid);
            tabs.refresh();
            let title = status::title(&session.name(), grid.edit_count());
            if window.title().as_deref() != Some(title.as_str()) {
                window.set_title(Some(&title));
            }
            if status_bench {
                // The status acceptance test reads every update of the row count.
                println!("STATUS {}", shown.rows);
                if complete {
                    app.quit();
                }
            }
            was_complete = complete;
            glib::ControlFlow::Continue
        }
    };
    tick();
    glib::timeout_add_local(Duration::from_millis(100), tick);

    window.present();
    grid.grab_focus();
    match bench {
        Some(Bench::Scroll { frames }) => bench_scroll(app, &grid, &vadj, frames),
        Some(Bench::Jump { row, jumps }) => bench_jump(app, &grid, &vadj, row, jumps),
        Some(Bench::Open) => bench_open(app, &grid),
        Some(Bench::Save { edits }) => bench_save(app, &grid, session, &save, &vadj, edits),
        Some(Bench::Find { text }) => bench_find(app, &grid, &find, text.into()),
        Some(Bench::Status) | None => {}
    }
}

/// Find benchmark (SRCH-2). Once the file is indexed, opens the find bar and types TEXT
/// one character every 200 ms, as a quick typist would, and prints a line per event with
/// milliseconds since the first keystroke:
/// - `FIND <ms> key <cancelled 0|1> <text>`: the text changed; whether that cancelled a
///   running search;
/// - `FIND <ms> start <text>`: a search started;
/// - `FIND <ms> done <hit:N/TOTAL | nothing | cancelled | notready> <text>`: it came back.
///
/// Quits once the search for the whole TEXT is done.
fn bench_find(app: &gtk::Application, grid: &GridView, find: &FindBar, text: Rc<str>) {
    let t0 = Rc::new(Cell::new(None::<Instant>));
    let ms = {
        let t0 = t0.clone();
        move || t0.get().map_or(0.0, |t| t.elapsed().as_secs_f64() * 1e3)
    };
    let (app, full) = (app.clone(), text.clone());
    find.0
        .trace
        .replace(Some(Box::new(move |event| match event {
            FindEvent::Key { text, cancelled } => {
                println!("FIND {:.1} key {} {text}", ms(), u8::from(cancelled))
            }
            FindEvent::Start(text) => println!("FIND {:.1} start {text}", ms()),
            FindEvent::Done(done, found) => {
                use grid_view::Found;
                let result = match found {
                    Found::Hit { ordinal, total } | Found::Skipped { ordinal, total } => {
                        format!("hit:{ordinal}/{total}")
                    }
                    Found::Nothing => "nothing".into(),
                    Found::Cancelled => "cancelled".into(),
                    Found::NotReady => "notready".into(),
                };
                println!("FIND {:.1} done {result} {done}", ms());
                if done == &*full && !matches!(found, Found::Cancelled) {
                    app.quit();
                }
            }
        })));
    let (grid, find) = (grid.clone(), find.clone());
    let mut typed = 0;
    glib::timeout_add_local(Duration::from_millis(200), move || {
        if !grid.is_complete() {
            return glib::ControlFlow::Continue;
        }
        if typed == 0 {
            find.open(false);
            t0.set(Some(Instant::now()));
        }
        // One character at the end, as a keystroke does (`set_text` would empty the
        // field first: two changes).
        let ch = text
            .chars()
            .nth(typed)
            .map(String::from)
            .unwrap_or_default();
        typed += 1;
        let mut end = -1;
        find.0.entry.insert_text(&ch, &mut end);
        if typed >= text.chars().count() {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

/// Open benchmark (BENCH-1, APP-3). Prints `FIRST_FRAME {json}` at the end of the first
/// frame that shows rows (or, for a file with none, the first frame at all), with the rows
/// indexed so far and whether indexing was complete; then, once indexing is done,
/// `OPENED {json}` with memory use, and quits. The caller times from spawn to each line.
fn bench_open(app: &gtk::Application, grid: &GridView) {
    let Some(clock) = grid.frame_clock() else {
        return;
    };
    let (app, first) = (app.clone(), Cell::new(false));
    clock.connect_after_paint({
        let grid = grid.downgrade();
        move |_| {
            let Some(grid) = grid.upgrade() else { return };
            // An .xlsx shows an empty workbook until its import is done (XLSX-1).
            let ready = grid.is_complete() && !grid.is_loading();
            let empty = ready && grid.row_count() == 0;
            if !first.get() && (grid.painted_top_row().is_some() || empty) {
                first.set(true);
                // How far indexing had got when rows first showed (APP-3).
                println!(
                    r#"FIRST_FRAME {{"rows_indexed":{},"complete":{}}}"#,
                    grid.row_count(),
                    grid.is_complete()
                );
            }
            if first.get() && ready {
                println!(
                    r#"OPENED {{"rows":{},"sheets":{},"rss_anon_kib":{},"rss_file_kib":{},"vm_hwm_kib":{}}}"#,
                    grid.row_count(),
                    grid.sheet_count(),
                    proc_status_kib("RssAnon:"),
                    proc_status_kib("RssFile:"),
                    proc_status_kib("VmHWM:"),
                );
                app.quit();
            }
        }
    });
    // Keep frames coming until indexing finishes.
    grid.add_tick_callback(|_, _| glib::ControlFlow::Continue);
}

/// Where and how a Save As writes (SAVE-2, XLSX-1).
struct Target {
    path: PathBuf,
    format: Format,
}

/// What a save writes: a CSV file (one sheet), or an Excel workbook (XLSX-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Csv { delimiter: u8, encoding: Encoding },
    Xlsx,
}

/// A save running on the job pool (SAVE-3, ENG-8).
struct Running {
    job: jobs::Job<SaveResult>,
    target: Option<Target>,
    progress: Arc<SaveProgress>,
    /// Bytes the save is expected to write, for the progress percentage.
    estimate: u64,
    /// Close the window once this save succeeds (APP-9).
    close_after: bool,
}

enum SaveState {
    Idle,
    Saving(Running),
    Saved(Duration),
    Cancelled,
    Failed(String),
}

/// What a save job returns: its stats and how long it took, or why it failed.
type SaveResult = Result<(SaveStats, Duration), suite_file::SaveError>;

/// Ctrl+S: write the table to its own file on the job pool (save is atomic, SAVE-1); with
/// a `target`, to another file, delimiter, or encoding (Save As, SAVE-2). Editing is paused
/// until it finishes, so no edit can be lost between snapshot and reopen. With
/// `close_after`, the window closes once the save succeeds.
fn start_save(
    grid: &GridView,
    session: &Rc<Session>,
    save: &Rc<RefCell<SaveState>>,
    target: Option<Target>,
    close_after: bool,
) {
    if matches!(*save.borrow(), SaveState::Saving(..))
        || grid.busy()
        || (target.is_none() && grid.edit_count() == 0)
    {
        return;
    }
    let Some(path) = target
        .as_ref()
        .map(|t| t.path.clone())
        .or_else(|| session.path.borrow().clone())
    else {
        return;
    };
    let format = target
        .as_ref()
        .map_or_else(|| session.format(grid), |t| t.format);
    let progress = Arc::new(SaveProgress::default());
    let (running, estimate) = match format {
        // A CSV file holds one sheet and no formatting: a workbook of more sheets, or with
        // formats, goes through Save As (XLSX-1, STYLE-6).
        Format::Csv { .. } if grid.sheet_count() > 1 || grid.has_formatting() => return,
        Format::Csv {
            delimiter,
            encoding,
        } => {
            let job = match grid.save_job() {
                Ok(job) if target.is_some() => job.with_format(delimiter, encoding),
                Ok(job) => job,
                Err(_) => {
                    *save.borrow_mut() = SaveState::Failed("wait for indexing to finish".into());
                    return;
                }
            };
            let estimate = job.estimated_bytes();
            let running = jobs::spawn({
                let progress = progress.clone();
                move |cancel| {
                    let t = Instant::now();
                    file_format::save_csv(&path, &job, &progress, cancel.flag())
                        .map(|s| (s, t.elapsed()))
                }
            });
            (running, estimate)
        }
        Format::Xlsx => {
            if let Err(e) = xlsx::check_limits(&grid.sheet_sizes()) {
                let window = grid.root().and_downcast::<gtk::Window>();
                alert(
                    window.as_ref(),
                    "This workbook doesn't fit an .xlsx file",
                    &e,
                );
                return;
            }
            if !grid.is_complete() {
                *save.borrow_mut() = SaveState::Failed("wait for indexing to finish".into());
                return;
            }
            let (book, sizes) = (grid.workbook_snapshot(), grid.user_sizes());
            let package = session.package.borrow().clone();
            let estimate = book.sheets().iter().map(|s| s.table.total_rows()).sum();
            let running = jobs::spawn({
                let progress = progress.clone();
                move |cancel| {
                    let t = Instant::now();
                    save_workbook(&path, book, package, &sizes, &progress, cancel)
                        .map(|s| (s, t.elapsed()))
                }
            });
            (running, estimate)
        }
    };
    grid.set_saving(true);
    *save.borrow_mut() = SaveState::Saving(Running {
        job: running.clone(),
        target,
        progress,
        estimate,
        close_after,
    });
    let (grid, session, save) = (grid.downgrade(), session.clone(), save.clone());
    glib::spawn_future_local(async move {
        let result = running.finished().await;
        let Some(grid) = grid.upgrade() else { return };
        if finish_save(&grid, &session, &save, result) {
            if let Some(window) = grid.root().and_downcast::<gtk::Window>() {
                window.close();
            }
        }
    });
}

/// Write a workbook snapshot to `path` as .xlsx (XLSX-1): each sheet's formula results
/// first (for the file's cached values), then the package read at open, patched
/// (ADR 0010), or a new one.
fn save_workbook(
    path: &Path,
    book: Workbook,
    package: Option<Arc<xlsx::Package>>,
    sizes: &std::collections::HashMap<data_model::SheetId, xlsx::SheetSizes>,
    progress: &SaveProgress,
    cancel: &jobs::CancelToken,
) -> Result<SaveStats, suite_file::SaveError> {
    let calculated = std::sync::atomic::AtomicU64::new(0);
    let book = xlsx::SaveBook::from_workbook(book, package, sizes, cancel.shared(), &calculated)
        .ok_or(suite_file::SaveError::Cancelled)?;
    xlsx::save(path, book, progress, cancel.flag())
}

/// When a save has finished: reopen the saved file (its edits are now on disk; after a
/// Save As, the new file with its delimiter and encoding, or the workbook) or report the
/// error with the edits still in memory. A cancelled save changes nothing. Returns whether
/// the save succeeded and asked for the window to close.
fn finish_save(
    grid: &GridView,
    session: &Rc<Session>,
    save: &Rc<RefCell<SaveState>>,
    result: Result<SaveResult, jobs::Stopped>,
) -> bool {
    let (target, close_after) = match &mut *save.borrow_mut() {
        SaveState::Saving(r) => (r.target.take(), r.close_after),
        _ => return false,
    };
    let result = match result {
        Ok(Err(suite_file::SaveError::Cancelled)) | Err(jobs::Stopped::Cancelled) => {
            grid.set_saving(false);
            *save.borrow_mut() = SaveState::Cancelled;
            return false;
        }
        Ok(done) => done.map_err(|e| e.to_string()),
        Err(jobs::Stopped::Panicked) => Err("the save stopped unexpectedly".to_owned()),
    };
    let format = target
        .as_ref()
        .map_or_else(|| session.format(grid), |t| t.format);
    if let (Ok(_), Some(t)) = (&result, target) {
        remember(&t.path);
        session.path.replace(Some(t.path));
        match t.format {
            Format::Csv {
                delimiter,
                encoding,
            } => {
                session.kind.set(DocKind::Csv);
                session.encoding.set(Some(encoding));
                session.choice.set(DelimiterChoice::Fixed(delimiter));
            }
            Format::Xlsx => session.kind.set(DocKind::Xlsx),
        }
    }
    if format == Format::Xlsx {
        match result {
            Ok(done) => check_saved_workbook(grid, session, save, done, close_after),
            Err(e) => {
                grid.set_saving(false);
                eprintln!("brick: save failed: {e}");
                *save.borrow_mut() = SaveState::Failed(e);
            }
        }
        return false;
    }
    grid.set_saving(false);
    let next = match result
        .and_then(|done| session.open().map(|t| (done, t)).map_err(|e| e.to_string()))
    {
        Ok(((stats, took), table)) => {
            // The reopened file has the same columns: the types the user set (TYPE-2) and
            // the filters (FILT-3) come back, types first since filters read them.
            let (types, filters) = (grid.type_overrides(), grid.filters());
            grid.replace_workbook(session.workbook(table));
            grid.restore_type_overrides(types);
            grid.restore_filters(filters);
            if std::env::var_os("BRICK_TIMINGS").is_some() {
                eprintln!(
                    "saved {} rows, {} bytes in {took:?}",
                    stats.rows, stats.bytes
                );
            }
            SaveState::Saved(took)
        }
        Err(e) => {
            eprintln!("brick: save failed: {e}");
            SaveState::Failed(e)
        }
    };
    let close = close_after && matches!(next, SaveState::Saved(_));
    *save.borrow_mut() = next;
    close
}

/// After an .xlsx save (XLSX-1): read the saved workbook back ("Checking the saved
/// file…") and show it, so what the window shows is what the file holds. Each sheet's
/// header row and column types come back by position, and the active sheet's filters.
fn check_saved_workbook(
    grid: &GridView,
    session: &Rc<Session>,
    save: &Rc<RefCell<SaveState>>,
    (stats, took): (SaveStats, Duration),
    close_after: bool,
) {
    let path = session.path.borrow().clone().unwrap_or_default();
    if let SaveState::Saving(r) = &*save.borrow() {
        r.progress.set_checking();
    }
    session.identity.set(recovery::identity(&path));
    let job = jobs::spawn({
        let path = path.clone();
        move |cancel| xlsx::import(&path, cancel.flag(), &xlsx::Progress::default())
    });
    let (grid, save, session) = (grid.downgrade(), save.clone(), session.clone());
    glib::spawn_future_local(async move {
        let result = job.finished().await;
        let Some(grid) = grid.upgrade() else { return };
        grid.set_saving(false);
        match result {
            Ok(Ok(imported)) => {
                session.package.replace(Some(imported.package.clone()));
                let (settings, active, filters) = (
                    grid.sheet_settings(),
                    grid.active_position(),
                    grid.filters(),
                );
                // The sheet that showed shows again, with the window's cursor and scroll.
                let mut book = workbook_of(imported);
                if let Some(id) = book.sheets().get(active).map(|s| s.id) {
                    book.set_active(id);
                }
                grid.replace_workbook(book);
                grid.restore_sheet_settings(settings);
                grid.restore_filters(filters);
                if std::env::var_os("BRICK_TIMINGS").is_some() {
                    eprintln!("saved {} rows as .xlsx in {took:?}", stats.rows);
                }
                *save.borrow_mut() = SaveState::Saved(took);
                if close_after {
                    if let Some(window) = grid.root().and_downcast::<gtk::Window>() {
                        window.close();
                    }
                }
            }
            failed => {
                let why = match failed {
                    Ok(Err(e)) => e.to_string(),
                    _ => "reading it back stopped".to_owned(),
                };
                eprintln!("brick: the saved workbook can't be read back: {why}");
                *save.borrow_mut() =
                    SaveState::Failed(format!("the saved file can't be read back: {why}"));
            }
        }
    });
}

/// Delimiters Save As offers, in dropdown order.
const SAVE_DELIMITERS: [(&str, u8); 4] = [
    ("Comma", b','),
    ("Tab", b'\t'),
    ("Semicolon", b';'),
    ("Pipe", b'|'),
];

/// Encodings Save As offers, in dropdown order. UTF-16 is written with its BOM.
const SAVE_ENCODINGS: [(&str, Encoding); 5] = [
    ("UTF-8", Encoding::UTF8),
    (
        "UTF-8 with BOM",
        Encoding {
            charset: Charset::Utf8,
            bom: true,
        },
    ),
    (
        "UTF-16 LE",
        Encoding {
            charset: Charset::Utf16Le,
            bom: true,
        },
    ),
    (
        "UTF-16 BE",
        Encoding {
            charset: Charset::Utf16Be,
            bom: true,
        },
    ),
    (
        "Windows-1252",
        Encoding {
            charset: Charset::Windows1252,
            bom: false,
        },
    ),
];

/// Ctrl+Shift+S (SAVE-2, XLSX-1): pick the format (CSV, or an Excel workbook) and for
/// CSV the delimiter and encoding (the file's own preselected), then the file, in the
/// desktop's save dialog; the workbook is written there and the window shows that file
/// from then on. A workbook of more than one sheet saves only as .xlsx.
fn save_as(
    window: &gtk::ApplicationWindow,
    grid: &GridView,
    session: &Rc<Session>,
    save: &Rc<RefCell<SaveState>>,
    close_after: bool,
) {
    if matches!(*save.borrow(), SaveState::Saving(..)) {
        return;
    }
    let formats = gtk::DropDown::from_strings(&["CSV", "Excel workbook (.xlsx)"]);
    let sheets = grid.sheet_count();
    let formatted = grid.has_formatting();
    // A CSV file holds one sheet and no formatting (XLSX-1, STYLE-6).
    let xlsx_only = sheets > 1 || formatted;
    formats.set_selected(u32::from(session.kind.get() == DocKind::Xlsx || xlsx_only));
    let note = gtk::Label::new(Some(if sheets > 1 {
        "A CSV file holds one sheet: save this workbook as .xlsx"
    } else {
        "A CSV file can't keep formatting: save as .xlsx"
    }));
    note.add_css_class("dim-label");
    note.set_wrap(true);
    note.set_visible(xlsx_only);
    if session.is_xlsm() {
        // An .xlsm name keeps the macros; an .xlsx one drops them (ADR 0010).
        note.set_text("Keep the .xlsm name to keep its macros: an .xlsx file drops them");
        note.set_visible(true);
        formats.set_selected(1);
    }
    let delimiters = gtk::DropDown::from_strings(&SAVE_DELIMITERS.map(|(n, _)| n));
    delimiters.set_selected(
        SAVE_DELIMITERS
            .iter()
            .position(|(_, d)| *d == grid.delimiter())
            .unwrap_or(0) as u32,
    );
    let encodings = gtk::DropDown::from_strings(&SAVE_ENCODINGS.map(|(n, _)| n));
    let current = grid.encoding();
    encodings.set_selected(
        SAVE_ENCODINGS
            .iter()
            .position(|(_, e)| *e == current)
            .or_else(|| {
                SAVE_ENCODINGS
                    .iter()
                    .position(|(_, e)| e.charset == current.charset)
            })
            .unwrap_or(0) as u32,
    );
    let form = gtk::Grid::builder()
        .row_spacing(10)
        .column_spacing(12)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    let mut csv_rows = Vec::new();
    for (row, (label, dropdown)) in [
        ("Format", &formats),
        ("Delimiter", &delimiters),
        ("Encoding", &encodings),
    ]
    .into_iter()
    .enumerate()
    {
        let label = gtk::Label::new(Some(label));
        label.set_xalign(1.0);
        form.attach(&label, 0, row as i32, 1, 1);
        dropdown.set_hexpand(true);
        form.attach(dropdown, 1, row as i32, 1, 1);
        if row > 0 {
            csv_rows.push((label, dropdown.clone()));
        }
    }
    form.attach(&note, 0, 3, 2, 1);
    // Delimiter and encoding are CSV's; a workbook of several sheets can't be CSV.
    let show_format = move |formats: &gtk::DropDown| {
        if formats.selected() == 0 && xlsx_only {
            formats.set_selected(1);
        }
        let csv = formats.selected() == 0;
        for (label, dropdown) in &csv_rows {
            label.set_visible(csv);
            dropdown.set_visible(csv);
        }
    };
    show_format(&formats);
    formats.connect_selected_notify(show_format);
    let cancel = gtk::Button::with_label("Cancel");
    let choose = gtk::Button::with_label("Choose File…");
    choose.add_css_class("suggested-action");
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&cancel);
    buttons.append(&choose);
    form.attach(&buttons, 0, 4, 2, 1);
    let dialog = gtk::Window::builder()
        .title("Save As")
        .transient_for(window)
        .modal(true)
        .resizable(false)
        .child(&form)
        .default_widget(&choose)
        .build();
    let esc = gtk::EventControllerKey::new();
    esc.connect_key_pressed({
        let dialog = dialog.downgrade();
        move |_, key, _, _| {
            if key != gtk::gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            if let Some(d) = dialog.upgrade() {
                d.close();
            }
            glib::Propagation::Stop
        }
    });
    dialog.add_controller(esc);
    cancel.connect_clicked({
        let dialog = dialog.downgrade();
        move |_| {
            if let Some(d) = dialog.upgrade() {
                d.close();
            }
        }
    });
    choose.connect_clicked({
        let (dialog, window, grid, session, save) = (
            dialog.downgrade(),
            window.downgrade(),
            grid.downgrade(),
            session.clone(),
            save.clone(),
        );
        move |_| {
            let format = if formats.selected() == 1 {
                Format::Xlsx
            } else {
                Format::Csv {
                    delimiter: SAVE_DELIMITERS[delimiters.selected() as usize].1,
                    encoding: SAVE_ENCODINGS[encodings.selected() as usize].1,
                }
            };
            if let Some(d) = dialog.upgrade() {
                d.close();
            }
            let (Some(window), Some(grid)) = (window.upgrade(), grid.upgrade()) else {
                return;
            };
            choose_save_file(&window, &grid, &session, &save, format, close_after);
        }
    });
    dialog.present();
}

/// The second step of Save As: the desktop's save dialog, starting next to the current
/// file with its name, the extension following the format (`.tsv` for tabs, `.xlsx`).
fn choose_save_file(
    window: &gtk::ApplicationWindow,
    grid: &GridView,
    session: &Rc<Session>,
    save: &Rc<RefCell<SaveState>>,
    format: Format,
    close_after: bool,
) {
    let (filters, csv) = file_filters();
    let name = suggested_name(&session.name(), session.path.borrow().is_none(), format);
    let builder = gtk::FileDialog::builder()
        .title("Save As")
        .modal(true)
        .filters(&filters)
        .default_filter(&csv)
        .initial_name(name);
    let folder = session
        .path
        .borrow()
        .as_deref()
        .and_then(Path::parent)
        .map(gtk::gio::File::for_path);
    let dialog = match folder {
        Some(folder) => builder.initial_folder(&folder).build(),
        None => builder.build(),
    };
    let (window, grid, session, save) = (
        window.clone(),
        grid.downgrade(),
        session.clone(),
        save.clone(),
    );
    glib::spawn_future_local(async move {
        let file = match dialog.save_future(Some(&window)).await {
            Ok(file) => file,
            Err(e) if e.matches(gtk::DialogError::Dismissed) => return,
            Err(e) => {
                alert(
                    Some(window.upcast_ref()),
                    "Cannot show the file dialog",
                    &e.to_string(),
                );
                return;
            }
        };
        let Some(path) = file.path() else {
            alert(
                Some(window.upcast_ref()),
                &format!("Cannot save to {}", file.uri()),
                "Only local files can be saved.",
            );
            return;
        };
        if window_for(&path).is_some_and(|w| w != window) {
            alert(
                Some(window.upcast_ref()),
                &format!("{} is open in another window", path.display()),
                "Close it there first, or save under another name.",
            );
            return;
        }
        let Some(grid) = grid.upgrade() else { return };
        let target = Target { path, format };
        start_save(&grid, &session, &save, Some(target), close_after);
    });
}

/// The name Save As starts with: the file's own, with `.tsv` for tab-separated output,
/// `.csv` otherwise when it had one of the extensions Brick writes, and `.xlsx` for a
/// workbook; `Untitled.csv`/`.tsv`/`.xlsx` for a new table.
fn suggested_name(name: &str, untitled: bool, format: Format) -> String {
    let xlsm = Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("xlsm"));
    let ext = match format {
        // A macro workbook keeps its name, and so its macros (ADR 0010).
        Format::Xlsx if xlsm => "xlsm",
        Format::Xlsx => "xlsx",
        Format::Csv {
            delimiter: b'\t', ..
        } => "tsv",
        Format::Csv { .. } => "csv",
    };
    if untitled {
        return format!("{name}.{ext}");
    }
    let path = Path::new(name);
    let ours = ["csv", "tsv", "xlsx", "xlsm"];
    match path.extension().and_then(|e| e.to_str()) {
        Some(e) if ours.iter().any(|o| e.eq_ignore_ascii_case(o)) => {
            path.with_extension(ext).to_string_lossy().into_owned()
        }
        _ if format == Format::Xlsx => path.with_extension(ext).to_string_lossy().into_owned(),
        _ => name.to_owned(),
    }
}

/// The find bar (SRCH-1–3), over the status bar. Ctrl+F opens it on the text field;
/// typing searches as you type from the cursor, its own cell first (each keystroke cancels
/// the running search); Enter or Ctrl+G goes to the next match, Shift+Enter or
/// Ctrl+Shift+G to the previous one; Esc closes it. Searches the whole file or the
/// cursor's column, with or without matching case. The note counts the matches as they
/// stream in, then says which one the cursor is on ("12 of 2,725,768").
///
/// Ctrl+H (or the replace toggle) shows the replace row: Replace (or Enter there)
/// replaces the text in the match under the cursor and goes to the next; Replace All
/// changes every match shown as one undo step.
#[derive(Clone)]
struct FindBar(Rc<FindParts>);

struct FindParts {
    root: gtk::Revealer,
    entry: gtk::SearchEntry,
    case: gtk::CheckButton,
    scope: gtk::DropDown,
    note: gtk::Label,
    replacing: gtk::ToggleButton,
    replace_row: gtk::Box,
    with: gtk::Entry,
    grid: glib::WeakRef<GridView>,
    /// Hears what the bar does (`--bench-find`).
    trace: RefCell<Option<FindTrace>>,
}

type FindTrace = Box<dyn Fn(FindEvent)>;

/// What the find bar did, for `--bench-find`.
enum FindEvent<'a> {
    /// The text changed; whether that cancelled a running search.
    Key {
        text: &'a str,
        cancelled: bool,
    },
    Start(&'a str),
    Done(&'a str, grid_view::Found),
}

impl FindBar {
    fn new(grid: &GridView) -> Self {
        let entry = gtk::SearchEntry::builder()
            .placeholder_text("Find")
            .hexpand(true)
            .build();
        let button = |icon: &str, tip: &str| {
            let b = gtk::Button::from_icon_name(icon);
            b.add_css_class("flat");
            b.set_tooltip_text(Some(tip));
            b
        };
        let previous = button("go-up-symbolic", "Previous match (Shift+Enter)");
        let next = button("go-down-symbolic", "Next match (Enter)");
        let case = gtk::CheckButton::with_label("Match case");
        let scope = gtk::DropDown::from_strings(&["Whole file", "Current column"]);
        let note = gtk::Label::new(None);
        note.add_css_class("dim-label");
        // Room for "2,725,768 of 2,725,768", so the field doesn't jump as the count grows.
        note.set_width_chars(22);
        note.set_xalign(1.0);
        let close = button("window-close-symbolic", "Close (Esc)");
        let replacing = gtk::ToggleButton::builder()
            .icon_name("edit-find-replace-symbolic")
            .tooltip_text(format!("Replace ({})", keys::label(keys::Command::Replace)))
            .css_classes(["flat"])
            .build();
        let bar_row = || {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.set_margin_start(6);
            row.set_margin_end(6);
            row.set_margin_top(4);
            row.set_margin_bottom(4);
            row
        };
        let row = bar_row();
        for w in [
            entry.upcast_ref::<gtk::Widget>(),
            previous.upcast_ref(),
            next.upcast_ref(),
            replacing.upcast_ref(),
            case.upcast_ref(),
            scope.upcast_ref(),
            note.upcast_ref(),
            close.upcast_ref(),
        ] {
            row.append(w);
        }
        let with = gtk::Entry::builder()
            .placeholder_text("Replace with")
            .hexpand(true)
            .build();
        let replace = gtk::Button::with_label("Replace");
        let replace_all = gtk::Button::with_label("Replace All");
        let replace_row = bar_row();
        replace_row.set_margin_top(0);
        replace_row.append(&with);
        replace_row.append(&replace);
        replace_row.append(&replace_all);
        replace_row.set_visible(false);
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        rows.append(&row);
        rows.append(&replace_row);
        let root = gtk::Revealer::builder()
            .child(&rows)
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .build();
        let bar = Self(Rc::new(FindParts {
            root,
            entry,
            case,
            scope,
            note,
            replacing,
            replace_row,
            with,
            grid: grid.downgrade(),
            trace: RefCell::new(None),
        }));
        let p = &bar.0;
        // A keystroke stops the search for the text before it at once; the search for the
        // new text starts once typing pauses (`search-changed`).
        p.entry.connect_changed({
            let bar = bar.clone();
            move |entry| {
                let cancelled = bar.0.grid.upgrade().is_some_and(|g| g.cancel_search());
                let text = entry.text();
                bar.trace(FindEvent::Key {
                    text: &text,
                    cancelled,
                });
                if text.is_empty() {
                    bar.0.note.set_text("");
                }
            }
        });
        p.entry.connect_search_changed({
            let bar = bar.clone();
            move |_| bar.run(Step::Here)
        });
        p.entry.connect_activate({
            let bar = bar.clone();
            move |_| bar.run(Step::Next)
        });
        p.entry.connect_next_match({
            let bar = bar.clone();
            move |_| bar.run(Step::Next)
        });
        p.entry.connect_previous_match({
            let bar = bar.clone();
            move |_| bar.run(Step::Previous)
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed({
            let bar = bar.clone();
            move |_, key, _, mods| {
                use gtk::gdk::{Key, ModifierType};
                if matches!(key, Key::Return | Key::KP_Enter)
                    && mods.contains(ModifierType::SHIFT_MASK)
                {
                    bar.run(Step::Previous);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        });
        p.entry.add_controller(keys);
        previous.connect_clicked({
            let bar = bar.clone();
            move |_| bar.run(Step::Previous)
        });
        next.connect_clicked({
            let bar = bar.clone();
            move |_| bar.run(Step::Next)
        });
        p.case.connect_toggled({
            let bar = bar.clone();
            move |_| bar.run(Step::Here)
        });
        p.scope.connect_selected_notify({
            let bar = bar.clone();
            move |_| bar.run(Step::Here)
        });
        p.entry.connect_stop_search({
            let bar = bar.clone();
            move |_| bar.close()
        });
        close.connect_clicked({
            let bar = bar.clone();
            move |_| bar.close()
        });
        p.replacing.connect_toggled({
            let bar = bar.clone();
            move |t| bar.0.replace_row.set_visible(t.is_active())
        });
        p.with.connect_activate({
            let bar = bar.clone();
            move |_| bar.replace_one()
        });
        replace.connect_clicked({
            let bar = bar.clone();
            move |_| bar.replace_one()
        });
        replace_all.connect_clicked({
            let bar = bar.clone();
            move |_| bar.replace_all()
        });
        // Esc anywhere in the bar closes it: the replace field, a button, the toggle.
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed({
            let bar = bar.clone();
            move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    bar.close();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        });
        p.root.add_controller(keys);
        bar
    }

    fn trace(&self, event: FindEvent) {
        if let Some(t) = self.0.trace.borrow().as_ref() {
            t(event);
        }
    }

    /// Search for the bar's text: from the cursor's cell (`Here`), or the next or previous
    /// match.
    fn run(&self, step: Step) {
        let p = &self.0;
        let Some(grid) = p.grid.upgrade() else {
            return;
        };
        let text = p.entry.text().to_string();
        if text.is_empty() {
            grid.cancel_search();
            p.note.set_text("");
            return;
        }
        self.trace(FindEvent::Start(&text));
        let bar = self.clone();
        grid.find(
            &text.clone(),
            p.case.is_active(),
            p.scope.selected() == 1,
            step,
            move |found| {
                bar.show(found);
                bar.trace(FindEvent::Done(&text, found));
            },
        );
    }

    /// Say how a find went.
    fn show(&self, found: grid_view::Found) {
        use grid_view::Found;
        let note = match found {
            Found::Hit { ordinal, total } => FindNote::Hit { ordinal, total },
            Found::Skipped { ordinal, total } => FindNote::Skipped { ordinal, total },
            Found::Nothing => FindNote::Nothing,
            Found::NotReady => FindNote::NotReady,
            Found::Cancelled => return,
        };
        self.0.note.set_text(&status::find_note(note));
    }

    /// Replace in the match under the cursor and go to the next (or first go to a match).
    fn replace_one(&self) {
        let p = &self.0;
        let (Some(grid), text) = (p.grid.upgrade(), p.entry.text()) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        let bar = self.clone();
        grid.replace_one(
            &text,
            p.case.is_active(),
            p.scope.selected() == 1,
            &p.with.text(),
            move |found| bar.show(found),
        );
    }

    /// Replace every match shown, as one undo step.
    fn replace_all(&self) {
        let p = &self.0;
        let (Some(grid), text) = (p.grid.upgrade(), p.entry.text()) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        let bar = self.clone();
        grid.replace_all(
            &text,
            p.case.is_active(),
            p.scope.selected() == 1,
            &p.with.text(),
            move |done| {
                use grid_view::Replaced;
                let note = match done {
                    Replaced::Cells { replaced, skipped } => FindNote::Replaced {
                        cells: replaced,
                        skipped,
                    },
                    Replaced::TooMany(cells) => FindNote::TooMany { cells },
                    Replaced::Nothing => FindNote::Nothing,
                    Replaced::NotReady => FindNote::NotReady,
                    Replaced::Cancelled => return,
                };
                bar.0.note.set_text(&status::find_note(note));
            },
        );
    }

    /// The status tick: the count so far while matches are counted.
    fn tick(&self, grid: &GridView) {
        if let Some(found) = grid.search_found() {
            self.0
                .note
                .set_text(&status::find_note(FindNote::Counting { found }));
        }
    }

    /// Show the bar on the find field, with the replace row (Ctrl+H) or as it was
    /// (Ctrl+F).
    fn open(&self, replace: bool) {
        let p = &self.0;
        p.root.set_reveal_child(true);
        if replace {
            p.replacing.set_active(true);
        }
        p.entry.grab_focus();
        p.entry.select_region(0, -1);
    }

    fn close(&self) {
        self.0.root.set_reveal_child(false);
        if let Some(g) = self.0.grid.upgrade() {
            g.cancel_job();
            g.grab_focus();
        }
    }
}

/// The bar under the grid (APP-2): rows and indexing progress on the left, save state (with
/// a Cancel button while saving, SAVE-3) and the file's format on the right.
struct StatusBar {
    root: gtk::Box,
    rows: gtk::Label,
    save: gtk::Label,
    cancel: gtk::Button,
    format: gtk::Label,
}

impl StatusBar {
    fn new() -> Self {
        let label = || {
            let l = gtk::Label::new(None);
            l.add_css_class("caption");
            l
        };
        let (rows, save, format) = (label(), label(), label());
        rows.set_hexpand(true);
        rows.set_xalign(0.0);
        save.set_ellipsize(gtk::pango::EllipsizeMode::End);
        format.add_css_class("dim-label");
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(18)
            .margin_start(10)
            .margin_end(10)
            .margin_top(3)
            .margin_bottom(3)
            .build();
        let cancel = gtk::Button::with_label("Cancel");
        cancel.add_css_class("flat");
        cancel.set_tooltip_text(Some("Cancel the save (Esc)"));
        cancel.set_focus_on_click(false); // keep the keyboard in the grid
        cancel.set_visible(false);
        root.append(&rows);
        root.append(&save);
        root.append(&cancel);
        root.append(&format);
        Self {
            root,
            rows,
            save,
            cancel,
            format,
        }
    }

    /// Show the grid's current state; returns what is shown.
    fn update(&self, grid: &GridView, session: &Session, save: &SaveState) -> status::Status {
        let shown = status::status(&status::Facts {
            rows: grid.row_count(),
            of: grid.unfiltered_row_count(),
            indexing: grid.index_progress(),
            delimiter: grid.delimiter(),
            detected: session.choice.get() == DelimiterChoice::Auto,
            encoding: grid.encoding(),
            workbook: session.kind.get() == DocKind::Xlsx,
            edits: grid.edit_count(),
            job: session
                .import_share()
                .map(|share| ("Opening", share))
                .or_else(|| grid.job_progress()),
            save: match save {
                SaveState::Idle => status::SaveView::Idle,
                SaveState::Saving(r) if r.progress.is_checking() => status::SaveView::Checking,
                SaveState::Saving(r) => status::SaveView::Saving(
                    (r.progress.written() as f64 / r.estimate.max(1) as f64).min(0.99),
                ),
                SaveState::Saved(took) => status::SaveView::Saved(*took),
                SaveState::Cancelled => status::SaveView::Cancelled,
                SaveState::Failed(e) => status::SaveView::Failed(e),
            },
            file_changed: grid.file_changed(),
            clip: grid.clip_view(),
        });
        for (label, text) in [
            (&self.rows, &shown.rows),
            (&self.save, &shown.save),
            (&self.format, &shown.format),
        ] {
            if label.text() != text.as_str() {
                label.set_text(text);
            }
        }
        self.cancel
            .set_visible(matches!(save, SaveState::Saving(_)));
        shown
    }
}

/// Scroll autopilot for the GRID-1 acceptance test. Measures the interval between finished
/// frames (frame clock after-paint) and the frame's CPU time (before- to after-paint).
fn bench_scroll(app: &gtk::Application, grid: &GridView, vadj: &gtk::Adjustment, frames: u32) {
    let Some(clock) = grid.frame_clock() else {
        return;
    };
    #[derive(Default)]
    struct Rec {
        started: bool,
        paint_start: Option<Instant>,
        ends: Vec<Instant>,
        cpu_ms: Vec<f64>,
        peak_cache: usize,
    }
    let rec = Rc::new(RefCell::new(Rec::default()));
    clock.connect_before_paint({
        let rec = rec.clone();
        move |_| rec.borrow_mut().paint_start = Some(Instant::now())
    });
    clock.connect_after_paint({
        let (rec, grid) = (rec.clone(), grid.downgrade());
        move |_| {
            let mut r = rec.borrow_mut();
            if let (true, Some(t0), Some(g)) = (r.started, r.paint_start, grid.upgrade()) {
                r.ends.push(Instant::now());
                r.cpu_ms.push(t0.elapsed().as_secs_f64() * 1e3);
                r.peak_cache = r.peak_cache.max(g.cache_bytes());
            }
        }
    });

    let ticks = Cell::new(0u32);
    let app = app.clone();
    let vadj = vadj.clone();
    grid.add_tick_callback(move |grid, _| {
        // An .xlsx shows an empty workbook until its import is done (XLSX-1).
        if !grid.is_complete() || grid.is_loading() {
            return glib::ControlFlow::Continue;
        }
        let n = ticks.get();
        ticks.set(n + 1);
        if n == 0 {
            // Start mid-file, then let one second of frames settle before measuring.
            grid.update_adjustments();
            vadj.set_value(vadj.upper() / 2.0);
        } else if n == 60 {
            rec.borrow_mut().started = true;
        } else if n > 60 + frames {
            print_report(
                &rec.borrow().ends,
                &rec.borrow().cpu_ms,
                rec.borrow().peak_cache,
                grid,
                &vadj,
            );
            app.quit();
            return glib::ControlFlow::Break;
        }
        vadj.set_value(vadj.value() + 37.0);
        glib::ControlFlow::Continue
    });
}

/// Scrollbar-jump driver for the GRID-2 acceptance test. Each jump sets the vertical
/// adjustment, which is what dragging the scrollbar does, then waits for the first finished
/// frame (frame clock after-paint) whose top row is the target. Jumps are 300 ms apart so
/// each starts from an idle widget; the first target is `first`, the rest are random.
fn bench_jump(
    app: &gtk::Application,
    grid: &GridView,
    vadj: &gtk::Adjustment,
    first: u64,
    jumps: u32,
) {
    let Some(clock) = grid.frame_clock() else {
        return;
    };
    #[derive(Default)]
    struct Pending {
        target: u64,
        since: Option<Instant>,
        done_ms: Vec<f64>,
    }
    let pending = Rc::new(RefCell::new(Pending::default()));
    clock.connect_after_paint({
        let (pending, grid) = (pending.clone(), grid.downgrade());
        move |_| {
            let Some(grid) = grid.upgrade() else { return };
            let mut p = pending.borrow_mut();
            if let Some(t0) = p.since {
                if grid.painted_top_row() == Some(p.target) {
                    p.done_ms.push(t0.elapsed().as_secs_f64() * 1e3);
                    p.since = None;
                }
            }
        }
    });

    let (app, grid, vadj) = (app.clone(), grid.clone(), vadj.clone());
    let sent = Cell::new(0u32);
    let mut seed = 0x6A1D_0002_u64;
    glib::timeout_add_local(Duration::from_millis(300), move || {
        if !grid.is_complete() {
            return glib::ControlFlow::Continue;
        }
        let rows = grid.row_count();
        let n = sent.get();
        if n == jumps {
            let p = pending.borrow();
            let mut ms = p.done_ms.clone();
            ms.sort_by(f64::total_cmp);
            let pct = |q: f64| {
                ms.get(((q / 100.0) * (ms.len().max(1) - 1) as f64).round() as usize)
                    .copied()
                    .unwrap_or(f64::NAN)
            };
            println!(
                r#"{{"rows":{rows},"window":[{},{}],"jumps":{jumps},"landed":{},"first_row":{first},"first_jump_ms":{:.2},"p50":{:.2},"p99":{:.2},"max":{:.2}}}"#,
                grid.width(),
                grid.height(),
                p.done_ms.len(),
                p.done_ms.first().copied().unwrap_or(f64::NAN),
                pct(50.0),
                pct(99.0),
                ms.last().copied().unwrap_or(f64::NAN),
            );
            app.quit();
            return glib::ControlFlow::Break;
        }
        if pending.borrow().since.is_some() {
            return glib::ControlFlow::Continue; // previous jump has not painted yet
        }
        // Keep targets where the target row can sit at the top of the view.
        let last_top = rows.saturating_sub((vadj.page_size() / ROW_H).ceil() as u64);
        let target = if n == 0 {
            first.min(last_top)
        } else {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % last_top.max(1)
        };
        sent.set(n + 1);
        let mut p = pending.borrow_mut();
        p.target = target;
        p.since = Some(Instant::now());
        drop(p);
        vadj.set_value(target as f64 * ROW_H);
        glib::ControlFlow::Continue
    });
}

/// Save driver for the SAVE-3 acceptance test. Once indexed, pastes `edits` cells spread
/// down the file, saves, and while the save runs scrolls 37 px a frame, records the bytes
/// written each frame, and measures how late a 10 ms main-loop timer fires. Prints
/// `SAVE {json}` once the save is done and quits.
fn bench_save(
    app: &gtk::Application,
    grid: &GridView,
    session: &Rc<Session>,
    save: &Rc<RefCell<SaveState>>,
    vadj: &gtk::Adjustment,
    edits: u32,
) {
    let Some(clock) = grid.frame_clock() else {
        return;
    };
    #[derive(Default)]
    struct Rec {
        saving: bool,
        paint_start: Option<Instant>,
        ends: Vec<Instant>,
        cpu_ms: Vec<f64>,
        written: Vec<u64>,
        last_timer: Option<Instant>,
        late_ms: Vec<f64>,
    }
    let rec = Rc::new(RefCell::new(Rec::default()));
    clock.connect_before_paint({
        let rec = rec.clone();
        move |_| rec.borrow_mut().paint_start = Some(Instant::now())
    });
    clock.connect_after_paint({
        let rec = rec.clone();
        move |_| {
            let mut r = rec.borrow_mut();
            if let (true, Some(t0)) = (r.saving, r.paint_start) {
                r.ends.push(Instant::now());
                r.cpu_ms.push(t0.elapsed().as_secs_f64() * 1e3);
            }
        }
    });
    glib::timeout_add_local(Duration::from_millis(10), {
        let rec = rec.clone();
        move || {
            let mut r = rec.borrow_mut();
            let now = Instant::now();
            if let (true, Some(last)) = (r.saving, r.last_timer) {
                let late = (now - last).as_secs_f64() * 1e3 - 10.0;
                r.late_ms.push(late.max(0.0));
            }
            r.last_timer = Some(now);
            glib::ControlFlow::Continue
        }
    });

    // The edits are pasted in one frame and the save starts in the next, so the paste work
    // is not counted against the save; measuring starts with the save.
    let rows = Cell::new(None);
    let (app, session, save, vadj) = (app.clone(), session.clone(), save.clone(), vadj.clone());
    grid.add_tick_callback(move |grid, _| {
        let Some(total) = rows.get() else {
            if !grid.is_complete() {
                return glib::ControlFlow::Continue;
            }
            rows.set(Some(grid.row_count()));
            let step = grid.row_count() / u64::from(edits);
            for i in 0..edits {
                let cell = grid::Cell {
                    row: u64::from(i) * step,
                    col: i % 8,
                };
                grid.paste_at(cell, "edited");
            }
            return glib::ControlFlow::Continue;
        };
        if !rec.borrow().saving {
            let mut r = rec.borrow_mut();
            r.saving = true;
            r.last_timer = None;
            drop(r);
            start_save(grid, &session, &save, None, false);
        }
        let line = match &*save.borrow() {
            SaveState::Saving(s) => {
                rec.borrow_mut().written.push(s.progress.written());
                vadj.set_value(vadj.value() + 37.0);
                return glib::ControlFlow::Continue;
            }
            SaveState::Saved(took) => {
                let mut r = rec.borrow_mut();
                r.saving = false;
                let mut written = r.written.clone();
                written.retain(|&w| w > 0);
                written.dedup();
                format!(
                    r#"{{"rows":{},"edits":{edits},"save_ms":{},"progress_samples":{},"loop_late_ms":{},{}}}"#,
                    total,
                    took.as_millis(),
                    written.len(),
                    summary(&mut r.late_ms),
                    frame_stats(grid, &r.ends, &r.cpu_ms),
                )
            }
            SaveState::Failed(e) => format!(r#"{{"error":{e:?}}}"#),
            SaveState::Cancelled => r#"{"error":"the save was cancelled"}"#.to_owned(),
            SaveState::Idle => r#"{"error":"the save did not start"}"#.to_owned(),
        };
        println!("SAVE {line}");
        app.quit();
        glib::ControlFlow::Break
    });
}

/// `{"p50":..,"p99":..,"max":..}` of `xs` (sorted in place).
fn summary(xs: &mut [f64]) -> String {
    xs.sort_by(f64::total_cmp);
    let p = |q: f64| {
        xs.get(((q / 100.0) * (xs.len().max(1) - 1) as f64).round() as usize)
            .copied()
            .unwrap_or(0.0)
    };
    format!(
        r#"{{"p50":{:.2},"p99":{:.2},"max":{:.2}}}"#,
        p(50.0),
        p(99.0),
        xs.last().copied().unwrap_or(0.0)
    )
}

/// JSON fields for frames finished at `ends` taking `cpu_ms` each: the display's refresh
/// rate, frame count and rate, frames that missed it, and interval and CPU percentiles.
fn frame_stats(grid: &GridView, ends: &[Instant], cpu_ms: &[f64]) -> String {
    let mut intervals: Vec<f64> = ends
        .windows(2)
        .map(|w| (w[1] - w[0]).as_secs_f64() * 1e3)
        .collect();
    let mut cpu = cpu_ms.to_vec();
    let refresh = grid
        .native()
        .and_then(|n| n.surface())
        .and_then(|s| s.display().monitor_at_surface(&s))
        .map_or(60.0, |m| m.refresh_rate() as f64 / 1000.0);
    let budget = 1000.0 / refresh;
    let missed = intervals.iter().filter(|&&i| i > budget * 1.5).count();
    let mean_fps = if intervals.is_empty() {
        0.0
    } else {
        1000.0 * intervals.len() as f64 / intervals.iter().sum::<f64>()
    };
    format!(
        r#""refresh_hz":{refresh:.1},"frames":{},"mean_fps":{mean_fps:.1},"missed_frames":{missed},"interval_ms":{},"frame_cpu_ms":{}"#,
        intervals.len(),
        summary(&mut intervals),
        summary(&mut cpu),
    )
}

fn print_report(
    ends: &[Instant],
    cpu_ms: &[f64],
    peak_cache: usize,
    grid: &GridView,
    vadj: &gtk::Adjustment,
) {
    println!(
        r#"{{"rows":{},"window":[{},{}],{},"grid_cache_peak_bytes":{peak_cache},"rss_anon_kib":{},"end_row":{}}}"#,
        grid.row_count(),
        grid.width(),
        grid.height(),
        frame_stats(grid, ends, cpu_ms),
        proc_status_kib("RssAnon:"),
        (vadj.value() / ROW_H) as u64,
    );
}

/// A `kB` field of /proc/self/status, e.g. `RssAnon:` or `VmHWM:`.
fn proc_status_kib(key: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with(key))?
                .split_whitespace()
                .nth(1)?
                .parse()
                .ok()
        })
        .unwrap_or(0)
}
