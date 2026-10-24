//! Windows: one per image, with the app's menu bar, the tool column, the canvas, and the
//! colour row and status bar below; opening files (the command line and File → Open), and
//! crashed images again to offer their unsaved pixels back ([`crate::recovery`]); the New
//! Image dialog; and the benchmarks (`--bench-stroke`, `--bench-open`). Saving is
//! [`crate::saving`]'s.

use crate::canvas::Canvas;
use crate::colours;
use crate::image_menu;
use crate::keys::{self, Command, Focus};
use crate::recent;
use crate::recovery::{self, Crashed, Recovered};
use crate::saving::{Opened, Saver};
use crate::settings::{self, Background};
use crate::text;
use crate::tools::{self, Settings, Tool};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use image_model::{Color, Image, Style, MAX_SIDE};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Instant;
use suite_file::canonical;
use suite_ui::keys::Primary;

/// `etch`'s first image (always white: the remembered size and background are New
/// Image's).
pub const DEFAULT_SIZE: (u32, u32) = (1024, 768);
/// The benchmark's image: 8K.
const BENCH_SIZE: (u32, u32) = (7680, 4320);

thread_local! {
    /// What New Image offers: the size and background last used, read from the settings
    /// file the first time.
    static NEW: Cell<Option<settings::Settings>> = const { Cell::new(None) };
    /// Each window's controls, for New Image's background colour: the active window's.
    static CONTROLS: RefCell<Vec<(glib::WeakRef<gtk::ApplicationWindow>, Weak<Controls>)>> =
        const { RefCell::new(Vec::new()) };
    /// The windows of the files open, by canonical path: opening one again shows its window.
    static OPEN: RefCell<Vec<(PathBuf, glib::WeakRef<gtk::ApplicationWindow>)>> =
        const { RefCell::new(Vec::new()) };
}

/// What a window runs once its image shows.
#[derive(Clone, Copy)]
enum Bench {
    /// `--bench-stroke FRAMES`.
    Stroke(u32),
    /// `--bench-open FILE`: the process started at `start`; reading and decoding the file
    /// took `read_ms`.
    Open { start: Instant, read_ms: f64 },
}

