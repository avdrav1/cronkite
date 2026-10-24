//! The suite's apps updating themselves (ADR 0012): at startup, off the UI thread, fetch
//! the app's `latest.json` and `latest.json.sig`, believe the manifest only when the
//! signature verifies with a trusted key ([`keys`]), it names this app, and it offers a
//! newer version for this platform ([`manifest`]), then download that file, check its size
//! and SHA-256, and stage it in the app's cache folder's `updates/` ([`stage`]), where the
//! platform's apply step (UPD-4 to UPD-6) takes it for the next start.
//!
//! Nothing is fetched when the build has the updater compiled out
//! (`CRONKITE_NO_SELF_UPDATE` set at build time, for packages others update), when
//! `CRONKITE_NO_UPDATE` is set (anything but empty or `0`), when the user turned
//! "Update Automatically" off ([`automatic`]), or when the program isn't an installer's
//! install ([`install`]). Every failure is one line in the log and nothing half-done: the
//! next start tries again.
//!
//! Toolkit-free: it depends on `suite-core` and `suite-platform` only, of the suite's
//! crates. HTTPS is `ureq` on `rustls` with `ring`, certificates checked by the system's
//! verifier; `ring` also checks the signature and the SHA-256.

mod apply;
pub mod install;
pub mod keys;
pub mod manifest;
pub mod session;
pub mod stage;

use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use suite_core::AppKind;

pub use session::{launch, notice, Session};
pub use stage::{staged, Staged};

/// Why a check stopped without staging anything.
#[derive(Debug)]
pub enum Problem {
    /// The network, the server, or an HTTP error.
    Network(String),
    /// `latest.json.sig` is missing, malformed, or doesn't verify with a trusted key.
    Signature,
    /// The signed manifest is another app's (or names none).
    OtherApp(String),
    /// The release has nothing for this platform.
    NoPlatform(String),
    /// The signed manifest isn't a release this updater understands.
    Manifest(String),
    /// The download isn't the file the manifest names: size or SHA-256.
    Download(String),
    /// Reading or writing the updates folder.
    Io(std::io::Error),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Network(e) => write!(f, "{e}"),
            Problem::Signature => f.write_str("latest.json's signature doesn't verify"),
            Problem::OtherApp(app) => write!(f, "latest.json is {app}'s"),
            Problem::NoPlatform(p) => write!(f, "the release has nothing for {p}"),
            Problem::Manifest(e) | Problem::Download(e) => f.write_str(e),
            Problem::Io(e) => write!(f, "updates folder: {e}"),
        }
    }
}

impl std::error::Error for Problem {}

impl From<std::io::Error> for Problem {
    fn from(e: std::io::Error) -> Self {
        Problem::Io(e)
    }
}

/// Why nothing was fetched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    /// Built with `CRONKITE_NO_SELF_UPDATE`.
    CompiledOut,
    /// `CRONKITE_NO_UPDATE` is set.
    Disabled,
    /// "Update Automatically" is off.
    SettingOff,
    /// Not an installer's install (a build in `target/`, a package).
    NotInstalled,
}

/// What a check did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing fetched.
    Skipped(Skip),
    /// Another running instance is checking; this one leaves it to it.
    Busy,
    /// The release isn't newer than the running version.
    UpToDate,
    /// This version was downloaded and staged by an earlier check.
    AlreadyStaged(String),
    /// This version is now downloaded, verified, and staged.
    Staged(String),
}

/// One check's inputs: what [`start`] takes from the system, so tests give their own.
pub struct Check {
    pub app: AppKind,
    /// The running version, `X.Y.Z`.
    pub running: String,
    /// The release folder holding `latest.json` and `latest.json.sig`.
    pub base: String,
    pub keys: Vec<keys::PublicKey>,
    /// `latest.json`'s platform key ([`manifest::PLATFORM`]).
    pub platform: String,
    /// The updates folder ([`updates_dir`]).
    pub updates: PathBuf,
    /// Whether a release staged now or earlier is applied after the check (Linux, UPD-4):
    /// the running app's checks do; tests of the check alone don't.
    pub apply: bool,
}

/// What decides whether a check may fetch anything.
pub struct Gate<'a> {
    pub compiled_out: bool,
    /// `CRONKITE_NO_UPDATE`'s value.
    pub no_update: Option<&'a OsStr>,
    /// The settings file [`automatic`] reads.
    pub settings: &'a Path,
    pub installed: bool,
}

impl Gate<'_> {
    /// Why this gate fetches nothing, if it doesn't.
    pub fn skip(&self) -> Option<Skip> {
        if self.compiled_out {
            Some(Skip::CompiledOut)
        } else if self.no_update.is_some_and(|v| !v.is_empty() && v != "0") {
            Some(Skip::Disabled)
        } else if !automatic_in(self.settings) {
            Some(Skip::SettingOff)
        } else if !self.installed {
            Some(Skip::NotInstalled)
        } else {
            None
        }
    }
}

/// Check once, unless `gate` says not to fetch anything.
pub fn check(gate: &Gate, check: &Check) -> Result<Outcome, Problem> {
    match gate.skip() {
        Some(skip) => Ok(Outcome::Skipped(skip)),
        None => run(check),
    }
}

