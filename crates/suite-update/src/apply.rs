//! Applying a staged release, per platform (ADR 0012, item 4): Linux right after a check
//! staged it (UPD-4), macOS by swapping the bundle at the last instance's exit or at a
//! start (UPD-5), Windows by the staged Setup.exe at the last instance's exit (UPD-6).
//! Each re-checks the staged file's SHA-256 against its record before using it, and
//! records what it applied ([`crate::session::record_applied`]).

use crate::session::Session;
use crate::stage::Staged;
use std::path::Path;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// After a check found `staged` in `updates` (just staged, or by an earlier check), on the
/// check's thread with the updates lock held.
#[allow(unused_variables)]
pub(crate) fn after_check(app: suite_core::AppKind, updates: &Path, staged: &Staged) {
    #[cfg(target_os = "linux")]
    linux::after_check(app, updates, staged);
}

/// At [`crate::session::launch`].
#[allow(unused_variables)]
pub(crate) fn at_launch(session: &Session) {
    #[cfg(target_os = "macos")]
    macos::at_launch(session);
}

/// At [`Session::exit`].
#[allow(unused_variables)]
pub(crate) fn at_exit(session: &Session) {
    #[cfg(target_os = "macos")]
    macos::at_exit(session);
    #[cfg(windows)]
    windows::at_exit(session);
}