/// The app's actions, shortcuts, menu bar (every window shows it on Linux and Windows,
/// ending with Help: Update Automatically and About Etch; on macOS it's the system's,
/// after GTK's app menu with About, Update Automatically, and Quit), and style.
pub fn install_app_actions(app: &gtk::Application) {
    colours::install_style();
    let new = gio::ActionEntry::builder("new")
        .activate(|app: &gtk::Application, _, _| ask_new_image(app))
        .build();
    let open = gio::ActionEntry::builder("open")
        .activate(|app: &gtk::Application, _, _| choose_and_open(app))
        .build();
    // File → Open Recent's items: the path to open.
    let open_recent = gio::ActionEntry::builder("open-recent")
        .parameter_type(Some(glib::VariantTy::STRING))
        .activate(|app: &gtk::Application, _, target| {
            if let Some(path) = target.and_then(|t| t.str()) {
                crate::window::open(app, Path::new(path), None);
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
    app.add_action_entries([new, open, open_recent, quit]);
    let update = suite_ui::chrome::AutoUpdate {
        on: suite_update::automatic(crate::APP),
        keep: |on| suite_update::set_automatic(crate::APP, on),
    };
    suite_ui::chrome::install(app, crate::APP, env!("CARGO_PKG_VERSION"), update);
    for (action, accels) in keys::accels(Primary::HERE) {
        let accels: Vec<&str> = accels.iter().map(String::as_str).collect();
        app.set_accels_for_action(action, &accels);
    }

    // Each item shows its key from the key table (the window's key handler's too).
    let append = |menu: &gio::Menu, items: &[(&str, &str)]| {
        for &(label, action) in items {
            menu.append_item(&keys::menu_item(label, action));
        }
    };
    let menu_of = |items: &[(&str, &str)]| {
        let menu = gio::Menu::new();
        append(&menu, items);
        menu
    };
    let file = menu_of(&[("_New Image…", "app.new"), ("_Open…", "app.open")]);
    file.append_submenu(Some("Open _Recent"), &recent::menu());
    append(
        &file,
        &[
            ("_Save", "win.save"),
            ("Save _As…", "win.save-as"),
            ("_Export…", "win.export"),
            ("_Close", "window.close"),
        ],
    );
    // macOS has Quit in the app menu; Windows' File ends with Exit.
    if let Some(exit) = suite_ui::chrome::exit_section() {
        file.append_section(None, &exit);
    } else if !suite_ui::chrome::MENU_BAR {
        append(&file, &[("_Quit", "app.quit")]);
    }
    let edit = menu_of(&[("_Undo", "win.undo"), ("_Redo", "win.redo")]);
    edit.append_section(
        None,
        &menu_of(&[
            ("Cu_t", "win.cut"),
            ("_Copy", "win.copy"),
            ("_Paste", "win.paste"),
        ]),
    );
    edit.append_section(
        None,
        &menu_of(&[
            ("Select _All", "win.select-all"),
            ("_Deselect", "win.deselect"),
            ("De_lete", "win.delete"),
        ]),
    );
    let view = menu_of(&[
        ("Zoom _In", "win.zoom-in"),
        ("Zoom _Out", "win.zoom-out"),
        ("_Actual Size", "win.actual-size"),
        ("_Fit to Window", "win.fit"),
    ]);
    let labels = ["File", "Edit", "View", "Image"].map(keys::mnemonic);
    let menu = suite_ui::chrome::menu_bar(
        crate::APP,
        &[
            (&labels[0], &file),
            (&labels[1], &edit),
            (&labels[2], &view),
            (&labels[3], &image_menu::menu()),
        ],
    );
    // Linux's menu bar ends with Help too (Windows' has it from `menu_bar`).
    if !suite_ui::chrome::MENU_BAR {
        menu.append_item(&suite_ui::chrome::help_menu(crate::APP));
    }
    app.set_menubar(Some(&menu));
}

/// A window with a new white image of `size`. The pixels are allocated off the UI thread:
/// a large image takes a moment.
pub fn open_blank(app: &gtk::Application, size: (u32, u32)) {
    open_new(app, size, Color::WHITE, None, None);
}

/// `--bench-stroke FRAMES`: a window with an 8K image at 100%, then the benchmark.
pub fn open_bench(app: &gtk::Application, frames: u32) {
    open_new(app, BENCH_SIZE, Color::WHITE, Some(frames), None);
}

/// Crash recovery: the image of `crashed` opened again, its window offering the journal's
/// pixels back: its file as it is now, or a new white Untitled image of the journal's
/// size.
pub fn reopen(app: &gtk::Application, crashed: Crashed) {
    match crashed.header.path.clone() {
        Some(path) => open_with(app, &path, None, Some(crashed)),
        None => {
            let size = crashed.header.size;
            open_new(app, size, Color::WHITE, None, Some(crashed));
        }
    }
}

/// A window with a new image of `size` all `color`, then `bench` or offering back the
/// pixels of `crashed`, read with it.
fn open_new(
    app: &gtk::Application,
    (w, h): (u32, u32),
    color: Color,
    bench: Option<u32>,
    crashed: Option<Crashed>,
) {
    let app = app.clone();
    // Running until the window shows, though no window may be open yet.
    let hold = app.hold();
    glib::spawn_future_local(async move {
        let _hold = hold;
        let made = gio::spawn_blocking(move || {
            // An Untitled image's journal goes onto it whatever it holds.
            Image::new(w, h, color)
                .map(|image| (image, crashed.and_then(|c| Recovered::read(c, None))))
        })
        .await;
        match made {
            Ok(Ok((image, recovered))) => {
                let window = new_window(&app, &untitled(&app));
                show(
                    &app,
                    &window,
                    image,
                    None,
                    bench.map(Bench::Stroke),
                    recovered,
                );
            }
            Ok(Err(e)) => alert(&app, "Cannot make the image", &e.to_string()).await,
            Err(_) => {
                alert(
                    &app,
                    "Cannot make the image",
                    "Making it failed unexpectedly.",
                )
                .await
            }
        }
    });
}

/// "Images" (PNG, JPEG, BMP) and "All files", the first as the default.
fn file_filters() -> (gio::ListStore, gtk::FileFilter) {
    let images = gtk::FileFilter::new();
    images.set_name(Some("Images (PNG, JPEG, BMP)"));
    for mime in ["image/png", "image/jpeg", "image/bmp"] {
        images.add_mime_type(mime);
    }
    for suffix in ["png", "jpg", "jpeg", "bmp"] {
        images.add_suffix(suffix);
    }
    let all = gtk::FileFilter::new();
    all.set_name(Some("All files"));
    all.add_pattern("*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&images);
    filters.append(&all);
    (filters, images)
}

/// File → Open (Ctrl+O): the desktop's file dialog (xdg-desktop-portal through
/// `gtk::FileDialog`, GTK's own without a portal), filtered to images. The image opens in
/// a window of its own, as in the suite's other apps: an Untitled window stays as it is.
fn choose_and_open(app: &gtk::Application) {
    let (filters, images) = file_filters();
    let dialog = gtk::FileDialog::builder()
        .title("Open")
        .modal(true)
        .filters(&filters)
        .default_filter(&images)
        .build();
    let (app, parent) = (app.clone(), app.active_window());
    glib::spawn_future_local(async move {
        match dialog.open_future(parent.as_ref()).await {
            Ok(file) => open_file(&app, &file),
            Err(e) if e.matches(gtk::DialogError::Dismissed) => {}
            Err(e) => alert(&app, "Cannot show the file dialog", &e.to_string()).await,
        }
    });
}

/// Open `file` as [`open`] does; one that isn't local (no path) alerts instead.
pub fn open_file(app: &gtk::Application, file: &gio::File) {
    match file.path() {
        Some(path) => open(app, &path, None),
        None => {
            let (app, uri) = (app.clone(), file.uri());
            glib::spawn_future_local(async move {
                alert(
                    &app,
                    &format!("Cannot open {uri}"),
                    "Only local files can be opened.",
                )
                .await
            });
        }
    }
}

/// Open the image file at `path` in its own window: its existing one, or a new one titled
/// with the file's name that says "Opening…" while the file is read and decoded off the UI
/// thread. A file that can't be opened gives an alert naming it and the reason, and its
/// window closes. The file goes to the top of the recent files, or, when it can't be
/// opened, leaves them. A crashed session's unsaved pixels for the file are offered back
/// ([`crate::recovery`]). `bench`: `--bench-open`, started at that instant (the recent
/// files and crashed sessions left alone).
pub fn open(app: &gtk::Application, path: &Path, bench: Option<Instant>) {
    open_with(app, path, bench, None);
}

/// [`open`], offering back the pixels of `crashed` (else of the file's own crashed
/// session, if it has one).
fn open_with(
    app: &gtk::Application,
    path: &Path,
    bench: Option<Instant>,
    crashed: Option<Crashed>,
) {
    if let Some(window) = window_for(path) {
        window.present();
        recent::add(path);
        return;
    }
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let window = new_window(app, &name);
    let opening = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    opening.set_halign(gtk::Align::Center);
    opening.set_valign(gtk::Align::Center);
    let spinner = gtk::Spinner::new();
    spinner.start();
    opening.append(&spinner);
    opening.append(&gtk::Label::new(Some("Opening…")));
    window.set_child(Some(&suite_ui::notice::host(&opening)));
    OPEN.with_borrow_mut(|open| open.push((canonical(path), window.downgrade())));
    window.present();

    let (app, path) = (app.clone(), path.to_owned());
    glib::spawn_future_local(async move {
        let p = path.clone();
        let read = gio::spawn_blocking(move || {
            let t = Instant::now();
            let identity = recovery::identity(&p);
            let read = image_io::read(&p);
            let took = t.elapsed();
            let crashed = match crashed {
                Some(c) => Some(c),
                None if bench.is_none() => recovery::crashed_for(&canonical(&p)),
                None => None,
            };
            // Read, the journal's pixels go onto the file if it's as it was; else the
            // journal is only kept or discarded.
            let recovery = match &read {
                Ok(_) => Ok(crashed.and_then(|c| Recovered::read(c, identity))),
                Err(_) => Err(crashed.filter(|c| c.journal.exists())),
            };
            (read, took, identity, recovery)
        })
        .await;
        let (error, crashed) = match read {
            Ok((Ok((image, format)), took, identity, recovery)) => {
                let read_ms = took.as_secs_f64() * 1e3;
                let (w, h) = (image.width(), image.height());
                tracing::info!("opened {}: {w}×{h} in {read_ms:.0} ms", path.display());
                let bench = bench.map(|start| Bench::Open { start, read_ms });
                if bench.is_none() {
                    recent::add(&path);
                }
                let file = Opened {
                    path,
                    format,
                    identity,
                };
                let recovered = recovery.ok().flatten();
                show(&app, &window, image, Some(file), bench, recovered);
                return;
            }
            Ok((Err(e), _, _, recovery)) => (e.to_string(), recovery.err().flatten()),
            Err(_) => ("Reading it failed unexpectedly.".to_owned(), None),
        };
        eprintln!("etch: cannot open {}: {error}", path.display());
        if bench.is_some() {
            std::process::exit(1);
        }
        recent::remove(&path);
        let _ = gtk::AlertDialog::builder()
            .message(format!("Cannot open “{name}”"))
            .detail(&error)
            .buttons(["Close"])
            .modal(true)
            .build()
            .choose_future(Some(&window))
            .await;
        if let Some(crashed) = crashed {
            recovery::changed(&window, &crashed).await;
        }
        window.close();
    });
}

/// The window `path` is open in, if any.
fn window_for(path: &Path) -> Option<gtk::ApplicationWindow> {
    let path = canonical(path);
    OPEN.with_borrow_mut(|open| {
        open.retain(|(_, w)| w.upgrade().is_some());
        open.iter()
            .find(|(p, _)| *p == path)
            .and_then(|(_, w)| w.upgrade())
    })
}

/// `window` now shows the file at `path` (saved as it): opening that file shows it.
pub fn moved(window: &gtk::ApplicationWindow, path: &Path) {
    let path = canonical(path);
    OPEN.with_borrow_mut(|open| {
        open.retain(|(_, w)| w.upgrade().is_some_and(|w| w != *window));
        open.push((path, window.downgrade()));
    });
}

/// An empty window titled `title`, with the menu bar, taking dropped files.
fn new_window(app: &gtk::Application, title: &str) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(title)
        .default_width(1100)
        .default_height(800)
        .show_menubar(true)
        .build();
    // Another etch may have changed the recent files meanwhile.
    window.connect_is_active_notify(|window| {
        if window.is_active() {
            recent::refresh();
        }
    });
    accept_drops(app, &window);
    window
}

/// Files dropped on `window` (from a file manager) open as Open does, each in its own
/// window: an image already open brings its window forward, and a file that isn't one
/// alerts with the reason. They open once the drop is done, so the drag's source isn't
/// kept waiting on the alerts.
fn accept_drops(app: &gtk::Application, window: &gtk::ApplicationWindow) {
    let target = suite_ui::launch::file_drop_target();
    target.connect_drop({
        let app = app.downgrade();
        move |_, value, _, _| {
            let (Some(app), Ok(files)) = (app.upgrade(), value.get::<gdk::FileList>()) else {
                return false;
            };
            let files = files.files();
            if files.is_empty() {
                return false;
            }
            glib::idle_add_local_once(move || {
                for file in &files {
                    open_file(&app, file);
                }
            });
            true
        }
    });
    window.add_controller(target);
}

async fn alert(app: &gtk::Application, message: &str, detail: &str) {
    eprintln!("etch: {message}: {detail}");
    let _ = gtk::AlertDialog::builder()
        .message(message)
        .detail(detail)
        .buttons(["Close"])
        .modal(true)
        .build()
        .choose_future(app.active_window().as_ref())
        .await;
}

/// "Untitled", or "Untitled N" with the first N no other window has (its title marked
/// "• " or not).
fn untitled(app: &gtk::Application) -> String {
    let titles: Vec<_> = app
        .windows()
        .iter()
        .filter_map(|w| w.title())
        .map(|t| t.trim_start_matches("• ").to_owned())
        .collect();
    (1..)
        .map(|n| match n {
            1 => "Untitled".to_owned(),
            n => format!("Untitled {n}"),
        })
        .find(|t| !titles.iter().any(|w| w == t))
        .unwrap_or_default()
}

/// The status bar's labels: pointer position, the selection's size, zoom, image size.
struct Status {
    position: gtk::Label,
    selection: gtk::Label,
    zoom: gtk::Label,
    size: gtk::Label,
}

impl Status {
    fn new() -> (gtk::Box, Status) {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 24);
        bar.set_margin_start(12);
        bar.set_margin_end(12);
        bar.set_margin_top(4);
        bar.set_margin_bottom(4);
        let label = |expand: bool| {
            let l = gtk::Label::new(None);
            l.set_xalign(0.0);
            l.set_hexpand(expand);
            bar.append(&l);
            l
        };
        let status = Status {
            position: label(true),
            selection: label(false),
            zoom: label(false),
            size: label(false),
        };
        (bar, status)
    }

    fn show(&self, canvas: &Canvas) {
        self.position.set_text(
            &canvas
                .pointer_pixel()
                .map(|(x, y)| format!("x: {x}  y: {y}"))
                .unwrap_or_default(),
        );
        self.selection.set_text(
            &canvas
                .selection_size()
                .map(|(w, h)| format!("Selection: {w}×{h}"))
                .unwrap_or_default(),
        );
        if let Some(v) = canvas.view() {
            self.zoom.set_text(&v.percent());
        }
        let (w, h) = canvas.image_size();
        self.size.set_text(&format!("{w}×{h}"));
    }
}

/// What the tool column, the colour row, the size, and the keys set, and the widgets that
/// show it: the canvas acts with the current tool.
struct Controls {
    settings: Cell<Settings>,
    canvas: Canvas,
    indicator: colours::Indicator,
    /// The foreground colour in hex, which typing sets (while it has the keys, what's
    /// typed stays as it is).
    hex: gtk::Entry,
    /// The size's label, field, and unit, shown while the tool has a size.
    sizing: gtk::Box,
    size: gtk::SpinButton,
    /// The fill's tolerance, its label and field, shown while the fill is the tool.
    tolerating: gtk::Box,
    tolerance: gtk::SpinButton,
    /// The brush's Hard and Soft buttons, shown while the brush is the tool.
    edge: gtk::Box,
    hard: gtk::ToggleButton,
    soft: gtk::ToggleButton,
    /// The rectangle's and ellipse's Outline, Filled, and Both buttons ([`STYLES`]), shown
    /// while one of them is the tool.
    style: gtk::Box,
    styles: [gtk::ToggleButton; 3],
    /// The text's font family list, Bold, and Italic, shown while text is the tool.
    font: gtk::Box,
    families: &'static [String],
    family: gtk::DropDown,
    bold: gtk::ToggleButton,
    italic: gtk::ToggleButton,
    /// Set while the controls show the settings: their changes then aren't the user's.
    showing: Cell<bool>,
}

/// The styles of the style buttons, in order.
const STYLES: [Style; 3] = [Style::Outline, Style::Filled, Style::Both];

impl Controls {
    fn update(&self, f: impl FnOnce(&mut Settings)) {
        let mut s = self.settings.get();
        f(&mut s);
        self.settings.set(s);
        self.show();
    }

    /// A change the user made in a control: applied unless the controls are showing the
    /// settings.
    fn changed(&self, f: impl FnOnce(&mut Settings)) {
        if !self.showing.get() {
            self.update(f);
        }
    }

    fn show(&self) {
        let s = self.settings.get();
        self.showing.set(true);
        self.canvas.set_action(s.action());
        self.indicator.set(s.foreground, s.background);
        if !self.hex_has_keys() {
            self.show_hex(s.foreground);
        }
        self.sizing.set_visible(s.width().is_some());
        let sizes = s.sizes();
        self.size
            .set_range(*sizes.start() as f64, *sizes.end() as f64);
        if let Some(w) = s.width().filter(|&w| self.size.value() as u32 != w) {
            self.size.set_value(w as f64);
        }
        self.tolerating.set_visible(s.tool == Tool::Fill);
        if self.tolerance.value() as u8 != s.tolerance {
            self.tolerance.set_value(s.tolerance as f64);
        }
        self.edge.set_visible(s.tool == Tool::Brush);
        let edge = if s.soft { &self.soft } else { &self.hard };
        if !edge.is_active() {
            edge.set_active(true);
        }
        self.style.set_visible(s.has_style());
        let style = STYLES
            .iter()
            .position(|&t| t == s.style)
            .map(|i| &self.styles[i]);
        if let Some(b) = style.filter(|b| !b.is_active()) {
            b.set_active(true);
        }
        self.font.set_visible(s.tool == Tool::Text);
        if let Some(i) = self.families.iter().position(|f| f == s.family) {
            if self.family.selected() != i as u32 {
                self.family.set_selected(i as u32);
            }
        }
        self.bold.set_active(s.bold);
        self.italic.set_active(s.italic);
        self.showing.set(false);
    }

    /// After a control changed the text's look: the keys go back to the text being typed.
    fn back_to_text(&self) {
        if self.canvas.typing() {
            let canvas = self.canvas.clone();
            // Once the event is handled: a list that closes gives the keys to its button.
            glib::idle_add_local_once(move || {
                if canvas.typing() {
                    canvas.grab_focus();
                }
            });
        }
    }

    /// Whether the hex field is where the window's keys go (even while another window has
    /// them: switching away and back keeps what's typed).
    fn hex_has_keys(&self) -> bool {
        let window = self.hex.root().and_downcast::<gtk::Window>();
        window
            .and_then(|w| GtkWindowExt::focus(&w))
            .is_some_and(|f| f.is_ancestor(&self.hex))
    }

    /// The hex field shows `colour`, valid.
    fn show_hex(&self, colour: Color) {
        self.hex.set_text(&colours::hex(colour));
        self.hex.remove_css_class("error");
        self.hex.update_state(&[gtk::accessible::State::Invalid(
            gtk::AccessibleInvalidState::False,
        )]);
    }

    /// Enter in the hex field: a colour sets the foreground and gives the keys back to the
    /// image; anything else is marked invalid and changes nothing.
    fn hex_entered(&self) {
        match colours::parse_hex(&self.hex.text()) {
            Some(colour) => {
                self.update(|s| s.foreground = colour);
                self.show_hex(colour);
                self.canvas.grab_focus();
            }
            None => {
                self.hex.add_css_class("error");
                self.hex.update_state(&[gtk::accessible::State::Invalid(
                    gtk::AccessibleInvalidState::True,
                )]);
                self.hex.error_bell();
            }
        }
    }

    /// GTK's colour chooser, alpha included, for the foreground colour (`background`: the
    /// background), offering the colour it is.
    fn choose(self: &Rc<Self>, background: bool) {
        let s = self.settings.get();
        let (title, colour) = match background {
            true => ("Background Colour", s.background),
            false => ("Foreground Colour", s.foreground),
        };
        let dialog = gtk::ColorDialog::builder()
            .title(title)
            .modal(true)
            .with_alpha(true)
            .build();
        let parent = self.canvas.root().and_downcast::<gtk::Window>();
        let controls = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let chosen = dialog
                .choose_rgba_future(parent.as_ref(), Some(&colours::rgba(colour)))
                .await;
            let Some(controls) = controls.upgrade() else {
                return;
            };
            match chosen {
                Ok(c) => {
                    let c = colours::from_rgba(&c);
                    controls.update(|s| match background {
                        true => s.background = c,
                        false => s.foreground = c,
                    });
                }
                Err(e) if e.matches(gtk::DialogError::Dismissed) => {}
                Err(e) => tracing::warn!("cannot show the colour chooser: {e}"),
            }
            controls.back_to_text();
        });
    }
}

