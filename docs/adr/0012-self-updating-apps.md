# ADR 0012: Apps That Update Themselves

Status: Accepted (2026-10-21)

Date: 2026-10-21

## Context

Each app is installed per user by its own installer: `install.sh` on Linux (the binary in `~/.local/bin/<app>`, its desktop entry, icon, and licences in `~/.local/share`) and macOS (`~/Applications/<App>.app`), and `install.ps1` or `Setup.exe` on Windows (`%LOCALAPPDATA%\Programs\<App>`, ADR 0011). The only way to update is to run the installer again, so most users stay on the version they first installed.

Each app's `latest.json` on `dl.cronkite.cc` (Brick's at the root, the others under `/slate`, `/frame`, and `/etch`) already lists, for each platform, the release file's `url`, `sha256`, and `bytes` (`scripts/release_json.py`). The installers check the download against that sha256. That check catches corruption, but not a compromised server: whoever can write `latest.json` can also write a matching sha256.

The apps make no network requests today. The workspace has no HTTP client or signature crate.

The user asked for the apps to update themselves at startup. Decisions (2026-10-21):

1. **Download silently, apply on the next start.** No prompt, and no restart in the middle of a session.
2. **Sign the release manifest with Ed25519**, using an offline key. The public keys are compiled into the apps.
3. **On by default**, with a setting to turn it off.

## Decision

1. **A shared crate, `suite-update`** (all four apps need it; toolkit-free). It may depend on `suite-core`, `suite-platform`, and `suite-file`, and nothing else in the workspace. CLAUDE.md's suite rules gain it. It owns these steps:
   - **Check:** on a background thread started after the first window shows, fetch the app's `latest.json` and `latest.json.sig`. Nothing runs on the UI thread, and the app never waits for the network (invariant 4).
   - **Verify:** the signature over the manifest's exact bytes, then the manifest itself:
     - its `app` field must name this app, so one app's signed manifest can't be served as another's;
     - its `version` must be newer than the running `CARGO_PKG_VERSION`, so an older signed manifest can't downgrade anyone;
     - it must have an entry for this platform.
   - **Download** that platform's file into the app's cache folder (`suite_platform::dirs`, `updates/`). The download must match the manifest's `bytes` and `sha256`, otherwise it's deleted.
   - **Stage:** the verified file and a small record (version, file, sha256) are left in `updates/`, ready to apply.
   - **Apply** (decision 4), then record the applied version, so the next start can say "Updated to Brick 0.6.0" once.
   - Every failure (offline, HTTP error, bad signature, checksum mismatch, a full disk) is silent apart from one line on stderr. Nothing half-done is left behind, and the next start tries again.
2. **Signing.**
   - `release_json.py` adds `"app": "<app>"` to `latest.json`. The fields older installers read are unchanged.
   - `release_json.py` writes `latest.json.sig`: a base64 Ed25519 signature over the file's exact bytes.
   - The private key lives outside the repository and outside CI, at a path `ship.sh` is given (`CRONKITE_RELEASE_KEY`). `ship.sh` refuses a key file inside the work tree. `upload` sends the signature with the manifest. `check` and `verify` check the signature against the repository's public keys.
   - Two key pairs are made, and both public keys are compiled in. One signs releases; the other is kept offline in a separate place, so a lost or leaked key can be replaced without stranding installed apps.
   - A mismatched pair (a cached old signature beside a new manifest) just fails verification until the next start.
   - The curl and PowerShell installers keep their sha256 check. They run once, by hand, from HTTPS.
3. **HTTP and crypto:**
   - `ureq` with `rustls` on `ring`, and the operating system's certificate store (`rustls-native-certs`). It's pure Rust, the same on all three platforms, needs no OpenSSL, and every crate's licence is already on `rust_licenses.py`'s allowlist (MIT, Apache-2.0, ISC).
     - *Amended by UPD-2 (2026-10-21):* the system's own verifier, through `rustls-platform-verifier` (ureq's `platform-verifier`): on Linux the system's certificates (`rustls-native-certs`), on macOS Security.framework, on Windows CryptoAPI, which also check revocation there. No `webpki-roots` (its licence, CDLA-Permissive-2.0, isn't on the allowlist). `untrusted`'s ISC text is written as source comments, which `rust_licenses.py` now reads.
   - `ring` also verifies the Ed25519 signature and computes SHA-256, so the suite carries one crypto library.
   - Rejected: shelling out to `curl` (not on every Linux install, and its errors arrive as text); GIO's TLS (needs glib-networking, which isn't bundled on macOS or Windows).
