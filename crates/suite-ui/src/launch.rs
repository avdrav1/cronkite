//! How the apps start (MAC-5, WIN-4): every file reaches an app through GApplication's
//! `open`, whoever names it, and `activate` means no file.
//!
//! - Command line (`brick FILE`, a Linux file manager's `Exec=brick %f`, Windows'
//!   `"brick.exe" "%1"` from Explorer's Open with or a registered file type): the app
//!   reads its own arguments, and [`run`] hands the files to GApplication's `open`; with
//!   none it emits `activate`.
//! - macOS (Finder, `open -a`, a file dropped on the Dock icon): Launch Services starts
//!   the app without arguments and sends an Apple Event, which GTK turns into
//!   `g_application_open` in the running process. GTK answers it only for an app with
//!   `HANDLES_OPEN` (`gtkapplication-quartz.c`); without it the spike's opens were dropped
//!   (DEC-11 C3). The app's `activate` comes first, so [`no_file`] holds back the window
//!   for no file until the launch has finished and shows it only if no file opened.
//!
//! GApplication never parses the command line. On Windows `g_application_run` ignores the
//! arguments it's given and reads the process's own (`g_win32_get_command_line`), keeping
//! as many of the last ones as it was given, so it would see the apps' own options
//! (`--bench-*`, an unknown option to it) and files as the shell wrote them. [`run`]
//! gives it the program alone, and opens the files from `handle-local-options`, which
//! GApplication emits before it chooses between `open` and `activate`; the same on every
//! system.
//!
//! The apps stay `NON_UNIQUE`: on Linux and Windows each launch is its own process, which
//! opens its own files; on macOS Launch Services keeps one process per app and GTK
//! delivers its opens inside it, so a second `open -a` reaches the running app without
//! D-Bus.

use gtk::{gio, glib, prelude::*};
use std::ops::ControlFlow;
use std::path::PathBuf;

/// Run `app` with `files` from the command line: they reach its `open` handler; without
/// any, `activate`. Relative paths are made absolute first, against the working folder.
/// On macOS GTK starts first, so its app menu has "Update Automatically"
/// (`chrome::macos_app_menu`).
pub fn run(app: &impl IsA<gio::Application>, files: &[PathBuf]) -> glib::ExitCode {
    #[cfg(target_os = "macos")]
    crate::chrome::macos_app_menu();
    let files: Vec<gio::File> = files
        .iter()
        .map(|f| gio::File::for_path(std::path::absolute(f).unwrap_or_else(|_| f.clone())))
        .collect();
    app.connect_handle_local_options(move |app, _| {
        if files.is_empty() {
            return ControlFlow::Continue(());
        }
        // What GApplication itself does with files on its command line.
        if let Err(e) = app.register(gio::Cancellable::NONE) {
            eprintln!("Failed to register: {e}");
            return ControlFlow::Break(glib::ExitCode::FAILURE);
        }
        app.open(&files, "");
        ControlFlow::Break(glib::ExitCode::SUCCESS)
    });
    // The program alone: on Windows, one argument keeps GApplication to one of the real
    // command line's, which it takes for the program's name.
    let program = std::env::args_os().next().unwrap_or_default();
    app.run_with_args_os(&[program])
}

/// `activate`'s work: `f` makes the app's window for no file (a start window, a new
/// document), unless a window is open by the time the launch has finished. `after_launch`
/// says when that is (`suite_platform::launch::when_finished`): on macOS a file from
/// Finder reaches `open` after `activate`, and only its window shows; elsewhere it's now.
/// The app is held meanwhile, so it doesn't quit for want of a window.
pub fn no_file(
    app: &gtk::Application,
    after_launch: fn(Box<dyn FnOnce()>),
    f: impl FnOnce(&gtk::Application) + 'static,
) {
    let (app, hold) = (app.clone(), app.hold());
    after_launch(Box::new(move || {
        if app.windows().is_empty() {
            f(&app);
        }
        drop(hold);
    }));
}

/// A drop target for files dragged from the file manager (Explorer, Finder, Files): a
/// [`gtk::gdk::FileList`] with [`gtk::gdk::DragAction::COPY`]. The caller connects `drop`.
pub fn file_drop_target() -> gtk::DropTarget {
    let target = gtk::DropTarget::new(
        gtk::gdk::FileList::static_type(),
        gtk::gdk::DragAction::COPY,
    );
    // Windows: GDK sets the drop's actions to those of each status GTK reports. When the
    // pointer crosses into another widget, GTK reports none for a moment. If the pointer
    // then stops, no new move puts the actions back, and letting go there was refused
    // (WIN-3's runner logs: the actions empty from the last move on). Answering COPY on
    // every enter and move sets them again.
    #[cfg(windows)]
    {
        target.connect_enter(|_, _, _| gtk::gdk::DragAction::COPY);
        target.connect_motion(|_, _, _| gtk::gdk::DragAction::COPY);
    }
    target
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// What `run` makes GApplication emit for `files`: `activate`, or `open` with paths.
    fn emitted(files: &[PathBuf]) -> Vec<String> {
        let app = gio::Application::new(
            None,
            gio::ApplicationFlags::NON_UNIQUE | gio::ApplicationFlags::HANDLES_OPEN,
        );
        let seen = Rc::new(RefCell::new(Vec::new()));
        app.connect_activate({
            let seen = seen.clone();
            move |_| seen.borrow_mut().push("activate".to_owned())
        });
        app.connect_open({
            let seen = seen.clone();
            move |_, files, _| {
                for f in files {
                    let path = f.path().expect("a local file");
                    seen.borrow_mut().push(format!("open {}", path.display()));
                }
            }
        });
        assert_eq!(run(&app, files), glib::ExitCode::SUCCESS);
        seen.take()
    }

    /// One test, so no two runs share the main context at once. It runs again as a child
    /// process, with `CHILD` naming a file and libtest's own options last on its command
    /// line, where GLib on Windows would have taken them for the app's (WIN-1's note).
    #[test]
    fn files_reach_open_and_none_activates() {
        const CHILD: &str = "SUITE_UI_LAUNCH_CHILD_FILE";
        if let Some(file) = std::env::var_os(CHILD) {
            for line in emitted(&[PathBuf::from(file)]) {
                println!("emitted: {line}");
            }
            return;
        }
        assert_eq!(emitted(&[]), ["activate"]);
        let cwd = std::env::current_dir().unwrap();
        let absolute = std::env::temp_dir().join("c.pptx");
        assert_eq!(
            emitted(&[
                PathBuf::from("a b.csv"),
                PathBuf::from("-x.md"),
                PathBuf::from("ü 文件.docx"),
                absolute.clone(),
            ]),
            [
                format!("open {}", cwd.join("a b.csv").display()),
                format!("open {}", cwd.join("-x.md").display()),
                format!("open {}", cwd.join("ü 文件.docx").display()),
                format!("open {}", absolute.display()),
            ]
        );
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "launch::tests::files_reach_open_and_none_activates",
                "--exact",
                "--nocapture",
                "--test-threads",
                "1",
            ])
            .env(CHILD, "--bench-x.csv")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&child.stdout);
        assert!(
            child.status.success(),
            "{stdout}\n{}",
            String::from_utf8_lossy(&child.stderr)
        );
        // libtest's "test … ... " precedes the first line.
        let seen: Vec<_> = stdout
            .lines()
            .filter_map(|l| l.split_once("emitted: "))
            .map(|(_, s)| s)
            .collect();
        assert_eq!(
            seen,
            [format!("open {}", cwd.join("--bench-x.csv").display())]
        );
    }
}
