//! The updates folder (`<cache>/updates/`, ADR 0012): the release file a check downloaded
//! and verified, and `staged`, the record naming it, which the apply step (UPD-4 to UPD-6)
//! reads. A check holds `.lock` while it runs, so two running instances never download at
//! once. A download goes to `.download` and becomes the release file only once its size
//! and SHA-256 are the manifest's; then `staged` is written, replacing the old record, and
//! the older release files go. The record is written last, so it never names a file that
//! isn't complete, and a check cut short (the app quitting mid-download) leaves only
//! `.download`, which the next check removes.

use crate::manifest::Release;
use crate::Problem;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const LOCK: &str = ".lock";
const PARTIAL: &str = ".download";
const RECORD: &str = "staged";

/// A staged release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Staged {
    pub version: String,
    /// The release file: the Linux or macOS tarball, the Windows Setup.exe.
    pub file: PathBuf,
    pub sha256: String,
}

/// The release staged in `updates`, if its record names a file that's there.
pub fn staged(updates: &Path) -> Option<Staged> {
    let text = fs::read_to_string(updates.join(RECORD)).ok()?;
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .map(str::to_owned)
    };
    let name = field("file").filter(|n| file_name_ok(n))?;
    let file = updates.join(name);
    file.is_file().then(|| Staged {
        version: field("version").unwrap_or_default(),
        file,
        sha256: field("sha256").unwrap_or_default(),
    })
}

/// The lock on `updates` (made if missing), or `None` while another process holds it.
pub fn lock(updates: &Path) -> io::Result<Option<File>> {
    fs::create_dir_all(updates)?;
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(updates.join(LOCK))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(fs::TryLockError::WouldBlock) => Ok(None),
        Err(fs::TryLockError::Error(e)) => Err(e),
    }
}

/// Stage `release` in `updates` (whose lock the caller holds), downloading it with `fetch`
/// (URL, a sink for the body as it arrives) unless it's staged already.
pub fn stage(
    updates: &Path,
    release: &Release,
    fetch: impl FnOnce(&str, &mut dyn FnMut(&[u8]) -> Result<(), Problem>) -> Result<(), Problem>,
) -> Result<crate::Outcome, Problem> {
    let d = &release.download;
    let name = d
        .url
        .rsplit('/')
        .next()
        .and_then(|n| n.split(['?', '#']).next())
        .filter(|n| file_name_ok(n))
        .ok_or_else(|| Problem::Manifest(format!("{} names no file to keep", d.url)))?;
    if let Some(s) = staged(updates) {
        let same = s.version == release.version
            && s.sha256 == d.sha256
            && fs::metadata(&s.file).is_ok_and(|m| m.len() == d.bytes);
        if same {
            return Ok(crate::Outcome::AlreadyStaged(s.version));
        }
    }
    let partial = updates.join(PARTIAL);
    let got = download(&partial, d.bytes, &d.sha256, |sink| fetch(&d.url, sink));
    if let Err(e) = got {
        let _ = fs::remove_file(&partial);
        return Err(e);
    }
    let file = updates.join(name);
    fs::rename(&partial, &file)?;
    let record = format!(
        "version={}\nfile={name}\nsha256={}\n",
        release.version, d.sha256
    );
    write_replacing(&updates.join(RECORD), record.as_bytes())?;
    // The older release files; the owner locks (UPD-5) and the applied record (its
    // version's notice, still to show) stay.
    for entry in fs::read_dir(updates)?.flatten() {
        let n = entry.file_name();
        let n = n.to_string_lossy();
        let keep = n == LOCK
            || n == RECORD
            || n == name
            || n == crate::session::APPLIED
            || n.starts_with("owner-");
        if !keep {
            let path = entry.path();
            let _ = if path.is_dir() {
                fs::remove_dir_all(path)
            } else {
                fs::remove_file(path)
            };
        }
    }
    Ok(crate::Outcome::Staged(release.version.clone()))
}

/// Download into `path` through `fetch`, refusing as soon as the body runs past `bytes`,
/// then a body of another size or SHA-256; synced to disk once it's right.
fn download(
    path: &Path,
    bytes: u64,
    sha256: &str,
    fetch: impl FnOnce(&mut dyn FnMut(&[u8]) -> Result<(), Problem>) -> Result<(), Problem>,
) -> Result<(), Problem> {
    let mut out = io::BufWriter::new(File::create(path)?);
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    let mut got = 0u64;
    fetch(&mut |chunk| {
        got += chunk.len() as u64;
        if got > bytes {
            return Err(Problem::Download(format!(
                "the download is larger than its {bytes} bytes"
            )));
        }
        digest.update(chunk);
        out.write_all(chunk).map_err(Problem::Io)
    })?;
    if got != bytes {
        return Err(Problem::Download(format!(
            "the download is {got} bytes, not {bytes}"
        )));
    }
    let hex: String = digest
        .finish()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if hex != sha256 {
        return Err(Problem::Download(format!(
            "the download's sha256 is {hex}, not {sha256}"
        )));
    }
    out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    Ok(())
}

/// Write `data` to `path` through a temp file beside it and a rename, so a reader sees the
/// old contents or the new, never part.
pub fn write_replacing(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut f = File::create(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, path)
}

/// A release file's name as the updates folder keeps it: letters, digits, `.`, `_`, `-`,
/// not starting with a dot, not one of the folder's own files.
fn file_name_ok(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name != RECORD
        && !name.starts_with("owner-")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}
