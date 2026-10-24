---
name: ship-it
description: Cut a release of a Cronkite app (Brick, the spreadsheet; Slate, the word processor; Frame, the presentation app; or Etch, the image editor) for Linux, macOS, and Windows (a preview) and deploy it to Cloudflare: preflight checks, version bump, release builds (Linux in a container, macOS on GitHub's macos-14 runner, Windows on windows-latest), tarball, DMG, and Setup.exe upload to R2, download page to Cloudflare Pages, git tag. Use whenever the user says "ship it", "release", "deploy", "cut a build", "publish a version", "push to Cloudflare", or asks to put a new build online, even if they don't name Cloudflare.
---

# ship-it

The app is a native Linux binary, a macOS app bundle, and a Windows installer, so Cloudflare hosts the release, not the app itself:

- **R2** stores the Linux tarball, the macOS tarball and DMG (Apple Silicon, MAC-7), the Windows `Setup.exe` (x86_64, WIN-6), their `.sha256` files, and `latest.json` (version, each platform's URL and checksum).
- **Pages** serves the app's download page (`sites/<app>/` at `<app>.cronkite.cc`), which shows the release in `latest.json` and offers the visitor's platform first, and its installers, `install.sh` (Linux, macOS) and `install.ps1` (Windows).

Every step runs through `scripts/ship.sh`. Anything that leaves the machine (upload, site deploy, push) needs an explicit yes from the user in chat first, every time.

Which app: ask if the user didn't say. Put the app before every command (`scripts/ship.sh etch build`); without one, the commands act on Brick. Each app's release differs only in these:

| App | Version in | Bucket prefix | Pages project | Tag | CHANGELOG heading | Release smoke (`verify`) |
| --- | --- | --- | --- | --- | --- | --- |
| Brick | `Cargo.toml` `[workspace.package]` | none | `CF_PAGES_PROJECT` | `vX.Y.Z` | `## vX.Y.Z (YYYY-MM-DD)` | `--bench-open` on a CSV; `check_install.sh` |
| Slate | `apps/slate/Cargo.toml` | `slate/` | `CF_SLATE_PAGES_PROJECT` | `slate-vX.Y.Z` | `## Slate vX.Y.Z (YYYY-MM-DD)` | still running on a .md; `check_install_slate.sh` |
| Frame | `apps/frame/Cargo.toml` | `frame/` | `CF_FRAME_PAGES_PROJECT` | `frame-vX.Y.Z` | `## Frame vX.Y.Z (YYYY-MM-DD)` | `--bench-draw` on the fixture deck; `check_install_frame.sh` |
| Etch | `apps/etch/Cargo.toml` | `etch/` | `CF_ETCH_PAGES_PROJECT` | `etch-vX.Y.Z` | `## Etch vX.Y.Z (YYYY-MM-DD)` | `--bench-stroke`; `check_install_etch.sh` |

Below, `vX.Y.Z` means the app's tag and the download base is `$DOWNLOAD_BASE_URL` plus the bucket prefix. The macOS release of every app is built and checked by `.github/workflows/release-macos.yml` on GitHub (needs `gh`, logged in); its verify job is the macOS release smoke. Pages projects are created on the first `site` deploy if missing; each `<app>.cronkite.cc` custom domain is attached to its project once, in the Cloudflare dashboard. An app ships only once it has its desktop entry and icon (`packaging/dev.cronkite.<App>.*`), its download page and installer (`sites/<app>/`), and its install check; `preflight` fails without them. An app's first release also deploys the suite page (below), which lists it: deploy it after the app's own page, so its install command works by then.

## 0. One-time setup

If `.env.ship` is missing, stop and walk the user through it:

1. Copy `.env.ship.example` to `.env.ship`; confirm `.env.ship` is in `.gitignore`.
2. In the Cloudflare dashboard: create the R2 bucket, enable public access (custom domain or r2.dev), create a Pages project with the name in the app's Pages variable (or let the first `site` deploy create it).
3. Create an API token with Pages Edit and R2 Storage Edit. The user pastes it into `.env.ship` themselves.

Never print, echo, log, or commit token values. Never ask the user to paste a token into chat.

## 1. Preflight

```sh
scripts/ship.sh <app> preflight
```

Stop on any FAIL and report it. Common fixes: commit or stash changes, switch to `main`, bump the version if the tag exists. Do not "fix" failing tests by skipping them.

## 2. Pick the version

Propose a bump from what shipped since the last tag (`git log $(git describe --tags --abbrev=0 2>/dev/null)..HEAD --oneline`; story IDs are in commit subjects):

- `0.0.x` patch: before the app's first milestone release, any internal build.
- `minor`: a milestone gate passed (Brick's M3 gate = `0.1.0`; an app's first gated release is its `0.1.0`).
- `patch` after 0.1.0: fixes only.

Confirm with the user, then:

```sh
scripts/ship.sh bump <patch|minor|major|X.Y.Z>
```

Write the release notes into `CHANGELOG.md` under the app's heading from the table, newest first, one line per story (`- ENG-4: Auto-detect delimiter`). Commit: `release: vX.Y.Z`.

## 3. Build

```sh
scripts/ship.sh <app> build    # compiles in an Ubuntu 24.04 container (needs docker)
scripts/ship.sh <app> verify   # starts the binary on clean Arch, Ubuntu 24.04, and Fedora; runs the installer there
```

Check the output: tarball exists, size is plausible, `dist/<name>/<app> --version` prints the new version, and `verify` ends with `RELEASE OK` and `INSTALL OK`. Stop on any FAIL. Report size and sha256.

The apps update themselves (ADR 0012): every installed copy downloads a newer release's file named in the signed `latest.json` at its next start and applies it (Linux at once, macOS and Windows when the last instance quits), then says "Updated to <App> X.Y.Z" once. So a release reaches everyone without them doing anything, and a broken one does too: there's no rollback but a newer release. Before building a release, the update checks must be green at the release commit: `scripts/check_update.sh` (Linux, Docker), and on GitHub `update-macos.yml` and `update-windows.yml` (`gh workflow run update-macos.yml --ref main`, likewise Windows; both run on pushes that change `crates/suite-update`). They install an old test-key build with the real installer, serve a newer one signed with the test key, and check it arrives, with tampered releases refused. `verify` (and `build --macos`/`--windows` on their runners) runs `release_key.py trusts` on each platform's program: it must carry both release keys and never the test key (a `--features suite-update/test-key` build), so a test build can't ship.

Every release carries `THIRD-PARTY-RUST.txt` (PKG-6, `scripts/rust_licenses.py`): each Rust crate compiled into the app for that platform, its licence, and the licence texts. `build` prints the crate count and the licences used, and fails on a crate without a licence text or with a licence outside the script's allowlist (`ALLOWED`: MIT, Apache-2.0, BSD, ISC, Zlib, 0BSD, Unicode, BSL-1.0, CC0-1.0, Unlicense, MPL-2.0; never GPL, AGPL, or LGPL). If a dependency update trips it, stop and tell the user which crate and licence; don't widen the allowlist without their yes. The Linux installer puts the notice and the app's LICENSE files in `~/.local/share/doc/<app>/`; the Mac bundle has it in `Contents/Resources/licenses/`.

Then the macOS build. It runs on GitHub at the release commit, so that commit must be on GitHub first: ask before pushing it (`git push origin main`; the tag comes later).

```sh
scripts/ship.sh <app> build --macos   # dispatches release-macos.yml at HEAD, waits, then fetch-macos
```

`release-macos.yml` builds the app's ad-hoc-signed `<App>.app` (GTK inside, `scripts/mac_bundle.sh`), its DMG, and the tarball the installer takes, then on a fresh runner with Homebrew moved away installs it with the curl installer and from the DMG and starts it (`scripts/check_mac_install.sh`). `fetch-macos` only takes a green run at HEAD: it puts `dist/<app>-X.Y.Z-arm64-macos.{tar.gz,dmg}` and their `.sha256` files in `dist/` and rewrites `latest.json` with both platforms. To release all four apps at once, run the workflow once (`gh workflow run release-macos.yml --ref main`, its `apps` input defaults to all four), watch it (`gh run watch RUN_ID --exit-status`), then `scripts/ship.sh <app> fetch-macos RUN_ID` per app. On an arm64 Mac with Homebrew's `gtk4 adwaita-icon-theme librsvg dylibbundler`, `build --macos` builds locally instead.

Check: the run's URL, its `verify` job's `check_mac_install: OK`, the run summary's sizes, and the screenshots in its `macos-install` artifact (`gh run download RUN_ID -n macos-install`). Report the DMG's size and sha256.

Then Windows, a preview (ADR 0011: unsigned, per user, Windows 10 and 11 x64). Windows ships only when the user says so; a release without it is complete. It also builds on GitHub at the pushed release commit:

```sh
scripts/ship.sh <app> build --windows   # dispatches release-windows.yml at HEAD, waits, then fetch-windows
```

`.github/workflows/release-windows.yml` (the same `apps` input; release tags and pushes that change the bundling run it too) runs `scripts/ship.sh <app> build --windows` on `windows-latest` in MSYS2's UCRT64 shell, which builds the app and `scripts/win_bundle.sh` makes its self-contained folder and unsigned per-user Inno Setup installer, `dist/<app>-X.Y.Z-x86_64-windows-setup.exe` and `.sha256` (the stripped exe and every DLL it needs in `bin\`, GTK's data, the fonts, `licenses\THIRD-PARTY.txt` from `scripts/win_licenses.py`, which refuses GPL code, and `THIRD-PARTY-RUST.txt`; cairo and libtiff rebuilt without lzo and jbig by `scripts/win_libs.sh`, whose pins follow MSYS2's packages: when MSYS2 updates cairo or libtiff the build stops until the pin is updated). Its `verify` job, on a fresh runner with MSYS2 moved away, installs each app through Setup.exe and through `install.ps1`, starts it, opens and saves a file, and uninstalls it with nothing left (`scripts/check_win_install.ps1`). `fetch-windows` only takes a green run at HEAD: it puts the Setup.exe and its `.sha256` from the artifact `windows-<app>` in `dist/` (a run of another version has no file of this version's name, and `fetch-windows` says so) and rewrites `latest.json` with `windows-x86_64`, keeping Linux and macOS. For all four apps: `gh workflow run release-windows.yml --ref main`, `gh run watch RUN_ID --exit-status`, then `scripts/ship.sh <app> fetch-windows RUN_ID` per app.

Check: the run's URL, its `verify` job's `check_win_install: OK (N checks)`, the run summary's sizes, and the screenshots in its `windows-install` artifact (`gh run download RUN_ID -n windows-install`), per app and install way (`<app>-setup-*` through Setup.exe, `<app>-ps1-*` through `install.ps1`): `1-start-menu`, the window started from its Start menu entry (a new file or the start window), with Windows' own title bar, the menu bar row, and on Brick and Frame the toolbar row; `2-open-with`, the sample file opened through Open with's command, its name in the title; `3-saved`, the edit before Control+S saved it. Also `settings-apps.png` (Settings → Apps → Installed apps listing the app with its version and "Cronkite"), `programs-and-features.png`, and `brick-setup-uninstall-*` (the uninstaller's own dialogs). Look for drawn windows (not black or empty) and text in the bundled fonts. Report the Setup.exe's size and sha256.

