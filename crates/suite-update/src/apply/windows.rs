//! Windows (UPD-6, ADR 0012 item 4). A running program and its DLLs are locked, so the
//! staged Setup.exe runs when the last running instance exits ([`at_exit`]): with no other
//! instance holding an owner lock in the updates folder, and with the updates lock taken
//! (never racing a check, this process's own included), the staged file's SHA-256 is
//! checked against its record again, then Setup.exe starts detached, with install.ps1's
//! silent per-user options and `/NOCLOSEAPPLICATIONS`, and the version is recorded for the
//! next start's notice.
//!
//! Setup closes nothing: told not to close applications, `packaging/windows/setup.iss`
//! waits up to 30 s for the installed program to be free (this process finishing its exit),
//! and if it's still running then (an instance started meanwhile, or one running with
//! updates off, which holds no owner lock) it stops before changing anything (exit code 7),
//! and the update waits for a later exit.
//!
//! The staged file stays until an instance at least as new as it exits, which removes it
//! and its record; a staged file that isn't the one downloaded is removed too, so the next
//! check downloads it again.

use crate::stage::{self, Staged};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

/// Setup.exe's options: install.ps1's silent per-user install, but leaving running
/// programs alone (Setup waits for the program, then gives up rather than close it).
pub(crate) const OPTIONS: [&str; 5] = [
    "/VERYSILENT",
    "/SUPPRESSMSGBOXES",
    "/NORESTART",
    "/CURRENTUSER",
    "/NOCLOSEAPPLICATIONS",
];

/// Setup's log, in the updates folder.
pub(crate) const LOG: &str = "setup.log";

/// What an exit does with the release staged in the updates folder.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Nothing is staged.
    Nothing,
    /// Install this release: newer than the running version, the file as downloaded.
    Install(Staged),
    /// The staged release (this version) isn't newer than the running one: removed.
    Removed(String),
    /// The staged file isn't the one downloaded (its SHA-256 is `sha256`): removed.
    Refused { version: String, sha256: String },
}

/// What to do with the release staged in `updates`, whose lock the caller holds, when the
/// running version is `running`.
pub(crate) fn prepare(updates: &Path, running: &str) -> io::Result<Step> {
    let Some(staged) = stage::staged(updates) else {
        return Ok(Step::Nothing);
    };
    if !crate::manifest::newer(&staged.version, running) {
        unstage(updates, &staged)?;
        return Ok(Step::Removed(staged.version));
    }
    let sha256 = sha256(&staged.file)?;
    if sha256 != staged.sha256 {
        unstage(updates, &staged)?;
        return Ok(Step::Refused {
            version: staged.version,
            sha256,
        });
    }
    Ok(Step::Install(staged))
}

