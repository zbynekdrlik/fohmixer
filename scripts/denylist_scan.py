#!/usr/bin/env python3
"""Scan git content for private site data (spec §5.2; ported from iemmixer's
`scripts/denylist_scan.py`).

The denylist (one term per line, `#` comments) is private: a mode-600 file
outside the repo for local runs, the DENYLIST secret in CI. Output never
contains a term, a matched line or an email address — only locations and the
entry number.

Commit mode scans each commit's author/committer names and emails together
with its message and added lines; with `--identities FILE` it also rejects
every commit whose author or committer email is not listed there.

`--boundary FILE` lists the tips of the kept legacy history (#3: the first
commits keep their original identity and history is never rewritten): their
ancestors are left out of every commit scan. With `--identities`, a boundary
that would hide a commit whose author and committer are both allowed is itself
a finding, so the boundary cannot be moved forward over new commits.

Matching is case-insensitive. A term that starts (ends) with a letter or digit
must not be preceded (followed) by one, where letters include diacritics and
`_` is a separator: `kit` does not hit `kitten`, `x_kit_y` is a hit, and a
term ending in `.` such as `10.0.` hits `10.0.0.5`.
"""
from __future__ import annotations

import argparse
import hashlib
import re
import shlex
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

EXIT_CLEAN = 0
EXIT_HIT = 1
EXIT_USAGE = 2

FULL_SHA = re.compile(r"^[0-9a-f]{40}$")


class UsageError(Exception):
    """A bad input file or argument: exit 2, the message never quotes a term."""


@dataclass(frozen=True)
class Hit:
    where: str
    entry: int

    def render(self) -> str:
        return f"{self.where}: denylist entry {self.entry}"


@dataclass(frozen=True)
class IdentityProblem:
    where: str
    role: str

    def render(self) -> str:
        return f"{self.where}: {self.role} email is not an allowed identity"


@dataclass(frozen=True)
class BoundaryProblem:
    where: str

    def render(self) -> str:
        return f"{self.where}: hidden by the boundary, but its author and committer are allowed identities"


Finding = Hit | IdentityProblem | BoundaryProblem


def meaningful_lines(path: Path) -> list[str]:
    stripped = (raw.strip() for raw in path.read_text(encoding="utf-8").splitlines())
    return [line for line in stripped if line and not line.startswith("#")]


def load_identities(path: Path | None) -> set[str] | None:
    if path is None:
        return None
    return {line.lower() for line in meaningful_lines(path)}


def load_terms(path: Path) -> list[str]:
    return meaningful_lines(path)


def load_boundary(path: Path | None) -> list[str]:
    if path is None:
        return []
    shas: list[str] = []
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if not FULL_SHA.match(line):
            # the line itself is never printed: a wrong file passed here could be the denylist
            raise UsageError(f"boundary {path}: line {number} is not a full commit SHA")
        shas.append(line)
    if not shas:
        raise UsageError(f"boundary {path} lists no commit")
    return shas


def compile_term(term: str) -> re.Pattern[str]:
    # a `\t`, `\n` or `\r` string escape right before the term is a boundary too
    left = r"(?:(?<![^\W_])|(?<=\\[ntr]))" if term[:1].isalnum() else ""
    right = r"(?![^\W_])" if term[-1:].isalnum() else ""
    return re.compile(left + re.escape(term) + right, re.IGNORECASE)


def line_key(path: str, line: str) -> str:
    return hashlib.sha256(f"{path}\n{line}".encode("utf-8")).hexdigest()


def load_allow(path: Path | None) -> set[str]:
    if path is None:
        return set()
    if not path.exists():
        raise UsageError(f"allow file {path} does not exist")
    return {line.split()[0] for line in meaningful_lines(path)}


class Scanner:
    def __init__(self, terms: list[str], allow: set[str]) -> None:
        self.patterns = [compile_term(term) for term in terms]
        self.allow = allow

    def entries_in(self, text: str) -> list[int]:
        return [number for number, pattern in enumerate(self.patterns, start=1) if pattern.search(text)]

    def scan_path(self, path: str, prefix: str) -> list[Hit]:
        return [Hit(f"{prefix}{path}: path", entry) for entry in self.entries_in(path)]

    def scan_line(self, path: str, line: str, where: str) -> list[Hit]:
        entries = self.entries_in(line)
        if not entries or line_key(path, line) in self.allow:
            return []
        return [Hit(where, entry) for entry in entries]


def git(repo: Path, *args: str) -> bytes:
    return subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True).stdout


def decode(data: bytes) -> str:
    return data.decode("utf-8", errors="replace")


def check_boundary_exists(repo: Path, boundary: list[str]) -> None:
    for sha in boundary:
        found = subprocess.run(["git", "-C", str(repo), "cat-file", "-e", f"{sha}^{{commit}}"],
                               capture_output=True)
        if found.returncode != 0:
            raise UsageError(f"boundary commit {sha} is not in {repo} (a shallow clone needs fetch-depth: 0)")


