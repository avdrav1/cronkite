//! Whether the running program is an install our installers made (ADR 0012, decision 5):
//! only those update themselves; `cargo run`, builds in `target/`, and packages others make
//! never do.
//!
//! - Linux: the program is `~/.local/bin/<command>` and its desktop entry is in
//!   `$XDG_DATA_HOME/applications` (else `~/.local/share/applications`), as `install.sh`
//!   puts them.
//! - macOS: a self-contained `<Name>.app` (`suite_platform::bundle`) in a folder the user
//!   can write, so the bundle can be swapped.
//! - Windows: the per-user install folder `suite_platform::bundle` finds.

use std::path::{Path, PathBuf};
use suite_core::AppKind;

/// Where a Linux install puts things, from the home folder and the data folder.
pub struct LinuxHome {
    pub home: PathBuf,
    pub data: PathBuf,
}

impl LinuxHome {
    /// The user's: `$HOME` (else the system's idea of it) and `$XDG_DATA_HOME` when
    /// absolute, else `~/.local/share`, as `install.sh` has them.
    pub fn current() -> Option<LinuxHome> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(std::env::home_dir)?;
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".local/share"));
        Some(LinuxHome { home, data })
    }
}

/// Whether `exe` (the running program's path, links resolved) is `app` as `install.sh`
/// installed it under `home`.
pub fn linux(app: AppKind, exe: &Path, home: &LinuxHome) -> bool {
    let bin = home.home.join(".local/bin").join(app.command());
    let entry = home
        .data
        .join("applications")
        .join(format!("{}.desktop", app.app_id()));
    std::fs::canonicalize(&bin).is_ok_and(|b| b == exe) && entry.is_file()
}

/// Whether the running program is an installer's install of `app`.
pub fn current(app: AppKind) -> bool {
    let Ok(exe) = std::env::current_exe().and_then(std::fs::canonicalize) else {
        return false;
    };
    if cfg!(target_os = "macos") {
        suite_platform::bundle::install_root(&exe)
            .and_then(|root| root.parent().map(Path::to_path_buf))
            .is_some_and(|folder| writable(&folder))
    } else if cfg!(windows) {
        suite_platform::bundle::install_root(&exe).is_some()
    } else {
        LinuxHome::current().is_some_and(|home| linux(app, &exe, &home))
    }
}

/// Whether a file can be made in `folder`: tried, then removed.
fn writable(folder: &Path) -> bool {
    let probe = folder.join(format!(".cronkite-update-probe-{}", std::process::id()));
    let made = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    made
}
