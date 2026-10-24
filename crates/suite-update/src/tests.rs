//! Checks against a real HTTP server on 127.0.0.1, serving manifests signed with the test
//! key (`test-key/`), as UPD-2's criterion asks.

use super::*;
use base64::Engine;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

const TEST_PRIVATE: &str = include_str!("../test-key/test.key");

/// What the server answers for a path.
#[derive(Clone)]
enum Reply {
    Ok(Vec<u8>),
    /// Headers promising `promised` bytes, then `body`, then the connection drops.
    Cut {
        body: Vec<u8>,
        promised: usize,
    },
}

struct Server {
    base: String,
    routes: Arc<Mutex<HashMap<String, Reply>>>,
    hits: Arc<Mutex<HashMap<String, usize>>>,
    requests: Arc<AtomicUsize>,
    /// Answer as HTTP/1.1 keep-alive (no `Connection: close`), then hang up on the next
    /// request on that connection instead of answering it: a server that closed an idle
    /// connection the client pooled.
    stale: Arc<AtomicBool>,
}

impl Server {
    fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes: Arc<Mutex<HashMap<String, Reply>>> = Arc::default();
        let hits: Arc<Mutex<HashMap<String, usize>>> = Arc::default();
        let requests = Arc::new(AtomicUsize::new(0));
        let stale = Arc::new(AtomicBool::new(false));
        let (r, h, n, s) = (
            routes.clone(),
            hits.clone(),
            requests.clone(),
            stale.clone(),
        );
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut head = Vec::new();
                let mut byte = [0u8];
                while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                    head.push(byte[0]);
                }
                let line = String::from_utf8_lossy(&head);
                let path = line.split(' ').nth(1).unwrap_or("").to_owned();
                n.fetch_add(1, Ordering::SeqCst);
                *h.lock().unwrap().entry(path.clone()).or_default() += 1;
                let reply = r.lock().unwrap().get(&path).cloned();
                let keep_alive = s.load(Ordering::SeqCst);
                let connection = if keep_alive {
                    ""
                } else {
                    "Connection: close\r\n"
                };
                let _ = match reply {
                    Some(Reply::Ok(body)) => stream
                        .write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{connection}\r\n",
                                body.len()
                            )
                            .as_bytes(),
                        )
                        .and_then(|_| stream.write_all(&body)),
                    Some(Reply::Cut { body, promised }) => stream
                        .write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {promised}\r\nConnection: close\r\n\r\n"
                            )
                            .as_bytes(),
                        )
                        .and_then(|_| stream.write_all(&body)),
                    None => stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    ),
                };
                if keep_alive {
                    // Wait for a reused connection's next request, then drop it unanswered.
                    let _ = stream.read(&mut byte);
                }
            }
        });
        Server {
            base,
            routes,
            hits,
            requests,
            stale,
        }
    }

    fn put(&self, path: &str, reply: Reply) {
        self.routes.lock().unwrap().insert(path.to_owned(), reply);
    }

    fn hits(&self, path: &str) -> usize {
        self.hits.lock().unwrap().get(path).copied().unwrap_or(0)
    }
}

fn sign(data: &[u8], seed: &[u8; 32]) -> Vec<u8> {
    let pair = ring::signature::Ed25519KeyPair::from_seed_unchecked(seed).unwrap();
    let sig = base64::engine::general_purpose::STANDARD.encode(pair.sign(data).as_ref());
    format!("{sig}\n").into_bytes()
}

fn sha256(data: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, data)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

const TARBALL: &[u8] = b"pretend this is brick 0.6.0's tarball";

/// A test: a server, an updates folder, and Brick 0.5.0 checking against them.
struct World {
    server: Server,
    dir: tempfile_dir::Dir,
    seed: [u8; 32],
}

impl World {
    fn new() -> World {
        World {
            server: Server::start(),
            dir: tempfile_dir::Dir::new(),
            seed: keys::key_of(TEST_PRIVATE),
        }
    }

