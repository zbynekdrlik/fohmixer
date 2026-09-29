#!/usr/bin/env python3
"""Scan git content for private site data (spec §5.2; ported from iemmixer's
`scripts/denylist_scan.py`).

The denylist (one term per line, `#` comments) is private: a mode-600 file
outside the repo for local runs, the DENYLIST secret in CI. Output never
contains a term, a matched line or an email address — only locations and the
entry number. A path component that holds a term is printed as `[redacted]`
(the whole path when a term spans components).

Commit mode scans each commit's author/committer names and emails together
with its message, the paths it adds or changes and its added lines; with
`--identities FILE` it also rejects every commit whose author or committer
email is not listed there.

`--boundary FILE` lists the tips of the kept legacy history (#3: the first
commits keep their original identity and history is never rewritten): their
ancestors are left out of every commit scan. With `--identities`, a boundary
that would hide a commit with any allowed identity is itself a finding, so the
boundary cannot be moved forward over new commits.

Matching is case-insensitive. A term that starts (ends) with a letter or digit
must not be preceded (followed) by one, where letters include diacritics and
`_` is a separator: `kit` does not hit `kitten`, `x_kit_y` is a hit, and a
term ending in `.` such as `10.0.` hits `10.0.0.5`. Text is split into lines
at `\\n` only (a trailing `\\r` dropped), the same in tree and commit mode, so a
line key (`--hash`) is the same in both.

Exit codes: 0 clean, 1 findings, 2 the scan could not run (a missing or
unreadable input file, a bad boundary, a failing git command).
"""
from __future__ import annotations

import argparse
import codecs
import hashlib
import re
import shlex
import subprocess
import sys
import unicodedata
from collections.abc import Iterator
from dataclasses import dataclass
from pathlib import Path

EXIT_CLEAN = 0
EXIT_HIT = 1
EXIT_USAGE = 2

FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
REDACTED = "[redacted]"
# git's C-quoting of a path in a diff header (core.quotePath)
C_ESCAPES = {"a": 7, "b": 8, "t": 9, "n": 10, "v": 11, "f": 12, "r": 13, '"': 34, "\\": 92}
UTF16_BOMS = (codecs.BOM_UTF16_LE, codecs.BOM_UTF16_BE)
# A commit's diff against its first parent (the root commit's too) whose labels, colours, attributes
# and paths depend neither on the user's git config nor on the repo's .gitattributes
DIFF = ["show", "--format=", "--no-show-signature", "--no-color", "--no-ext-diff", "--no-textconv", "--text",
        "--no-renames", "--no-relative", "--root", "-m", "--first-parent"]


class UsageError(Exception):
    """The scan cannot run: exit 2, the message never quotes a term."""


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
        return f"{self.where}: hidden by the boundary, but it has an allowed identity"


Finding = Hit | IdentityProblem | BoundaryProblem


def read_text(path: Path, what: str) -> str:
    try:
        # bytes, then UTF-8: no universal newlines, so a lone `\r` stays inside its line
        return path.read_bytes().decode("utf-8")
    except (OSError, UnicodeDecodeError) as error:
        # the error text is left out: a decode error quotes the bytes around it
        raise UsageError(f"{what} {path} cannot be read ({type(error).__name__})") from None


def meaningful_lines(path: Path, what: str) -> list[str]:
    stripped = (raw.strip() for raw in read_text(path, what).splitlines())
    return [line for line in stripped if line and not line.startswith("#")]


def load_identities(path: Path | None) -> set[str] | None:
    if path is None:
        return None
    return {line.lower() for line in meaningful_lines(path, "identity list")}


def load_terms(path: Path) -> list[str]:
    return meaningful_lines(path, "denylist")


def load_boundary(path: Path | None) -> list[str]:
    if path is None:
        return []
    shas: list[str] = []
    for number, raw in enumerate(read_text(path, "boundary").splitlines(), start=1):
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
    return {line.split()[0] for line in meaningful_lines(path, "allow file")}


def text_lines(text: str) -> list[str]:
    """Lines as git counts them: split at `\\n` only, one trailing `\\r` dropped."""
    lines = text.split("\n")
    if lines[-1] == "":
        lines.pop()
    return [line.removesuffix("\r") for line in lines]