/// A choice of a few, labelled `label`: linked toggle buttons, one on, each with its
/// name and tooltip.
fn choice<const N: usize>(
    label: &str,
    options: [(&str, &str); N],
) -> (gtk::Box, [gtk::ToggleButton; N]) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.add_css_class("linked");
    row.set_valign(gtk::Align::Center);
    let buttons = options.map(|(name, tip)| {
        let b = gtk::ToggleButton::with_label(name);
        b.set_tooltip_text(Some(tip));
        b.set_focusable(false);
        row.append(&b);
        b
    });
    for b in &buttons[1..] {
        b.set_group(Some(&buttons[0]));
    }
    row.update_property(&[gtk::accessible::Property::Label(label)]);
    (row, buttons)
}

/// A toggle button showing `markup`, named `name` (for screen readers), with `tip`.
fn toggle(markup: &str, name: &str, tip: &str) -> gtk::ToggleButton {
    let label = gtk::Label::new(None);
    label.set_markup(markup);
    let b = gtk::ToggleButton::new();
    b.set_child(Some(&label));
    b.set_tooltip_text(Some(tip));
    b.set_focusable(false);
    b.set_valign(gtk::Align::Center);
    b.update_property(&[gtk::accessible::Property::Label(name)]);
    b
}

/// The text's font: a family list (searched by typing), Bold, and Italic.
fn font_controls(context: &gtk::pango::Context) -> (gtk::Box, &'static [String], gtk::DropDown) {
    let families = text::families(context);
    let names: Vec<&str> = families.iter().map(String::as_str).collect();
    let string = gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        None::<gtk::Expression>,
        "string",
    );
    let family = gtk::DropDown::new(Some(gtk::StringList::new(&names)), Some(string));
    family.set_enable_search(true);
    family.set_search_match_mode(gtk::StringFilterMatchMode::Substring);
    family.set_valign(gtk::Align::Center);
    family.set_size_request(150, -1);
    family.set_tooltip_text(Some("Font (type to search)"));
    // A drop-down is named by the item it shows: its description says what it is.
    family.update_property(&[
        gtk::accessible::Property::Label("Font"),
        gtk::accessible::Property::Description("Font"),
    ]);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&family);
    (row, families, family)
}

