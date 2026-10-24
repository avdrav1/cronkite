//! Applying a real tarball to an install in a temp home, as `install.sh` laid it out.

use super::*;
use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;

const APP: AppKind = AppKind::Brick;

/// A temp home with Brick installed as `install.sh` installs it, default apps set, and
/// another app's desktop entry beside Brick's.
struct Home {
    root: PathBuf,
    home: LinuxHome,
    updates: PathBuf,
}

impl Home {
    fn new(name: &str) -> Home {
        let root =
            std::env::temp_dir().join(format!("suite-update-linux-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let home = LinuxHome {
            home: root.join("home"),
            data: root.join("home/.local/share"),
        };
        let put = |rel: &str, data: &[u8]| {
            let path = home.home.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, data).unwrap();
            path
        };
        let bin = put(".local/bin/brick", b"old binary");
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        put(
            ".local/share/applications/dev.cronkite.Brick.desktop",
            b"[Desktop Entry]\nName=Brick\nExec=/old/brick %f\n",
        );
        put(
            ".local/share/applications/other.desktop",
            b"[Desktop Entry]\nName=Other\n",
        );
        put(
            ".local/share/icons/hicolor/scalable/apps/dev.cronkite.Brick.svg",
            b"<svg>old</svg>",
        );
        put(".local/share/doc/brick/LICENSE", b"old licence");
        put(".local/share/doc/brick/LICENSE-APACHE", b"old apache");
        put(".local/share/doc/brick/THIRD-PARTY-RUST.txt", b"old crates");
        put(
            ".config/mimeapps.list",
            b"[Default Applications]\ntext/csv=dev.cronkite.Brick.desktop\ntext/plain=gedit.desktop\n",
        );
        let updates = root.join("home/.cache/brick/updates");
        fs::create_dir_all(&updates).unwrap();
        Home {
            root,
            home,
            updates,
        }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.home.home.join(rel)
    }

    fn read(&self, rel: &str) -> Vec<u8> {
        fs::read(self.path(rel)).unwrap()
    }

