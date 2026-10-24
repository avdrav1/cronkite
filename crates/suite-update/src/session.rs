//! A running instance's part in applying updates (ADR 0012, item 4). The apps are
//! non-unique, so each instance of an installed app holds a `suite_file` owner lock in its
//! updates folder from [`launch`] until it exits; an update that can't be applied while the
//! app runs (macOS's bundle, Windows' install folder) is applied only by an instance that
//! finds no other live owner there: at [`Session::exit`], or at a later [`launch`].
//!
//! The step that applies a release records its version ([`record_applied`]), so the first
//! start of that version can say so once ([`notice`]).

use crate::{Gate, Skip};
use std::fs;
use std::path::{Path, PathBuf};
use suite_core::AppKind;

/// This process's part in applying `app`'s updates, from [`launch`] until [`Session::exit`].
pub struct Session {
    pub app: AppKind,
    /// The running version, `X.Y.Z`.
    pub running: &'static str,
    /// The updates folder ([`crate::updates_dir`]); `None` when this run applies nothing
    /// (why: the gate's [`Skip`]).
    pub updates: Option<PathBuf>,
}

/// Call first in `main`, before GTK starts and before any thread does: registers this
/// instance in the updates folder and applies what only a start can apply (macOS: a release
/// staged by an instance that was killed, swapped in, then the new bundle run in this
/// process's place). Nothing at all unless the app updates itself
/// ([`Gate::skip`]: compiled out, `CRONKITE_NO_UPDATE`, the setting off, not installed).
pub fn launch(app: AppKind, running: &'static str) -> Session {
    let updates = match gate_skip(app) {
        Some(skip) => {
            tracing::debug!("update: {skip:?}");
            None
        }
        None => {
            let dir = crate::updates_dir(app);
            match fs::create_dir_all(&dir).and_then(|_| suite_file::owner::hold(&dir)) {
                Ok(()) => Some(dir),
                Err(e) => {
                    tracing::warn!("update: updates folder: {e}");
                    None
                }
            }
        }
    };
    let session = Session {
        app,
        running,
        updates,
    };
    crate::apply::at_launch(&session);
    session
}

impl Session {
    /// Call when the app's last window has closed and `main` is about to return: applies
    /// what only an exit can (macOS's bundle swap, Windows' Setup) when no other instance
    /// of the app is running.
    pub fn exit(self) {
        crate::apply::at_exit(&self);
    }
}

/// The gate [`crate::start`] checks, for a run of `app`.
fn gate_skip(app: AppKind) -> Option<Skip> {
    let no_update = std::env::var_os("CRONKITE_NO_UPDATE");
    let settings = crate::settings_path(app);
    Gate {
        compiled_out: option_env!("CRONKITE_NO_SELF_UPDATE").is_some(),
        no_update: no_update.as_deref(),
        settings: &settings,
        installed: crate::install::current(app),
    }
    .skip()
}

/// Whether another running process holds an owner lock in `updates`.
pub fn others_running(updates: &Path) -> bool {
    let me = std::process::id();
    let Ok(entries) = fs::read_dir(updates) else {
        return false;
    };
    entries.flatten().any(|e| {
        let name = e.file_name();
        let pid = name
            .to_str()
            .and_then(|n| n.strip_prefix("owner-")?.strip_suffix(".lock"))
            .and_then(|p| p.parse::<u32>().ok());
        match pid {
            Some(pid) if pid != me => {
                let alive = suite_file::owner::alive(updates, pid);
                if !alive {
                    suite_file::owner::release(updates, pid);
                }
                alive
            }
            _ => false,
        }
    })
}

pub(crate) const APPLIED: &str = "applied";

/// Record that `version` has been applied (or handed to the installer that applies it),
/// for [`notice`] at its first start.
pub fn record_applied(updates: &Path, version: &str) -> std::io::Result<()> {
    crate::stage::write_replacing(
        &updates.join(APPLIED),
        format!("version={version}\n").as_bytes(),
    )
}

/// "Updated to <App> X.Y.Z" at the first start of a version an update applied, once: the
/// record goes when it names the running version or an older one (a newer one stays: its
/// installer hasn't run yet, or failed).
pub fn notice(app: AppKind, running: &str) -> Option<String> {
    notice_in(&crate::updates_dir(app), app, running)
}

pub(crate) fn notice_in(updates: &Path, app: AppKind, running: &str) -> Option<String> {
    let path = updates.join(APPLIED);
    let text = fs::read_to_string(&path).ok()?;
    let version = text
        .lines()
        .find_map(|l| l.strip_prefix("version="))
        .unwrap_or_default()
        .trim()
        .to_owned();
    if crate::manifest::newer(&version, running) {
        return None;
    }
    let _ = fs::remove_file(&path);
    (version == running).then(|| format!("Updated to {} {running}", app.display_name()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("suite-update-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_notice_shows_once_at_the_applied_versions_first_start() {
        let dir = folder("notice");
        assert_eq!(notice_in(&dir, AppKind::Brick, "0.6.0"), None, "no record");
        record_applied(&dir, "0.6.0").unwrap();
        assert_eq!(
            notice_in(&dir, AppKind::Brick, "0.5.0"),
            None,
            "not applied yet"
        );
        assert_eq!(
            notice_in(&dir, AppKind::Brick, "0.6.0").as_deref(),
            Some("Updated to Brick 0.6.0")
        );
        assert_eq!(
            notice_in(&dir, AppKind::Brick, "0.6.0"),
            None,
            "never again"
        );
        record_applied(&dir, "0.6.0").unwrap();
        assert_eq!(
            notice_in(&dir, AppKind::Brick, "0.7.0"),
            None,
            "an older record goes"
        );
        assert!(!dir.join(APPLIED).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn another_live_owner_counts_and_a_gone_ones_lock_goes() {
        let dir = folder("owners");
        suite_file::owner::hold(&dir).unwrap();
        assert!(!others_running(&dir), "this process alone");
        let other = fs::File::create(suite_file::owner::lock_path(&dir, 777_777)).unwrap();
        other.lock().unwrap();
        assert!(others_running(&dir));
        drop(other);
        assert!(!others_running(&dir));
        assert!(!suite_file::owner::lock_path(&dir, 777_777).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
