//! macOS (UPD-5, ADR 0012 item 4): GTK loads icons, pixbuf loaders, and fonts from the
//! bundle while the app runs, so a staged release can't replace the bundle under a running
//! instance. It's swapped in ([`super::mac_bundle`]) by an instance that finds no other
//! running ([`crate::session::others_running`]):
//!
//! - at its exit ([`at_exit`]), for the next start;
//! - at a start ([`at_launch`]: a release staged by an instance that was killed, or that
//!   exited while a check still held the updates lock), before GTK, the bundle's
//!   environment, or any thread; then this process runs the new bundle's executable in its
//!   own place (`execv`, same arguments and pid), so the first window is the new version's.
//!   That start carries `CRONKITE_UPDATE_RELAUNCHED` (the version it came from), so it
//!   never swaps and runs itself again: no loop, whatever the bundle holds.

use super::mac_bundle::{self, Applied};
use crate::session::{others_running, Session};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};

/// Set on the run [`at_launch`] starts in its own place: the version it updated from.
const RELAUNCHED: &str = "CRONKITE_UPDATE_RELAUNCHED";

pub(super) fn at_launch(session: &Session) {
    if let Some(from) = std::env::var_os(RELAUNCHED) {
        // Before any thread (the caller's contract), so nothing reads the environment
        // meanwhile; the app's own children never see it.
        std::env::remove_var(RELAUNCHED);
        tracing::info!(
            "update: {} {} started in place of {}",
            session.app,
            session.running,
            from.to_string_lossy()
        );
        return;
    }
    let Some(updates) = session.updates.as_deref() else {
        return;
    };
    if others_running(updates) {
        return;
    }
    let Some(bundle) = running_bundle() else {
        return;
    };
    if apply(session, updates, &bundle) {
        relaunch(session, &bundle);
    }
}

pub(super) fn at_exit(session: &Session) {
    let Some(updates) = session.updates.as_deref() else {
        return;
    };
    if others_running(updates) {
        tracing::debug!("update: another instance runs; the update waits for it");
        return;
    }
    if let Some(bundle) = running_bundle() {
        apply(session, updates, &bundle);
    }
}

/// The running program's `<Name>.app`.
fn running_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .ok()?;
    suite_platform::bundle::install_root(&exe)
}

/// Swap the staged release in for `bundle`; whether it did. The outcome goes to the log.
fn apply(session: &Session, updates: &Path, bundle: &Path) -> bool {
    let app = session.app;
    match mac_bundle::apply(updates, session.running, bundle, app.command(), || {
        others_running(updates)
    }) {
        Ok(Applied::Version(v)) => {
            tracing::info!("update: {app} {v} swapped in at {}", bundle.display());
            true
        }
        Ok(Applied::Deferred) => {
            tracing::info!("update: another instance started; the update waits for it");
            false
        }
        Ok(outcome) => {
            tracing::debug!("update: {outcome:?}");
            false
        }
        Err(e) => {
            tracing::warn!("update: not applied: {e}");
            false
        }
    }
}

/// Run the new bundle's executable in this process's place, with this run's arguments;
/// returns only when that fails.
fn relaunch(session: &Session, bundle: &Path) {
    let exe = bundle.join("Contents/MacOS").join(session.app.command());
    let mut args = std::env::args_os();
    let mut command = std::process::Command::new(&exe);
    if let Some(arg0) = args.next() {
        command.arg0(arg0);
    }
    let err = command.args(args).env(RELAUNCHED, session.running).exec();
    tracing::warn!("update: can't start {}: {err}", exe.display());
}

#[cfg(test)]
mod tests {
    use super::mac_bundle::tests::{folder, stage_release};
    use super::*;
    use std::fs;
    use std::process::Command;

    fn run(cmd: &mut Command) -> std::process::Output {
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "{cmd:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    /// A real ad-hoc-signed bundle, as `scripts/mac_bundle.sh` makes them (a Mach-O
    /// executable, `Contents/Frameworks`, resources), at `version`, in `dir/name.app`.
    fn signed_bundle(dir: &Path, version: &str) -> PathBuf {
        let app = dir.join("Brick.app");
        let c = app.join("Contents");
        fs::create_dir_all(c.join("MacOS")).unwrap();
        fs::create_dir_all(c.join("Frameworks")).unwrap();
        fs::create_dir_all(c.join("Resources")).unwrap();
        fs::write(
            c.join("Info.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>brick</string>
<key>CFBundleIdentifier</key><string>dev.cronkite.UpdateTest</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
</dict></plist>
"#
            ),
        )
        .unwrap();
        fs::copy("/usr/bin/true", c.join("MacOS/brick")).unwrap();
        fs::write(c.join("Resources/version.txt"), version).unwrap();
        run(Command::new("codesign")
            .args(["--force", "-s", "-"])
            .arg(&app));
        app
    }

    #[test]
    fn the_running_bundle_is_found_from_its_executable() {
        let dir = folder("found");
        let app = signed_bundle(&dir.join("Applications"), "1.0.0");
        let exe = app.join("Contents/MacOS/brick");
        assert_eq!(
            suite_platform::bundle::install_root(&exe).as_deref(),
            Some(app.as_path())
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_swapped_in_signed_bundle_verifies_and_carries_no_quarantine() {
        let dir = folder("signed");
        let apps = dir.join("Applications");
        let updates = dir.join("updates");
        let app = signed_bundle(&apps, "1.0.0");
        // The release, made as mac_bundle.sh makes it: tar of the signed bundle alone.
        let build = dir.join("build");
        fs::create_dir_all(&build).unwrap();
        signed_bundle(&build, "1.0.1");
        let tarball = dir.join("release.tar.gz");
        run(Command::new("tar")
            .env("COPYFILE_DISABLE", "1")
            .arg("-C")
            .arg(&build)
            .arg("-czf")
            .arg(&tarball)
            .arg("Brick.app"));
        stage_release(&updates, "1.0.1", &fs::read(&tarball).unwrap());

        let got = mac_bundle::apply(&updates, "1.0.0", &app, "brick", || false);
        assert_eq!(got, Ok(Applied::Version("1.0.1".into())));
        assert_eq!(
            fs::read_to_string(app.join("Contents/Resources/version.txt")).unwrap(),
            "1.0.1"
        );
        run(Command::new("codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(&app));
        let quarantine = Command::new("xattr")
            .args(["-r", "-p", "com.apple.quarantine"])
            .arg(&app)
            .output()
            .unwrap();
        assert!(
            quarantine.stdout.is_empty(),
            "quarantined: {}",
            String::from_utf8_lossy(&quarantine.stdout)
        );
        // It runs.
        run(&mut Command::new(app.join("Contents/MacOS/brick")));
        let _ = fs::remove_dir_all(&dir);
    }
}