/// Remove the record, then the file: a Setup.exe still running from it stays, and goes
/// with the next release a check stages.
fn unstage(updates: &Path, staged: &Staged) -> io::Result<()> {
    match fs::remove_file(updates.join(stage::RECORD)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let _ = fs::remove_file(&staged.file);
    Ok(())
}

fn sha256(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0; 64 * 1024];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => digest.update(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(digest
        .finish()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Setup.exe's command line: the program, quoted, then [`OPTIONS`] and its log.
pub(crate) fn command_line(setup: &Path, log: &Path) -> OsString {
    let mut line = OsString::from("\"");
    line.push(setup);
    line.push("\"");
    for option in OPTIONS {
        line.push(" ");
        line.push(option);
    }
    line.push(" /LOG=\"");
    line.push(log);
    line.push("\"");
    line
}

/// At the exit of an instance: Setup.exe for a newer staged release when this is the last
/// instance running.
#[cfg(windows)]
pub(super) fn at_exit(session: &crate::session::Session) {
    let Some(updates) = session.updates.as_deref() else {
        return;
    };
    let name = session.app.display_name();
    let pending = stage::staged(updates)
        .filter(|s| crate::manifest::newer(&s.version, session.running))
        .map(|s| s.version);
    if let Some(version) = &pending {
        if crate::session::others_running(updates) {
            tracing::info!("update: {name} {version} waits until every running {name} has quit");
            return;
        }
    }
    let _lock = match stage::lock(updates) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            if let Some(version) = &pending {
                tracing::info!("update: {name} {version} waits: a check is running");
            }
            return;
        }
        Err(e) => {
            tracing::warn!("update: updates folder: {e}");
            return;
        }
    };
    match prepare(updates, session.running) {
        Ok(Step::Nothing) => {}
        Ok(Step::Removed(version)) => {
            tracing::debug!("update: removed {name} {version}'s Setup.exe, which is installed")
        }
        Ok(Step::Refused { version, sha256 }) => tracing::warn!(
            "update: refused {name} {version}'s Setup.exe: its sha256 is {sha256}, not the \
             download's; removed, the next start downloads it again"
        ),
        Ok(Step::Install(staged)) => {
            let log = updates.join(LOG);
            match start(&staged.file, &log, updates) {
                Ok(pid) => {
                    tracing::info!(
                        "update: installing {name} {} (Setup.exe, process {pid}, log {})",
                        staged.version,
                        log.display()
                    );
                    if let Err(e) = crate::session::record_applied(updates, &staged.version) {
                        tracing::warn!("update: updates folder: {e}");
                    }
                }
                Err(e) => tracing::warn!("update: {} didn't start: {e}", staged.file.display()),
            }
        }
        Err(e) => tracing::warn!("update: updates folder: {e}"),
    }
}

/// Start `setup` with [`command_line`] in `dir`, detached: no console, its own process
/// group, out of this process's job where the job allows it, inheriting no handle (a pipe
/// this process writes to must not wait for Setup). Its process id.
#[cfg(windows)]
#[allow(unsafe_code)]
fn start(setup: &Path, log: &Path, dir: &Path) -> io::Result<u32> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED};
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
        PROCESS_INFORMATION, STARTUPINFOW,
    };
    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(Some(0)).collect()
    }
    let program = wide(setup.as_os_str());
    let cwd = wide(dir.as_os_str());
    let flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    let mut denied = None;
    // A job that doesn't allow breaking away refuses the flag: then Setup stays in it.
    for flags in [flags | CREATE_BREAKAWAY_FROM_JOB, flags] {
        let mut line = wide(&command_line(setup, log));
        let info = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        let mut process = PROCESS_INFORMATION::default();
        // SAFETY: every pointer is to a live, NUL-terminated buffer or struct of this
        // frame; the command line is mutable, as CreateProcessW requires.
        let ok = unsafe {
            CreateProcessW(
                program.as_ptr(),
                line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                flags,
                std::ptr::null(),
                cwd.as_ptr(),
                &info,
                &mut process,
            )
        };
        if ok != 0 {
            // SAFETY: the handles CreateProcessW returned, closed once.
            unsafe {
                CloseHandle(process.hThread);
                CloseHandle(process.hProcess);
            }
            return Ok(process.dwProcessId);
        }
        let e = io::Error::last_os_error();
        if e.raw_os_error() != Some(ERROR_ACCESS_DENIED as i32) {
            return Err(e);
        }
        denied = Some(e);
    }
    Err(denied.unwrap_or_else(|| io::Error::other("CreateProcessW failed")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const SETUP: &str = "brick-0.6.0-x86_64-windows-setup.exe";

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("suite-update-win-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `updates` with `data` staged as Brick `version`'s Setup.exe, recorded with `data`'s
    /// SHA-256.
    fn staged(updates: &Path, version: &str, data: &[u8]) -> Staged {
        let file = updates.join(SETUP);
        fs::write(&file, data).unwrap();
        let sha = sha256(&file).unwrap();
        fs::write(
            updates.join(stage::RECORD),
            format!("version={version}\nfile={SETUP}\nsha256={sha}\n"),
        )
        .unwrap();
        stage::staged(updates).unwrap()
    }

    #[test]
    fn a_newer_release_as_downloaded_is_installed() {
        let dir = folder("install");
        let s = staged(&dir, "0.6.0", b"MZ the new Setup.exe");
        assert_eq!(s.sha256.len(), 64);
        assert_eq!(prepare(&dir, "0.5.0").unwrap(), Step::Install(s.clone()));
        assert!(s.file.is_file(), "the file stays for Setup");
        assert_eq!(stage::staged(&dir), Some(s));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_staged_file_changed_since_its_download_is_refused_and_removed() {
        let dir = folder("tampered");
        let s = staged(&dir, "0.6.0", b"MZ the new Setup.exe");
        // The same size, one byte changed.
        fs::write(&s.file, b"MZ the new Setup.exf").unwrap();
        let tampered = sha256(&s.file).unwrap();
        assert_eq!(
            prepare(&dir, "0.5.0").unwrap(),
            Step::Refused {
                version: "0.6.0".into(),
                sha256: tampered
            }
        );
        assert!(!s.file.exists(), "the changed file goes");
        assert!(!dir.join(stage::RECORD).exists(), "and its record");
        assert_eq!(prepare(&dir, "0.5.0").unwrap(), Step::Nothing);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_release_no_newer_than_the_running_one_is_removed() {
        for running in ["0.6.0", "0.7.0"] {
            let dir = folder(&format!("installed-{running}"));
            let s = staged(&dir, "0.6.0", b"MZ the new Setup.exe");
            assert_eq!(
                prepare(&dir, running).unwrap(),
                Step::Removed("0.6.0".into())
            );
            assert!(!s.file.exists() && !dir.join(stage::RECORD).exists());
            let _ = fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn nothing_staged_is_nothing_to_do() {
        let dir = folder("nothing");
        assert_eq!(prepare(&dir, "0.5.0").unwrap(), Step::Nothing);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn setup_runs_silently_for_this_user_and_closes_nothing() {
        let setup =
            Path::new(r"C:\Users\Ada Lovelace\AppData\Local\Cronkite\Brick\updates").join(SETUP);
        let log =
            Path::new(r"C:\Users\Ada Lovelace\AppData\Local\Cronkite\Brick\updates").join(LOG);
        let line = command_line(&setup, &log);
        let line = line.to_string_lossy();
        assert_eq!(
            line,
            format!(
                "\"{}\" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CURRENTUSER \
                 /NOCLOSEAPPLICATIONS /LOG=\"{}\"",
                setup.display(),
                log.display()
            )
        );
        assert!(!line.contains("/CLOSEAPPLICATIONS"));
        assert!(!line.contains("/FORCECLOSEAPPLICATIONS"));
    }

    #[cfg(windows)]
    #[test]
    fn a_program_starts_detached_and_a_missing_one_is_an_error() {
        let dir = folder("start");
        let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap();
        // whoami refuses Setup's options and exits; it started, with a process id.
        let pid = start(&system.join(r"System32\whoami.exe"), &dir.join(LOG), &dir).unwrap();
        assert_ne!(pid, 0);
        let missing = start(&dir.join(SETUP), &dir.join(LOG), &dir).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
        let _ = fs::remove_dir_all(&dir);
    }
}
