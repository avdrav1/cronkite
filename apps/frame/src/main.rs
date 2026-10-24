//! Frame: a fast, lightweight Linux-native presentation app.
//!
//! Usage: `frame [FILE.pptx] [--version] [--bench-draw FRAMES]`
//!
//! FR0 is a viewer (docs/specs/frame.md, ADR 0004): each `.pptx` opens in its own window
//! with a slide list and a canvas, and F5 starts a full-screen slideshow. Without a file,
//! and on Ctrl+N, it opens a new deck, Frame's built-in blank 16:9 one, ready to type;
//! Ctrl+O picks a file, and files dropped on a window open. FR1
//! edits (ADR 0006): slide text on the canvas, adding, duplicating, deleting, and moving
//! slides, moving and resizing shapes with snapping, formatting in the inspector beside
//! the canvas, inserting text boxes, shapes, pictures, and tables, text shrinking to fit
//! its box, and writing titles and bullets in an outline of the deck; with undo.
//! `--bench-draw` redraws the deck's densest slide for FRAMES frames and prints how long
//! each took (FRAME-4's acceptance test).

// On Windows a GUI program: no console window opens with it (WIN-3).
#![cfg_attr(windows, windows_subsystem = "windows")]

mod arrange;
mod deck;
mod editing;
mod fit;
mod insert;
mod inspector;
mod keys;
mod menus;
mod outline;
mod render;
mod saving;
mod slide_list;
mod slide_view;
mod window;

use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::Cell;
use std::io::IsTerminal;
use std::path::PathBuf;
use suite_core::AppKind;

const APP: AppKind = AppKind::Frame;
/// The desktop entry and icon take this name (packaging/dev.cronkite.Frame.*), so docks
/// match the window to its entry.
const APP_ID: &str = APP.app_id();
const USAGE: &str = "usage: frame [FILE.pptx] [--version] [--bench-draw FRAMES]";

struct Args {
    file: Option<PathBuf>,
    /// `--bench-draw FRAMES`.
    bench: Option<u32>,
}

fn parse_args() -> Result<Args, String> {
    let (mut file, mut bench) = (None, None);
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--version" => {
                println!("frame {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--bench-draw" => {
                let v = it.next().ok_or("--bench-draw needs a frame count")?;
                let n: u32 = v
                    .parse()
                    .map_err(|_| format!("bad number for --bench-draw: {v:?}"))?;
                bench = Some(n.max(1));
            }
            _ if file.is_none() && !a.starts_with("--") => file = Some(PathBuf::from(a)),
            _ => return Err(format!("unexpected argument {a:?}")),
        }
    }
    if bench.is_some() && file.is_none() {
        return Err("--bench-draw needs a file".into());
    }
    Ok(Args { file, bench })
}

fn main() -> glib::ExitCode {
    // Before anything prints: on Windows the terminal Frame was started from (WIN-3).
    suite_platform::console::attach();
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("frame: {e}\n{USAGE}");
            return glib::ExitCode::from(2);
        }
    };
    // Logs and the startup line go to stderr: stdout carries the benchmark's report.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    tracing::info!("application starting");
    if let Err(e) = suite_platform::current() {
        eprintln!("frame: {e}");
        return glib::ExitCode::FAILURE;
    }
    eprintln!("{}", suite_ui::startup_message(APP));
    // This run's part in applying updates (ADR 0012); before the bundle's environment.
    let updates = suite_update::launch(APP, env!("CARGO_PKG_VERSION"));
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
        let new = gio::ActionEntry::builder("new")
            .activate(|app: &gtk::Application, _, _| window::new_deck(app))
            .build();
        let open = gio::ActionEntry::builder("open")
            .activate(|app: &gtk::Application, _, _| window::choose_and_open(app))
            .build();
        // The menu bar's Close Window and Quit (macOS) and Exit (Windows): Quit closes each
        // window.
        let close = gio::ActionEntry::builder("close")
            .activate(|app: &gtk::Application, _, _| {
                if let Some(w) = app.active_window() {
                    w.close();
                }
            })
            .build();
        let quit = gio::ActionEntry::builder("quit")
            .activate(|app: &gtk::Application, _, _| {
                for window in app.windows() {
                    window.close();
                }
            })
            .build();
        app.add_action_entries([new, open, close, quit]);
        suite_ui::chrome::install(app, APP, env!("CARGO_PKG_VERSION"));
        if suite_ui::chrome::MENU_BAR {
            app.set_menubar(Some(&menus::bar()));
        }
        for (action, accels) in keys::accels() {
            let accels: Vec<&str> = accels.iter().map(String::as_str).collect();
            app.set_accels_for_action(action, &accels);
        }
    });
    // Files, from the command line or the system (Finder), each open in their own window;
    // the benchmark runs on the command line's (`suite_ui::launch`).
    let bench = Cell::new(args.bench);
    app.connect_open(move |app, files, _| {
        for file in files {
            match file.path() {
                Some(path) => window::open(app, &path, bench.take()),
                None => window::alert(
                    app.active_window().as_ref(),
                    &format!("Cannot open {}", file.uri()),
                    "Only local files can be opened.",
                ),
            }
        }
    });
    app.connect_activate(|app| {
        suite_ui::launch::no_file(app, suite_platform::launch::when_finished, window::new_deck)
    });
    let code = suite_ui::launch::run(&app, args.file.as_slice());
    updates.exit();
    code
}