/// The hex field (the foreground colour, typed: Enter sets it; Alt+H, its label's
/// mnemonic, puts the keys in it) and the Custom… button, which opens GTK's colour chooser
/// for the foreground, a right-click for the background.
fn hex_controls(controls: &Rc<Controls>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_valign(gtk::Align::Center);
    let hex = &controls.hex;
    let label = gtk::Label::with_mnemonic(&keys::mnemonic("Hex"));
    label.set_mnemonic_widget(Some(hex));
    row.append(&label);
    hex.set_width_chars(9);
    hex.set_max_width_chars(9);
    hex.set_tooltip_text(Some(
        "The foreground colour in hex: #RGB, #RRGGBB, or #RRGGBBAA with alpha (Enter sets it)",
    ));
    hex.update_property(&[gtk::accessible::Property::Label("Hex")]);
    hex.connect_activate({
        let controls = Rc::downgrade(controls);
        move |_| {
            if let Some(c) = controls.upgrade() {
                c.hex_entered();
            }
        }
    });
    row.append(hex);
    let custom = gtk::Button::with_label("Custom…");
    custom.set_tooltip_text(Some(
        "Choose the foreground colour, alpha too (right-click: the background)",
    ));
    custom.set_focusable(false);
    custom.connect_clicked({
        let controls = Rc::downgrade(controls);
        move |_| {
            if let Some(c) = controls.upgrade() {
                c.choose(false);
            }
        }
    });
    let right = gtk::GestureClick::new();
    right.set_button(gdk::BUTTON_SECONDARY);
    right.connect_pressed({
        let controls = Rc::downgrade(controls);
        move |_, _, _, _| {
            if let Some(c) = controls.upgrade() {
                c.choose(true);
            }
        }
    });
    custom.add_controller(right);
    row.append(&custom);
    row
}