    fn check(&self) -> Check {
        Check {
            app: AppKind::Brick,
            running: "0.5.0".into(),
            base: format!("{}/brick/", self.server.base),
            keys: vec![keys::key_of(keys::TEST_PEM)],
            platform: manifest::PLATFORM.into(),
            updates: self.dir.path().join("updates"),
            apply: false,
        }
    }

    fn manifest(&self, app: &str, version: &str, platform: &str, file: &[u8]) -> Vec<u8> {
        format!(
            r#"{{"app":"{app}","platforms":{{"{platform}":{{"url":"{base}/brick/releases/v{version}/brick-{version}.tar.gz","sha256":"{sha}","bytes":{len}}}}},"version":"{version}","date":"2026-10-21","url":"x","sha256":"y","bytes":1}}"#,
            base = self.server.base,
            sha = sha256(file),
            len = file.len()
        )
        .into_bytes()
    }

    /// Serve `manifest` signed with `seed`, and `file` as `version`'s download.
    fn serve(&self, manifest: &[u8], seed: &[u8; 32], version: &str, file: Reply) {
        self.server
            .put("/brick/latest.json", Reply::Ok(manifest.to_vec()));
        self.server
            .put("/brick/latest.json.sig", Reply::Ok(sign(manifest, seed)));
        self.server.put(
            &format!("/brick/releases/v{version}/brick-{version}.tar.gz"),
            file,
        );
    }

    fn serve_release(&self, version: &str, file: &[u8]) {
        let m = self.manifest("brick", version, manifest::PLATFORM, file);
        self.serve(&m, &self.seed, version, Reply::Ok(file.to_vec()));
    }

    fn file_hits(&self, version: &str) -> usize {
        self.server.hits(&format!(
            "/brick/releases/v{version}/brick-{version}.tar.gz"
        ))
    }

    /// Nothing staged and no release file or partial download left in the updates folder.
    fn assert_nothing_staged(&self) {
        let updates = self.dir.path().join("updates");
        assert_eq!(staged(&updates), None);
        let left: Vec<String> = std::fs::read_dir(&updates)
            .map(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n != ".lock")
                    .collect()
            })
            .unwrap_or_default();
        assert!(left.is_empty(), "left in updates/: {left:?}");
    }
}

/// A temporary folder removed when dropped (no dev-dependency for it).
mod tempfile_dir {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub struct Dir(PathBuf);

