//! Linux (UPD-4): right after a check staged a release (or found it staged), its tarball's
//! binary, desktop entry, icon, and licences replace the ones `install.sh` installed, while
//! the app runs. Running processes keep the binary they started from (a rename leaves their
//! open file as it was), so the next start is the new version.
//!
//! The files are `install.sh`'s, from the release tarball's one folder:
//!
//! - `<command>` → `~/.local/bin/<command>`, mode 755;
//! - `<app id>.desktop` → `$XDG_DATA_HOME/applications/`, its `Exec=<command>` made the
//!   binary's full path as `install.sh` makes it;
//! - `<app id>.svg` → `$XDG_DATA_HOME/icons/hicolor/scalable/apps/`;
//! - `LICENSE`, `LICENSE-APACHE`, `THIRD-PARTY-RUST.txt` → `$XDG_DATA_HOME/doc/<command>/`,
//!   one the release doesn't carry removed there.
//!
//! Default apps (`mimeapps.list`) are never touched; the desktop database and icon cache
//! are refreshed as `install.sh` refreshes them.
//!
//! Order: the staged file's SHA-256 is checked against its record first, and the tarball is
//! read from that same open file. Every file is then written and synced beside its target
//! (`.<name>.update`), and only once all are there are they renamed over the installed
//! ones: licences, icon, desktop entry, and the binary last. A failure before the renames
//! (a tampered file, an entry escaping the tarball's folder, a missing file, a full disk)
//! removes what was written and leaves the install as it was; the binary going last means
//! the installed binary is the new one only once everything else is, so a run cut short
//! is applied again by the next check.
//!
//! Applied once: an instance still running the old version finds the newer release on
//! every check, so when the tarball's binary is already the installed one (same size and
//! SHA-256) nothing is renamed and nothing recorded again.

use crate::install::LinuxHome;
use crate::stage::Staged;
use std::fs::{self, File};
use std::io::{self, Read, Seek, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use suite_core::AppKind;

/// The licence files `install.sh` copies into the doc folder, when the release has them.
const LICENCES: [&str; 3] = ["LICENSE", "LICENSE-APACHE", "THIRD-PARTY-RUST.txt"];

/// The largest desktop entry read into memory to rewrite its `Exec`.
const DESKTOP_LIMIT: u64 = 1 << 20;

pub(super) fn after_check(app: AppKind, updates: &Path, staged: &Staged) {
    match LinuxHome::current() {
        Some(home) => apply_in(app, updates, staged, &home),
        None => tracing::warn!(
            "update: no home folder to install {app} {} in",
            staged.version
        ),
    }
}

/// [`after_check`] for the install under `home`.
pub(crate) fn apply_in(app: AppKind, updates: &Path, staged: &Staged, home: &LinuxHome) {
    let v = &staged.version;
    match apply(app, home, staged) {
        Ok(Applied::Installed) => {
            refresh_caches(home);
            if let Err(e) = crate::session::record_applied(updates, v) {
                tracing::warn!("update: recording {app} {v} as applied: {e}");
            }
            tracing::info!("update: {app} {v} installed for the next start");
        }
        Ok(Applied::Already) => tracing::debug!("update: {app} {v} is installed already"),
        Err(e) => tracing::warn!("update: {app} {v} not installed: {e}"),
    }
}

/// What [`apply`] did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Applied {
    /// The release's files replaced the installed ones.
    Installed,
    /// The installed binary is the release's already; nothing changed.
    Already,
}

/// Where `install.sh` puts `app`'s files under `home`.
struct Targets {
    bin: PathBuf,
    desktop: PathBuf,
    icon: PathBuf,
    doc: PathBuf,
}

impl Targets {
    fn new(app: AppKind, home: &LinuxHome) -> Targets {
        let id = app.app_id();
        Targets {
            bin: home.home.join(".local/bin").join(app.command()),
            desktop: home.data.join("applications").join(format!("{id}.desktop")),
            icon: home
                .data
                .join("icons/hicolor/scalable/apps")
                .join(format!("{id}.svg")),
            doc: home.data.join("doc").join(app.command()),
        }
    }
}

