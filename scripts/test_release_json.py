"""Tests for release_json.py and release_key.py. Run: python3 -m unittest discover -s scripts"""
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release_json  # noqa: E402
import release_key  # noqa: E402

URL = "https://dl.example.com/slate/releases/v1.2.3"


class LatestJson(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.dist = Path(self.dir.name)
        self.linux = self.put("slate-1.2.3-x86_64-linux.tar.gz", b"linux tarball")
        self.mac_tar = self.dist / "slate-1.2.3-arm64-macos.tar.gz"
        self.mac_dmg = self.dist / "slate-1.2.3-arm64-macos.dmg"
        self.setup = self.dist / "slate-1.2.3-x86_64-windows-setup.exe"

    def tearDown(self):
        self.dir.cleanup()

    def put(self, name, data):
        (self.dist / name).write_bytes(data)
        return self.dist / name

    def release(self):
        return release_json.release("slate", URL, "1.2.3", str(self.linux), str(self.mac_tar), str(self.mac_dmg),
                                    str(self.setup), "2026-10-16")

    def test_every_platform_with_linux_at_the_top_level_last(self):
        self.put(self.mac_tar.name, b"mac tarball")
        self.put(self.mac_dmg.name, b"mac dmg")
        self.put(self.setup.name, b"MZ setup")
        rel = self.release()
        self.assertEqual(list(rel), ["app", "platforms", "version", "date", "url", "sha256", "bytes"])
        self.assertEqual(rel["app"], "slate")
        self.assertEqual(list(rel["platforms"]), ["linux-x86_64", "macos-arm64", "windows-x86_64"])
        # install.ps1 reads exactly these three (WIN-5).
        self.assertEqual(rel["platforms"]["windows-x86_64"], {
            "url": f"{URL}/slate-1.2.3-x86_64-windows-setup.exe",
            "sha256": hashlib.sha256(b"MZ setup").hexdigest(),
            "bytes": 8,
        })
        mac = rel["platforms"]["macos-arm64"]
        self.assertEqual(mac["url"], f"{URL}/slate-1.2.3-arm64-macos.tar.gz")
        self.assertEqual(mac["dmg"], f"{URL}/slate-1.2.3-arm64-macos.dmg")
        self.assertEqual(mac["dmg_bytes"], 7)
        for key in ("url", "sha256", "bytes"):
            self.assertEqual(rel[key], rel["platforms"]["linux-x86_64"][key])
        self.assertEqual(release_json.check(rel, str(self.dist)), [])

    def test_a_platform_not_built_is_left_out(self):
        self.put(self.setup.name, b"MZ setup")
        rel = self.release()
        self.assertEqual(list(rel["platforms"]), ["linux-x86_64", "windows-x86_64"])
        self.put(self.mac_tar.name, b"mac tarball")  # the DMG is still missing
        self.assertNotIn("macos-arm64", self.release()["platforms"])
        self.setup.unlink()
        self.assertEqual(list(self.release()["platforms"]), ["linux-x86_64"])

    def test_the_old_installers_sed_still_finds_linux(self):
        # install.sh before MAC-7: sed -n 's/.*"url":"\([^"]*\)".*/\1/p', greedy, so the last match.
        self.put(self.mac_tar.name, b"mac tarball")
        self.put(self.mac_dmg.name, b"mac dmg")
        self.put(self.setup.name, b"MZ setup")
        line = release_json.dumps(self.release())
        self.assertNotIn("\n", line)
        self.assertEqual(re.sub(r'.*"url":"([^"]*)".*', r"\1", line), str(URL + "/" + self.linux.name))
        self.assertEqual(json.loads(line), self.release())

    def test_check_finds_a_changed_or_missing_file(self):
        self.put(self.setup.name, b"MZ setup")
        rel = self.release()
        self.put(self.setup.name, b"MZ setup, rebuilt")
        self.assertEqual(release_json.check(rel, str(self.dist)),
                         ["windows-x86_64: " + str(self.setup) + " doesn't match its sha256 and size"])
        self.setup.unlink()
        self.assertEqual(release_json.check(rel, str(self.dist)), ["windows-x86_64: " + str(self.setup) + " missing"])

    def test_check_wants_linux_last_and_each_file_in_the_versions_folder(self):
        rel = self.release()
        moved = {"version": rel["version"], "url": rel["url"], "sha256": rel["sha256"], "bytes": rel["bytes"],
                 "platforms": rel["platforms"]}
        self.assertIn("the top-level url, sha256, and bytes aren't last", release_json.check(moved, str(self.dist)))
        other = dict(rel, version="1.2.4")
        self.assertEqual(release_json.check(other, str(self.dist)),
                         [f"linux-x86_64: {rel['url']} isn't in v1.2.4's folder"])

    def test_check_wants_the_apps_own_files(self):
        rel = self.release()
        self.assertEqual(release_json.check(dict(rel, app="brick"), str(self.dist)),
                         [f"linux-x86_64: {rel['url']} isn't brick's"])
        del rel["app"]
        self.assertEqual(release_json.check(rel, str(self.dist)), ["no app"])


def _ed25519():
    return shutil.which("openssl") is not None and subprocess.run(
        ["openssl", "genpkey", "-algorithm", "ed25519"], capture_output=True).returncode == 0


@unittest.skipUnless(_ed25519(), "needs openssl with Ed25519 (OpenSSL 3)")
class Signature(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        tmp = Path(self.dir.name)
        self.dist, self.keys, self.private = tmp / "dist", tmp / "release-keys", tmp / "private"
        self.dist.mkdir()
        self.key, _ = release_key.new(self.private, "release-1", self.keys)
        self.key2, _ = release_key.new(self.private, "release-2", self.keys)
        linux = self.dist / "slate-1.2.3-x86_64-linux.tar.gz"
        linux.write_bytes(b"linux tarball")
        self.rel = release_json.release("slate", URL, "1.2.3", str(linux), "", "", "", "2026-10-21")
        self.out = self.dist / "latest.json"

    def tearDown(self):
        self.dir.cleanup()

    def test_a_signed_release_verifies_with_either_release_key(self):
        for key, name in ((self.key, "release-1"), (self.key2, "release-2")):
            self.assertTrue(release_json.write(self.rel, self.out, str(key), self.keys))
            data = self.out.read_bytes()
            self.assertEqual(json.loads(data), self.rel)
            self.assertEqual(release_key.verifies(data, Path(f"{self.out}.sig").read_text(), self.keys), name)
            self.assertEqual(release_json.check_signature(self.out, self.keys), [])

    def test_a_tampered_byte_fails(self):
        release_json.write(self.rel, self.out, str(self.key), self.keys)
        data = bytearray(self.out.read_bytes())
        data[data.index(b"1.2.3")] = ord("9")
        self.out.write_bytes(bytes(data))
        self.assertEqual(release_json.check_signature(self.out, self.keys),
                         [f"{self.out}.sig doesn't verify with any key in {self.keys}"])

    def test_another_key_neither_signs_nor_verifies(self):
        stranger, _ = release_key.new(self.private, "stranger", Path(self.dir.name) / "elsewhere")
        with self.assertRaisesRegex(release_key.KeyProblem, "isn't the private half of any key"):
            release_json.write(self.rel, self.out, str(stranger), self.keys)
        self.assertFalse(self.out.exists())
        # A signature made with it, put beside the manifest by someone else.
        release_json.write(self.rel, self.out, None, self.keys)
        Path(f"{self.out}.sig").write_text(release_key.sign(str(stranger), self.out.read_bytes()))
        self.assertEqual(release_json.check_signature(self.out, self.keys),
                         [f"{self.out}.sig doesn't verify with any key in {self.keys}"])

    def test_unsigned_drops_a_stale_signature_and_check_fails(self):
        release_json.write(self.rel, self.out, str(self.key), self.keys)
        self.assertFalse(release_json.write(dict(self.rel, date="2026-10-22"), self.out, None, self.keys))
        self.assertFalse(Path(f"{self.out}.sig").exists())
        self.assertEqual(release_json.check_signature(self.out, self.keys),
                         [f"{self.out}.sig missing: unsigned (set CRONKITE_RELEASE_KEY and write it again)"])

    def test_a_key_inside_the_repository_is_refused(self):
        inside = release_key.ROOT / "target" / "release-test.key"
        inside.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(self.key, inside)
        try:
            with self.assertRaisesRegex(release_key.KeyProblem, "inside the repository"):
                release_key.sign(str(inside), b"x")
        finally:
            inside.unlink()
        with self.assertRaisesRegex(release_key.KeyProblem, "inside the repository"):
            release_key.new(release_key.ROOT / "target" / "keys", "release-3", self.keys)
        self.assertFalse((release_key.ROOT / "target" / "keys").exists())

    def test_an_encrypted_key_signs_with_its_passphrase(self):
        with mock.patch.dict(os.environ, {release_key.PASS: "correct horse"}):
            key, _ = release_key.new(self.private, "release-3", self.keys)
            self.assertTrue(release_json.write(self.rel, self.out, str(key), self.keys))
        self.assertIn(b"ENCRYPTED", key.read_bytes())
        with mock.patch.dict(os.environ, {release_key.PASS: "wrong"}):
            with self.assertRaisesRegex(release_key.KeyProblem, "can't sign"):
                release_key.sign(str(key), b"x")

    def test_the_cli_signs_only_with_a_release_key_and_checks_against_them(self):
        # The CLI uses the repository's keys (packaging/release-keys), which this test key isn't.
        script = str(Path(release_json.__file__))
        args = [sys.executable, script, "write", "slate", URL, "1.2.3", str(self.dist / "slate-1.2.3-x86_64-linux.tar.gz"),
                "none", "none", "none", str(self.out)]
        r = subprocess.run(args, env=dict(os.environ, CRONKITE_RELEASE_KEY=str(self.key)), capture_output=True, text=True)
        self.assertEqual((r.returncode, r.stderr.strip()),
                         (1, f"release_json: {self.key} isn't the private half of any key in {release_key.KEYS}"))
        self.assertFalse(self.out.exists())
        env = {k: v for k, v in os.environ.items() if k != "CRONKITE_RELEASE_KEY"}
        r = subprocess.run(args, env=env, capture_output=True, text=True)
        self.assertEqual((r.returncode, r.stderr.strip()),
                         (0, f"release_json: {self.out} is unsigned (CRONKITE_RELEASE_KEY not set)"))
        r = subprocess.run([sys.executable, script, "check", str(self.out), str(self.dist)], capture_output=True, text=True)
        self.assertEqual((r.returncode, r.stdout.strip()),
                         (1, f"FAIL {self.out}: {self.out}.sig missing: unsigned (set CRONKITE_RELEASE_KEY and write it again)"))

    def test_the_update_checks_sign_with_the_test_key_only_when_asked(self):
        script = str(Path(release_json.__file__))
        args = [sys.executable, script, "write", "slate", URL, "1.2.3", str(self.dist / "slate-1.2.3-x86_64-linux.tar.gz"),
                "none", "none", "none", str(self.out)]
        test_key = release_json.TEST_KEYS / "test.key"
        # Without the switch the test key is a key inside the repository: refused.
        r = subprocess.run(args, env=dict(os.environ, CRONKITE_RELEASE_KEY=str(test_key)), capture_output=True, text=True)
        self.assertEqual(r.returncode, 1)
        self.assertIn("inside the repository", r.stderr)
        # With it, a release key is refused, and the test key signs what only test.pub verifies.
        test = dict(os.environ, CRONKITE_RELEASE_TEST_KEY="1")
        r = subprocess.run(args, env=dict(test, CRONKITE_RELEASE_KEY=str(self.key)), capture_output=True, text=True)
        self.assertEqual((r.returncode, r.stderr.strip()),
                         (1, f"release_json: CRONKITE_RELEASE_TEST_KEY=1 signs only with {test_key}"))
        r = subprocess.run(args, env=dict(test, CRONKITE_RELEASE_KEY=str(test_key)), capture_output=True, text=True)
        self.assertEqual((r.returncode, r.stderr), (0, ""))
        data, sig = self.out.read_bytes(), Path(f"{self.out}.sig").read_text()
        self.assertEqual(release_key.verifies(data, sig, release_json.TEST_KEYS), "test")
        self.assertIsNone(release_key.verifies(data, sig))


class Trusts(unittest.TestCase):
    """release_key.py trusts: a release program carries every release key, never the test key."""

    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.root = Path(self.dir.name)
        self.keys = self.root / "keys"
        self.keys.mkdir()
        for name, body in (("release-1", "AAAA1="), ("release-2", "BBBB2=")):
            (self.keys / f"{name}.pub").write_text(f"-----BEGIN PUBLIC KEY-----\n{body}\n-----END PUBLIC KEY-----\n")
        self.test_pub = self.root / "test.pub"
        self.test_pub.write_text("-----BEGIN PUBLIC KEY-----\nTTTT3=\n-----END PUBLIC KEY-----\n")

    def tearDown(self):
        self.dir.cleanup()

    def program(self, name, *bodies):
        path = self.root / name
        path.write_bytes(b"\x7fELF..." + b"...".join(b.encode() for b in bodies) + b"...")
        path.chmod(0o755)
        return path

    def trusts(self, path):
        return release_key.trusts(path, self.keys, self.test_pub)

    def test_a_release_build_carries_both_release_keys_and_not_the_test_key(self):
        self.assertEqual(self.trusts(self.program("brick", "AAAA1=", "BBBB2=")), [])
        p = self.program("test-build", "AAAA1=", "BBBB2=", "TTTT3=")
        self.assertEqual(self.trusts(p), [f"{p} trusts the test key (built with suite-update's test-key feature)"])
        p = self.program("old", "AAAA1=")
        self.assertEqual(self.trusts(p), [f"{p} lacks the release key release-2"])

    def test_a_tarball_is_judged_by_the_app_program_in_it(self):
        import tarfile
        good = self.program("brick", "AAAA1=", "BBBB2=")
        bad = self.program("bad", "AAAA1=", "BBBB2=", "TTTT3=")
        lib = self.program("libgtk.dylib")
        for tar_name, member, program in (("linux.tar.gz", "brick-1.0.0-x86_64-linux/brick", good),
                                          ("mac.tar.gz", "Brick.app/Contents/MacOS/brick", bad)):
            with tarfile.open(self.root / tar_name, "w:gz") as tar:
                tar.add(program, member)
                tar.add(lib, "Brick.app/Contents/Frameworks/libgtk.dylib")
        self.assertEqual(self.trusts(self.root / "linux.tar.gz"), [])
        self.assertEqual(self.trusts(self.root / "mac.tar.gz"),
                         [f"{self.root / 'mac.tar.gz'}:Brick.app/Contents/MacOS/brick trusts the test key "
                          "(built with suite-update's test-key feature)"])
        with tarfile.open(self.root / "empty.tar.gz", "w:gz") as tar:
            tar.add(lib, "Brick.app/Contents/Frameworks/libgtk.dylib")
        self.assertEqual(self.trusts(self.root / "empty.tar.gz"), [f"{self.root / 'empty.tar.gz'} holds no program"])


if __name__ == "__main__":
    unittest.main()
