"""Tests for rust_licenses.py. Run: python3 -m unittest discover -s scripts"""
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import rust_licenses  # noqa: E402

MIT = """MIT License

Copyright (c) 2020 Someone

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction."""

APACHE = """                                 Apache License
                           Version 2.0, January 2004

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION"""

BSD3 = """Redistribution and use in source and binary forms, with or without modification,
are permitted provided that the following conditions are met:
3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from this software."""

UNICODE = """UNICODE LICENSE V3
Permission is hereby granted, free of charge, to any person obtaining a copy of data
files and any associated documentation (the "Data Files") or software"""


class Expressions(unittest.TestCase):
    def test_an_expression_becomes_its_alternatives(self):
        alts = rust_licenses.alternatives
        self.assertEqual(alts("MIT"), [{"MIT"}])
        self.assertEqual(alts("MIT OR Apache-2.0"), [{"MIT"}, {"Apache-2.0"}])
        self.assertEqual(alts("(MIT OR Apache-2.0) AND Unicode-3.0"),
                         [{"MIT", "Unicode-3.0"}, {"Apache-2.0", "Unicode-3.0"}])
        self.assertEqual(alts("(Apache-2.0 OR MIT) AND BSD-3-Clause"),
                         [{"Apache-2.0", "BSD-3-Clause"}, {"MIT", "BSD-3-Clause"}])
        self.assertEqual(alts("MIT/Apache-2.0"), [{"MIT"}, {"Apache-2.0"}], "crates.io's old syntax")
        self.assertEqual(alts("Apache-2.0 WITH LLVM-exception OR MIT"),
                         [{"Apache-2.0 WITH LLVM-exception"}, {"MIT"}])

    def test_a_malformed_expression_is_an_error(self):
        for expr in ["", "(MIT OR Apache-2.0", "MIT OR", "MIT Apache-2.0", "AND MIT"]:
            with self.assertRaises(ValueError, msg=expr):
                rust_licenses.alternatives(expr)


class Texts(unittest.TestCase):
    def test_a_text_is_known_by_its_wording(self):
        self.assertEqual(rust_licenses.text_licences(MIT), {"MIT", "MIT-0"})
        self.assertEqual(rust_licenses.text_licences(APACHE), {"Apache-2.0"})
        self.assertIn("BSD-3-Clause", rust_licenses.text_licences(BSD3))
        self.assertIn("Unicode-3.0", rust_licenses.text_licences(UNICODE))
        self.assertEqual(rust_licenses.text_licences(
            "Licensed under the Apache License, Version 2.0 or the MIT license, at your option."), set(),
            "a COPYRIGHT file naming licences is no licence text")

    def test_a_text_written_as_comments_counts(self):
        isc = ("// Copyright 2015-2016 Brian Smith.\n//\n"
               "// Permission to use, copy, modify, and/or distribute this software for any\n"
               "// purpose with or without fee is hereby granted, provided that the above\n"
               "// copyright notice and this permission notice appear in all copies.\n")
        self.assertEqual(rust_licenses.text_licences(isc), {"ISC"})

    def test_texts_print_without_trailing_space_or_surrounding_blank_lines(self):
        self.assertEqual(rust_licenses.normalise("\r\n\nMIT  \r\nline\t\n\n"), "MIT\nline")