/// A file written beside its target, renamed over it once all are written; removed if it
/// never was.
struct Pending {
    tmp: PathBuf,
    target: PathBuf,
}

/// The files written so far; dropping it removes those not yet renamed.
#[derive(Default)]
struct Written(Vec<Pending>);

impl Written {
    /// The temp file beside `target`, made (truncated) for writing, its folder made if
    /// missing; replaces an earlier entry for the same target.
    fn create(&mut self, target: &Path, mode: Option<u32>) -> io::Result<File> {
        let dir = target
            .parent()
            .ok_or_else(|| io::Error::other("no folder"))?;
        fs::create_dir_all(dir)?;
        let mut name = std::ffi::OsString::from(".");
        name.push(target.file_name().unwrap_or_default());
        name.push(".update");
        let tmp = dir.join(name);
        let file = File::create(&tmp)?;
        if let Some(mode) = mode {
            file.set_permissions(fs::Permissions::from_mode(mode))?;
        }
        self.0.retain(|p| p.target != target);
        self.0.push(Pending {
            tmp,
            target: target.to_owned(),
        });
        Ok(file)
    }

    fn get(&self, target: &Path) -> Option<&Pending> {
        self.0.iter().find(|p| p.target == target)
    }

    /// Rename the file for `target` over it.
    fn commit(&mut self, target: &Path) -> io::Result<()> {
        let Some(i) = self.0.iter().position(|p| p.target == target) else {
            return Ok(());
        };
        fs::rename(&self.0[i].tmp, target)?;
        self.0.remove(i);
        Ok(())
    }
}

impl Drop for Written {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = fs::remove_file(&p.tmp);
        }
    }
}

/// Install `staged`, a Linux release tarball of `app`, under `home`; see the module docs.
pub(crate) fn apply(app: AppKind, home: &LinuxHome, staged: &Staged) -> io::Result<Applied> {
    let mut file = File::open(&staged.file)?;
    let got = sha256_of(&mut file)?;
    if got != staged.sha256 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{}'s sha256 is {got}, not {}",
                staged.file.display(),
                staged.sha256
            ),
        ));
    }
    file.rewind()?;

    let t = Targets::new(app, home);
    let id = app.app_id();
    let (bin_name, desktop_name, icon_name) = (
        app.command().to_owned(),
        format!("{id}.desktop"),
        format!("{id}.svg"),
    );
    let mut written = Written::default();
    let mut bin_sha = None;
    let mut top: Option<std::ffi::OsString> = None;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(io::BufReader::new(file)));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let parts = inside(&path)?;
        let Some(first) = parts.first() else { continue };
        match &top {
            None => top = Some(first.clone()),
            Some(t) if t != first => return Err(bad(format!("{} isn't in {t:?}", path.display()))),
            Some(_) => {}
        }
        let [_, name] = parts.as_slice() else {
            if parts.len() == 1 && !entry.header().entry_type().is_dir() {
                return Err(bad(format!(
                    "{} isn't in the release's folder",
                    path.display()
                )));
            }
            continue;
        };
        let name = name.to_str().unwrap_or_default();
        let (target, mode) = if name == bin_name {
            (t.bin.clone(), Some(0o755))
        } else if name == desktop_name {
            (t.desktop.clone(), None)
        } else if name == icon_name {
            (t.icon.clone(), None)
        } else if LICENCES.contains(&name) {
            (t.doc.join(name), None)
        } else {
            continue;
        };
        if !entry.header().entry_type().is_file() {
            return Err(bad(format!("{} isn't a file", path.display())));
        }
        let mut out = written.create(&target, mode)?;
        if target == t.desktop {
            if entry.size() > DESKTOP_LIMIT {
                return Err(bad(format!("{} is too large", path.display())));
            }
            let mut text = Vec::new();
            entry.read_to_end(&mut text)?;
            out.write_all(&full_exec(&text, app.command(), &t.bin))?;
        } else if target == t.bin {
            let mut digest = Hashing {
                out: &mut out,
                digest: ring::digest::Context::new(&ring::digest::SHA256),
            };
            io::copy(&mut entry, &mut digest)?;
            bin_sha = Some(hex(digest.digest.finish().as_ref()));
        } else {
            io::copy(&mut entry, &mut out)?;
        }
        out.sync_all()?;
    }
    for (target, what) in [
        (&t.bin, "binary"),
        (&t.desktop, "desktop entry"),
        (&t.icon, "icon"),
    ] {
        if written.get(target).is_none() {
            return Err(bad(format!("the release has no {what}")));
        }
    }

    let new_len = written
        .get(&t.bin)
        .and_then(|p| fs::metadata(&p.tmp).ok())
        .map(|m| m.len());
    if fs::metadata(&t.bin).ok().map(|m| m.len()) == new_len
        && File::open(&t.bin).and_then(|mut f| sha256_of(&mut f)).ok() == bin_sha
    {
        return Ok(Applied::Already);
    }

    for name in LICENCES {
        let target = t.doc.join(name);
        if written.get(&target).is_some() {
            written.commit(&target)?;
        } else {
            match fs::remove_file(&target) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
    }
    written.commit(&t.icon)?;
    written.commit(&t.desktop)?;
    written.commit(&t.bin)?;
    for dir in [
        Some(t.doc.as_path()),
        t.icon.parent(),
        t.desktop.parent(),
        t.bin.parent(),
    ]
    .into_iter()
    .flatten()
    {
        let _ = File::open(dir).and_then(|d| d.sync_all());
    }
    Ok(Applied::Installed)
}

