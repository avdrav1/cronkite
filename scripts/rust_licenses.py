#!/usr/bin/env python3
"""Licence notices for the Rust crates compiled into an app's release (PKG-6).

    python3 scripts/rust_licenses.py APP TARGET OUT

Writes OUT (THIRD-PARTY-RUST.txt) for app APP (brick, slate, frame, etch) built for TARGET
(x86_64-unknown-linux-gnu for the Linux tarball, aarch64-apple-darwin for the macOS
bundle). `scripts/ship.sh APP build` runs it for the tarball, scripts/mac_bundle.sh for
the bundle. It reads only Cargo.lock's dependency graph and the crates' sources (cargo
downloads any it lacks), so it runs on any host, for any target.

The crates are those `cargo tree -p APP --target TARGET -e normal` lists, the graph of
`cargo build --release -p APP` for that target with its features: every library compiled
into the binary and the procedural macros whose expansions are (serde_derive, glib-macros),
with their own dependencies (syn, quote). Build dependencies (build scripts' crates:
system-deps, pkg-config) and dev-dependencies leave no code in the binary and are not
listed. The suite's own crates are listed too, under the repository's LICENSE files.

For each crate the notice gives its version, the SPDX licence it declares, the licence it
is used under here, where its source is, and the texts of every licence file it ships
(LICENSE*, LICENCE*, COPYING*, COPYRIGHT*, NOTICE*, UNLICENSE, LICENSES/*, and its
`license-file`); the texts follow the list, each identical text once, with the crates that
carry it.

It fails, printing every problem, when a crate declares no licence, can't be used under
the allowlist below (no GPL, AGPL, or LGPL: a statically linked Rust crate under the LGPL
needs a review first), or ships no text of the licence it is used under. Of a choice (OR)
it takes the first allowlisted alternative whose texts are all there, by ALLOWED's order.
It prints the crate count and how many crates use each licence.
"""

import json
import os
import re
import subprocess
import sys
import textwrap

# The licences a crate may be used under, in order of preference when it offers a choice.
# MPL-2.0 is file-level copyleft: fine to ship with this notice, which says where each
# crate's source is.
ALLOWED = [
    "MIT",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-3-Clause",
    "BSD-2-Clause",
    "ISC",
    "Zlib",
    "0BSD",
    "MIT-0",
    "Unicode-3.0",
    "Unicode-DFS-2016",
    "BSL-1.0",
    "CC0-1.0",
    "Unlicense",
    "MPL-2.0",
]

# How each allowlisted licence's text reads: every pattern must be found in a file, matched
# on its lower-cased text with whitespace collapsed.
TEXT = {
    "MIT": [r"permission is hereby granted, free of charge, to any person obtaining a copy"],
    "MIT-0": [r"permission is hereby granted, free of charge, to any person obtaining a copy"],
    "Apache-2.0": [r"apache license", r"terms and conditions for use, reproduction, and distribution"],
    "BSD-3-Clause": [r"redistribution and use in source and binary forms",
                     r"neither the name|names of its contributors may not be used|may not be used to endorse"],
    "BSD-2-Clause": [r"redistribution and use in source and binary forms"],
    "ISC": [r"permission to use, copy, modify, and(/or)? distribute this software for any purpose "
            r"with or without fee is hereby granted, provided that"],
    "0BSD": [r"permission to use, copy, modify, and/or distribute this software for any purpose "
             r"with or without fee is hereby granted\."],
    "Zlib": [r"provided ['‘`]as-is['’], without any express or implied warranty",
             r"altered source versions must be plainly marked"],
    "Unicode-3.0": [r"unicode", r"data files"],
    "Unicode-DFS-2016": [r"unicode", r"data files"],
    "BSL-1.0": [r"boost software license"],
    "CC0-1.0": [r"cc0 1\.0|creative commons zero|creativecommons\.org/publicdomain/zero"],
    "Unlicense": [r"this is free and unencumbered software released into the public domain"],
    "MPL-2.0": [r"mozilla public license,? version 2\.0"],
}

LICENCE_FILE = re.compile(r"^(licen[cs]e|copying|copyright|notice|unlicense)", re.IGNORECASE)

TITLES = {"brick": "Brick", "slate": "Slate", "frame": "Frame", "etch": "Etch"}

RULE = "=" * 80


def spdx_tokens(expr):
    # "MIT/Apache-2.0" is the old crates.io way of writing "MIT OR Apache-2.0".
    return re.findall(r"\(|\)|[^\s()]+", expr.replace("/", " OR "))


