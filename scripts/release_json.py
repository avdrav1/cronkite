#!/usr/bin/env python3
"""An app's latest.json, which install.sh, install.ps1, the download page, and the apps'
updater read, and its signature, latest.json.sig.

    release_json.py write APP URL VERSION LINUX_TAR MAC_TAR MAC_DMG WIN_SETUP OUT
    release_json.py check LATEST_JSON DIST

`write` writes OUT as one line: "app" (the command: brick, slate, frame, etch), each
platform's download under "platforms" ("linux-x86_64": url, sha256, bytes; "macos-arm64":
the same for the tarball the curl installer takes, and dmg, dmg_sha256, dmg_bytes for the
DMG, MAC-7; "windows-x86_64": url, sha256, bytes of the Setup.exe install.ps1 runs,
WIN-6), then Linux's version, date, url, sha256, and bytes at the top level, last.
Installers from before MAC-7 read those top-level fields with a sed that takes a key's
last match on the line, so the order matters. URL is the release's folder
(.../releases/vX.Y.Z); a macOS or Windows file that isn't there is left out.

With CRONKITE_RELEASE_KEY naming the release's private key (outside the repository;
release_key.py), `write` also writes OUT.sig: the base64 Ed25519 signature of OUT's exact
bytes, which must verify with one of the public keys in packaging/release-keys (ADR 0012).
Without it OUT is written unsigned and a stale OUT.sig removed, so `check` fails until the
release is signed. The update checks (scripts/check_update.sh and its macOS and Windows
twins) set CRONKITE_RELEASE_TEST_KEY=1 and CRONKITE_RELEASE_KEY to
crates/suite-update/test-key/test.key: signed with the key only `test-key` builds trust.

`check` compares LATEST_JSON with the files in DIST: every download it names is there, with
its sha256 and size, named for its app, and the top-level fields are Linux's; and
LATEST_JSON.sig verifies with a key in packaging/release-keys.
"""
import datetime
import hashlib
import json
import os
import sys
from pathlib import Path

import release_key

KEY = "CRONKITE_RELEASE_KEY"
# Update checks only (UPD-4 to UPD-6): CRONKITE_RELEASE_TEST_KEY=1 signs with the test key
# the apps trust under suite-update's `test-key` feature; CRONKITE_RELEASE_KEY must then name
# exactly crates/suite-update/test-key/test.key, and the signature must verify with test.pub.
TEST_KEY = "CRONKITE_RELEASE_TEST_KEY"
TEST_KEYS = release_key.ROOT / "crates" / "suite-update" / "test-key"


