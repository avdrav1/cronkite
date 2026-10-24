//! User-interface conventions shared by every app in the suite, in GTK 4
//! (docs/adr/0002-gtk4-for-the-suite.md): [`keys`], the primary modifier every shortcut
//! uses (Control on Linux and Windows, Command on macOS), [`im`], input methods for the
//! apps' own text widgets, and [`chrome`], where menus and a window's buttons go (header
//! bars on Linux; the system's title bars, a menu bar, and a toolbar row on macOS and
//! Windows), "Update Automatically", and the About dialog; [`notice`] shows
//! `Updated to <App> X.Y.Z` once after an update.
//!
//! [`fonts`] chooses how text finds its fonts and adds the bundled ones on macOS;
//! [`appearance`] switches the theme (and Windows' title bars) between light and dark;
//! [`launch`] hands the files an app is asked to open to GApplication's `open` and makes
//! the drop target for files dragged onto a window;
//! [`renderer`] picks how GTK draws. The command palette and dialogs may move here once
//! two apps need the same one.

pub mod appearance;
pub mod chrome;
pub mod fonts;
pub mod im;
pub mod keys;
pub mod launch;
pub mod notice;
pub mod renderer;

use suite_core::AppKind;

/// The line an app prints when it starts: `Brick starting...`.
pub fn startup_message(app: AppKind) -> String {
    format!("{} starting...", app.display_name())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_message_names_the_app() {
        assert_eq!(startup_message(AppKind::Brick), "Brick starting...");
        assert_eq!(startup_message(AppKind::Slate), "Slate starting...");
        assert_eq!(startup_message(AppKind::Frame), "Frame starting...");
    }
}