def alternatives(expr):
    """SPDX expression `expr` as the sets of licences that each satisfy it (OR of ANDs),
    `ID WITH EXCEPTION` as one licence. Raises ValueError on a malformed expression."""
    tokens = spdx_tokens(expr)
    pos = 0

    def peek():
        return tokens[pos] if pos < len(tokens) else None

    def take():
        nonlocal pos
        if pos >= len(tokens):
            raise ValueError(f"SPDX expression {expr!r} ends early")
        pos += 1
        return tokens[pos - 1]

    def atom():
        if peek() == "(":
            take()
            alts = any_of()
            if take() != ")":
                raise ValueError(f"unbalanced SPDX expression {expr!r}")
            return alts
        name = take()
        if name in ("AND", "OR", "WITH", ")"):
            raise ValueError(f"can't read SPDX expression {expr!r}")
        if peek() == "WITH":
            take()
            name = f"{name} WITH {take()}"
        return [frozenset([name])]

    def all_of():
        alts = atom()
        while peek() == "AND":
            take()
            right = atom()
            alts = [a | b for a in alts for b in right]
        return alts

    def any_of():
        alts = all_of()
        while peek() == "OR":
            take()
            alts = alts + [a for a in all_of() if a not in alts]
        return alts

    if not tokens:
        raise ValueError("empty SPDX expression")
    alts = any_of()
    if pos != len(tokens):
        raise ValueError(f"can't read SPDX expression {expr!r}")
    return alts


def text_licences(text):
    """The allowlisted licences whose text `text` is, also when it's written as source
    comments (`// Permission to use, …`, as untrusted's LICENSE.txt is)."""
    uncommented = "\n".join(re.sub(r"^\s*(?://+|#+|\*+|/\*+)", "", line) for line in text.splitlines())
    flat = " ".join(uncommented.lower().split())
    return {lic for lic, patterns in TEXT.items() if all(re.search(p, flat) for p in patterns)}


def base_licence(lic):
    """The licence whose text an `ID WITH EXCEPTION` needs: ID's."""
    return lic.split(" WITH ")[0]


def choose(expr, texts):
    """(licences used, problem): the first allowlisted alternative of `expr` (by ALLOWED's
    order) whose licences all have a text among `texts` (file name -> text)."""
    try:
        alts = alternatives(expr)
    except ValueError as e:
        return None, str(e)
    allowed = [a for a in alts if all(lic in ALLOWED for lic in a)]
    if not allowed:
        return None, f"{expr} is outside the allowlist"
    found = set()
    for text in texts.values():
        found |= text_licences(text)
    covered = [a for a in allowed if all(base_licence(lic) in found for lic in a)]
    if not covered:
        names = ", ".join(sorted(texts)) or "no licence files"
        return None, f"no text of {' or '.join(' AND '.join(sorted(a)) for a in allowed)} ({names})"
    best = min(covered, key=lambda a: sorted(ALLOWED.index(lic) for lic in a))
    return sorted(best, key=ALLOWED.index), None


def normalise(text):
    """A licence text as the notice prints it: LF line ends, no trailing spaces, no blank
    lines around it."""
    lines = [line.rstrip() for line in text.replace("\r\n", "\n").replace("\r", "\n").split("\n")]
    while lines and not lines[0]:
        lines.pop(0)
    while lines and not lines[-1]:
        lines.pop()
    return "\n".join(lines)


def licence_files(pkg_dir, license_file=None):
    """{relative path: text} of a crate's licence files: in its root, in LICENSES/ (REUSE),
    and its `license-file`."""
    paths = []
    for name in sorted(os.listdir(pkg_dir)):
        if LICENCE_FILE.match(name) and os.path.isfile(os.path.join(pkg_dir, name)):
            paths.append(name)
    reuse = os.path.join(pkg_dir, "LICENSES")
    if os.path.isdir(reuse):
        paths += [os.path.join("LICENSES", n) for n in sorted(os.listdir(reuse))
                  if os.path.isfile(os.path.join(reuse, n))]
    if license_file and os.path.normpath(license_file) not in paths:
        paths.append(os.path.normpath(license_file))
    texts = {}
    for rel in paths:
        with open(os.path.join(pkg_dir, rel), "rb") as f:
            texts[rel] = normalise(f.read().decode("utf-8", errors="replace"))
    return texts


def cargo(root, *args):
    return subprocess.run(["cargo", *args], cwd=root, capture_output=True, text=True, check=True).stdout


def compiled_crates(root, app, target):
    """(name, version) of every crate `cargo build --release -p APP --target TARGET`
    compiles into the binary: its normal dependencies, proc-macros included."""
    out = cargo(root, "tree", "--locked", "--quiet", "-p", app, "--target", target, "-e", "normal",
                "--prefix", "none", "--format", "{p}")
    crates = set()
    for line in out.splitlines():
        parts = line.split()
        if len(parts) >= 2:
            crates.add((parts[0], parts[1].lstrip("v")))
    return crates


