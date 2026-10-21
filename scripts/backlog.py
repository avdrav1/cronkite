#!/usr/bin/env python3
"""Read and update BACKLOG.md.

  backlog.py next [APP]        print the next story to build (JSON); APP is brick, slate,
                               frame, etch, or suite (work across the apps; default: each,
                               in that order)
  backlog.py show ID           print one story (JSON)
  backlog.py set ID STATUS     set a story's status
  backlog.py status            milestone progress summary
  backlog.py gate MS           check a milestone's gate box (human-approved only)
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BACKLOG = ROOT / "BACKLOG.md"
# Each app's milestones, in order: a milestone unlocks when the previous one of the same
# app has its gate checked. `suite` is work across the apps (XP: other platforms).
CHAINS = {
    "brick": ["S0", "M0", "M1", "M2", "M3", "M4", "M5"],
    "slate": ["SL0", "SL1", "SL3", "SL2", "SL4"],
    "frame": ["FR0", "FR1"],
    "etch": ["ET0", "ET1", "ET2", "ET3"],
    "suite": ["XP0", "XP1", "XP2", "UP0"],
}
MILESTONES = [ms for chain in CHAINS.values() for ms in chain]
STATUSES = {"todo", "in-progress", "review", "done", "blocked"}
COLS = ["id", "ms", "pri", "status", "depends", "story", "acceptance"]


def load():
    text = BACKLOG.read_text()
    stories, gates = [], {}
    for line in text.splitlines():
        m = re.match(r"- \[( |x)\] (S0|M\d+|SL\d+|FR\d+|ET\d+|XP\d+):", line)
        if m:
            gates[m.group(2)] = m.group(1) == "x"
            continue
        if line.startswith("| ") and not line.startswith("| ID") and not line.startswith("| ---"):
            # Markdown escapes a pipe inside a cell as `\|`; only bare pipes split cells.
            cells = [c.strip().replace("\\|", "|") for c in re.split(r"(?<!\\)\|", line.strip().strip("|"))]
            if len(cells) == len(COLS):
                s = dict(zip(COLS, cells))
                s["depends"] = [] if s["depends"] in ("-", "") else [d.strip() for d in s["depends"].split(",")]
                stories.append(s)
    return text, stories, gates


def chain_of(ms):
    return next(chain for chain in CHAINS.values() if ms in chain)


def unlocked(ms, gates):
    chain = chain_of(ms)
    i = chain.index(ms)
    return i == 0 or gates.get(chain[i - 1], False)


def next_story(app=None):
    if app is None:
        for name in CHAINS:
            got = next_story(name)
            if got["action"] != "complete":
                return got
        return {"action": "complete", "message": "Every non-P2 story is done."}
    chain = CHAINS[app]
    _, stories, gates = load()
    done = {s["id"] for s in stories if s["status"] == "done"}
    for status in ("in-progress", "review"):
        for s in stories:
            if s["status"] == status and s["ms"] in chain:
                return {"action": "resume", **s}
    for ms in chain:
        open_in_ms = [s for s in stories if s["ms"] == ms and s["status"] != "done" and s["pri"] != "P2"]
        if not open_in_ms:
            if not gates.get(ms, False):
                return {"action": "gate", "milestone": ms,
                        "message": f"All {ms} stories are done. Verify the {ms} gate and ask the user to approve it."}
            continue
        if not unlocked(ms, gates):
            prev = chain[chain.index(ms) - 1]
            return {"action": "gate", "milestone": prev,
                    "message": f"{prev} gate is unchecked; {ms} is locked."}
        for pri in ("P0", "P1"):
            for s in open_in_ms:
                if s["pri"] == pri and s["status"] == "todo" and all(d in done for d in s["depends"]):
                    return {"action": "start", **s}
        blocked = [{"id": s["id"], "waiting_on": [d for d in s["depends"] if d not in done]} for s in open_in_ms]
        return {"action": "stuck", "milestone": ms, "blocked": blocked}
    return {"action": "complete", "message": f"Every non-P2 {app} story is done."}


def set_status(sid, status):
    if status not in STATUSES:
        sys.exit(f"status must be one of {sorted(STATUSES)}")
    text, _, _ = load()
    pat = re.compile(rf"^(\| {re.escape(sid)} \| [^|]+ \| [^|]+ \| )([a-z-]+)( \|)", re.M)
    new, n = pat.subn(rf"\g<1>{status}\g<3>", text)
    if n != 1:
        sys.exit(f"story {sid} not found")
    BACKLOG.write_text(new)
    print(f"{sid} -> {status}")


def check_gate(ms):
    text, _, _ = load()
    new, n = re.subn(rf"^- \[ \] {ms}:", f"- [x] {ms}:", text, flags=re.M)
    if n != 1:
        sys.exit(f"gate {ms} not found or already checked")
    BACKLOG.write_text(new)
    print(f"gate {ms} checked")


def summary():
    _, stories, gates = load()
    for ms in MILESTONES:
        mine = [s for s in stories if s["ms"] == ms]
        d = sum(s["status"] == "done" for s in mine)
        print(f"{ms}: {d}/{len(mine)} done, gate {'passed' if gates.get(ms) else 'open'}")


if __name__ == "__main__":
    a = sys.argv[1:]
    if not a or (a[0] == "next" and len(a) == 1):
        print(json.dumps(next_story(), indent=2))
    elif a[0] == "next" and len(a) == 2 and a[1] in CHAINS:
        print(json.dumps(next_story(a[1]), indent=2))
    elif a[0] == "show" and len(a) == 2:
        _, stories, _ = load()
        hit = [s for s in stories if s["id"] == a[1]]
        print(json.dumps(hit[0], indent=2) if hit else f"no story {a[1]}")
    elif a[0] == "set" and len(a) == 3:
        set_status(a[1], a[2])
    elif a[0] == "gate" and len(a) == 2:
        check_gate(a[1])
    elif a[0] == "status":
        summary()
    else:
        print(__doc__)