fn bad(why: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why)
}

/// `path`'s components, refusing a path that could leave the folder it's unpacked in:
/// absolute, or with `..`.
fn inside(path: &Path) -> io::Result<Vec<std::ffi::OsString>> {
    let mut parts = Vec::new();
    for c in path.components() {
        match c {
            Component::Normal(p) => parts.push(p.to_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(bad(format!(
                    "{} leaves the release's folder",
                    path.display()
                )))
            }
        }
    }
    Ok(parts)
}

/// The desktop entry with `Exec=<command>` made the binary's full path, as `install.sh`'s
/// sed makes it.
fn full_exec(text: &[u8], command: &str, bin: &Path) -> Vec<u8> {
    let exec = format!("Exec={command}");
    let mut full = b"Exec=".to_vec();
    full.extend_from_slice(bin.as_os_str().as_bytes());
    let mut out = Vec::with_capacity(text.len() + full.len());
    for (i, line) in text.split(|&b| b == b'\n').enumerate() {
        if i > 0 {
            out.push(b'\n');
        }
        match line.strip_prefix(exec.as_bytes()) {
            Some(rest) if rest.is_empty() || rest.starts_with(b" ") => {
                out.extend_from_slice(&full);
                out.extend_from_slice(rest);
            }
            _ => out.extend_from_slice(line),
        }
    }
    out
}

/// The desktop database and icon cache, refreshed as `install.sh` does when the tools are
/// there; neither touches `mimeapps.list`.
fn refresh_caches(home: &LinuxHome) {
    let tools: [(&str, &[&str], PathBuf); 2] = [
        (
            "update-desktop-database",
            &["-q"],
            home.data.join("applications"),
        ),
        (
            "gtk-update-icon-cache",
            &["-q", "-t"],
            home.data.join("icons/hicolor"),
        ),
    ];
    for (tool, args, dir) in tools {
        let _ = std::process::Command::new(tool)
            .args(args)
            .arg(dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// A writer passing bytes on while hashing them.
struct Hashing<'a> {
    out: &'a mut File,
    digest: ring::digest::Context,
}

impl Write for Hashing<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.out.write(buf)?;
        self.digest.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

fn sha256_of(file: &mut File) -> io::Result<String> {
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0; 64 * 1024];
    loop {
        match file.read(&mut buf) {
            Ok(0) => return Ok(hex(digest.finish().as_ref())),
            Ok(n) => digest.update(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests;