## 4. Dry run, then confirm

```sh
scripts/ship.sh <app> upload --dry-run
scripts/ship.sh <app> site --dry-run
```

Show the user what will happen in this shape and wait for a clear yes:

```
Ready to ship vX.Y.Z
- Upload <app>-X.Y.Z-x86_64-linux.tar.gz (N MB), <app>-X.Y.Z-arm64-macos.tar.gz (N MB) and .dmg (N MB), <app>-X.Y.Z-x86_64-windows-setup.exe (N MB) + checksums + latest.json and its signature to R2 bucket <bucket>/<prefix>
- Deploy download page to Pages project <project>
- Tag vX.Y.Z locally
Proceed?
```

A release without the macOS or Windows files (`upload` says "no macOS build in dist/" or "no Windows build in dist/") still ships; its `latest.json` then has no `macos-arm64` or `windows-x86_64`, the Mac installer and `install.ps1` say there's no release for them yet, and the page lists the platform as not in this release yet.

`latest.json` is signed (ADR 0012): `upload` signs it with the private key at `CRONKITE_RELEASE_KEY` (in `.env.ship`; outside the repository; `CRONKITE_RELEASE_KEY_PASS` if it's encrypted) and runs `release_json.py check` before anything leaves the machine, so an unsigned or mis-signed release stops there ("latest.json.sig missing: unsigned", "isn't the private half of any key in packaging/release-keys"). The dry run's last two lines put `latest.json` and `latest.json.sig`. If the key is missing, ask the user for it; never make a new key pair for a release: the apps trust only `packaging/release-keys/release-1.pub` and `release-2.pub`.

The keys: `release-1` signs every release (`CRONKITE_RELEASE_KEY` points at it). `release-2` is the offline backup: kept off this machine (an encrypted copy on removable media or a password manager, `openssl pkey -in release-2.key -aes-256-cbc -out release-2.key.enc`), used only when `release-1` is lost or leaked. Every app trusts both, so either one's signature is believed. Neither key ever goes into the repository, CI, or chat. If `release-1` is lost or leaked, with the user:

1. Fetch `release-2` from its offline place and point `CRONKITE_RELEASE_KEY` at it (`CRONKITE_RELEASE_KEY_PASS` if encrypted).
2. Make the next backup: `python3 scripts/release_key.py new <offline folder> release-3` (it writes `packaging/release-keys/release-3.pub`). If `release-1` leaked, delete `packaging/release-keys/release-1.pub` so new builds stop trusting it; `trusts` then wants `release-2` and `release-3`.
3. Ship a release of every app signed with `release-2`. Installed copies believe it (they trust `release-2`) and, once updated, trust `release-2` and `release-3` only.
4. `release-2` signs from then on; `release-3` goes offline as the backup. If both keys are ever lost, installed copies can't be updated: every user reinstalls by hand.

The first release with the updater (UP0) goes to copies that have none: its CHANGELOG entry and each download page say to run the installer once more, after which updates arrive by themselves.

## 5. Ship

After the yes, in order:

```sh
scripts/ship.sh <app> upload
scripts/ship.sh <app> site
scripts/ship.sh <app> tag
```

Then verify, and report each check:

- `curl -fsSI <download base>/releases/vX.Y.Z/<name>.tar.gz` returns 200, and so do the macOS tarball and DMG and the Windows Setup.exe.
- `curl -fsS <pages URL from wrangler output>/ | grep -c '"version":"X.Y.Z"'` is 1: the deploy writes `latest.json` into the page (Pages fails to publish a `.json` file in the `slate` project).
- `curl -fsS <pages URL>/install.sh | grep -c "<download base>"` is 1, and so is `curl -fsS <pages URL>/install.ps1 | grep -c "<download base>"` (the deploy filled in the download URL).
- Downloaded tarball, DMG, and Setup.exe match their sha256.
- Optionally, on this machine: `curl -fsSL <pages URL>/install.sh | sh` reports the new version (in a throwaway `HOME`, `XDG_CONFIG_HOME`, and `XDG_DATA_HOME` to leave this machine's defaults alone).

If upload succeeds but the site deploy fails, the old page still points at the old version; rerun `site` alone after fixing. If a step fails mid-way, do not delete uploaded objects; report what landed.

## 6. Push

Ask before running `git push origin main vX.Y.Z`.

## 7. Report

```
Shipped vX.Y.Z
Download: <url>  (N MB, sha256 <first 12 chars>)
Mac DMG: <url>  (N MB, sha256 <first 12 chars>; release-macos.yml run <url>)
Windows Setup.exe: <url>  (N MB, sha256 <first 12 chars>; release-windows.yml run <url>)
Page: <pages url>
Stories: <IDs>
Not done: <push pending | none>
```

The download page also serves `install.sh` (stories PKG-1, MAC-7), which reads `latest.json`: the top-level `url` and `sha256` on Linux (installers from before MAC-7 read only those, so `latest.json` keeps them, last on its line), `platforms.macos-arm64` on a Mac; and `install.ps1` (WIN-5), which reads `platforms.windows-x86_64` and runs the Setup.exe silently. `scripts/release_json.py` writes `latest.json` (and `verify` checks it against `dist/`). So a site deploy is all a new version needs; nothing goes to a package registry. Pushing a release tag runs `release-macos.yml` and `release-windows.yml` again for that app; those runs are checks, not the release.

The download page offers Windows visitors (`navigator.userAgentData.platform`, `navigator.platform`, or "Windows NT" in the user agent) the Setup.exe on its button, the PowerShell command `irm https://<app>.cronkite.cc/install.ps1 | iex` first under Install, and SmartScreen's way past an unsigned download first under Downloads: "Windows protected your PC" → More info → Run anyway. Until Authenticode signing, a browser-downloaded Setup.exe always gets that prompt; `irm | iex` doesn't (PowerShell's downloads carry no mark of the web).

The suite page at cronkite.cc (`sites/cronkite/`) is not part of a release: it links to the apps' installers and holds no version. It also serves the suite's installers (PKG-7): `install.sh` (`curl -fsSL https://cronkite.cc/install.sh | sh`, Linux and macOS) and `install.ps1` (`irm https://cronkite.cc/install.ps1 | iex`, Windows), which fetch and run each app's installer from `https://<app>.cronkite.cc`, so a new app release needs no suite deploy; they have no placeholders. Deploy it only when it changes (an app's first release adds its row, an installer changes), with the same yes first: `scripts/ship.sh suite-site --dry-run` (it lists the files it deploys, the installers among them), then `scripts/ship.sh suite-site` (Pages project `CF_SUITE_PAGES_PROJECT`). A change to an installer, the suite's or an app's, runs `install-suite.yml` on push, which installs and uninstalls the suite from the live releases on Linux containers, macOS, and Windows; let it go green first. A new app joins the suite in both installers' app lists, the page, and the three checks (`scripts/check_install_suite.sh`, `check_mac_install_suite.sh`, `check_win_install_suite.ps1`).
