//! Swapping a staged macOS release in for an app bundle (UPD-5, ADR 0012 item 4): the
//! filesystem half of [`super::macos`], given the bundle's path, so it runs and is tested on
//! any Unix.
//!
//! The release tarball (`scripts/mac_bundle.sh`) holds `<Name>.app` alone. With the updates
//! lock held, so no check (this process's own included) stages meanwhile, and only when the
//! staged version is newer than the running one, [`apply`]:
//!
//! 1. checks the staged file's SHA-256 against its record again (a file changed since the
//!    check verified it is removed, and the next check downloads it anew);
//! 2. unpacks it beside the bundle, in the same folder under a hidden temp name
//!    (`.<Name>.app.update-<pid>`), so the swap is a rename on one volume, and checks it
//!    holds one `.app` with `Contents/Info.plist` and `Contents/MacOS/<command>`;
//! 3. asks once more whether another instance runs (one may have started while it
//!    unpacked), and leaves the bundle alone if one does;
//! 4. swaps it in by two renames: the bundle aside (`.<Name>.app.old-<pid>`), the new one in
//!    its place; when the second fails the first is undone, so a bundle is always there;
//! 5. removes the old bundle and the staged file, and records the version applied
//!    ([`crate::session::record_applied`]) for the next start's notice.
//!
//! Leftovers of an apply cut short (a crash between the steps) are removed by the next one.
//! Files written here get no quarantine attribute (only programs that ask for it set it),
//! and the bundle's ad-hoc signature is in its files, so it travels with them.

use crate::stage;
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

/// What [`apply`] did.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Applied {
    /// No staged release newer than the running version.
    Nothing,
    /// A check holds the updates lock: the next exit or start tries again.
    Busy,
    /// Another instance started while the release was unpacked: the bundle stays.
    Deferred,
    /// This version is in the bundle's place.
    Version(String),
}

/// Swap the release staged in `updates`, if newer than `running`, in for `bundle`, whose
/// executable is `Contents/MacOS/<command>`; `others` says whether another instance of the
/// app runs now. An error leaves the bundle as it was.
pub(super) fn apply(
    updates: &Path,
    running: &str,
    bundle: &Path,
    command: &str,
    others: impl Fn() -> bool,
) -> Result<Applied, String> {
    let Some(_lock) = stage::lock(updates).map_err(|e| format!("updates lock: {e}"))? else {
        return Ok(Applied::Busy);
    };
    let Some(staged) =
        stage::staged(updates).filter(|s| crate::manifest::newer(&s.version, running))
    else {
        return Ok(Applied::Nothing);
    };
    let sha = sha256(&staged.file).map_err(|e| format!("{}: {e}", staged.file.display()))?;
    if sha != staged.sha256 {
        let _ = fs::remove_file(&staged.file);
        return Err(format!(
            "the staged {} has sha256 {sha}, not {}: removed",
            staged.file.display(),
            staged.sha256
        ));
    }
    let places =
        Places::of(bundle).ok_or_else(|| format!("{} isn't a bundle", bundle.display()))?;
    places.sweep();
    let unpacked = unpack(&staged.file, &places.update, command);
    let new = match unpacked {
        Ok(new) => new,
        Err(e) => {
            let _ = fs::remove_dir_all(&places.update);
            return Err(format!("{}: {e}", staged.file.display()));
        }
    };
    if others() {
        let _ = fs::remove_dir_all(&places.update);
        return Ok(Applied::Deferred);
    }
    let swapped = swap(bundle, &new, &places.old, |from, to| fs::rename(from, to));
    let _ = fs::remove_dir_all(&places.update);
    swapped.map_err(|e| format!("swapping in {}: {e}", bundle.display()))?;
    if let Err(e) = fs::remove_dir_all(&places.old) {
        tracing::warn!("update: removing {}: {e}", places.old.display());
    }
    let _ = fs::remove_file(&staged.file);
    if let Err(e) = crate::session::record_applied(updates, &staged.version) {
        tracing::warn!("update: recording {}: {e}", staged.version);
    }
    Ok(Applied::Version(staged.version))
}

/// The temp names beside a bundle `<Name>.app`.
struct Places {
    folder: PathBuf,
    /// `.<Name>.app.`: what this process's and earlier applies' temp names start with.
    prefix: String,
    /// Where the release is unpacked.
    update: PathBuf,
    /// Where the old bundle goes before it's removed.
    old: PathBuf,
}