/// Fetch, verify, and stage; see the module docs.
fn run(c: &Check) -> Result<Outcome, Problem> {
    let Some(_lock) = stage::lock(&c.updates)? else {
        return Ok(Outcome::Busy);
    };
    let agent = agent(c);
    let base = c.base.trim_end_matches('/');
    let manifest = get(&agent, &format!("{base}/latest.json"), 64 * 1024)?;
    let signature = get(&agent, &format!("{base}/latest.json.sig"), 1024)?;
    let release = manifest::verify(&manifest, &signature, &c.keys, c.app.command(), &c.platform)?;
    if !manifest::newer(&release.version, &c.running) {
        return Ok(Outcome::UpToDate);
    }
    let outcome = stage::stage(&c.updates, &release, |url, out| download(&agent, url, out))?;
    if let Some(s) = stage::staged(&c.updates).filter(|s| c.apply && s.version == release.version) {
        apply::after_check(c.app, &c.updates, &s);
    }
    Ok(outcome)
}

/// The HTTP client: rustls on ring, the system's certificate verifier, timeouts that end a
/// stalled connection, this app and version as the user agent, and a new connection per
/// request: a check makes three, and a pooled connection the server has closed meanwhile
/// fails the next one ("Peer disconnected", seen with Python's `http.server`).
fn agent(c: &Check) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::Rustls)
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .max_idle_connections(0)
        .max_idle_connections_per_host(0)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(600)))
        .user_agent(format!("{}/{}", c.app.command(), c.running))
        .build()
        .new_agent()
}

fn get(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, Problem> {
    agent
        .get(url)
        .call()
        .and_then(|mut r| r.body_mut().with_config().limit(limit).read_to_vec())
        .map_err(|e| Problem::Network(format!("{url}: {e}")))
}

/// `url`'s body, written into `out` as it arrives.
fn download(
    agent: &ureq::Agent,
    url: &str,
    out: &mut dyn FnMut(&[u8]) -> Result<(), Problem>,
) -> Result<(), Problem> {
    let response = agent
        .get(url)
        .call()
        .map_err(|e| Problem::Network(format!("{url}: {e}")))?;
    let mut reader = response.into_body().into_reader();
    let mut buf = vec![0; 64 * 1024];
    loop {
        match std::io::Read::read(&mut reader, &mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => out(&buf[..n])?,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(Problem::Network(format!("{url}: {e}"))),
        }
    }
}

/// Where `app` stages updates: its cache folder's `updates/`.
pub fn updates_dir(app: AppKind) -> PathBuf {
    suite_platform::dirs::cache_dir(app).join("updates")
}

/// The settings file "Update Automatically" is kept in: `update` in the app's settings
/// folder.
pub fn settings_path(app: AppKind) -> PathBuf {
    suite_platform::dirs::config_dir(app).join("update")
}

/// Whether "Update Automatically" is on: unless the user turned it off, it is.
pub fn automatic(app: AppKind) -> bool {
    automatic_in(&settings_path(app))
}

fn automatic_in(settings: &Path) -> bool {
    std::fs::read_to_string(settings)
        .map_or(true, |s| !s.lines().any(|l| l.trim() == "automatic=false"))
}

/// Turn "Update Automatically" on or off, kept across runs.
pub fn set_automatic(app: AppKind, on: bool) -> std::io::Result<()> {
    set_automatic_in(&settings_path(app), on)
}

fn set_automatic_in(settings: &Path, on: bool) -> std::io::Result<()> {
    if let Some(dir) = settings.parent() {
        std::fs::create_dir_all(dir)?;
    }
    stage::write_replacing(settings, format!("automatic={on}\n").as_bytes())
}

/// The release folder: `CRONKITE_UPDATE_URL` when set (the update checks' local server;
/// what it serves must still verify), else the app's own.
fn base_url(app: AppKind) -> String {
    std::env::var("CRONKITE_UPDATE_URL")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| app.release_url().to_owned())
}

/// Check for an update of `app`, running `running` (its `CARGO_PKG_VERSION`), on a thread
/// of its own; the first call in a process does, later ones nothing. Call it once the first
/// window shows. Its result goes to the log.
pub fn start(app: AppKind, running: &'static str) {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("update".into())
        .spawn(move || {
            let no_update = std::env::var_os("CRONKITE_NO_UPDATE");
            let settings = settings_path(app);
            let gate = Gate {
                compiled_out: option_env!("CRONKITE_NO_SELF_UPDATE").is_some(),
                no_update: no_update.as_deref(),
                settings: &settings,
                installed: install::current(app),
            };
            let c = Check {
                app,
                running: running.to_owned(),
                base: base_url(app),
                keys: keys::trusted(),
                platform: manifest::PLATFORM.to_owned(),
                updates: updates_dir(app),
                apply: true,
            };
            match check(&gate, &c) {
                Ok(Outcome::Staged(v)) => {
                    tracing::info!("update: {app} {v} downloaded for the next start")
                }
                Ok(outcome) => tracing::debug!("update: {outcome:?}"),
                Err(problem) => tracing::warn!("update: {problem}"),
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("update: no thread ({e})");
    }
}

#[cfg(test)]
mod tests;