def collect(root, app, target):
    """(app version, crates, problems). Each crate: dict(name, version, own, declared,
    used, source, files: {path: text})."""
    meta = json.loads(cargo(root, "metadata", "--locked", "--format-version", "1",
                            "--filter-platform", target))
    packages = {}
    for p in meta["packages"]:
        packages.setdefault((p["name"], p["version"]), []).append(p)
    workspace = meta["workspace_root"]
    repo_texts = licence_files(workspace)
    version = None
    crates, problems = [], []
    for name, ver in sorted(compiled_crates(root, app, target)):
        found = packages.get((name, ver), [])
        if len(found) != 1:
            problems.append(f"{name} {ver}: {len(found)} packages of that name and version in cargo metadata")
            continue
        p = found[0]
        own = p["source"] is None
        if name == app and own:
            version = ver
        files = licence_files(os.path.dirname(p["manifest_path"]), p.get("license_file"))
        if own and not files:
            files = dict(repo_texts)
        declared = p.get("license")
        if not declared:
            problems.append(f"{name} {ver}: declares no SPDX licence"
                            + (f" (only license-file {p['license_file']})" if p.get("license_file") else ""))
            continue
        if not files:
            problems.append(f"{name} {ver}: {declared}, but the crate has no licence file")
            continue
        used, problem = choose(declared, files)
        if problem:
            problems.append(f"{name} {ver}: {problem}")
            continue
        source = (f"https://crates.io/crates/{name}/{ver}" if p["source"].startswith("registry+")
                  else p["source"]) if not own else "https://github.com/avdrav1/cronkite"
        crates.append({"name": name, "version": ver, "own": own, "declared": declared, "used": used,
                       "source": source, "repository": p.get("repository"), "files": files})
    return version, crates, problems


def wrap(paragraph):
    return textwrap.fill(paragraph, width=88, break_on_hyphens=False, break_long_words=False)


def render(title, version, target, crates):
    """THIRD-PARTY-RUST.txt."""
    texts = {}  # text -> [(crate, file)]
    for c in crates:
        for f, t in c["files"].items():
            texts.setdefault(t, []).append((f"{c['name']} {c['version']}", f))
    order = sorted(texts, key=lambda t: texts[t][0])
    number = {t: i + 1 for i, t in enumerate(order)}
    own = [c for c in crates if c["own"]]
    intro = (
        f"{title} is built from the {len(crates)} Rust crates below, at the versions in its Cargo.lock: "
        "every crate compiled into the program for this platform, including the procedural macros "
        "that generate some of its code. Each entry gives the crate's licence as it declares it, the "
        "licence it is used under here, its source, and its licence files, whose texts follow the "
        "list, each text once, numbered."
    )
    ours = (
        f"{len(own)} of the crates are {title}'s own and the Cronkite suite's "
        f"({', '.join(c['name'] for c in own)}), free software under the MIT or Apache-2.0 licence."
    )
    out = [f"Rust crates in {title} {version} ({target})", "", wrap(intro), "", wrap(ours), ""]
    for c in crates:
        used = " AND ".join(c["used"])
        out.append(f"{c['name']} {c['version']}: {c['declared']}"
                   + ("" if used == c["declared"] else f", used under {used}"))
        out.append(f"  Source:   {c['source']}")
        if c["repository"] and not c["own"]:
            out.append(f"  Project:  {c['repository']}")
        out.append("  Licences: " + ", ".join(f"{f} [{number[t]}]" for f, t in c["files"].items()))
        out.append("")
    for t in order:
        users = texts[t]
        out.append(RULE)
        out.append(f"[{number[t]}] {users[0][1]} of {users[0][0]}")
        if len(users) > 1:
            out.append(wrap("Also: " + ", ".join(f"{u} ({f})" for u, f in users[1:])))
        out.append(RULE)
        out.append("")
        out.append(t)
        out.append("")
    return "\n".join(out)


def main(app, target, path):
    if app not in TITLES:
        raise SystemExit(f"rust_licenses: unknown app {app}")
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    version, crates, problems = collect(root, app, target)
    if version is None:
        problems.append(f"{app}: not among its own crates")
    if problems:
        print("rust_licenses: " + "\n  ".join([f"{app} for {target}: no notice, or a licence the release can't carry:",
                                               *problems]), file=sys.stderr)
        return 1
    with open(path, "w") as out:
        out.write(render(TITLES[app], version, target, crates))
    counts = {}
    for c in crates:
        key = " AND ".join(c["used"])
        counts[key] = counts.get(key, 0) + 1
    third = sum(1 for c in crates if not c["own"])
    print(f"rust crates in {app} {version} for {target}: {len(crates)} ({third} third-party, "
          f"{len(crates) - third} the suite's own); used under "
          + ", ".join(f"{k} {v}" for k, v in sorted(counts.items(), key=lambda kv: (-kv[1], kv[0]))))
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 4:
        raise SystemExit(__doc__.split("\n\n")[1])
    sys.exit(main(*sys.argv[1:]))