impl Places {
    fn of(bundle: &Path) -> Option<Places> {
        let folder = bundle.parent()?.to_path_buf();
        let name = bundle.file_name()?.to_str()?;
        let prefix = format!(".{name}.");
        let pid = std::process::id();
        Some(Places {
            update: folder.join(format!("{prefix}update-{pid}")),
            old: folder.join(format!("{prefix}old-{pid}")),
            folder,
            prefix,
        })
    }

    /// Remove what an earlier apply cut short left: its unpacked release or the old bundle
    /// it set aside. Called with the updates lock held and no other instance running, so
    /// none is in use.
    fn sweep(&self) {
        let Ok(entries) = fs::read_dir(&self.folder) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let leftover = name.to_str().is_some_and(|n| {
                n.strip_prefix(&self.prefix)
                    .is_some_and(|rest| rest.starts_with("update-") || rest.starts_with("old-"))
            });
            if leftover {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
}

/// Unpack the `.tar.gz` at `tarball` into `dir` (made anew) and return the one `.app` it
/// holds, which must have `Contents/Info.plist` and an executable `Contents/MacOS/<command>`.
/// Entries reaching outside `dir` are skipped (the `tar` crate's `unpack`).
fn unpack(tarball: &Path, dir: &Path, command: &str) -> io::Result<PathBuf> {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir(dir)?;
    let gz = flate2::read::GzDecoder::new(BufReader::new(File::open(tarball)?));
    tar::Archive::new(gz).unpack(dir)?;
    let mut top = Vec::new();
    for entry in fs::read_dir(dir)? {
        top.push(entry?.path());
    }
    let bad = |why: String| io::Error::new(io::ErrorKind::InvalidData, why);
    let [app] = top.as_slice() else {
        return Err(bad(format!(
            "the release holds {} entries, not one .app",
            top.len()
        )));
    };
    if app.extension().is_none_or(|e| e != "app") || !app.is_dir() {
        return Err(bad(format!("the release holds {}, not a .app", name(app))));
    }
    let exe = app.join("Contents/MacOS").join(command);
    if !app.join("Contents/Info.plist").is_file() || !executable(&exe) {
        return Err(bad(format!(
            "{} has no Contents/Info.plist or Contents/MacOS/{command}",
            name(app)
        )));
    }
    Ok(app.clone())
}

fn name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Put `new` in `bundle`'s place by two renames through `rename`: `bundle` to `old`, then
/// `new` to `bundle`; when the second fails, `old` goes back.
fn swap(
    bundle: &Path,
    new: &Path,
    old: &Path,
    rename: impl Fn(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    rename(bundle, old)?;
    if let Err(e) = rename(new, bundle) {
        if let Err(back) = rename(old, bundle) {
            tracing::error!(
                "update: {} couldn't go back to {}: {back}",
                old.display(),
                bundle.display()
            );
        }
        return Err(e);
    }
    Ok(())
}

/// The SHA-256 of the file at `path`, in lowercase hex.
fn sha256(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        digest.update(&buf[..n]);
    }
    Ok(digest
        .finish()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::manifest::{Download, Release};
    use std::cell::Cell;

    /// A fresh folder: `Applications/` with the bundle and `updates/`.
    pub(crate) fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("suite-update-mac-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Applications")).unwrap();
        fs::create_dir_all(dir.join("updates")).unwrap();
        dir
    }

    /// A release tarball of `files` (path, contents, mode).
    pub(crate) fn tarball(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        for (path, data, mode) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(*mode);
            h.set_cksum();
            tar.append_data(&mut h, path, *data).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    /// Brick.app at version `v`: its executable says `v`.
    fn release(v: &str) -> Vec<u8> {
        tarball(&[
            ("Brick.app/Contents/Info.plist", b"<plist/>", 0o644),
            ("Brick.app/Contents/MacOS/brick", v.as_bytes(), 0o755),
            ("Brick.app/Contents/Frameworks/libgtk.dylib", b"gtk", 0o644),
        ])
    }

    /// The installed Brick.app at 1.0.0, with a file only it has.
    fn install(dir: &Path) -> PathBuf {
        let app = dir.join("Applications/Brick.app");
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        fs::create_dir_all(app.join("Contents/Frameworks")).unwrap();
        fs::write(app.join("Contents/Info.plist"), "<plist/>").unwrap();
        fs::write(app.join("Contents/MacOS/brick"), "1.0.0").unwrap();
        fs::write(app.join("Contents/Frameworks/old-only.dylib"), "old").unwrap();
        app
    }

    /// Stage `data` as `version` through the check's own staging.
    pub(crate) fn stage_release(updates: &Path, version: &str, data: &[u8]) {
        let hex = |d: &[u8]| -> String {
            ring::digest::digest(&ring::digest::SHA256, d)
                .as_ref()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
        };
        let release = Release {
            version: version.into(),
            download: Download {
                url: format!("http://127.0.0.1/brick-{version}-arm64-macos.tar.gz"),
                sha256: hex(data),
                bytes: data.len() as u64,
            },
        };
        stage::stage(updates, &release, |_, sink| sink(data)).unwrap();
        assert!(stage::staged(updates).is_some());
    }

    fn exe(app: &Path) -> String {
        fs::read_to_string(app.join("Contents/MacOS/brick")).unwrap()
    }

    /// The names in the bundle's folder.
    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir.join("Applications"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn applied(updates: &Path) -> Option<String> {
        fs::read_to_string(updates.join("applied")).ok()
    }

    #[test]
    fn a_newer_staged_release_takes_the_bundles_place() {
        let dir = folder("swap");
        let (app, updates) = (install(&dir), dir.join("updates"));
        stage_release(&updates, "1.0.1", &release("1.0.1"));
        let got = apply(&updates, "1.0.0", &app, "brick", || false);
        assert_eq!(got, Ok(Applied::Version("1.0.1".into())));
        assert_eq!(exe(&app), "1.0.1");
        assert!(
            !app.join("Contents/Frameworks/old-only.dylib").exists(),
            "the old bundle is gone whole"
        );
        assert!(app.join("Contents/Frameworks/libgtk.dylib").is_file());
        assert_eq!(listing(&dir), ["Brick.app"], "nothing left beside it");
        assert_eq!(applied(&updates).as_deref(), Some("version=1.0.1\n"));
        assert_eq!(stage::staged(&updates), None, "the staged file is gone");
        // Applied once: the next exit finds nothing.
        let again = apply(&updates, "1.0.1", &app, "brick", || false);
        assert_eq!(again, Ok(Applied::Nothing));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_equal_or_older_staged_release_is_left_alone() {
        let dir = folder("older");
        let (app, updates) = (install(&dir), dir.join("updates"));
        stage_release(&updates, "1.0.0", &release("1.0.0-other"));
        assert_eq!(
            apply(&updates, "1.0.0", &app, "brick", || false),
            Ok(Applied::Nothing)
        );
        assert_eq!(
            apply(&updates, "1.2.0", &app, "brick", || false),
            Ok(Applied::Nothing)
        );
        assert_eq!(exe(&app), "1.0.0");
        assert_eq!(applied(&updates), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_staged_is_nothing_to_do() {
        let dir = folder("none");
        let (app, updates) = (install(&dir), dir.join("updates"));
        assert_eq!(
            apply(&updates, "1.0.0", &app, "brick", || false),
            Ok(Applied::Nothing)
        );
        assert_eq!(exe(&app), "1.0.0");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_staged_file_changed_since_its_check_is_refused_and_removed() {
        let dir = folder("tampered");
        let (app, updates) = (install(&dir), dir.join("updates"));
        stage_release(&updates, "1.0.1", &release("1.0.1"));
        let file = stage::staged(&updates).unwrap().file;
        let mut bytes = fs::read(&file).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&file, bytes).unwrap();
        let got = apply(&updates, "1.0.0", &app, "brick", || false);
        assert!(got.as_ref().is_err_and(|e| e.contains("sha256")), "{got:?}");
        assert_eq!(exe(&app), "1.0.0");
        assert_eq!(listing(&dir), ["Brick.app"]);
        assert!(
            !file.exists(),
            "removed, for the next check to download again"
        );
        assert_eq!(applied(&updates), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn while_a_check_holds_the_lock_nothing_is_touched() {
        let dir = folder("busy");
        let (app, updates) = (install(&dir), dir.join("updates"));
        stage_release(&updates, "1.0.1", &release("1.0.1"));
        let held = stage::lock(&updates).unwrap().unwrap();
        assert_eq!(
            apply(&updates, "1.0.0", &app, "brick", || false),
            Ok(Applied::Busy)
        );
        assert_eq!(exe(&app), "1.0.0");
        drop(held);
        assert_eq!(
            apply(&updates, "1.0.0", &app, "brick", || false),
            Ok(Applied::Version("1.0.1".into()))
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_instance_started_meanwhile_defers_the_swap() {
        let dir = folder("deferred");
        let (app, updates) = (install(&dir), dir.join("updates"));
        stage_release(&updates, "1.0.1", &release("1.0.1"));
        let asked = Cell::new(0);
        let got = apply(&updates, "1.0.0", &app, "brick", || {
            asked.set(asked.get() + 1);
            true
        });
        assert_eq!(got, Ok(Applied::Deferred));
        assert_eq!(asked.get(), 1);
        assert_eq!(exe(&app), "1.0.0");
        assert_eq!(listing(&dir), ["Brick.app"], "the unpacked copy is removed");
        assert!(stage::staged(&updates).is_some(), "still staged for later");
        assert_eq!(applied(&updates), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_release_without_the_app_is_refused() {
        let bad: [(&str, Vec<u8>); 4] = [
            (
                "no executable",
                tarball(&[("Brick.app/Contents/Info.plist", b"<plist/>", 0o644)]),
            ),
            (
                "not executable",
                tarball(&[
                    ("Brick.app/Contents/Info.plist", b"<plist/>", 0o644),
                    ("Brick.app/Contents/MacOS/brick", b"x", 0o644),
                ]),
            ),
            (
                "another program",
                tarball(&[
                    ("Brick.app/Contents/Info.plist", b"<plist/>", 0o644),
                    ("Brick.app/Contents/MacOS/slate", b"x", 0o755),
                ]),
            ),
            (
                "two apps",
                tarball(&[
                    ("Brick.app/Contents/Info.plist", b"<plist/>", 0o644),
                    ("Brick.app/Contents/MacOS/brick", b"x", 0o755),
                    ("Other.app/Contents/MacOS/brick", b"x", 0o755),
                ]),
            ),
        ];
        for (why, data) in bad {
            let dir = folder("refused");
            let (app, updates) = (install(&dir), dir.join("updates"));
            stage_release(&updates, "1.0.1", &data);
            let got = apply(&updates, "1.0.0", &app, "brick", || false);
            assert!(got.is_err(), "{why}: {got:?}");
            assert_eq!(exe(&app), "1.0.0", "{why}");
            assert_eq!(listing(&dir), ["Brick.app"], "{why}");
            assert_eq!(applied(&updates), None, "{why}");
            let _ = fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn an_entry_reaching_outside_lands_nowhere() {
        let dir = folder("escape");
        let (app, updates) = (install(&dir), dir.join("updates"));
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        for (path, data, mode) in [
            ("Brick.app/Contents/Info.plist", &b"<plist/>"[..], 0o644),
            ("Brick.app/Contents/MacOS/brick", b"1.0.1", 0o755),
            ("../evil", b"evil", 0o644),
        ] {
            // Written into the header by hand: the builder refuses `..`.
            let mut h = tar::Header::new_gnu();
            h.as_gnu_mut().unwrap().name[..path.len()].copy_from_slice(path.as_bytes());
            h.set_size(data.len() as u64);
            h.set_mode(mode);
            h.set_entry_type(tar::EntryType::Regular);
            h.set_cksum();
            tar.append(&h, data).unwrap();
        }
        let data = tar.into_inner().unwrap().finish().unwrap();
        stage_release(&updates, "1.0.1", &data);
        let got = apply(&updates, "1.0.0", &app, "brick", || false);
        assert_eq!(got, Ok(Applied::Version("1.0.1".into())));
        assert!(!dir.join("Applications/evil").exists());
        assert!(!dir.join("evil").exists());
        assert_eq!(listing(&dir), ["Brick.app"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_a_cut_short_apply_left_is_removed() {
        let dir = folder("sweep");
        let (app, updates) = (install(&dir), dir.join("updates"));
        for left in [".Brick.app.update-1", ".Brick.app.old-2"] {
            fs::create_dir_all(dir.join("Applications").join(left).join("x")).unwrap();
        }
        fs::create_dir_all(dir.join("Applications/.Slate.app.old-3")).unwrap();
        stage_release(&updates, "1.0.1", &release("1.0.1"));
        let got = apply(&updates, "1.0.0", &app, "brick", || false);
        assert_eq!(got, Ok(Applied::Version("1.0.1".into())));
        assert_eq!(
            listing(&dir),
            [".Slate.app.old-3", "Brick.app"],
            "Brick's leftovers go, another app's stay"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_second_rename_puts_the_old_bundle_back() {
        let dir = folder("rollback");
        let app = install(&dir);
        let new = dir.join("Applications/.Brick.app.update-9/Brick.app");
        fs::create_dir_all(new.join("Contents/MacOS")).unwrap();
        fs::write(new.join("Contents/MacOS/brick"), "1.0.1").unwrap();
        let old = dir.join("Applications/.Brick.app.old-9");
        let calls = Cell::new(0);
        let got = swap(&app, &new, &old, |from, to| {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                return Err(io::Error::other("no room"));
            }
            fs::rename(from, to)
        });
        assert!(got.is_err());
        assert_eq!(calls.get(), 3, "aside, in (failed), back");
        assert_eq!(exe(&app), "1.0.0");
        assert!(!old.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