/// The colour row and the size, left of the status: the colours, their hex and chooser,
/// the palette, the current tool's size, the text's font, the brush's edge, and the
/// shapes' style.
fn colour_row(controls: &Rc<Controls>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.append(&controls.indicator);
    row.append(&hex_controls(controls));
    let pick: Rc<dyn Fn(Color, bool)> = {
        let controls = Rc::downgrade(controls);
        Rc::new(move |colour, background| {
            if let Some(c) = controls.upgrade() {
                c.update(|s| match background {
                    true => s.background = colour,
                    false => s.foreground = colour,
                });
            }
        })
    };
    let palette = colours::palette(pick);
    palette.set_valign(gtk::Align::Center);
    row.append(&palette);
    let label = gtk::Label::with_mnemonic(&keys::mnemonic("Size"));
    label.set_mnemonic_widget(Some(&controls.size));
    controls.sizing.append(&label);
    controls.size.set_valign(gtk::Align::Center);
    controls
        .size
        .update_property(&[gtk::accessible::Property::Label("Size")]);
    controls.size.connect_value_changed({
        let controls = Rc::downgrade(controls);
        move |spin| {
            if let Some(c) = controls.upgrade() {
                c.changed(|s| s.set_width(spin.value() as u32));
            }
        }
    });
    // Enter in the size gives the keys back to the text being typed.
    controls.size.connect_activate({
        let controls = Rc::downgrade(controls);
        move |_| {
            if let Some(c) = controls.upgrade() {
                c.back_to_text();
            }
        }
    });
    controls.sizing.append(&controls.size);
    controls.sizing.append(&gtk::Label::new(Some("px")));
    row.append(&controls.sizing);
    let label = gtk::Label::with_mnemonic(&keys::mnemonic("Tolerance"));
    label.set_mnemonic_widget(Some(&controls.tolerance));
    controls.tolerating.append(&label);
    controls.tolerance.set_valign(gtk::Align::Center);
    controls.tolerance.set_tooltip_text(Some(
        "Tolerance: how far, 0 to 255 in any channel, a colour may be from the clicked one and still fill (0: exactly its colour)",
    ));
    controls
        .tolerance
        .update_property(&[gtk::accessible::Property::Label("Tolerance")]);
    controls.tolerance.connect_value_changed({
        let controls = Rc::downgrade(controls);
        move |spin| {
            if let Some(c) = controls.upgrade() {
                c.changed(|s| s.tolerance = spin.value() as u8);
            }
        }
    });
    controls.tolerating.append(&controls.tolerance);
    row.append(&controls.tolerating);
    controls.family.connect_selected_notify({
        let controls = Rc::downgrade(controls);
        move |d| {
            let Some(c) = controls.upgrade() else { return };
            if let Some(family) = c.families.get(d.selected() as usize) {
                if !c.showing.get() {
                    c.update(|s| s.family = family);
                    c.back_to_text();
                }
            }
        }
    });
    for (button, set) in [
        (
            &controls.bold,
            (|s, on| s.bold = on) as fn(&mut Settings, bool),
        ),
        (&controls.italic, |s, on| s.italic = on),
    ] {
        button.connect_toggled({
            let controls = Rc::downgrade(controls);
            move |b| {
                if let Some(c) = controls.upgrade() {
                    c.changed(|s| set(s, b.is_active()));
                }
            }
        });
        controls.font.append(button);
    }
    row.append(&controls.font);
    controls.soft.connect_toggled({
        let controls = Rc::downgrade(controls);
        move |soft| {
            if let Some(c) = controls.upgrade() {
                if c.settings.get().soft != soft.is_active() {
                    c.changed(|s| s.soft = soft.is_active());
                }
            }
        }
    });
    row.append(&controls.edge);
    for (button, style) in controls.styles.iter().zip(STYLES) {
        button.connect_toggled({
            let controls = Rc::downgrade(controls);
            move |b| {
                if let Some(c) = controls.upgrade() {
                    if b.is_active() && c.settings.get().style != style {
                        c.changed(|s| s.style = style);
                    }
                }
            }
        });
    }
    row.append(&controls.style);
    row
}

