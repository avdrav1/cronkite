#!/usr/bin/env python3
"""A local site that makes the apps' real installers install a given LIVE release instead
of the latest one: the update-in-the-wild checks (UPD-7) install an older release this way,
then let it update itself from the live latest.json.

    pin_release.py SITE APP:VERSION...

For each APP (brick, slate, frame, etch), writes SITE/APP/latest.json naming the live files
of VERSION in its download folder (https://dl.cronkite.cc for Brick, .../slate, .../frame,
.../etch; each file's sha256 from its live .sha256, its bytes from the server's
Content-Length), in release_json.py's layout and key order, and copies the live installers
from https://APP.cronkite.cc into SITE/APP/install.sh and install.ps1. Serve SITE and run
an installer with APP_DOWNLOAD_URL (BRICK_DOWNLOAD_URL, ...) at SITE/APP: install.sh and
install.ps1 read latest.json and check each download's sha256 only, never latest.json.sig,
so this latest.json is unsigned. The apps never read it: they check the live one.

A platform whose files aren't in the release folder is left out; Linux's must be there.
"""
import email.utils
import sys
import urllib.error
import urllib.request
from pathlib import Path

import release_json

APPS = ("brick", "slate", "frame", "etch")
DL = "https://dl.cronkite.cc"
# A user agent of our own: some CDNs refuse urllib's.
HEADERS = {"User-Agent": "cronkite-pin-release/1"}


def download_base(app):
    """The app's download folder, as suite-core's AppKind::release_url has it."""
    return DL if app == "brick" else f"{DL}/{app}"


def _request(url, method="GET"):
    return urllib.request.urlopen(urllib.request.Request(url, method=method, headers=HEADERS), timeout=60)


def get(url):
    with _request(url) as r:
        return r.read()


def live_file(url):
    """(url, sha256, bytes) of a live release file and its Last-Modified date; None when
    it isn't there."""
    try:
        with _request(url, "HEAD") as r:
            size = int(r.headers["Content-Length"])
            modified = r.headers.get("Last-Modified")
        sha = get(f"{url}.sha256").decode().split()[0]
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None
        raise
    if len(sha) != 64:
        raise SystemExit(f"pin_release: {url}.sha256 isn't a sha256: {sha!r}")
    date = email.utils.parsedate_to_datetime(modified).strftime("%Y-%m-%d") if modified else ""
    return {"url": url, "sha256": sha, "bytes": size}, date


def pinned(app, version):
    """latest.json's dict for the live release `version` of `app`."""
    folder = f"{download_base(app)}/releases/v{version}"
    linux = live_file(f"{folder}/{app}-{version}-x86_64-linux.tar.gz")
    if not linux:
        raise SystemExit(f"pin_release: {app} {version} has no Linux release at {folder}")
    linux, date = linux
    platforms = {"linux-x86_64": linux}
    mac = live_file(f"{folder}/{app}-{version}-arm64-macos.tar.gz")
    dmg = live_file(f"{folder}/{app}-{version}-arm64-macos.dmg")
    if mac and dmg:
        platforms["macos-arm64"] = dict(mac[0], dmg=dmg[0]["url"], dmg_sha256=dmg[0]["sha256"], dmg_bytes=dmg[0]["bytes"])
    win = live_file(f"{folder}/{app}-{version}-x86_64-windows-setup.exe")
    if win:
        platforms["windows-x86_64"] = win[0]
    return {"app": app, "platforms": platforms, "version": version, "date": date, **linux}


def main(argv):
    if len(argv) < 3:
        print("usage: pin_release.py SITE APP:VERSION...", file=sys.stderr)
        return 2
    site = Path(argv[1])
    for spec in argv[2:]:
        app, _, version = spec.partition(":")
        if app not in APPS or not version:
            print(f"pin_release: {spec!r} isn't APP:VERSION with APP one of {', '.join(APPS)}", file=sys.stderr)
            return 2
        rel = pinned(app, version)
        out = site / app
        out.mkdir(parents=True, exist_ok=True)
        (out / "latest.json").write_text(release_json.dumps(rel) + "\n")
        for installer in ("install.sh", "install.ps1"):
            (out / installer).write_bytes(get(f"https://{app}.cronkite.cc/{installer}"))
        print(f"{app} {version}: {', '.join(rel['platforms'])} -> {out / 'latest.json'}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
