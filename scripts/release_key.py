#!/usr/bin/env python3
"""Release signing keys: Ed25519, through the openssl command (OpenSSL 3), ADR 0012.

    release_key.py new KEY_DIR NAME

`new` makes a key pair: the private key KEY_DIR/NAME.key (mode 0600 in a 0700 folder,
which must be outside the repository) and its public key packaging/release-keys/NAME.pub,
which the apps compile in (UPD-2) and `release_json.py check` verifies against. With
CRONKITE_RELEASE_KEY_PASS set, the private key is encrypted with that passphrase, which
signing then reads from the same variable.

The release keys are two: one signs releases (ship.sh's CRONKITE_RELEASE_KEY), the other
is kept offline elsewhere, so a lost or leaked key can be replaced without stranding
installed apps.
"""
import base64
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
KEYS = ROOT / "packaging" / "release-keys"
PASS = "CRONKITE_RELEASE_KEY_PASS"


class KeyProblem(Exception):
    """A key that can't be used, with the reason in words."""


def _openssl(*args, data=None):
    return subprocess.run(["openssl", *args], input=data, capture_output=True)


def _pass_args(flag):
    return [flag, f"env:{PASS}"] if os.environ.get(PASS) else []


def inside_repo(path):
    """Whether `path` (resolved, links followed) lies in the repository's work tree."""
    return Path(path).resolve().is_relative_to(ROOT)


def new(key_dir, name, keys=KEYS):
    """Make KEY_DIR/NAME.key and keys/NAME.pub; returns their paths."""
    key_dir = Path(key_dir)
    if inside_repo(key_dir):
        raise KeyProblem(f"{key_dir} is inside the repository; a private key never goes there")
    key, pub = key_dir / f"{name}.key", Path(keys) / f"{name}.pub"
    for p in (key, pub):
        if p.exists():
            raise KeyProblem(f"{p} already exists")
    key_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(key_dir, 0o700)
    cipher = ["-aes-256-cbc", *_pass_args("-pass")] if os.environ.get(PASS) else []
    old = os.umask(0o077)
    try:
        r = _openssl("genpkey", "-algorithm", "ed25519", *cipher, "-out", str(key))
    finally:
        os.umask(old)
    if r.returncode:
        raise KeyProblem(f"openssl genpkey: {r.stderr.decode().strip()}")
    Path(keys).mkdir(parents=True, exist_ok=True)
    r = _openssl("pkey", "-in", str(key), *_pass_args("-passin"), "-pubout", "-out", str(pub))
    if r.returncode:
        key.unlink()
        raise KeyProblem(f"openssl pkey: {r.stderr.decode().strip()}")
    return key, pub


def sign(key, data, test=False):
    """Base64 of the Ed25519 signature of `data` (bytes) with the private key file `key`.
    `test`: the key is suite-update's test key, which lives in the repository."""
    if not Path(key).is_file():
        raise KeyProblem(f"{key}: no such key file")
    if inside_repo(key) and not test:
        raise KeyProblem(f"{key} is inside the repository; keep the release key outside it")
    with tempfile.TemporaryDirectory() as tmp:
        msg = Path(tmp) / "msg"
        msg.write_bytes(data)
        r = _openssl("pkeyutl", "-sign", "-rawin", "-inkey", str(key), *_pass_args("-passin"), "-in", str(msg))
    if r.returncode or len(r.stdout) != 64:
        raise KeyProblem(f"{key}: can't sign with it ({r.stderr.decode().strip() or 'not an Ed25519 key'})")
    return base64.b64encode(r.stdout).decode()


def verifies(data, signature, keys=KEYS):
    """The name of the public key in `keys` that `signature` (base64) of `data` verifies
    with, or None."""
    try:
        raw = base64.b64decode(signature.strip(), validate=True)
    except ValueError:
        return None
    if len(raw) != 64:
        return None
    with tempfile.TemporaryDirectory() as tmp:
        msg, sig = Path(tmp) / "msg", Path(tmp) / "sig"
        msg.write_bytes(data)
        sig.write_bytes(raw)
        for pub in sorted(Path(keys).glob("*.pub")):
            r = _openssl("pkeyutl", "-verify", "-rawin", "-pubin", "-inkey", str(pub), "-in", str(msg),
                         "-sigfile", str(sig))
            if r.returncode == 0:
                return pub.stem
    return None


def main(argv):
    if len(argv) == 4 and argv[1] == "new":
        try:
            key, pub = new(argv[2], argv[3])
        except KeyProblem as e:
            print(f"release_key: {e}", file=sys.stderr)
            return 1
        print(f"private key {key}{' (encrypted)' if os.environ.get(PASS) else ''}")
        print(f"public key  {pub}")
        return 0
    print(__doc__.strip().split("\n\n")[1], file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