    impl Dir {
        pub fn new() -> Dir {
            static N: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "suite-update-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Dir(path)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[test]
fn a_newer_release_is_downloaded_checked_and_staged_once() {
    let w = World::new();
    w.serve_release("0.6.0", TARBALL);
    assert_eq!(run(&w.check()).unwrap(), Outcome::Staged("0.6.0".into()));
    let s = staged(&w.check().updates).expect("staged");
    assert_eq!(s.version, "0.6.0");
    assert_eq!(s.sha256, sha256(TARBALL));
    assert_eq!(s.file.file_name().unwrap(), "brick-0.6.0.tar.gz");
    assert_eq!(std::fs::read(&s.file).unwrap(), TARBALL);
    // The next start finds it staged and downloads nothing.
    assert_eq!(
        run(&w.check()).unwrap(),
        Outcome::AlreadyStaged("0.6.0".into())
    );
    assert_eq!(w.file_hits("0.6.0"), 1);
}

#[test]
fn a_later_release_replaces_the_staged_one() {
    let w = World::new();
    w.serve_release("0.6.0", TARBALL);
    run(&w.check()).unwrap();
    let newer = b"brick 0.7.0".to_vec();
    w.serve_release("0.7.0", &newer);
    assert_eq!(run(&w.check()).unwrap(), Outcome::Staged("0.7.0".into()));
    let updates = w.check().updates;
    assert_eq!(
        std::fs::read(staged(&updates).unwrap().file).unwrap(),
        newer
    );
    assert!(!updates.join("brick-0.6.0.tar.gz").exists());
}

#[test]
fn a_signature_by_another_key_is_refused() {
    let w = World::new();
    let m = w.manifest("brick", "0.6.0", manifest::PLATFORM, TARBALL);
    w.serve(&m, &[7; 32], "0.6.0", Reply::Ok(TARBALL.to_vec()));
    assert!(matches!(run(&w.check()), Err(Problem::Signature)));
    w.assert_nothing_staged();
    assert_eq!(w.file_hits("0.6.0"), 0);
}

#[test]
fn a_changed_byte_in_the_manifest_is_refused() {
    let w = World::new();
    let m = w.manifest("brick", "0.6.0", manifest::PLATFORM, TARBALL);
    w.serve(&m, &w.seed, "0.6.0", Reply::Ok(TARBALL.to_vec()));
    let mut tampered = m.clone();
    let at = tampered.windows(5).position(|x| x == b"0.6.0").unwrap();
    tampered[at + 2] = b'9';
    w.server.put("/brick/latest.json", Reply::Ok(tampered));
    assert!(matches!(run(&w.check()), Err(Problem::Signature)));
    w.assert_nothing_staged();
}

#[test]
fn a_missing_or_garbled_signature_is_refused() {
    let w = World::new();
    w.serve_release("0.6.0", TARBALL);
    w.server.put(
        "/brick/latest.json.sig",
        Reply::Ok(b"not base64!\n".to_vec()),
    );
    assert!(matches!(run(&w.check()), Err(Problem::Signature)));
    w.server
        .routes
        .lock()
        .unwrap()
        .remove("/brick/latest.json.sig");
    let e = run(&w.check()).unwrap_err();
    assert!(
        matches!(&e, Problem::Network(m) if m.contains("latest.json.sig") && m.contains("404")),
        "{e}"
    );
    w.assert_nothing_staged();
}

#[test]
fn another_apps_signed_manifest_is_refused() {
    let w = World::new();
    let m = w.manifest("slate", "0.6.0", manifest::PLATFORM, TARBALL);
    w.serve(&m, &w.seed, "0.6.0", Reply::Ok(TARBALL.to_vec()));
    assert!(matches!(run(&w.check()), Err(Problem::OtherApp(a)) if a == "slate"));
    // A manifest from before UPD-1, which names no app, is refused too.
    let old = String::from_utf8(m)
        .unwrap()
        .replace(r#""app":"slate","#, "");
    w.serve(
        old.as_bytes(),
        &w.seed,
        "0.6.0",
        Reply::Ok(TARBALL.to_vec()),
    );
    assert!(matches!(run(&w.check()), Err(Problem::OtherApp(a)) if a == "no app"));
    w.assert_nothing_staged();
    assert_eq!(w.file_hits("0.6.0"), 0);
}

#[test]
fn an_equal_or_older_version_stages_nothing() {
    let w = World::new();
    for version in ["0.5.0", "0.4.9", "0.1.0"] {
        w.serve_release(version, TARBALL);
        assert_eq!(run(&w.check()).unwrap(), Outcome::UpToDate, "{version}");
        assert_eq!(w.file_hits(version), 0);
    }
    w.assert_nothing_staged();
}

#[test]
fn a_release_without_this_platform_is_refused() {
    let w = World::new();
    let m = w.manifest("brick", "0.6.0", "plan9-mips", TARBALL);
    w.serve(&m, &w.seed, "0.6.0", Reply::Ok(TARBALL.to_vec()));
    assert!(matches!(run(&w.check()), Err(Problem::NoPlatform(p)) if p == manifest::PLATFORM));
    w.assert_nothing_staged();
}

#[test]
fn a_download_with_another_checksum_or_size_is_refused() {
    let w = World::new();
    let m = w.manifest("brick", "0.6.0", manifest::PLATFORM, TARBALL);
    // Same size, one byte changed.
    let mut changed = TARBALL.to_vec();
    changed[0] ^= 1;
    w.serve(&m, &w.seed, "0.6.0", Reply::Ok(changed));
    let e = run(&w.check()).unwrap_err();
    assert!(
        matches!(&e, Problem::Download(m) if m.contains("sha256")),
        "{e}"
    );
    w.assert_nothing_staged();
    // Longer and shorter.
    let mut longer = TARBALL.to_vec();
    longer.extend_from_slice(b" and then some");
    w.serve(&m, &w.seed, "0.6.0", Reply::Ok(longer));
    let e = run(&w.check()).unwrap_err();
    assert!(
        matches!(&e, Problem::Download(m) if m.contains("larger")),
        "{e}"
    );
    w.assert_nothing_staged();
    w.serve(&m, &w.seed, "0.6.0", Reply::Ok(TARBALL[..10].to_vec()));
    let e = run(&w.check()).unwrap_err();
    assert!(
        matches!(&e, Problem::Download(m) if m.contains("10 bytes")),
        "{e}"
    );
    w.assert_nothing_staged();
}

#[test]
fn a_dropped_connection_stages_nothing_and_the_next_check_succeeds() {
    let w = World::new();
    let m = w.manifest("brick", "0.6.0", manifest::PLATFORM, TARBALL);
    let cut = Reply::Cut {
        body: TARBALL[..12].to_vec(),
        promised: TARBALL.len(),
    };
    w.serve(&m, &w.seed, "0.6.0", cut);
    let e = run(&w.check()).unwrap_err();
    assert!(matches!(&e, Problem::Network(_)), "{e}");
    w.assert_nothing_staged();
    w.serve_release("0.6.0", TARBALL);
    assert_eq!(run(&w.check()).unwrap(), Outcome::Staged("0.6.0".into()));
}

#[test]
fn no_server_is_a_network_problem() {
    let w = World::new();
    let mut c = w.check();
    let free = TcpListener::bind("127.0.0.1:0").unwrap();
    c.base = format!("http://{}", free.local_addr().unwrap());
    drop(free);
    assert!(matches!(run(&c), Err(Problem::Network(_))));
    w.assert_nothing_staged();
}

#[test]
fn another_instance_checking_leaves_it_to_that_one() {
    let w = World::new();
    w.serve_release("0.6.0", TARBALL);
    let held = stage::lock(&w.check().updates).unwrap().expect("free");
    assert_eq!(run(&w.check()).unwrap(), Outcome::Busy);
    assert_eq!(w.server.requests.load(Ordering::SeqCst), 0);
    drop(held);
    assert_eq!(run(&w.check()).unwrap(), Outcome::Staged("0.6.0".into()));
}

#[test]
fn nothing_is_fetched_when_disabled_turned_off_or_not_installed() {
    let w = World::new();
    w.serve_release("0.6.0", TARBALL);
    let settings = w.dir.path().join("config/update");
    let gate = |compiled_out, no_update: Option<&'static str>, installed| Gate {
        compiled_out,
        no_update: no_update.map(OsStr::new),
        settings: &settings,
        installed,
    };
    let cases = [
        (gate(true, None, true), Skip::CompiledOut),
        (gate(false, Some("1"), true), Skip::Disabled),
        (gate(false, Some("yes"), true), Skip::Disabled),
        (gate(false, None, false), Skip::NotInstalled),
    ];
    for (g, skip) in cases {
        assert_eq!(check(&g, &w.check()).unwrap(), Outcome::Skipped(skip));
    }
    set_automatic_in(&settings, false).unwrap();
    assert_eq!(
        check(&gate(false, None, true), &w.check()).unwrap(),
        Outcome::Skipped(Skip::SettingOff)
    );
    assert_eq!(w.server.requests.load(Ordering::SeqCst), 0);
    w.assert_nothing_staged();
    // Turned back on, and CRONKITE_NO_UPDATE empty or 0, it checks.
    set_automatic_in(&settings, true).unwrap();
    for no_update in [None, Some(""), Some("0")] {
        let _ = check(&gate(false, no_update, true), &w.check()).unwrap();
    }
    assert_eq!(w.file_hits("0.6.0"), 1);
}

#[test]
fn update_automatically_is_on_until_turned_off() {
    let dir = tempfile_dir::Dir::new();
    let settings = dir.path().join("settings/update");
    assert!(automatic_in(&settings));
    set_automatic_in(&settings, false).unwrap();
    assert!(!automatic_in(&settings));
    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        "automatic=false\n"
    );
    set_automatic_in(&settings, true).unwrap();
    assert!(automatic_in(&settings));
}

#[test]
fn only_install_shs_layout_counts_as_installed_on_linux() {
    let dir = tempfile_dir::Dir::new();
    let home = install::LinuxHome {
        home: dir.path().join("home"),
        data: dir.path().join("data"),
    };
    let bin = home.home.join(".local/bin/brick");
    std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
    std::fs::write(&bin, b"brick").unwrap();
    let exe = std::fs::canonicalize(&bin).unwrap();
    assert!(
        !install::linux(AppKind::Brick, &exe, &home),
        "no desktop entry"
    );
    let entry = home.data.join("applications/dev.cronkite.Brick.desktop");
    std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
    std::fs::write(&entry, b"[Desktop Entry]").unwrap();
    assert!(install::linux(AppKind::Brick, &exe, &home));
    // A build elsewhere, another app's name, or this test's own binary.
    let elsewhere = dir.path().join("target/release/brick");
    std::fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
    std::fs::write(&elsewhere, b"brick").unwrap();
    let elsewhere = std::fs::canonicalize(elsewhere).unwrap();
    assert!(!install::linux(AppKind::Brick, &elsewhere, &home));
    assert!(!install::linux(AppKind::Slate, &exe, &home));
    assert!(!install::current(AppKind::Brick));
}

#[test]
fn the_release_keys_are_two_and_only_they_are_trusted() {
    let release: Vec<_> = super::keys::trusted();
    #[cfg(not(feature = "test-key"))]
    {
        assert_eq!(release.len(), 2);
        assert_ne!(release[0], release[1]);
        assert!(!release.contains(&keys::key_of(keys::TEST_PEM)));
    }
    #[cfg(feature = "test-key")]
    assert_eq!(release[2], keys::key_of(keys::TEST_PEM));
}

#[test]
fn versions_compare_as_numbers() {
    assert!(manifest::newer("0.10.0", "0.9.9"));
    assert!(manifest::newer("1.0.0", "0.99.99"));
    assert!(!manifest::newer("0.5.0", "0.5.0"));
    assert!(!manifest::newer("0.4.10", "0.5.0"));
    for bad in ["0.6", "0.6.0.1", "0.6.x", "v0.6.0", "0.6.0-rc1", ""] {
        assert!(!manifest::newer(bad, "0.5.0"), "{bad}");
        assert_eq!(manifest::parse_version(bad), None, "{bad}");
    }
}

#[test]
fn a_release_file_name_that_would_escape_the_folder_is_refused() {
    let w = World::new();
    let m = String::from_utf8(w.manifest("brick", "0.6.0", manifest::PLATFORM, TARBALL))
        .unwrap()
        .replace("brick-0.6.0.tar.gz", "..%2Fx");
    w.serve(m.as_bytes(), &w.seed, "0.6.0", Reply::Ok(TARBALL.to_vec()));
    assert!(matches!(run(&w.check()), Err(Problem::Manifest(_))));
    w.assert_nothing_staged();
}

#[test]
fn a_connection_the_server_closed_is_never_reused() {
    let w = World::new();
    w.serve_release("0.6.0", TARBALL);
    w.server.stale.store(true, Ordering::SeqCst);
    assert_eq!(run(&w.check()).unwrap(), Outcome::Staged("0.6.0".into()));
}