/// Fill `window` with the image's view: the tool column, the canvas showing `image` (of
/// `file`; `None` for a new image), the colour row, and the status bar; then run `bench`,
/// if given, or offer back a crashed session's pixels, `recovered`.
fn show(
    app: &gtk::Application,
    window: &gtk::ApplicationWindow,
    image: Image,
    file: Option<Opened>,
    bench: Option<Bench>,
    recovered: Option<Recovered>,
) {
    let canvas = Canvas::new(image, matches!(bench, Some(Bench::Stroke(_))));
    let (edge, [hard, soft]) = choice(
        "Edge",
        [
            ("Hard", "Hard edge: solid to the rim"),
            ("Soft", "Soft edge: fades to transparent at the rim"),
        ],
    );
    let (style, styles) = choice(
        "Style",
        [
            (
                "Outline",
                "Outline: the shape's outline in the foreground colour",
            ),
            ("Filled", "Filled: the whole shape in the foreground colour"),
            (
                "Both",
                "Both: the outline in the foreground colour, filled with the background colour",
            ),
        ],
    );
    let (font, families, family) = font_controls(&canvas.pango_context());
    let controls = Rc::new(Controls {
        settings: Cell::new(Settings::default()),
        canvas: canvas.clone(),
        indicator: colours::Indicator::new(),
        hex: gtk::Entry::new(),
        sizing: gtk::Box::new(gtk::Orientation::Horizontal, 12),
        size: gtk::SpinButton::with_range(
            *tools::SIZES.start() as f64,
            *tools::SIZES.end() as f64,
            1.0,
        ),
        tolerating: gtk::Box::new(gtk::Orientation::Horizontal, 12),
        tolerance: gtk::SpinButton::with_range(0.0, 255.0, 1.0),
        edge,
        hard,
        soft,
        style,
        styles,
        font,
        families,
        family,
        bold: toggle("<b>B</b>", "Bold", "Bold"),
        italic: toggle("<i>I</i>", "Italic", "Italic"),
        showing: Cell::new(false),
    });
    controls.show();
    CONTROLS.with_borrow_mut(|c| {
        c.retain(|(w, _)| w.upgrade().is_some());
        c.push((window.downgrade(), Rc::downgrade(&controls)));
    });
    let (bar, status) = Status::new();
    bar.prepend(&colour_row(&controls));
    // The keys going elsewhere in the window undo what was typed in the hex field but not
    // entered; another window taking them leaves it be (the window keeps its focus).
    window.connect_focus_widget_notify({
        let controls = Rc::downgrade(&controls);
        move |_| {
            if let Some(c) = controls.upgrade().filter(|c| !c.hex_has_keys()) {
                c.show_hex(c.settings.get().foreground);
            }
        }
    });
    let middle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    middle.append(&tools::column());
    middle.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    middle.append(&canvas);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&middle);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&bar);
    window.set_child(Some(&suite_ui::notice::host(&content)));

    let action = |name: &str, f: fn(&Canvas)| {
        let canvas = canvas.clone();
        gio::ActionEntry::builder(name)
            .activate(move |_: &gtk::ApplicationWindow, _, _| f(&canvas))
            .build()
    };
    window.add_action_entries([
        action("undo", Canvas::undo),
        action("redo", Canvas::redo),
        action("zoom-in", |c| c.zoom_step(1, None)),
        action("zoom-out", |c| c.zoom_step(-1, None)),
        action("actual-size", Canvas::actual_size),
        action("fit", Canvas::fit),
        action("deselect", |c| {
            c.deselect();
        }),
        action("delete", |c| {
            c.delete_selection();
        }),
        action("cut", |c| {
            c.cut();
        }),
        action("copy", |c| {
            c.copy();
        }),
    ]);
    let with_window = |name: &str, f: fn(&gtk::ApplicationWindow, &Canvas)| {
        let canvas = canvas.clone();
        gio::ActionEntry::builder(name)
            .activate(move |window: &gtk::ApplicationWindow, _, _| f(window, &canvas))
            .build()
    };
    window.add_action_entries([
        with_window("select-all", select_all),
        with_window("paste", paste),
    ]);
    image_menu::add_actions(window, &canvas);
    // The tool: the tool column's buttons and the tool keys set it.
    let tool = gio::ActionEntry::builder("tool")
        .parameter_type(Some(glib::VariantTy::STRING))
        .state(Tool::default().id().to_variant())
        .activate(|_: &gtk::ApplicationWindow, action, target| {
            if let Some(target) = target {
                action.change_state(target);
            }
        })
        .change_state({
            let controls = controls.clone();
            move |_: &gtk::ApplicationWindow, action, state| {
                let picked = state.and_then(|s| s.str()).and_then(Tool::from_id);
                if let (Some(t), Some(state)) = (picked, state) {
                    action.set_state(state);
                    controls.update(|s| s.set_tool(t));
                }
            }
        })
        .build();
    window.add_action_entries([tool]);
    // A pick sets the colour and goes back to the tool used before the picker.
    canvas.connect_picked({
        let (window, controls) = (window.downgrade(), Rc::downgrade(&controls));
        move |colour, background| {
            let (Some(window), Some(controls)) = (window.upgrade(), controls.upgrade()) else {
                return;
            };
            let mut back = Tool::default();
            controls.update(|s| back = s.pick(colour, background));
            ActionGroupExt::activate_action(&window, "tool", Some(&back.id().to_variant()));
        }
    });

    // The window's keys from the key table (keys::BINDINGS): the tool keys, X, Space,
    // Escape, Delete, the arrows, Ctrl+X, C, V, and A (⌘ on macOS), and the Ctrl+Shift
    // (⇧⌘) ones (shifted, a letter may arrive upper case, which an accelerator misses).
    // None of them while a field has the keys, and only the Ctrl+Shift (⇧⌘) ones while
    // text is typed on the canvas: its keys are its letters (keys::takes).
    let handler = gtk::EventControllerKey::new();
    handler.set_propagation_phase(gtk::PropagationPhase::Capture);
    handler.connect_key_pressed({
        let (canvas, window, controls) = (canvas.clone(), window.downgrade(), controls.clone());
        move |_, key, _, state| {
            let Some(window) = window.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let focus = focus_of(&window, &canvas);
            if focus == Focus::Field {
                return glib::Propagation::Proceed;
            }
            // While an Image menu command runs, the image waits for it.
            if canvas.busy() {
                return glib::Propagation::Stop;
            }
            let binding = keys::window_key(Primary::HERE, key, state, focus);
            match binding.is_some_and(|b| run(&window, &canvas, &controls, b.command)) {
                true => glib::Propagation::Stop,
                false => glib::Propagation::Proceed,
            }
        }
    });
    handler.connect_key_released({
        let canvas = canvas.clone();
        move |_, key, _, _| {
            if key == gdk::Key::space {
                canvas.set_space(false);
            }
        }
    });
    window.add_controller(handler);

    canvas.connect_changed({
        let window = window.downgrade();
        move |canvas| {
            let Some(window) = window.upgrade() else {
                return;
            };
            status.show(canvas);
            let selected = canvas.selection_size().is_some();
            for (name, on) in [
                ("undo", canvas.can_undo()),
                ("redo", canvas.can_redo()),
                ("deselect", selected),
                ("delete", selected),
                ("cut", selected),
                ("copy", selected),
            ] {
                if let Some(a) = window
                    .lookup_action(name)
                    .and_downcast::<gio::SimpleAction>()
                {
                    a.set_enabled(on);
                }
            }
            image_menu::update(&window, canvas);
        }
    });
    Saver::attach(window, &canvas, file, bench.is_none());
    window.present();
    canvas.grab_focus();
    match bench {
        Some(Bench::Stroke(frames)) => bench_stroke(app, &canvas, frames),
        Some(Bench::Open { start, read_ms }) => bench_open(app, window, &canvas, start, read_ms),
        None => {
            if let Some(recovered) = recovered {
                recovered.offer(window, &canvas);
            }
        }
    }
}

/// Edit → Paste: reads the clipboard's image without waiting for it, converts it to the
/// image's pixels off the UI thread, then picks the selection tool (as Paint does, so the
/// pasted pixels can be moved at once) and floats them over the image. Nothing happens
/// when the clipboard holds no image.
fn paste(window: &gtk::ApplicationWindow, canvas: &Canvas) {
    let clipboard = canvas.clipboard();
    let (window, canvas) = (window.downgrade(), canvas.downgrade());
    glib::spawn_future_local(async move {
        let texture = match clipboard.read_texture_future().await {
            Ok(Some(texture)) => texture,
            Ok(None) => return,
            Err(e) => {
                tracing::info!("nothing to paste: {e}");
                return;
            }
        };
        let converted = gio::spawn_blocking(move || crate::canvas::pasted(&texture)).await;
        let (pixels, shown) = match converted {
            Ok(Ok(pasted)) => pasted,
            Ok(Err(e)) => {
                tracing::warn!("cannot paste the clipboard's image: {e}");
                return;
            }
            Err(_) => {
                tracing::warn!("pasting failed unexpectedly");
                return;
            }
        };
        let (Some(window), Some(canvas)) = (window.upgrade(), canvas.upgrade()) else {
            return;
        };
        let selection = Tool::Selection.id().to_variant();
        ActionGroupExt::activate_action(&window, "tool", Some(&selection));
        canvas.paste(pixels, shown);
    });
}

