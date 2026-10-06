#!/usr/bin/env python3
"""Integrity gate (adapted from iemmixer @ 22372bc): no ignored, skipped or
focused tests (Rust, Playwright, Python), no continue-on-error, self-hosted
runners or pull_request_target, every action pinned to a full commit SHA with
its version comment, no force-kill verb anywhere, comments included
(spec I7: nothing is force-killed on the Ableton PC), and every tap target
of the surface owns its touches and none takes a mouse event (#43 PR G)."""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SELF = {"scripts/check_integrity.py", "scripts/test_check_integrity.py"}
RUST_IGNORE = re.compile(r"#\[\s*ignore")
E2E_SKIP = re.compile(r"\b(?:test|it|describe)(?:\.describe)?\.(?:skip|only|fixme)\s*\(|\btest\.fail\s*\(|function assume\(")
# unittest's skip decorators (qualified or imported bare), skipTest, expected
# failures, and pytest's skip/xfail.
PY_SKIP = re.compile(
    r"@(?:unittest\.)?skip(?:If|Unless)?\b|\bunittest\.skip|\.skipTest\s*\(|\bexpectedFailure\b"
    r"|\bpytest\.mark\.(?:skip|skipif|xfail)\b|\bpytest\.skip\s*\(")
WORKFLOW_FORBIDDEN = re.compile(r"continue-on-error|self-hosted|pull_request_target")
USES = re.compile(r"^\s*-?\s*uses:\s*(\S+)(.*)$")
PINNED = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")
# The pin's release, e.g. `# v7.0.1`.
VERSION_COMMENT = re.compile(r"^\s+#\s*v\d+(?:\.\d+)*\s*$")
# Force-end verbs (I7): Windows' own (taskkill, tskill, Sysinternals pskill),
# PowerShell's, WMI/CIM's Terminate (-MethodName or its alias -Name; wmic's
# call terminate and delete), a job whose closing ends its processes, and the
# Rust/tokio/Python/.NET process handles' kill methods (called, or named in
# ForEach-Object). A request plus a bounded wait is the only stop.
FORCE_KILL = re.compile(
    r"(?i)\btaskkill\b|\btskill\b|\bpskill\b|terminateprocess|terminatejobobject|kill_on_job_close|stop-process"
    r"|\bshutdown(?:\.exe)?\s+/f\b|\.kill\s*\(|\bstart_kill\b|\bkill_on_drop\b|\.terminate\s*\("
    r"|-(?:method)?name\s+['\"]?terminate\b|\bwmic\b.*\b(?:call\s+terminate|delete)\b"
    r"|(?:\bforeach-object|%)\s+(?:-membername\s+)?['\"]?kill\b")
# Every element of a fohmixer-ui component or page that takes a pointerdown
# carries `use:owns_touches` (#43 PR G): only an active touchstart listener
# that prevents it stops WebKit's loupe of a tap followed by a hold. The login
# page is no surface.
TAP_TREES = ("crates/fohmixer-ui/src/components", "crates/fohmixer-ui/src/pages")
TAP_EXEMPT = {"crates/fohmixer-ui/src/pages/login.rs"}
TAP = re.compile(r"\bon:pointerdown\b")
# Nor does one take a mouse event: an owned ancestor's prevented touchstart
# swallows every compatibility mouse event of a touch on the iPad (the click
# too); Pointer Events are the input path.
MOUSE = re.compile(r"\bon:((?:dbl)?click|mouse(?:down|up|move|over|out|enter|leave))\b")
TAG_START = re.compile(r"<[a-z][a-z0-9-]*\b")
# A start tag's end: a `>` that is no closure's `=>` and no `->`.
TAG_END = re.compile(r"(?<![=-])>")
CODE_SUFFIXES = (".rs", ".ts", ".js", ".py", ".sh", ".ps1", ".psm1", ".psd1", ".cmd", ".bat", ".yml", ".yaml", ".toml")
PYTHON_TREES = ("live-script", "sim", "scripts", "tools")
CODE_TREES = ("crates", "e2e", "scripts", ".github", "live-script", "sim", "tools")


def files(root: Path, base: str, suffixes: tuple[str, ...]) -> list[Path]:
    top = root / base
    if not top.is_dir():
        return []
    return sorted(p for p in top.rglob("*") if p.is_file() and p.suffix in suffixes and "node_modules" not in p.parts)


def lines(path: Path) -> list[tuple[int, str]]:
    return list(enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines(), start=1))


def unowned_taps(text: str) -> list[int]:
    """The lines of `text` (a Rust source's `view!` markup) where an element
    takes `on:pointerdown` without `use:owns_touches` in its start tag."""
    found = []
    for tap in TAP.finditer(text):
        starts = list(TAG_START.finditer(text, 0, tap.start()))
        end = TAG_END.search(text, tap.end())
        if not starts or end is None:
            continue
        if "use:owns_touches" not in text[starts[-1].start():end.start()]:
            found.append(text.count("\n", 0, tap.start()) + 1)
    return found


def violations(root: Path) -> list[str]:
    found: list[str] = []
    for path in files(root, "crates", (".rs",)):
        rel = path.relative_to(root).as_posix()
        found += [f"{rel}:{n}: #[ignore] test" for n, line in lines(path) if RUST_IGNORE.search(line)]
    for base in TAP_TREES:
        for path in files(root, base, (".rs",)):
            rel = path.relative_to(root).as_posix()
            if rel in TAP_EXEMPT:
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            found += [f"{rel}:{n}: tap target without use:owns_touches (#43)" for n in unowned_taps(text)]
            found += [
                f"{rel}:{n}: on:{event.group(1)} on the surface: a prevented touchstart swallows it (#43)"
                for n, line in lines(path)
                for event in MOUSE.finditer(line)
            ]
    for path in files(root, "e2e", (".ts",)):
        rel = path.relative_to(root).as_posix()
        found += [f"{rel}:{n}: skipped or focused E2E test" for n, line in lines(path) if E2E_SKIP.search(line)]
    for base in PYTHON_TREES:
        for path in files(root, base, (".py",)):
            rel = path.relative_to(root).as_posix()
            if rel in SELF:
                continue
            found += [f"{rel}:{n}: skipped Python test" for n, line in lines(path) if PY_SKIP.search(line)]
    for path in files(root, ".github/workflows", (".yml", ".yaml")):
        rel = path.relative_to(root).as_posix()
        for n, line in lines(path):
            if WORKFLOW_FORBIDDEN.search(line):
                found.append(f"{rel}:{n}: forbidden workflow construct")
            match = USES.match(line)
            if match and not match.group(1).startswith("./"):
                if not PINNED.match(match.group(1)):
                    found.append(f"{rel}:{n}: action not pinned to a full commit SHA: {match.group(1)}")
                elif not VERSION_COMMENT.match(match.group(2)):
                    found.append(f"{rel}:{n}: pinned action without its version comment (# vX.Y.Z): {match.group(1)}")
    for base in CODE_TREES:
        for path in files(root, base, CODE_SUFFIXES):
            rel = path.relative_to(root).as_posix()
            if rel in SELF:
                continue
            found += [f"{rel}:{n}: force-kill command (spec I7)" for n, line in lines(path) if FORCE_KILL.search(line)]
    return sorted(found, key=_order)


def _order(item: str) -> tuple[str, int]:
    """Path, then line number, so the report reads top to bottom."""
    path, number, _ = item.split(":", 2)
    return path, int(number)


def main() -> int:
    found = violations(ROOT)
    for item in found:
        print(f"::error::{item}")
    if found:
        return 1
    print("integrity: clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
