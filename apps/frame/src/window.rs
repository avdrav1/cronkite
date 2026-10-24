//! Windows: a window per deck (slide list, canvas, inspector, status bar), new decks, the
//! file dialog, drops, and alerts (spec FR0); the deck window's undo history and
//! its keys (text editing itself is in [`crate::editing`], the slide list in
//! [`crate::slide_list`], the inspector in [`crate::inspector`], inserting in
//! [`crate::insert`], text fitting its box in [`crate::fit`], the outline in
//! [`crate::outline`]).

use crate::arrange;
use crate::deck::Deck;
use crate::editing;
use crate::fit;
use crate::insert;
use crate::inspector::{self, Inspector};
use crate::keys::{self, Action, Via};
use crate::outline::{self, OutlineView};
use crate::saving;
use crate::slide_list::{self, THUMB_WIDTH};
use crate::slide_view::{Editing, SlideView};
use deck_edit::{Change, Edit, History, ShapeEdit};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;
use suite_file::canonical;
use suite_ui::chrome::Toolbar;
use suite_ui::keys::Primary;

thread_local! {
    /// Each open deck's window, by canonical path: opening a file again brings its
    /// window forward.
    static OPEN: RefCell<Vec<(PathBuf, glib::WeakRef<gtk::ApplicationWindow>)>> =
        const { RefCell::new(Vec::new()) };
}