def blob_text(data: bytes) -> str | None:
    """A blob's text: UTF-16 with its BOM, else UTF-8; None when it holds a NUL (binary)."""
    if data.startswith(UTF16_BOMS):
        return data.decode("utf-16", errors="replace")
    if b"\0" in data:
        return None
    return decode(data)


def nfc(text: str) -> str:
    """One form for letters with diacritics, so a decomposed `á` cannot hide a term."""
    return unicodedata.normalize("NFC", text)


class Scanner:
    def __init__(self, terms: list[str], allow: set[str]) -> None:
        self.patterns = [compile_term(nfc(term)) for term in terms]
        self.allow = allow

    def entries_in(self, text: str) -> list[int]:
        text = nfc(text)
        return [number for number, pattern in enumerate(self.patterns, start=1) if pattern.search(text)]

    def shown(self, path: str) -> str:
        """The path as printed: each component holding a term is redacted; the whole path when
        a term spans components (a term without `/` always matches inside one component)."""
        whole = set(self.entries_in(path))
        if not whole:
            return path
        parts = path.split("/")
        part_hits = [set(self.entries_in(part)) for part in parts]
        if not whole <= set().union(*part_hits):
            return REDACTED
        redacted = "/".join(REDACTED if hit else part for part, hit in zip(parts, part_hits, strict=True))
        return REDACTED if self.entries_in(redacted) else redacted

    def scan_path(self, path: str, prefix: str) -> list[Hit]:
        return [Hit(f"{prefix}{self.shown(path)}: path", entry) for entry in self.entries_in(path)]

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


def metadata_of(repo: Path, sha: str) -> str:
    return decode(git(repo, "show", "-s", "--no-show-signature", "--format=%an%n%ae%n%cn%n%ce%n%B", sha))


def emails_in(metadata: str) -> list[str]:
    """The author and committer emails: lines 2 and 4 of the metadata (git names hold no newline)."""
    fields = metadata.split("\n")
    return [fields[1].strip().lower(), fields[3].strip().lower()]


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
        text = blob_text(git(repo, "cat-file", "blob", decode(obj)))
        if text is None:
            continue
        shown = scanner.shown(path)
        for number, line in enumerate(text_lines(text), start=1):
            hits += scanner.scan_line(path, line, f"{shown}:{number}")
    return hits


def unquote_c(quoted: str) -> str:
    """A path git wrote C-quoted (`"b/Kl\\303\\241vor"`) back to text."""
    body, out, i = quoted[1:-1], bytearray(), 0
    while i < len(body):
        if body[i] != "\\":
            out += body[i].encode("utf-8")
            i += 1
        elif body[i + 1] in "01234567":
            out.append(int(body[i + 1:i + 4], 8))
            i += 4
        else:
            out.append(C_ESCAPES[body[i + 1]])
            i += 2
    return decode(bytes(out))


def diff_path(label: str) -> str:
    """The path of a `+++ ` header label: git adds a tab when the label has a space."""
    label = label.removesuffix("\t")
    if label.startswith('"'):
        label = unquote_c(label)
    return label.removeprefix("b/")


def added_lines(diff: str) -> Iterator[tuple[str, str]]:
    """(path, line) for every added line; `---`/`+++` count as headers only before a file's first hunk."""
    path, in_hunk = "", False
    for line in text_lines(diff):
        if line.startswith("diff --git "):
            path, in_hunk = "", False
        elif line.startswith("@@"):
            in_hunk = True
        elif in_hunk and line.startswith("+"):
            yield path, line[1:]
        elif not in_hunk and line.startswith("+++ "):
            path = diff_path(line[4:])


def changed_paths(repo: Path, sha: str) -> list[str]:
    """Every path the commit adds or changes (binary and empty files too), raw bytes via -z."""
    names = git(repo, *DIFF, "--name-only", "-z", "--diff-filter=d", sha)
    return [decode(name) for name in names.split(b"\0") if name]


def added_text(repo: Path, sha: str) -> dict[str, list[str]]:
    """Each changed file's added lines. A file whose added lines hold a NUL is read whole from the
    commit instead, as tree mode reads it: UTF-16 text is scanned, binary content skipped."""
    patch = decode(git(repo, *DIFF, "--unified=0", "--src-prefix=a/", "--dst-prefix=b/", sha))
    by_path: dict[str, list[str]] = {}
    for path, line in added_lines(patch):
        by_path.setdefault(path, []).append(line)
    for path, lines in by_path.items():
        if any("\0" in line for line in lines):
            text = blob_text(git(repo, "cat-file", "blob", f"{sha}:{path}"))
            by_path[path] = [] if text is None else text_lines(text)
    return by_path