class Choice(unittest.TestCase):
    def test_the_first_allowlisted_alternative_with_its_texts_is_used(self):
        texts = {"LICENSE-APACHE": APACHE, "LICENSE-MIT": MIT}
        self.assertEqual(rust_licenses.choose("Apache-2.0 OR MIT", texts), (["MIT"], None))
        self.assertEqual(rust_licenses.choose("MIT OR Apache-2.0", {"LICENSE": APACHE}), (["Apache-2.0"], None),
                         "only Apache's text: used under Apache-2.0")
        self.assertEqual(
            rust_licenses.choose("(MIT OR Apache-2.0) AND Unicode-3.0", {"LICENSE-MIT": MIT, "LICENSE-UNICODE": UNICODE}),
            (["MIT", "Unicode-3.0"], None))

    def test_a_licence_outside_the_allowlist_fails(self):
        for expr in ["GPL-3.0-only", "AGPL-3.0-or-later", "LGPL-2.1-or-later", "GPL-2.0-only AND MIT",
                     "SSPL-1.0"]:
            used, problem = rust_licenses.choose(expr, {"LICENSE": MIT})
            self.assertIsNone(used, expr)
            self.assertIn("outside the allowlist", problem)
        self.assertEqual(rust_licenses.choose("GPL-3.0-only OR MIT", {"LICENSE": MIT}), (["MIT"], None),
                         "a choice that avoids the GPL is fine")

    def test_a_licence_without_its_text_fails(self):
        used, problem = rust_licenses.choose("MIT", {"COPYRIGHT": "Licensed under the MIT license."})
        self.assertIsNone(used)
        self.assertIn("no text of MIT (COPYRIGHT)", problem)
        used, problem = rust_licenses.choose("MIT AND BSD-3-Clause", {"LICENSE-MIT": MIT})
        self.assertIsNone(used, "AND needs every text")
        self.assertIn("no text of", problem)


class Files(unittest.TestCase):
    def test_a_crates_licence_files_are_its_top_level_reuse_and_license_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            pkg = Path(tmp)
            (pkg / "LICENSE-MIT").write_text(MIT + "\n")
            (pkg / "COPYING").write_text("see LICENSE-MIT")
            (pkg / "README.md").write_text("not a licence")
            (pkg / "LICENSES").mkdir()
            (pkg / "LICENSES" / "Apache-2.0.txt").write_text(APACHE)
            (pkg / "legal").mkdir()
            (pkg / "legal" / "terms.txt").write_text("custom terms")
            files = rust_licenses.licence_files(str(pkg), "legal/terms.txt")
            self.assertEqual(sorted(files), ["COPYING", "LICENSE-MIT", "LICENSES/Apache-2.0.txt", "legal/terms.txt"])
            self.assertEqual(files["LICENSE-MIT"], MIT)


class Notice(unittest.TestCase):
    def test_the_notice_lists_each_crate_and_each_text_once(self):
        crate = {"own": False, "repository": None}
        crates = [
            {**crate, "name": "alpha", "version": "1.0.0", "declared": "MIT OR Apache-2.0", "used": ["MIT"],
             "source": "https://crates.io/crates/alpha/1.0.0", "files": {"LICENSE-APACHE": APACHE, "LICENSE-MIT": MIT}},
            {**crate, "name": "beta", "version": "2.1.0", "declared": "MIT", "used": ["MIT"],
             "source": "https://crates.io/crates/beta/2.1.0", "repository": "https://example.org/beta",
             "files": {"LICENSE": MIT}},
            {**crate, "name": "slate", "version": "0.3.0", "declared": "MIT OR Apache-2.0", "used": ["MIT"],
             "own": True, "source": "https://github.com/avdrav1/cronkite", "files": {"LICENSE": "own MIT"}},
        ]
        text = rust_licenses.render("Slate", "0.3.0", "aarch64-apple-darwin", crates)
        self.assertTrue(text.startswith("Rust crates in Slate 0.3.0 (aarch64-apple-darwin)\n"))
        self.assertIn("Slate is built from the 3 Rust crates below", text)
        self.assertIn("1 of the crates are Slate's own and the Cronkite suite's (slate)", text)
        self.assertIn("alpha 1.0.0: MIT OR Apache-2.0, used under MIT\n"
                      "  Source:   https://crates.io/crates/alpha/1.0.0\n"
                      "  Licences: LICENSE-APACHE [1], LICENSE-MIT [2]\n", text)
        self.assertIn("beta 2.1.0: MIT\n  Source:   https://crates.io/crates/beta/2.1.0\n"
                      "  Project:  https://example.org/beta\n  Licences: LICENSE [2]\n", text)
        self.assertEqual(text.count(MIT), 1, "an identical text is printed once")
        self.assertIn("[2] LICENSE-MIT of alpha 1.0.0\nAlso: beta 2.1.0 (LICENSE)\n", text)
        self.assertIn("[3] LICENSE of slate 0.3.0", text)


if __name__ == "__main__":
    unittest.main()