/// Where `window`'s keys go: a field (a text field: the size, the tolerance, the hex field,
/// the font list's search), the text typed on `canvas`, or the image.
fn focus_of(window: &gtk::ApplicationWindow, canvas: &Canvas) -> Focus {
    let field =
        GtkWindowExt::focus(window).is_some_and(|f| f.is::<gtk::Text>() || f.is::<gtk::TextView>());
    match (field, canvas.typing()) {
        (true, _) => Focus::Field,
        (false, true) => Focus::Typing,
        (false, false) => Focus::Image,
    }
}

/// Runs a key's `command` in `window`, and says whether it did anything: Escape (as
/// Deselect) drops a shape being dragged or deselects, Delete deletes the selection, and
/// the arrows nudge it only when there is one; otherwise the key goes on.
fn run(
    window: &gtk::ApplicationWindow,
    canvas: &Canvas,
    controls: &Controls,
    command: Command,
) -> bool {
    match command {
        Command::Tool(t) => {
            ActionGroupExt::activate_action(window, "tool", Some(&t.id().to_variant()));
            true
        }
        Command::Swap => {
            controls.update(Settings::swap);
            true
        }
        Command::Pan => {
            canvas.set_space(true);
            true
        }
        Command::Nudge(dx, dy) => canvas.nudge(dx, dy),
        Command::DropShape => canvas.cancel_shape(),
        Command::Action("win.deselect") => canvas.cancel_shape() || canvas.deselect(),
        Command::Action("win.delete") => canvas.delete_selection(),
        Command::Action(name) => {
            if let Err(e) = WidgetExt::activate_action(window, name, None) {
                tracing::warn!("cannot run {name}: {e}");
            }
            true
        }
        Command::Label(_) => false,
    }
}

/// Edit → Select All: the selection becomes the tool (as in Paint, so the selection can be
/// moved at once) and selects the whole image.
fn select_all(window: &gtk::ApplicationWindow, canvas: &Canvas) {
    let selection = Tool::Selection.id().to_variant();
    ActionGroupExt::activate_action(window, "tool", Some(&selection));
    canvas.select_all();
}

/// New Image's size and background: the last used, from the settings file the first time.
fn new_settings() -> settings::Settings {
    NEW.get().unwrap_or_else(|| {
        let s = settings::Settings::load();
        NEW.set(Some(s));
        s
    })
}

/// The background colour of `window` (the colour row's); white without one.
pub(crate) fn background_of(window: Option<&gtk::Window>) -> Color {
    let found = window.and_then(|window| {
        CONTROLS.with_borrow(|c| {
            c.iter()
                .find(|(w, _)| {
                    w.upgrade()
                        .is_some_and(|w| w.upcast_ref::<gtk::Window>() == window)
                })
                .and_then(|(_, c)| c.upgrade())
                .map(|c| c.settings.get().background)
        })
    });
    found.unwrap_or(Settings::default().background)
}

/// File → New Image: asks for a width, a height, and a background (white, transparent, or
/// the active window's background colour), offering the last ones used, then opens a
/// window with an image of them. What was asked is remembered across runs.
fn ask_new_image(app: &gtk::Application) {
    let parent = app.active_window();
    let background = background_of(parent.as_ref());
    let dialog = gtk::Window::builder()
        .title("New Image")
        .modal(true)
        .resizable(false)
        .build();
    dialog.set_transient_for(parent.as_ref());
    let grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(8)
        .margin_start(16)
        .margin_end(16)
        .margin_top(16)
        .margin_bottom(16)
        .build();
    let last = new_settings();
    let (w, h) = last.new_size;
    let side = |row: i32, label: &str, value: u32| {
        let name = gtk::Label::with_mnemonic(label);
        name.set_xalign(0.0);
        let spin = gtk::SpinButton::with_range(1.0, MAX_SIDE as f64, 1.0);
        spin.set_value(value as f64);
        spin.set_activates_default(true);
        spin.update_property(&[gtk::accessible::Property::Label(&label.replace('_', ""))]);
        name.set_mnemonic_widget(Some(&spin));
        grid.attach(&name, 0, row, 1, 1);
        grid.attach(&spin, 1, row, 1, 1);
        grid.attach(&gtk::Label::new(Some("pixels")), 2, row, 1, 1);
        spin
    };
    let width = side(0, "_Width", w);
    let height = side(1, "_Height", h);
    let name = gtk::Label::new(Some("Background"));
    name.set_xalign(0.0);
    name.set_valign(gtk::Align::Start);
    grid.attach(&name, 0, 2, 1, 1);
    let choices = gtk::Box::new(gtk::Orientation::Vertical, 4);
    choices.update_property(&[gtk::accessible::Property::Label("Background")]);
    let colour = colours::hex(background);
    let backgrounds = Background::ALL.map(|b| {
        let label = match b {
            Background::White => "Wh_ite",
            Background::Transparent => "_Transparent",
            Background::Colour => "Back_ground colour",
        };
        let button = gtk::CheckButton::with_mnemonic(label);
        if b == Background::Colour {
            button.set_tooltip_text(Some(&format!("The background colour, {colour}")));
            button.update_property(&[gtk::accessible::Property::Description(&colour)]);
        }
        button.set_active(b == last.new_background);
        choices.append(&button);
        button
    });
    for b in &backgrounds[1..] {
        b.set_group(Some(&backgrounds[0]));
    }
    grid.attach(&choices, 1, 2, 2, 1);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(8);
    let cancel = gtk::Button::with_mnemonic("_Cancel");
    let create = gtk::Button::with_mnemonic("C_reate");
    create.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&create);
    grid.attach(&buttons, 0, 3, 3, 1);
    dialog.set_child(Some(&grid));
    dialog.set_default_widget(Some(&create));

    cancel.connect_clicked({
        let dialog = dialog.downgrade();
        move |_| {
            if let Some(d) = dialog.upgrade() {
                d.close();
            }
        }
    });
    create.connect_clicked({
        let (dialog, app) = (dialog.downgrade(), app.clone());
        move |_| {
            width.update();
            height.update();
            let chosen = settings::Settings {
                new_size: (width.value() as u32, height.value() as u32),
                new_background: Background::ALL
                    .into_iter()
                    .zip(&backgrounds)
                    .find(|(_, b)| b.is_active())
                    .map_or(Background::White, |(b, _)| b),
            };
            if let Some(d) = dialog.upgrade() {
                d.close();
            }
            remember(&app, chosen);
            open_new(
                &app,
                chosen.new_size,
                chosen.new_background.color(background),
                None,
                None,
            );
        }
    });
    let escape = gtk::ShortcutController::new();
    escape.add_shortcut(gtk::Shortcut::new(
        gtk::ShortcutTrigger::parse_string("Escape"),
        Some(gtk::NamedAction::new("window.close")),
    ));
    dialog.add_controller(escape);
    dialog.present();
}