    /// Every file under the home but the updates folder: its bytes and mode.
    fn files(&self) -> BTreeMap<PathBuf, (Vec<u8>, u32)> {
        fn walk(dir: &Path, skip: &Path, out: &mut BTreeMap<PathBuf, (Vec<u8>, u32)>) {
            for e in fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p == skip {
                    continue;
                }
                if p.is_dir() {
                    walk(&p, skip, out);
                } else {
                    let mode = fs::metadata(&p).unwrap().mode();
                    out.insert(p.clone(), (fs::read(&p).unwrap(), mode));
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(&self.home.home, &self.path(".cache"), &mut out);
        out
    }

    /// Stage `tarball` in the updates folder; `sha256` overrides its real one.
    fn stage(&self, tarball: &[u8], sha256: Option<&str>) -> Staged {
        let file = self.updates.join("brick-9.9.9-x86_64-linux.tar.gz");
        fs::write(&file, tarball).unwrap();
        let real = hex(ring::digest::digest(&ring::digest::SHA256, tarball).as_ref());
        Staged {
            version: "9.9.9".into(),
            file,
            sha256: sha256.map_or(real, str::to_owned),
        }
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A gzipped tarball of `entries` (path, bytes, mode); a path the tar crate would refuse
/// (`..`, absolute) is written into the header as is.
fn tarball(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut b = tar::Builder::new(gz);
    let mut dir = tar::Header::new_gnu();
    dir.set_entry_type(tar::EntryType::Directory);
    dir.set_mode(0o755);
    dir.set_size(0);
    dir.set_path("brick-9.9.9-x86_64-linux/").unwrap();
    dir.set_cksum();
    b.append(&dir, io::empty()).unwrap();
    for (path, data, mode) in entries {
        let mut h = tar::Header::new_gnu();
        h.set_mode(*mode);
        h.set_size(data.len() as u64);
        let name = &mut h.as_old_mut().name;
        name[..path.len()].copy_from_slice(path.as_bytes());
        h.set_cksum();
        b.append(&h, *data).unwrap();
    }
    b.into_inner().unwrap().finish().unwrap()
}

const NEW_BIN: &[u8] = b"\x7fELF new binary";
const DESKTOP: &[u8] = b"[Desktop Entry]\nName=Brick\nExec=brick %f\nTryExec=brick\n";

fn release() -> Vec<(&'static str, &'static [u8], u32)> {
    vec![
        ("brick-9.9.9-x86_64-linux/brick", NEW_BIN, 0o755),
        (
            "brick-9.9.9-x86_64-linux/dev.cronkite.Brick.desktop",
            DESKTOP,
            0o644,
        ),
        (
            "brick-9.9.9-x86_64-linux/dev.cronkite.Brick.svg",
            b"<svg>new</svg>",
            0o644,
        ),
        ("brick-9.9.9-x86_64-linux/README.md", b"readme", 0o644),
        ("brick-9.9.9-x86_64-linux/LICENSE", b"new licence", 0o644),
        (
            "brick-9.9.9-x86_64-linux/THIRD-PARTY-RUST.txt",
            b"new crates",
            0o644,
        ),
    ]
}

/// No `.<name>.update` file left anywhere in the install.
fn no_leftovers(h: &Home) {
    for p in h.files().keys() {
        let name = p.file_name().unwrap().to_string_lossy();
        assert!(!name.ends_with(".update"), "{} left behind", p.display());
    }
}

#[test]
fn a_staged_release_replaces_the_installed_files_and_never_mimeapps() {
    let h = Home::new("replace");
    let mimeapps = h.read(".config/mimeapps.list");
    let other = h.read(".local/share/applications/other.desktop");
    let old_inode = fs::metadata(h.path(".local/bin/brick")).unwrap().ino();
    // The running app's binary, open as a running process has it.
    let mut running = File::open(h.path(".local/bin/brick")).unwrap();
    let staged = h.stage(&tarball(&release()), None);

    apply_in(APP, &h.updates, &staged, &h.home);

    assert_eq!(h.read(".local/bin/brick"), NEW_BIN);
    let meta = fs::metadata(h.path(".local/bin/brick")).unwrap();
    assert_eq!(meta.mode() & 0o777, 0o755, "the binary stays executable");
    assert_ne!(meta.ino(), old_inode, "renamed over, not written into");
    let mut was = Vec::new();
    running.read_to_end(&mut was).unwrap();
    assert_eq!(
        was, b"old binary",
        "the running binary's open file is as it was"
    );
    let bin = h.path(".local/bin/brick");
    assert_eq!(
        String::from_utf8(h.read(".local/share/applications/dev.cronkite.Brick.desktop")).unwrap(),
        format!(
            "[Desktop Entry]\nName=Brick\nExec={} %f\nTryExec=brick\n",
            bin.display()
        ),
        "Exec is the binary's full path, as install.sh writes it"
    );
    assert_eq!(
        h.read(".local/share/icons/hicolor/scalable/apps/dev.cronkite.Brick.svg"),
        b"<svg>new</svg>"
    );
    assert_eq!(h.read(".local/share/doc/brick/LICENSE"), b"new licence");
    assert_eq!(
        h.read(".local/share/doc/brick/THIRD-PARTY-RUST.txt"),
        b"new crates"
    );
    assert!(
        !h.path(".local/share/doc/brick/LICENSE-APACHE").exists(),
        "a licence the release doesn't carry goes, as install.sh drops it"
    );
    assert!(!h.path(".local/share/doc/brick/README.md").exists());
    assert_eq!(
        h.read(".config/mimeapps.list"),
        mimeapps,
        "default apps untouched"
    );
    assert_eq!(h.read(".local/share/applications/other.desktop"), other);
    no_leftovers(&h);
    assert_eq!(
        crate::session::notice_in(&h.updates, APP, "9.9.9").as_deref(),
        Some("Updated to Brick 9.9.9"),
        "the applied version is recorded for its first start"
    );

    // An instance still running the old version checks again: nothing is applied twice.
    let before = h.files();
    let inode = fs::metadata(h.path(".local/bin/brick")).unwrap().ino();
    assert_eq!(apply(APP, &h.home, &staged).unwrap(), Applied::Already);
    apply_in(APP, &h.updates, &staged, &h.home);
    assert_eq!(h.files(), before);
    assert_eq!(
        fs::metadata(h.path(".local/bin/brick")).unwrap().ino(),
        inode
    );
    assert!(!h.updates.join("applied").exists(), "not recorded again");
}

#[test]
fn a_tampered_staged_file_leaves_the_install_as_it_was() {
    let h = Home::new("tampered");
    let before = h.files();
    let mut bytes = tarball(&release());
    let sha = hex(ring::digest::digest(&ring::digest::SHA256, &bytes).as_ref());
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    let staged = h.stage(&bytes, Some(&sha));

    let e = apply(APP, &h.home, &staged).unwrap_err();
    assert!(e.to_string().contains("sha256"), "{e}");
    apply_in(APP, &h.updates, &staged, &h.home);
    assert_eq!(h.files(), before);
    assert!(!h.updates.join("applied").exists());
}

#[test]
fn an_entry_escaping_the_release_folder_is_refused_and_nothing_changes() {
    for escaping in ["../../.local/bin/brick", "/tmp/suite-update-escaped"] {
        let h = Home::new("escape");
        let before = h.files();
        let mut entries = release();
        // After the binary, so its temp file is written before the bad entry is reached.
        entries.insert(1, (escaping, b"evil", 0o755));
        let staged = h.stage(&tarball(&entries), None);

        let e = apply(APP, &h.home, &staged).unwrap_err();
        assert!(e.to_string().contains("leaves the release's folder"), "{e}");
        assert_eq!(h.files(), before, "{escaping}");
        assert!(!Path::new("/tmp/suite-update-escaped").exists());
    }
}

#[test]
fn a_release_missing_a_file_or_with_a_link_for_one_is_refused() {
    let h = Home::new("incomplete");
    let before = h.files();
    let mut entries = release();
    entries.retain(|(p, _, _)| !p.ends_with(".svg"));
    let staged = h.stage(&tarball(&entries), None);
    let e = apply(APP, &h.home, &staged).unwrap_err();
    assert!(e.to_string().contains("no icon"), "{e}");
    assert_eq!(h.files(), before);

    let mut b = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    let mut link = tar::Header::new_gnu();
    link.set_entry_type(tar::EntryType::Symlink);
    link.set_size(0);
    b.append_link(&mut link, "brick-9.9.9-x86_64-linux/brick", "/bin/sh")
        .unwrap();
    let bytes = b.into_inner().unwrap().finish().unwrap();
    let staged = h.stage(&bytes, None);
    let e = apply(APP, &h.home, &staged).unwrap_err();
    assert!(e.to_string().contains("isn't a file"), "{e}");
    assert_eq!(h.files(), before);
}

#[test]
fn exec_becomes_the_full_path_as_install_sh_makes_it() {
    let bin = Path::new("/home/u/.local/bin/etch");
    assert_eq!(
        full_exec(b"Exec=etch %F\nTryExec=etch\nExec=etch\nExec=etcher %F", "etch", bin),
        b"Exec=/home/u/.local/bin/etch %F\nTryExec=etch\nExec=/home/u/.local/bin/etch\nExec=etcher %F"
    );
}