fn window_for(path: &Path) -> Option<gtk::ApplicationWindow> {
    let path = canonical(path);
    OPEN.with_borrow_mut(|open| {
        open.retain(|(_, w)| w.upgrade().is_some());
        open.iter()
            .find(|(p, _)| *p == path)
            .and_then(|(_, w)| w.upgrade())
    })
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// The header bar every window has on Linux: New (Ctrl+N) and Open (Ctrl+O) buttons, and
/// at its end the main menu (Update Automatically, About Frame). macOS and Windows windows
/// keep their own title bar, and New and Open are in the menu bar.
fn put_header(window: &gtk::ApplicationWindow) {
    if suite_ui::chrome::MENU_BAR {
        return;
    }
    let bar = gtk::HeaderBar::new();
    let new = gtk::Button::from_icon_name("document-new-symbolic");
    new.set_action_name(Some("app.new"));
    new.set_tooltip_text(Some(&format!("New ({})", keys::label(Action::New))));
    new.update_property(&[gtk::accessible::Property::Label("New")]);
    bar.pack_start(&new);
    let open = gtk::Button::from_icon_name("document-open-symbolic");
    open.set_action_name(Some("app.open"));
    open.set_tooltip_text(Some(&format!("Open… ({})", keys::label(Action::Open))));
    open.update_property(&[gtk::accessible::Property::Label("Open")]);
    bar.pack_start(&open);
    bar.pack_end(&suite_ui::chrome::main_menu(
        &suite_ui::chrome::app_section(crate::APP),
    ));
    window.set_titlebar(Some(&bar));
}

/// File → New (Ctrl+N), and `frame` without a file: a window with a new deck, Frame's
/// built-in blank one, titled "Untitled" ("Untitled 2", ... beside other new decks),
/// on its title slide with the caret in its title, ready to type. It's read off the UI
/// thread; it saves through Save As.
pub fn new_deck(app: &gtk::Application) {
    let app = app.clone();
    // Running until the window shows, though no window is open yet.
    let hold = app.hold();
    glib::spawn_future_local(async move {
        let _hold = hold;
        let error = match gio::spawn_blocking(Deck::blank).await {
            Ok(Ok(deck)) => {
                let window = gtk::ApplicationWindow::builder()
                    .application(&app)
                    .title(untitled(&app))
                    .default_width(1280)
                    .default_height(760)
                    .build();
                put_header(&window);
                accept_file_drops(&app, &window);
                let view = show_deck(&app, &window, Rc::new(deck), None);
                window.present();
                view.canvas.grab_focus();
                editing::edit_title(&view);
                return;
            }
            Ok(Err(e)) => e.to_string(),
            Err(_) => "Making it failed unexpectedly.".to_owned(),
        };
        eprintln!("frame: cannot make a new presentation: {error}");
        let _ = gtk::AlertDialog::builder()
            .message("Cannot make a new presentation")
            .detail(&error)
            .buttons(["Close"])
            .modal(true)
            .build()
            .choose_future(app.active_window().as_ref())
            .await;
    });
}

/// "Untitled", or "Untitled N" with the first N no other window has.
fn untitled(app: &gtk::Application) -> String {
    let titles: Vec<_> = app.windows().iter().filter_map(|w| w.title()).collect();
    (1..)
        .map(|n| match n {
            1 => "Untitled".to_owned(),
            n => format!("Untitled {n}"),
        })
        .find(|t| !titles.iter().any(|w| w == t))
        .unwrap_or_default()
}

/// "Presentations" (.pptx, .pptm) and "All files", the first as the default.
pub(crate) fn file_filters() -> (gio::ListStore, gtk::FileFilter) {
    let pptx = gtk::FileFilter::new();
    pptx.set_name(Some("Presentations"));
    pptx.add_mime_type("application/vnd.openxmlformats-officedocument.presentationml.presentation");
    pptx.add_mime_type("application/vnd.ms-powerpoint.presentation.macroEnabled.12");
    pptx.add_suffix("pptx");
    pptx.add_suffix("pptm");
    let all = gtk::FileFilter::new();
    all.set_name(Some("All files"));
    all.add_pattern("*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&pptx);
    filters.append(&all);
    (filters, pptx)
}

/// File → Open (Ctrl+O): the desktop's file dialog (xdg-desktop-portal through
/// `gtk::FileDialog`, GTK's own without a portal), filtered to presentations.
pub fn choose_and_open(app: &gtk::Application) {
    let (filters, pptx) = file_filters();
    let dialog = gtk::FileDialog::builder()
        .title("Open")
        .modal(true)
        .filters(&filters)
        .default_filter(&pptx)
        .build();
    let (app, parent) = (app.clone(), app.active_window());
    glib::spawn_future_local(async move {
        match dialog.open_future(parent.as_ref()).await {
            Ok(file) => match file.path() {
                Some(path) => open(&app, &path, None),
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

/// Files dropped on a window open as Open does, each in its own window ([`open_later`]).
fn accept_file_drops(app: &gtk::Application, window: &gtk::ApplicationWindow) {
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
            open_later(&app, &window, files);
            true
        }
    });
    window.add_controller(target);
}

/// Open `files`, dropped on `window`, as Open does, each in its own window. They open
/// after the drop is done.
fn open_later(
    app: &gtk::Application,
    window: &glib::WeakRef<gtk::ApplicationWindow>,
    files: Vec<gio::File>,
) {
    let (app, window) = (app.clone(), window.clone());
    glib::idle_add_local_once(move || {
        let parent = window.upgrade().map(|w| w.upcast::<gtk::Window>());
        for file in files {
            match file.path() {
                Some(path) => open(&app, &path, None),
                None => alert(
                    parent.as_ref(),
                    &format!("Cannot open {}", file.uri()),
                    "Only local files can be opened.",
                ),
            }
        }
    });
}

pub(crate) fn alert(parent: Option<&gtk::Window>, message: &str, detail: &str) {
    gtk::AlertDialog::builder()
        .message(message)
        .detail(detail)
        .modal(true)
        .build()
        .show(parent);
}

/// Open `path` in its own window: its existing one, or a new one that says "Opening…"
/// while the deck is read off the UI thread. A file that can't be read gives an alert
/// naming the file and the reason, and its window closes.
pub fn open(app: &gtk::Application, path: &Path, bench: Option<u32>) {
    if let Some(window) = window_for(path) {
        window.present();
        return;
    }
    let name = file_name(path);
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(&name)
        .default_width(1280)
        .default_height(760)
        .build();
    put_header(&window);
    let opening = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    opening.set_halign(gtk::Align::Center);
    opening.set_valign(gtk::Align::Center);
    let spinner = gtk::Spinner::new();
    spinner.start();
    opening.append(&spinner);
    opening.append(&gtk::Label::new(Some("Opening…")));
    window.set_child(Some(&suite_ui::notice::host(&opening)));
    accept_file_drops(app, &window);
    OPEN.with_borrow_mut(|open| open.push((canonical(path), window.downgrade())));
    window.present();

    let (app, path) = (app.clone(), path.to_owned());
    glib::spawn_future_local(async move {
        let p = path.clone();
        let read = gio::spawn_blocking(move || Deck::load(&p)).await;
        let error = match read {
            Ok(Ok(deck)) => {
                show_deck(&app, &window, Rc::new(deck), bench);
                return;
            }
            Ok(Err(e)) => e.to_string(),
            Err(_) => "Reading it failed unexpectedly.".to_owned(),
        };
        eprintln!("frame: cannot open {}: {error}", path.display());
        if bench.is_some() {
            std::process::exit(1);
        }
        let _ = gtk::AlertDialog::builder()
            .message(format!("Cannot open “{name}”"))
            .detail(&error)
            .buttons(["Close"])
            .modal(true)
            .build()
            .choose_future(Some(&window))
            .await;
        window.close();
    });
}

/// A deck's window once it's read: the slide list, the canvas, the inspector, and the
/// status bar, and the deck's undo history.
pub(crate) struct DeckWindow {
    app: gtk::Application,
    window: glib::WeakRef<gtk::ApplicationWindow>,
    pub deck: Rc<Deck>,
    pub canvas: SlideView,
    /// The slide list's thumbnails, by slide.
    pub thumbs: RefCell<Vec<SlideView>>,
    pub list: gtk::ListBox,
    scrolled: gtk::ScrolledWindow,
    status: gtk::Label,
    history: RefCell<History>,
    /// The canvas's input method, for typing.
    pub im: gtk::IMMulticontext,
    pub inspector: Inspector,
    /// The outline, shown in place of the canvas and the inspector.
    pub outline: OutlineView,
}

impl DeckWindow {
    pub fn count(&self) -> usize {
        self.deck.presentation().slides.len()
    }

    /// Show slide `index`: canvas, selected thumbnail, status, and the canvas's accessible
    /// description (the slide's text).
    pub fn go(&self, index: usize) {
        let n = self.count();
        let index = index.min(n.saturating_sub(1));
        self.canvas.set_index(index);
        self.describe(index);
        if n == 0 {
            self.status.set_text("No slides");
            return;
        }
        self.status.set_text(&format!("Slide {} of {n}", index + 1));
        if let Some(row) = self.list.row_at_index(index as i32) {
            if !row.is_selected() {
                self.list.select_row(Some(&row));
            }
            if let Some(b) = row.compute_bounds(&self.list) {
                let adj = self.scrolled.vadjustment();
                let (top, bottom) = (f64::from(b.y()), f64::from(b.y() + b.height()));
                if top < adj.value() {
                    adj.set_value(top);
                } else if bottom > adj.value() + adj.page_size() {
                    adj.set_value(bottom - adj.page_size());
                }
            }
        }
        outline::follow(self, index);
    }

    /// The canvas's accessible name and description: slide `index` with its layout's
    /// name, and its text, as they are now.
    pub fn describe(&self, index: usize) {
        let deck = self.deck.presentation();
        let (label, text) = match deck.slides.get(index) {
            Some(slide) => {
                let layout = slide.layout.and_then(|l| deck.layouts.get(l));
                let label = match layout {
                    Some(l) if !l.name.is_empty() => format!("Slide {}, {}", index + 1, l.name),
                    _ => format!("Slide {}", index + 1),
                };
                (label, slide.text())
            }
            None => ("No slides".to_owned(), String::new()),
        };
        self.canvas.update_property(&[
            gtk::accessible::Property::Label(&label),
            gtk::accessible::Property::Description(&text),
        ]);
    }

    pub fn step(&self, by: isize) {
        let i = self.canvas.index() as isize + by;
        self.go(i.clamp(0, self.count().saturating_sub(1) as isize) as usize);
    }

    pub fn window(&self) -> Option<gtk::ApplicationWindow> {
        self.window.upgrade()
    }

    /// Open `files`, decks dropped on the window, each in its own window.
    pub fn open_dropped(&self, files: Vec<gio::File>) {
        open_later(&self.app, &self.window, files);
    }

    /// Say `text` in the status bar until the slide changes.
    pub fn set_status(&self, text: &str) {
        self.status.set_text(text);
    }

    /// The status bar's usual "Slide N of M".
    pub fn show_slide_status(&self) {
        self.go(self.canvas.index());
    }

    /// The deck was saved as `path`: it saves there from now on, and the window is that
    /// file's (its title, and opening the file again brings it forward).
    pub fn moved_to(&self, path: &Path) {
        self.deck.set_path(path.to_owned());
        let Some(window) = self.window() else {
            return;
        };
        window.set_title(Some(&file_name(path)));
        let to = canonical(path);
        OPEN.with_borrow_mut(|open| {
            open.retain(|(p, w)| *p != to && w.upgrade().is_some_and(|w| w != window));
            open.push((to, window.downgrade()));
        });
    }

    /// Apply `edit` to the deck as a step of its undo history, and show what it changed;
    /// then fit the text it changed to its box, in the same step ([`crate::fit`]).
    pub fn apply(&self, edit: impl Into<Edit>) {
        let edit = edit.into();
        // What the edit changed that text fits to: the text or its looks (a new layout's
        // placeholders, a new text box), or only a box.
        let text = matches!(
            edit,
            Edit::Text(_)
                | Edit::Look(_)
                | Edit::Layout(_)
                | Edit::Shapes(ShapeEdit::Insert { .. })
        );
        let refit = text || matches!(edit, Edit::Frame(_));
        let change = self
            .history
            .borrow_mut()
            .apply(&mut self.deck.presentation_mut(), edit);
        self.changed(change, true);
        if refit {
            fit::refit(self, change, text);
        }
    }

    /// Apply `edit`, what the last edit caused (its text shrunk, its box grown), as part
    /// of that edit's step, and redraw it; the selection and editing stay as they are.
    pub fn amend(&self, edit: impl Into<Edit>) {
        let change = self
            .history
            .borrow_mut()
            .amend(&mut self.deck.presentation_mut(), edit);
        self.redraw(change.slide());
        self.inspector.refresh();
    }

    /// End the history's current step: the next edit won't join it.
    pub fn seal(&self) {
        self.history.borrow_mut().seal();
    }

    /// Ctrl+Z: undo the last step, showing where it was.
    pub fn undo(&self) {
        let editing = self.canvas.editing().is_some();
        let change = self
            .history
            .borrow_mut()
            .undo(&mut self.deck.presentation_mut());
        if let Some(change) = change {
            self.changed(change, editing);
        }
    }

    /// Ctrl+Shift+Z, Ctrl+Y: redo the last step undone.
    pub fn redo(&self) {
        let editing = self.canvas.editing().is_some();
        let change = self
            .history
            .borrow_mut()
            .redo(&mut self.deck.presentation_mut());
        if let Some(change) = change {
            self.changed(change, editing);
        }
    }

    /// Show what `change` changed. A text change: its slide, redrawn with its thumbnail
    /// and description, and, when `caret`, the caret where it leaves the text. A slide
    /// list change: the list rebuilt and the slide it shows. A shape's box: its slide
    /// and thumbnail redrawn, the shape selected. A shape's looks: the same, its text
    /// still edited as it was. A slide as a whole: it and its thumbnail redrawn, nothing
    /// selected. The inspector follows.
    fn changed(&self, change: Change, caret: bool) {
        // While the outline is shown, the canvas edits nothing.
        let caret = caret && !self.outline.is_shown();
        match change {
            Change::Text { shape, caret: at } => {
                if self.canvas.index() != shape.slide {
                    self.go(shape.slide);
                }
                self.describe(shape.slide);
                if let Some(thumb) = self.thumbs.borrow().get(shape.slide) {
                    thumb.queue_draw();
                }
                editing::show(self, caret.then(|| Editing::at(shape.shape, at)));
            }
            Change::Slides { show } => {
                editing::show(self, None);
                slide_list::rebuild(self);
                self.go(show);
            }
            Change::Shape { shape } => {
                if self.canvas.index() != shape.slide {
                    self.go(shape.slide);
                }
                if let Some(thumb) = self.thumbs.borrow().get(shape.slide) {
                    thumb.queue_draw();
                }
                editing::show(self, None);
                self.canvas.select(Some(shape.shape));
            }
            Change::Look { shape } => {
                if self.canvas.index() != shape.slide {
                    self.go(shape.slide);
                    self.canvas.select(Some(shape.shape));
                }
                self.redraw(shape.slide);
            }
            Change::Slide { slide } => {
                if self.canvas.index() != slide {
                    self.go(slide);
                }
                editing::show(self, None);
                self.canvas.select(None);
                self.describe(slide);
                self.redraw(slide);
            }
        }
        self.inspector.refresh();
        outline::changed(self, change);
    }

    /// Redraw slide `slide` on the canvas and in the slide list.
    fn redraw(&self, slide: usize) {
        if let Some(thumb) = self.thumbs.borrow().get(slide) {
            thumb.queue_draw();
        }
        self.canvas.queue_draw();
        if let Some(e) = self.canvas.editing() {
            editing::show(self, Some(e));
        }
    }
}

/// The full-screen slideshow, from slide `start` (the next shown one when it's hidden):
/// Space, Enter, Right, Down, Page Down, and a click go forward; Left, Up, Page Up, and
/// Backspace back; Home and End to the first and last. Hidden slides are skipped. Escape,
/// or going past the last slide, ends it, back in the deck's window on the slide shown.
fn slideshow(view: &Rc<DeckWindow>, start: usize) {
    let shown: Rc<Vec<usize>> = {
        let presentation = view.deck.presentation();
        let slides = &presentation.slides;
        Rc::new((0..slides.len()).filter(|&i| !slides[i].hidden).collect())
    };
    if shown.is_empty() {
        return;
    }
    let first = shown
        .iter()
        .position(|&i| i >= start)
        .unwrap_or(shown.len() - 1);
    let deck_window = view.window.upgrade();
    let show = gtk::Window::builder()
        .application(&view.app)
        .title(format!(
            "Slideshow: {}",
            deck_window
                .as_ref()
                .and_then(|w| w.title())
                .unwrap_or_default()
        ))
        .decorated(false)
        .build();
    show.set_transient_for(deck_window.as_ref());
    let canvas = SlideView::new(&view.deck, shown[first], 0);
    // Black around the slide where its shape differs from the screen's.
    canvas.set_backdrop(gdk::RGBA::BLACK);
    show.set_child(Some(&canvas));
    let at = Rc::new(Cell::new(first));
    let set = {
        let (canvas, shown, deck, at) =
            (canvas.clone(), shown.clone(), view.deck.clone(), at.clone());
        move |pos: usize| {
            at.set(pos);
            let index = shown[pos];
            canvas.set_index(index);
            canvas.update_property(&[
                gtk::accessible::Property::Label(&format!("Slideshow slide {}", index + 1)),
                gtk::accessible::Property::Description(&deck.presentation().slides[index].text()),
            ]);
        }
    };
    set(first);
    let set = Rc::new(set);
    let forward = {
        let (set, at, show, n) = (set.clone(), at.clone(), show.downgrade(), shown.len());
        move || {
            if at.get() + 1 < n {
                set(at.get() + 1);
            } else if let Some(show) = show.upgrade() {
                show.close();
            }
        }
    };
    let forward = Rc::new(forward);
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let (set, at, show, forward, n) = (
            set.clone(),
            at.clone(),
            show.downgrade(),
            forward.clone(),
            shown.len(),
        );
        move |_, key, _, _| {
            use gdk::Key;
            match key {
                Key::space
                | Key::Return
                | Key::KP_Enter
                | Key::Right
                | Key::KP_Right
                | Key::Down
                | Key::KP_Down
                | Key::Page_Down
                | Key::KP_Page_Down => forward(),
                Key::Left
                | Key::KP_Left
                | Key::Up
                | Key::KP_Up
                | Key::Page_Up
                | Key::KP_Page_Up
                | Key::BackSpace => set(at.get().saturating_sub(1)),
                Key::Home | Key::KP_Home => set(0),
                Key::End | Key::KP_End => set(n - 1),
                Key::Escape => {
                    if let Some(show) = show.upgrade() {
                        show.close();
                    }
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    });
    show.add_controller(keys);
    let click = gtk::GestureClick::new();
    click.connect_released(move |_, _, _, _| forward());
    show.add_controller(click);
    // However it ends, the deck's window comes back on the slide the show was on.
    show.connect_close_request({
        let (view, shown, at) = (view.clone(), shown.clone(), at.clone());
        move |_| {
            view.go(shown[at.get()]);
            if let Some(w) = view.window.upgrade() {
                w.present();
            }
            glib::Propagation::Proceed
        }
    });
    show.fullscreen();
    show.present();
}

fn show_deck(
    app: &gtk::Application,
    window: &gtk::ApplicationWindow,
    deck: Rc<Deck>,
    bench: Option<u32>,
) -> Rc<DeckWindow> {
    let canvas = SlideView::new(&deck, 0, 0);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.add_css_class("navigation-sidebar");
    list.update_property(&[gtk::accessible::Property::Label("Slides")]);
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .width_request(THUMB_WIDTH + 64)
        .build();
    // The slide list's context menu hangs off this box (crate::slide_list).
    let side = gtk::Box::new(gtk::Orientation::Vertical, 0);
    scrolled.set_vexpand(true);
    side.append(&scrolled);
    // The canvas, and the inspector to its right.
    let main = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    main.append(&canvas);
    main.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    paned.set_start_child(Some(&side));
    // The canvas and the inspector, or the outline in their place (crate::outline).
    let outline = OutlineView::new(&main);
    paned.set_end_child(Some(outline.widget()));
    paned.set_shrink_start_child(false);
    paned.set_resize_start_child(false);
    paned.set_vexpand(true);
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_margin_start(10);
    status.set_margin_end(10);
    status.set_margin_top(4);
    status.set_margin_bottom(4);
    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.append(&paned);
    page.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    page.append(&status);
    window.set_child(Some(&suite_ui::notice::host(&page)));

    let view = Rc::new_cyclic(|weak| DeckWindow {
        app: app.clone(),
        window: window.downgrade(),
        deck: deck.clone(),
        canvas: canvas.clone(),
        thumbs: RefCell::new(Vec::new()),
        list: list.clone(),
        scrolled,
        status: status.clone(),
        history: RefCell::new(History::new()),
        im: gtk::IMMulticontext::new(),
        inspector: Inspector::new(&deck, weak.clone()),
        outline,
    });
    main.append(view.inspector.widget());
    slide_list::rebuild(&view);
    editing::attach(&view);
    arrange::attach(&view);
    inspector::attach(&view);
    outline::attach(&view);
    list.connect_row_selected({
        let view = Rc::downgrade(&view);
        move |_, row| {
            if let (Some(view), Some(row)) = (view.upgrade(), row) {
                let i = row.index() as usize;
                if i != view.canvas.index() {
                    view.go(i);
                }
            }
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let view = view.clone();
        move |controller, key, _, mods| {
            use gdk::{Key, ModifierType};
            let shift = mods.contains(ModifierType::SHIFT_MASK);
            let outline = view.outline.has_focus();
            let text = view.canvas.editing().is_some() || outline;
            // Undo and Redo: the deck's one history, before the canvas's input method
            // sees the keys. New Slide adds a slide even while text is edited; Duplicate
            // and moving the slide up and down are the slide's only when no text is (on
            // the canvas or in the outline). Paste pastes a picture (crate::insert),
            // outside the inspector and the outline. The keys are crate::keys'.
            if let Some(action) = keys::find(Primary::HERE, Via::Window, key, mods) {
                match action {
                    Action::Undo => view.undo(),
                    Action::Redo => view.redo(),
                    Action::NewSlide => slide_list::new_slide(&view),
                    Action::Save => saving::save(&view),
                    Action::SaveAs => saving::save_as(&view),
                    Action::Outline => view.outline.toggle(),
                    Action::Duplicate if !text => slide_list::duplicate(&view),
                    Action::MoveUp if !text => slide_list::move_by(&view, -1),
                    Action::MoveDown if !text => slide_list::move_by(&view, 1),
                    Action::Paste if !view.inspector.has_focus() && !outline => {
                        if !insert::paste(&view) {
                            return glib::Propagation::Proceed;
                        }
                    }
                    _ => return glib::Propagation::Proceed,
                }
                return glib::Propagation::Stop;
            }
            // While text is edited, the keys are the text's (crate::editing, or
            // crate::outline); in the inspector, its controls'; in the menu bar (Windows'
            // row, after F10), the menus'.
            let modified = keys::modified(Primary::HERE, mods);
            let menu_bar = controller
                .widget()
                .and_downcast::<gtk::Window>()
                .and_then(|w| GtkWindowExt::focus(&w))
                .is_some_and(|f| f.ancestor(gtk::PopoverMenuBar::static_type()).is_some());
            if text || modified || view.inspector.has_focus() || menu_bar {
                return glib::Propagation::Proceed;
            }
            // A selected shape: the arrows nudge it a pixel (Shift: ten), Escape lets go,
            // Delete and Backspace delete it; other keys are the slide's.
            if view.canvas.selected().is_some() && !slide_list::has_focus(&view) {
                let by = if shift { 10.0 } else { 1.0 };
                let nudge = match key {
                    Key::Left | Key::KP_Left => Some((-by, 0.0)),
                    Key::Right | Key::KP_Right => Some((by, 0.0)),
                    Key::Up | Key::KP_Up => Some((0.0, -by)),
                    Key::Down | Key::KP_Down => Some((0.0, by)),
                    _ => None,
                };
                if let Some((dx, dy)) = nudge {
                    arrange::nudge(&view, dx, dy);
                    return glib::Propagation::Stop;
                }
                let delete = matches!(key, Key::Delete | Key::KP_Delete | Key::BackSpace);
                if delete && insert::delete(&view) {
                    return glib::Propagation::Stop;
                }
                if key == Key::Escape {
                    view.canvas.select(None);
                    return glib::Propagation::Stop;
                }
            }
            match key {
                Key::Page_Down
                | Key::KP_Page_Down
                | Key::Down
                | Key::KP_Down
                | Key::Right
                | Key::KP_Right => view.step(1),
                Key::Page_Up
                | Key::KP_Page_Up
                | Key::Up
                | Key::KP_Up
                | Key::Left
                | Key::KP_Left => view.step(-1),
                Key::Home | Key::KP_Home => view.go(0),
                Key::End | Key::KP_End => view.go(view.count().saturating_sub(1)),
                // Delete and Backspace in the slide list delete the slide.
                Key::Delete | Key::KP_Delete | Key::BackSpace if slide_list::has_focus(&view) => {
                    slide_list::delete(&view)
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    });
    window.add_controller(keys);
    // F5 shows the slideshow from the first slide, Shift+F5 from the current one.
    let from_start = gio::ActionEntry::builder("slideshow")
        .activate({
            let view = view.clone();
            move |_: &gtk::ApplicationWindow, _, _| slideshow(&view, 0)
        })
        .build();
    let from_here = gio::ActionEntry::builder("slideshow-here")
        .activate({
            let view = view.clone();
            move |_: &gtk::ApplicationWindow, _, _| slideshow(&view, view.canvas.index())
        })
        .build();
    let save = gio::ActionEntry::builder("save")
        .activate({
            let view = view.clone();
            move |_: &gtk::ApplicationWindow, _, _| saving::save(&view)
        })
        .build();
    // The menu bar's (macOS): Save As, Undo, Redo, the outline, and moving the slide, as
    // the window's keys do them.
    let run = |name: &str, run: fn(&Rc<DeckWindow>)| {
        let view = Rc::downgrade(&view);
        gio::ActionEntry::builder(name)
            .activate(move |_: &gtk::ApplicationWindow, _, _| {
                if let Some(view) = view.upgrade() {
                    run(&view);
                }
            })
            .build()
    };
    window.add_action_entries([
        from_start,
        from_here,
        save,
        run("save-as", saving::save_as),
        run("undo", |v| v.undo()),
        run("redo", |v| v.redo()),
        run("outline", |v| v.outline.toggle()),
        run("move-slide-up", |v| slide_list::move_by(v, -1)),
        run("move-slide-down", |v| slide_list::move_by(v, 1)),
    ]);
    // The deck's buttons: in the header bar on Linux, after its New and Open; on macOS and
    // Windows in a toolbar row over the deck, as the menu bar has New, Open, and Save.
    let bar = match window.titlebar().and_downcast::<gtk::HeaderBar>() {
        Some(header) => Toolbar::Header(header),
        None => {
            let bar = Toolbar::new();
            if let Some(row) = bar.put_on(window) {
                page.prepend(&row);
            }
            bar
        }
    };
    slide_list::attach(&view, window, &bar, &side);
    insert::attach(&view, &bar);
    if !suite_ui::chrome::MENU_BAR {
        let save = gtk::Button::from_icon_name("document-save-symbolic");
        save.set_action_name(Some("win.save"));
        save.set_tooltip_text(Some(&format!("Save ({})", keys::label(Action::Save))));
        save.update_property(&[gtk::accessible::Property::Label("Save")]);
        bar.pack_start(&save);
    }
    let play = gtk::Button::from_icon_name("media-playback-start-symbolic");
    play.set_action_name(Some("win.slideshow"));
    play.set_tooltip_text(Some(&format!(
        "Slideshow ({})",
        keys::label(Action::Slideshow)
    )));
    play.update_property(&[gtk::accessible::Property::Label("Slideshow")]);
    bar.pack_end(&play);
    bar.pack_end(view.outline.button());
    view.go(0);
    if let Some(frames) = bench {
        bench_draw(app, &view, frames);
    }
    view
}

/// FRAME-4's drawing benchmark: shows the deck's densest slide and redraws the canvas
/// every frame, measuring each frame's CPU time (frame clock before- to after-paint) and
/// the canvas snapshot's share of it; prints a JSON line and quits.
fn bench_draw(app: &gtk::Application, view: &Rc<DeckWindow>, frames: u32) {
    let index = view.deck.densest_slide();
    view.go(index);
    let canvas = view.canvas.clone();
    let Some(clock) = canvas.frame_clock() else {
        // Not realized yet: try again once it is.
        let (app, again) = (app.clone(), view.clone());
        view.canvas
            .connect_realize(move |_| bench_draw(&app, &again, frames));
        return;
    };
    #[derive(Default)]
    struct Rec {
        on: bool,
        start: Option<Instant>,
        cpu_ms: Vec<f64>,
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
    let (app, view) = (app.clone(), view.clone());
    canvas.add_tick_callback(move |canvas, _| {
        let n = ticks.get();
        ticks.set(n + 1);
        if n == 30 {
            rec.borrow_mut().on = true;
        } else if n > 30 + frames {
            let r = rec.borrow();
            let presentation = view.deck.presentation();
            let slide = &presentation.slides[index];
            println!(
                r#"{{"slide":{},"shapes":{},"canvas":[{},{}],"frames":{},"frame_cpu_ms":{},"snapshot_ms":{}}}"#,
                index + 1,
                slide.shapes.len(),
                canvas.width(),
                canvas.height(),
                r.cpu_ms.len(),
                summary(&r.cpu_ms),
                summary(&r.snapshot_ms),
            );
            app.quit();
            return glib::ControlFlow::Break;
        }
        canvas.queue_draw();
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