def emails_of(repo: Path, sha: str) -> list[str]:
    return [email.strip().lower() for email in decode(git(repo, "show", "-s", "--format=%ae%n%ce", sha)).splitlines()]


def scan_tree(scanner: Scanner, repo: Path, rev: str) -> list[Hit]:
    hits: list[Hit] = []
    for entry in git(repo, "ls-tree", "-r", "-z", "--full-tree", rev).split(b"\0"):
        if not entry:
            continue
        meta, _, raw_path = entry.partition(b"\t")
        _mode, kind, obj = meta.split()
        path = decode(raw_path)
        hits += scanner.scan_path(path, "")
        if kind != b"blob":
            continue
        data = git(repo, "cat-file", "blob", decode(obj))
        if b"\0" in data:
            continue
        for number, line in enumerate(decode(data).splitlines(), start=1):
            hits += scanner.scan_line(path, line, f"{path}:{number}")
    return hits


def scan_commits(
    scanner: Scanner, repo: Path, revlist_args: list[str], identities: set[str] | None = None,
    boundary: list[str] | None = None,
) -> list[Finding]:
    hits: list[Finding] = []
    # the boundary goes first, so a `--not` inside the caller's range cannot flip it
    excluded = [f"^{sha}" for sha in boundary or []]
    for sha in decode(git(repo, "rev-list", *excluded, *revlist_args)).split():
        short = sha[:12]
        metadata = decode(git(repo, "show", "-s", "--format=%an%n%ae%n%cn%n%ce%n%B", sha))
        hits += [Hit(f"{short} commit metadata", entry) for entry in scanner.entries_in(metadata)]
        if identities is not None:
            for role, email in zip(("author", "committer"), emails_of(repo, sha), strict=True):
                if email not in identities:
                    hits.append(IdentityProblem(short, role))
        diff = decode(git(repo, "show", "--format=", "--unified=0", "--no-color", "--no-ext-diff",
                          "--no-renames", "-m", "--first-parent", sha))
        path = ""
        for line in diff.splitlines():
            if line.startswith("+++ "):
                target = line[4:]
                path = target[2:] if target.startswith("b/") else target
                hits += scanner.scan_path(path, f"{short} ")
            elif line.startswith("+"):
                hits += scanner.scan_line(path, line[1:], f"{short} {path}")
    return hits


def scan_boundary(repo: Path, boundary: list[str], identities: set[str]) -> list[BoundaryProblem]:
    """Every commit the boundary hides must be legacy: at least one identity not allowed."""
    problems: list[BoundaryProblem] = []
    for sha in decode(git(repo, "rev-list", *boundary)).split():
        if all(email in identities for email in emails_of(repo, sha)):
            problems.append(BoundaryProblem(sha[:12]))
    return problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Scan git content for private site data.")
    parser.add_argument("--denylist", type=Path)
    parser.add_argument("--allow", type=Path, help="reviewed line keys (see --hash)")
    parser.add_argument("--identities", type=Path, help="allowed author/committer emails (commit mode)")
    parser.add_argument("--boundary", type=Path, help="tips of the kept legacy history, left out of commit scans")
    parser.add_argument("--repo", type=Path, default=Path("."))
    parser.add_argument("--tree", action="append", default=[], metavar="REV")
    parser.add_argument("--commits", action="append", default=[], metavar="REVLIST")
    parser.add_argument("--hash", nargs=2, metavar=("PATH", "LINE"))
    args = parser.parse_args(argv)

    if args.hash:
        path, number = args.hash
        lines = (args.repo / path).read_text(encoding="utf-8").splitlines()
        print(line_key(path, lines[int(number) - 1]))
        return EXIT_CLEAN
    if args.denylist is None or not (args.tree or args.commits):
        parser.error("--denylist and at least one --tree or --commits are required")

    try:
        terms = load_terms(args.denylist)
        if not terms:
            raise UsageError(f"denylist {args.denylist} has no terms")
        identities = load_identities(args.identities)
        if identities is not None and not identities:
            raise UsageError(f"identity list {args.identities} is empty")
        boundary = load_boundary(args.boundary)
        check_boundary_exists(args.repo, boundary)
        scanner = Scanner(terms, load_allow(args.allow))
    except UsageError as error:
        print(error, file=sys.stderr)
        return EXIT_USAGE

    hits: list[Finding] = []
    for rev in args.tree:
        hits += scan_tree(scanner, args.repo, rev)
    for spec in args.commits:
        hits += scan_commits(scanner, args.repo, shlex.split(spec), identities, boundary)
    if boundary and identities is not None:
        hits += scan_boundary(args.repo, boundary, identities)
    for hit in hits:
        print(hit.render())
    if hits:
        print(f"{len(hits)} finding(s)", file=sys.stderr)
        return EXIT_HIT
    print("denylist: clean")
    return EXIT_CLEAN


if __name__ == "__main__":
    sys.exit(main())