/// Makes `chosen` what New Image offers, and writes it to the settings file off the UI
/// thread (the app runs until it's written).
fn remember(app: &gtk::Application, chosen: settings::Settings) {
    if NEW.get() == Some(chosen) {
        return;
    }
    NEW.set(Some(chosen));
    let hold = app.hold();
    glib::spawn_future_local(async move {
        let _hold = hold;
        let path = settings::path();
        let shown = path.clone();
        match gio::spawn_blocking(move || settings::write(&path, &chosen)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!("cannot save {}: {e}", shown.display()),
            Err(_) => tracing::warn!("saving {} failed unexpectedly", shown.display()),
        }
    });
}

/// ETCH-6's benchmark: from the process's start until the end of the first frame that drew
/// the opened image (the frame clock's after-paint once the canvas has snapshotted it, so
/// the frame rendered its tiles); prints a JSON line and quits. `read_ms`: reading and
/// decoding the file, off the UI thread.
fn bench_open(
    app: &gtk::Application,
    window: &gtk::ApplicationWindow,
    canvas: &Canvas,
    start: Instant,
    read_ms: f64,
) {
    let shown_ms = start.elapsed().as_secs_f64() * 1e3;
    let Some(clock) = window.frame_clock() else {
        eprintln!("etch: the window has no frame clock");
        std::process::exit(1);
    };
    let done = Cell::new(false);
    canvas.queue_draw();
    let (app, canvas) = (app.clone(), canvas.downgrade());
    clock.connect_after_paint(move |_| {
        let Some(canvas) = canvas.upgrade() else {
            return;
        };
        if done.get() || canvas.frames_drawn() == 0 {
            return;
        }
        done.set(true);
        let (w, h) = canvas.image_size();
        println!(
            r#"{{"image":[{w},{h}],"canvas":[{},{}],"zoom":"{}","read_ms":{read_ms:.1},"shown_ms":{shown_ms:.1},"snapshot_ms":{:.1},"first_frame_ms":{:.1}}}"#,
            canvas.width(),
            canvas.height(),
            canvas.view().map(|v| v.percent()).unwrap_or_default(),
            canvas.last_draw_ms(),
            start.elapsed().as_secs_f64() * 1e3,
        );
        app.quit();
    });
}

/// ETCH-2's benchmark: draws a stroke one segment a frame, 16 widget pixels apart (a brisk
/// hand at 60 frames a second), bouncing around the canvas on the 8K image at 100%, for
/// `frames` frames after 30 to settle. Measures each frame's CPU time (frame clock before-
/// to after-paint, which holds the segment, the snapshot, and rendering), the segment's
/// own share, and the snapshot's; prints a JSON line and quits.
fn bench_stroke(app: &gtk::Application, canvas: &Canvas, frames: u32) {
    let Some(clock) = canvas.frame_clock() else {
        // Not realized yet: try again once it is.
        let app = app.clone();
        canvas.connect_realize(move |c| bench_stroke(&app, c, frames));
        return;
    };
    #[derive(Default)]
    struct Rec {
        on: bool,
        start: Option<Instant>,
        cpu_ms: Vec<f64>,
        segment_ms: Vec<f64>,
        snapshot_ms: Vec<f64>,
    }
    let rec = Rc::new(RefCell::new(Rec::default()));
    clock.connect_before_paint({
        let rec = rec.clone();
        move |_| rec.borrow_mut().start = Some(Instant::now())
    });
    clock.connect_after_paint({
        let (rec, canvas) = (rec.clone(), canvas.downgrade());
        move |_| {
            let mut r = rec.borrow_mut();
            if let (true, Some(t0), Some(c)) = (r.on, r.start, canvas.upgrade()) {
                r.cpu_ms.push(t0.elapsed().as_secs_f64() * 1e3);
                r.snapshot_ms.push(c.last_draw_ms());
            }
        }
    });
    let ticks = Cell::new(0u32);
    let pos = Cell::new((40.0f64, 40.0f64));
    let dir = Cell::new((16.0f64 * 0.8, 16.0f64 * 0.6));
    let app = app.clone();
    canvas.add_tick_callback(move |canvas, _| {
        let n = ticks.get();
        ticks.set(n + 1);
        if n < 30 {
            return glib::ControlFlow::Continue;
        }
        if n == 30 {
            rec.borrow_mut().on = true;
            canvas.draw_begin(pos.get());
            return glib::ControlFlow::Continue;
        }
        if n > 30 + frames {
            canvas.draw_end();
            let r = rec.borrow();
            let (w, h) = canvas.image_size();
            println!(
                r#"{{"image":[{w},{h}],"canvas":[{},{}],"zoom":"{}","frames":{},"frame_cpu_ms":{},"segment_ms":{},"snapshot_ms":{}}}"#,
                canvas.width(),
                canvas.height(),
                canvas.view().map(|v| v.percent()).unwrap_or_default(),
                r.cpu_ms.len(),
                summary(&r.cpu_ms),
                summary(&r.segment_ms),
                summary(&r.snapshot_ms),
            );
            app.quit();
            return glib::ControlFlow::Break;
        }
        // Bounce off the canvas's edges.
        let ((x, y), (mut dx, mut dy)) = (pos.get(), dir.get());
        let (cw, ch) = (canvas.width() as f64, canvas.height() as f64);
        if !(0.0..cw).contains(&(x + dx)) {
            dx = -dx;
        }
        if !(0.0..ch).contains(&(y + dy)) {
            dy = -dy;
        }
        let next = (x + dx, y + dy);
        pos.set(next);
        dir.set((dx, dy));
        let t = Instant::now();
        canvas.draw_to(next);
        rec.borrow_mut()
            .segment_ms
            .push(t.elapsed().as_secs_f64() * 1e3);
        glib::ControlFlow::Continue
    });
}

/// `{"p50":..,"p99":..,"max":..}` of `xs`.
fn summary(xs: &[f64]) -> String {
    let mut v = xs.to_vec();
    v.sort_by(f64::total_cmp);
    let at = |q: f64| {
        if v.is_empty() {
            0.0
        } else {
            v[((v.len() - 1) as f64 * q).round() as usize]
        }
    };
    format!(
        r#"{{"p50":{:.3},"p99":{:.3},"max":{:.3}}}"#,
        at(0.5),
        at(0.99),
        at(1.0)
    )
}