def _file(url, path):
    data = Path(path).read_bytes()
    return {"url": f"{url}/{os.path.basename(path)}", "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}


def release(app, url, version, linux_tar, mac_tar, mac_dmg, win_setup, date):
    """The release as a dict in latest.json's key order."""
    linux = _file(url, linux_tar)
    platforms = {"linux-x86_64": linux}
    if os.path.isfile(mac_tar) and os.path.isfile(mac_dmg):
        dmg = _file(url, mac_dmg)
        platforms["macos-arm64"] = dict(_file(url, mac_tar), dmg=dmg["url"], dmg_sha256=dmg["sha256"], dmg_bytes=dmg["bytes"])
    if os.path.isfile(win_setup):
        platforms["windows-x86_64"] = _file(url, win_setup)
    return {"app": app, "platforms": platforms, "version": version, "date": date, **linux}


def dumps(rel):
    return json.dumps(rel, separators=(",", ":"))


def write(rel, out, key=None, keys=release_key.KEYS, test=False):
    """Write `rel` to `out` and, with `key`, its signature to `out`.sig; True when signed.
    Raises release_key.KeyProblem for a key that can't sign or isn't one of `keys`. `test`:
    `key` is the update checks' test key (TEST_KEY)."""
    data = (dumps(rel) + "\n").encode()
    sig_path = Path(f"{out}.sig")
    signature = None
    if key:
        signature = release_key.sign(key, data, test=test)
        if not release_key.verifies(data, signature, keys):
            raise release_key.KeyProblem(f"{key} isn't the private half of any key in {keys}")
    Path(out).write_bytes(data)
    if signature:
        sig_path.write_text(signature + "\n")
    else:
        sig_path.unlink(missing_ok=True)
    return signature is not None


def check(rel, dist):
    """Problems with `rel` against the files in `dist`, as messages; none if it's right."""
    problems = []
    app = rel.get("app")
    if not app:
        problems.append("no app")
    platforms = rel.get("platforms", {})
    if list(rel)[-3:] != ["url", "sha256", "bytes"]:
        problems.append("the top-level url, sha256, and bytes aren't last")
    linux = platforms.get("linux-x86_64")
    if not linux:
        problems.append("no linux-x86_64")
    elif any(rel.get(k) != linux[k] for k in ("url", "sha256", "bytes")):
        problems.append("the top-level url, sha256, and bytes aren't linux-x86_64's")
    files = []
    for name, p in platforms.items():
        files.append((name, p.get("url"), p.get("sha256"), p.get("bytes")))
        if "dmg" in p:
            files.append((name + " dmg", p["dmg"], p.get("dmg_sha256"), p.get("dmg_bytes")))
    for name, url, sha, size in files:
        if not url or not sha or not isinstance(size, int):
            problems.append(f"{name}: url, sha256, or bytes missing")
            continue
        if f"/v{rel.get('version')}/" not in url:
            problems.append(f"{name}: {url} isn't in v{rel.get('version')}'s folder")
        if app and not os.path.basename(url).startswith(f"{app}-"):
            problems.append(f"{name}: {url} isn't {app}'s")
        path = os.path.join(dist, os.path.basename(url))
        if not os.path.isfile(path):
            problems.append(f"{name}: {path} missing")
            continue
        data = Path(path).read_bytes()
        if hashlib.sha256(data).hexdigest() != sha or len(data) != size:
            problems.append(f"{name}: {path} doesn't match its sha256 and size")
    return problems


def check_signature(path, keys=release_key.KEYS):
    """Problems with `path`.sig as the signature of `path`'s bytes; none if one of `keys`
    verifies it."""
    sig = Path(f"{path}.sig")
    if not sig.is_file():
        return [f"{sig} missing: unsigned (set {KEY} and write it again)"]
    if not release_key.verifies(Path(path).read_bytes(), sig.read_text(), keys):
        return [f"{sig} doesn't verify with any key in {keys}"]
    return []


def main(argv):
    if len(argv) == 10 and argv[1] == "write":
        date = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d")
        rel = release(*argv[2:9], date)
        try:
            if os.environ.get(TEST_KEY) == "1":
                key = os.environ.get(KEY)
                if not key or Path(key).resolve() != TEST_KEYS / "test.key":
                    raise release_key.KeyProblem(f"{TEST_KEY}=1 signs only with {TEST_KEYS / 'test.key'}")
                signed = write(rel, argv[9], key, keys=TEST_KEYS, test=True)
            else:
                signed = write(rel, argv[9], os.environ.get(KEY))
        except release_key.KeyProblem as e:
            print(f"release_json: {e}", file=sys.stderr)
            return 1
        if not signed:
            print(f"release_json: {argv[9]} is unsigned ({KEY} not set)", file=sys.stderr)
        return 0
    if len(argv) == 4 and argv[1] == "check":
        rel = json.loads(Path(argv[2]).read_text())
        problems = check(rel, argv[3]) + check_signature(argv[2])
        for p in problems:
            print(f"FAIL {argv[2]}: {p}")
        if problems:
            return 1
        print(f"ok   {argv[2]}: {rel['app']} {rel['version']} for {', '.join(rel['platforms'])}, signed")
        return 0
    print(__doc__.strip().split("\n\n")[1], file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