def scan_commit(scanner: Scanner, repo: Path, sha: str, identities: set[str] | None) -> list[Finding]:
    short = sha[:12]
    metadata = metadata_of(repo, sha)
    hits: list[Finding] = [Hit(f"{short} commit metadata", entry) for entry in scanner.entries_in(metadata)]
    if identities is not None:
        for role, email in zip(("author", "committer"), emails_in(metadata), strict=True):
            if email not in identities:
                hits.append(IdentityProblem(short, role))
    for path in changed_paths(repo, sha):
        hits += scanner.scan_path(path, f"{short} ")
    for path, lines in added_text(repo, sha).items():
        where = f"{short} {scanner.shown(path)}"
        for line in lines:
            hits += scanner.scan_line(path, line, where)
    return hits


def scan_commits(
    scanner: Scanner, repo: Path, revlist_args: list[str], identities: set[str] | None = None,
    boundary: list[str] | None = None,
) -> list[Finding]:
    # the boundary goes first, so a `--not` inside the caller's range cannot flip it
    excluded = [f"^{sha}" for sha in boundary or []]
    hits: list[Finding] = []
    for sha in decode(git(repo, "rev-list", *excluded, *revlist_args)).split():
        hits += scan_commit(scanner, repo, sha, identities)
    return hits


def scan_boundary(repo: Path, boundary: list[str], identities: set[str]) -> list[BoundaryProblem]:
    """Every commit the boundary hides must be legacy: neither of its identities allowed."""
    problems: list[BoundaryProblem] = []
    for sha in decode(git(repo, "rev-list", *boundary)).split():
        if any(email in identities for email in emails_in(metadata_of(repo, sha))):
            problems.append(BoundaryProblem(sha[:12]))
    return problems


def hash_line(repo: Path, path: str, number: str) -> str:
    """The allow key of a working-tree line, decoded and split exactly as the scan does."""
    try:
        text = blob_text((repo / path).read_bytes())
    except OSError as error:
        raise UsageError(f"--hash: the file cannot be read ({type(error).__name__})") from None
    lines = [] if text is None else text_lines(text)
    if not number.isdecimal() or not 1 <= int(number) <= len(lines):
        raise UsageError("--hash: no such line in that file (a binary file, or a number out of range)")
    return line_key(path, lines[int(number) - 1])


def run(args: argparse.Namespace) -> list[Finding]:
    terms = load_terms(args.denylist)
    if not terms:
        raise UsageError(f"denylist {args.denylist} has no terms")
    identities = load_identities(args.identities)
    if identities is not None and not identities:
        raise UsageError(f"identity list {args.identities} is empty")
    boundary = load_boundary(args.boundary)
    check_boundary_exists(args.repo, boundary)
    scanner = Scanner(terms, load_allow(args.allow))
    hits: list[Finding] = []
    for rev in args.tree:
        hits += scan_tree(scanner, args.repo, rev)
    for spec in args.commits:
        hits += scan_commits(scanner, args.repo, shlex.split(spec), identities, boundary)
    if boundary and identities is not None:
        hits += scan_boundary(args.repo, boundary, identities)
    return hits


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

    if not args.hash and (args.denylist is None or not (args.tree or args.commits)):
        parser.error("--denylist and at least one --tree or --commits are required")

    try:
        if args.hash:
            print(hash_line(args.repo, *args.hash))
            return EXIT_CLEAN
        hits = run(args)
    except UsageError as error:
        print(error, file=sys.stderr)
        return EXIT_USAGE
    except subprocess.CalledProcessError as error:
        # git's own message is left out: it can quote a path or a revision
        print(f"git {error.cmd[3]} failed (exit {error.returncode})", file=sys.stderr)
        return EXIT_USAGE
    for hit in hits:
        print(hit.render())
    if hits:
        print(f"{len(hits)} finding(s)", file=sys.stderr)
        return EXIT_HIT
    print("denylist: clean")
    return EXIT_CLEAN


if __name__ == "__main__":
    sys.exit(main())
