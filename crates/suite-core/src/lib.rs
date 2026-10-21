//! Application-independent primitives shared by every app in the suite.
//!
//! No UI, OS, or file code lives here, and nothing here depends on another suite crate or
//! on an app. A type belongs here only when at least two apps need the same thing.

use std::fmt;

/// Which application of the suite is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppKind {
    /// Spreadsheet; today a fast CSV editor.
    Brick,
    /// Word processor.
    Slate,
    /// Presentation app (working name).
    Frame,
    /// Raster image editor.
    Etch,
}

impl AppKind {
    /// Every application, in a stable order.
    pub const ALL: [AppKind; 4] = [
        AppKind::Brick,
        AppKind::Slate,
        AppKind::Frame,
        AppKind::Etch,
    ];

    /// The name shown to people: window titles, menus, messages.
    pub fn display_name(self) -> &'static str {
        match self {
            AppKind::Brick => "Brick",
            AppKind::Slate => "Slate",
            AppKind::Frame => "Frame",
            AppKind::Etch => "Etch",
        }
    }

    /// The command that starts it, which also names its per-user folders on Linux
    /// (`~/.local/state/brick`).
    pub const fn command(self) -> &'static str {
        match self {
            AppKind::Brick => "brick",
            AppKind::Slate => "slate",
            AppKind::Frame => "frame",
            AppKind::Etch => "etch",
        }
    }

    /// The application ID: GTK's, and the desktop entry's and icon's name, on Linux; the
    /// bundle identifier, and the name of its per-user folders, on macOS (ADR 0011).
    pub const fn app_id(self) -> &'static str {
        match self {
            AppKind::Brick => "dev.cronkite.Brick",
            AppKind::Slate => "dev.cronkite.Slate",
            AppKind::Frame => "dev.cronkite.Frame",
            AppKind::Etch => "dev.cronkite.Etch",
        }
    }

    /// Where its releases are published: the folder holding `latest.json`, its signature,
    /// and `releases/` (ADR 0012). Brick's is the bucket's root, the others' under their
    /// command.
    pub const fn release_url(self) -> &'static str {
        match self {
            AppKind::Brick => "https://dl.cronkite.cc",
            AppKind::Slate => "https://dl.cronkite.cc/slate",
            AppKind::Frame => "https://dl.cronkite.cc/frame",
            AppKind::Etch => "https://dl.cronkite.cc/etch",
        }
    }
}

impl fmt::Display for AppKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_names_are_correct() {
        let names: Vec<_> = AppKind::ALL.iter().map(|a| a.display_name()).collect();
        assert_eq!(names, ["Brick", "Slate", "Frame", "Etch"]);
        assert_eq!(AppKind::Etch.to_string(), "Etch");
        let commands: Vec<_> = AppKind::ALL.iter().map(|a| a.command()).collect();
        assert_eq!(commands, ["brick", "slate", "frame", "etch"]);
        assert_eq!(AppKind::Slate.app_id(), "dev.cronkite.Slate");
    }
}