4. **Applying, per platform.**
   - **Linux:** right after verification, the tarball's binary, desktop entry, icon, and licence files replace the installed ones, each by writing beside it and renaming. Running processes keep the binary they started from, so the next start is the new version. Default apps (`mimeapps.list`) are never touched.
   - **macOS:** the bundle can't be swapped while the app runs, since GTK loads icons, pixbuf loaders, and fonts from it lazily. The staged tarball is unpacked next to the bundle, then swapped in by two renames, in one of two places:
     - when the last running instance of the app exits;
     - otherwise, at the next start before any window opens, followed by a re-exec of the new bundle.
     The ad-hoc signature travels with the bundle. Files our process downloads carry no quarantine attribute.
   - **Windows:** a running exe and its DLLs are locked. When the last running instance exits, it starts the staged `Setup.exe` detached, with `install.ps1`'s options (`/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CURRENTUSER`). Setup replaces the files, the Start menu entry, and the Settings → Apps version. A staged update left behind by a crash is applied at that run's exit.
   - **"Last instance":** the apps are non-unique (more than one process can run the same app), so each running instance holds a `suite_file` owner lock in `updates/`. An update is applied only by an instance that finds no other live owner. Otherwise it waits for a later exit or start.
5. **What updates itself.** Only installs our installers made:
   - Linux: the running binary is `~/.local/bin/<app>` and its desktop entry is there;
   - macOS: a self-contained bundle (`suite_platform::bundle`) in a folder the user can write;
   - Windows: the per-user install folder `suite_platform::bundle` finds.

   Never `cargo run`, `target/` builds, or a future Flatpak or distro package. Those packages are built with a compile-time switch that removes the updater.
6. **Turning it off:**
   - an "Update Automatically" check item next to About in each app's menus, on by default and kept in the app's settings folder;
   - `CRONKITE_NO_UPDATE=1` turns it off for that run.

   When it's off, the app makes no network request at all.
7. **Where releases come from:** each app's release folder (`https://dl.cronkite.cc`, `…/slate`, `…/frame`, `…/etch`) sits beside `AppKind` in `suite-core`. `CRONKITE_UPDATE_URL` overrides it for the checks. Signatures are still verified against the compiled-in keys, so the override can't install anything unsigned.
8. **Testing:**
   - `suite-update`'s tests sign manifests with a test key that is compiled in only under a `test-key` feature, which `ship.sh build` never enables. `ship.sh verify` checks that the shipped binaries hold only the release keys.
   - A local HTTP server serves a newer signed release.
   - Per platform, a check installs an old build with the real installer, serves a newer one, starts and quits the app, and confirms the next start reports the new version. Tampered releases are refused, and the old version keeps working.

## Consequences

- **Bootstrap:** installed copies of today's releases have no updater. Their users run the installer once more, and from then on updates arrive by themselves. The release notes and download pages say so.
- **Network use:** one small request per start (`latest.json` is under 1 KB, plus its signature), and a download of 2 MB (Linux) or 15 to 21 MB (macOS, Windows) per release.
- **Trust:** the signing key becomes the suite's most sensitive secret. Whoever holds it can run code on every user's machine. It never touches CI. Losing both keys means every user reinstalls by hand.
- **A broken release reaches everyone** on their next start. There's no automatic rollback; the fix is a newer release, or rerunning the installer. Keeping the previous version for one start would cost a copy of the install and is left out until a release needs it.
- **Freeze:** a server serving an old, validly signed manifest can stop updates, but can't downgrade or replace an app.
- **Size:** `ureq`, `rustls`, and `ring` add about 1 to 2 MB to each binary.
- **Revisit if:** Developer ID or Authenticode signing arrives (verify the OS signature too), a Flatpak ships (it updates through Flathub, with the updater compiled out), or users ask to be asked first.
