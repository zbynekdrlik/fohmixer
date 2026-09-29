#!/usr/bin/env python3
"""Scan git content for private site data (spec §5.2; ported from iemmixer's
`scripts/denylist_scan.py`).

The denylist (one term per line, `#` comments) is private: a mode-600 file
outside the repo for local runs, the DENYLIST secret in CI. Output never
contains a term, a matched line or an email address — only locations and the
entry number, each finding line starting with `tree` or a commit's short SHA
(never with a path). A path component that holds a term is printed as `[redacted]`
(the whole path when a term spans components), control characters escaped.

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
term ending in `.` such as `203.0.113.` hits `203.0.113.5`. Text is split into lines
at `\\n` only (a trailing `\\r` dropped), the same in tree and commit mode, so a
line key (`--hash`) is the same in both.

Exit codes: 0 clean, 1 findings, 2 the scan could not run (a missing or
unreadable input file, a bad boundary, a failing git command).
"""
from __future__ import annotations

import argparse
import codecs
import hashlib
import os
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
# `author <name> <<email>> <time> <zone>` as stored in a commit object; every <…> on it counts
IDENTITY_LINE = re.compile(r"^(author|committer) (.*) -?\d+ [+-]\d{4}$")
ANGLED = re.compile(r"<([^<>]*)>")
ROLES = ("author", "committer")
# A signature in a commit header: its base64 and BEGIN/END markers are noise, not site data
SIGNATURE_HEADERS = (b"gpgsig ", b"gpgsig-sha256 ")
ARMOUR_MARKERS = {f"-----{edge} {kind}-----".encode("ascii") for edge in ("BEGIN", "END")
                  for kind in ("PGP SIGNATURE", "PGP MESSAGE", "SSH SIGNATURE", "SIGNED MESSAGE")}
BASE64 = re.compile(rb"[A-Za-z0-9+/=]*")
REDACTED = "[redacted]"
# git's C-quoting of a path in a diff header (core.quotePath)
C_ESCAPES = {"a": 7, "b": 8, "t": 9, "n": 10, "v": 11, "f": 12, "r": 13, '"': 34, "\\": 92}
UTF16_BOMS = (codecs.BOM_UTF16_LE, codecs.BOM_UTF16_BE)
# A commit's diff against its first parent (the root commit's too) whose labels, colours, attributes
# and paths depend neither on the user's git config nor on the repo's .gitattributes
DIFF = ["show", "--format=", "--no-show-signature", "--no-color", "--no-ext-diff", "--no-textconv", "--text",
        "--no-renames", "--no-relative", "--ignore-submodules=none", "--root", "-m", "--first-parent"]
GITLINK = "160000"
# a replace ref (refs/replace/*) or a graft (info/grafts) must not swap the scanned history for another
GIT_ENV = {**os.environ, "GIT_NO_REPLACE_OBJECTS": "1", "GIT_GRAFT_FILE": os.devnull}


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


def printable(text: str) -> str:
    """Control characters escaped, so a path cannot inject lines (a CI `::error` command) into the log."""
    return "".join(char if char.isprintable() else f"\\x{ord(char):02x}" if ord(char) < 0x100
                   else f"\\u{ord(char):04x}" for char in text)


class Scanner:
    def __init__(self, terms: list[str], allow: set[str]) -> None:
        self.patterns = [compile_term(nfc(term)) for term in terms]
        self.allow = allow

    def entries_in(self, text: str) -> list[int]:
        text = nfc(text)
        return [number for number, pattern in enumerate(self.patterns, start=1) if pattern.search(text)]

    def shown(self, path: str) -> str:
        """The path as printed: each component holding a term is redacted, the others have their
        control characters escaped; the whole path is redacted when a term spans components (a
        term without `/` always matches inside one component) or the printed form holds one."""
        whole = set(self.entries_in(path))
        parts = path.split("/")
        part_hits = [set(self.entries_in(part)) if whole else set() for part in parts]
        if not whole <= set().union(*part_hits):
            return REDACTED
        kept = "/".join(REDACTED if hit else printable(part) for part, hit in zip(parts, part_hits, strict=True))
        return REDACTED if self.entries_in(kept) else kept

    def scan_path(self, path: str, prefix: str) -> list[Hit]:
        return [Hit(f"{prefix}{self.shown(path)}: path", entry) for entry in self.entries_in(path)]

    def scan_line(self, path: str, line: str, where: str) -> list[Hit]:
        entries = self.entries_in(line)
        if not entries or line_key(path, line) in self.allow:
            return []
        return [Hit(where, entry) for entry in entries]


def git(repo: Path, *args: str, stdin: bytes | None = None) -> bytes:
    done = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, input=stdin, env=GIT_ENV)
    if done.returncode != 0:
        # git's own message is left out: it can quote a path or a revision
        raise UsageError(f"git {args[0]} failed (exit {done.returncode})")
    return done.stdout


def decode(data: bytes) -> str:
    return data.decode("utf-8", errors="replace")


def check_boundary_exists(repo: Path, boundary: list[str]) -> None:
    for sha in boundary:
        found = subprocess.run(["git", "-C", str(repo), "cat-file", "-e", f"{sha}^{{commit}}"],
                               capture_output=True, env=GIT_ENV)
        if found.returncode != 0:
            raise UsageError(f"boundary commit {sha} is not in {repo} (a shallow clone needs fetch-depth: 0)")


@dataclass(frozen=True)
class Metadata:
    text: str
    stored: dict[str, set[str]]
    rendered: dict[str, str]

    def emails(self, role: str) -> set[str]:
        return self.stored[role] | {self.rendered[role]}

    def unallowed_roles(self, identities: set[str]) -> list[str]:
        """Roles with no stored email, or any stored or rendered email that is not allowed."""
        return [role for role in ROLES if not self.stored[role] or not self.emails(role) <= identities]

    def has_allowed(self, identities: set[str]) -> bool:
        return any(email in identities for role in ROLES for email in self.emails(role))


def header_lines(head: bytes) -> list[bytes]:
    """A commit's header lines minus signature noise: a signature starts at a BEGIN marker on a
    gpgsig header or a continuation line (a mergetag's) and ends at its END marker or the next
    top-level header; inside it only base64 and the markers are left out (a Comment: stays)."""
    kept: list[bytes] = []
    armoured = False
    for line in head.split(b"\n"):
        if line.startswith(b" "):
            body = line[1:]
        else:
            armoured = False
            # only a gpgsig header can open a signature at the top level
            body = next((line[len(prefix):] for prefix in SIGNATURE_HEADERS if line.startswith(prefix)), b"")
        if body in ARMOUR_MARKERS:
            armoured = body.startswith(b"-----BEGIN")
        elif not (armoured and BASE64.fullmatch(body)):
            kept.append(line)
    return kept


def metadata_of(repo: Path, sha: str) -> Metadata:
    """A commit's metadata as stored (`cat-file`: no conversion by its `encoding` header, which could
    garble or hide text; signature noise left out) and as git renders it, so neither form hides a
    term. Identities are every <email> on every stored author/committer line plus the rendered one."""
    head, _, message = git(repo, "cat-file", "commit", sha).partition(b"\n\n")
    stored: dict[str, set[str]] = {role: set() for role in ROLES}
    for line in head.split(b"\n"):
        match = IDENTITY_LINE.match(decode(line))
        if match:
            stored[match.group(1)] |= {email.strip().lower() for email in ANGLED.findall(match.group(2))}
    shown = decode(git(repo, "show", "-s", "--no-show-signature", "--format=%an%n%ae%n%cn%n%ce%n%B", sha))
    fields = shown.split("\n") + ["", "", "", ""]
    rendered = {"author": fields[1].strip().lower(), "committer": fields[3].strip().lower()}
    text = "\n".join(decode(line) for line in header_lines(head)) + "\n" + decode(message) + "\n" + shown
    return Metadata(text, stored, rendered)


def scan_tree(scanner: Scanner, repo: Path, rev: str) -> list[Hit]:
    # every location starts with a fixed word, never with a path: a path starting with `::`
    # would otherwise read as a GitHub workflow command in the CI log
    hits: list[Hit] = []
    for entry in git(repo, "ls-tree", "-r", "-z", "--full-tree", rev).split(b"\0"):
        if not entry:
            continue
        meta, _, raw_path = entry.partition(b"\t")
        _mode, kind, obj = meta.split()
        path = decode(raw_path)
        hits += scanner.scan_path(path, "tree ")
        if kind != b"blob":
            continue
        text = blob_text(git(repo, "cat-file", "blob", decode(obj)))
        if text is None:
            continue
        shown = scanner.shown(path)
        for number, line in enumerate(text_lines(text), start=1):
            hits += scanner.scan_line(path, line, f"tree {shown}:{number}")
    return hits


def unquote_c(quoted: bytes) -> bytes:
    """A path git wrote C-quoted (`"b/Kl\\303\\241vor"`) back to its exact bytes."""
    body, out, i = quoted[1:-1], bytearray(), 0
    while i < len(body):
        if body[i:i + 1] != b"\\":
            out += body[i:i + 1]
            i += 1
        elif body[i + 1:i + 2] in (b"0", b"1", b"2", b"3", b"4", b"5", b"6", b"7"):
            out.append(int(body[i + 1:i + 4], 8))
            i += 4
        else:
            out.append(C_ESCAPES[body[i + 1:i + 2].decode("ascii")])
            i += 2
    return bytes(out)


def diff_path(label: bytes) -> bytes:
    """The path bytes of a `+++ ` header label: git adds a tab when the label has a space."""
    label = label.removesuffix(b"\t")
    if label.startswith(b'"'):
        label = unquote_c(label)
    return label.removeprefix(b"b/")


def added_lines(diff: bytes) -> Iterator[tuple[bytes, str]]:
    """(path bytes, line) for every added line; `---`/`+++` count as headers only before a file's
    first hunk. Lines split at `\\n` and lose one trailing `\\r`, as `text_lines` splits them."""
    path, in_hunk = b"", False
    for line in diff.split(b"\n"):
        if line.startswith(b"diff --git "):
            path, in_hunk = b"", False
        elif line.startswith(b"@@"):
            in_hunk = True
        elif in_hunk and line.startswith(b"+"):
            yield path, decode(line[1:]).removesuffix("\r")
        elif not in_hunk and line.startswith(b"+++ "):
            path = diff_path(line[4:])


@dataclass(frozen=True)
class Change:
    raw: bytes
    mode: str
    blob: str

    @property
    def path(self) -> str:
        return decode(self.raw)


def changes_of(repo: Path, sha: str) -> list[Change]:
    """The files a commit adds or changes against its first parent (binary, empty and gitlinks too),
    with their new blob ids. `--raw -z` gives each path's raw bytes: no quoting, no lossy lookup."""
    fields = git(repo, *DIFF, "--raw", "-z", "--no-abbrev", "--diff-filter=d", sha).split(b"\0")
    changes: list[Change] = []
    # records are ":<old mode> <new mode> <old id> <new id> <status>" NUL "<path>" NUL
    for meta, path in zip(fields[0:-1:2], fields[1::2], strict=True):
        _old_mode, new_mode, _old_id, new_id, _status = decode(meta).removeprefix(":").split()
        changes.append(Change(path, new_mode, new_id))
    return changes


def blob_data(repo: Path, ids: list[str]) -> dict[str, bytes]:
    """Blob contents by id, all from one `git cat-file --batch`."""
    if not ids:
        return {}
    out = git(repo, "cat-file", "--batch", stdin="".join(f"{blob}\n" for blob in ids).encode())
    data: dict[str, bytes] = {}
    position = 0
    for blob in ids:
        end = out.index(b"\n", position)
        header = out[position:end].split()
        if len(header) != 3:
            raise UsageError(f"blob {blob} is missing from the repository")
        size = int(header[2])
        data[blob] = out[end + 1:end + 1 + size]
        position = end + 1 + size + 1
    return data


def added_by_path(repo: Path, sha: str) -> dict[bytes, list[str]]:
    """Added lines keyed by the path's exact bytes (two names that decode alike stay apart); a
    label is raw bytes or C-quoted whatever core.quotePath says, and both give the exact bytes."""
    patch = git(repo, *DIFF, "--unified=0", "--src-prefix=a/", "--dst-prefix=b/", sha)
    by_path: dict[bytes, list[str]] = {}
    for path, line in added_lines(patch):
        by_path.setdefault(path, []).append(line)
    return by_path


def scan_changes(scanner: Scanner, repo: Path, sha: str, prefix: str) -> list[Hit]:
    """Paths and added lines. Each file is judged by its whole new blob, as tree mode judges it:
    binary is skipped, UTF-16 is scanned whole (its diff is not text), anything else by added line."""
    changes = changes_of(repo, sha)
    contents = blob_data(repo, [change.blob for change in changes if change.mode != GITLINK])
    added = added_by_path(repo, sha)
    hits: list[Hit] = []
    for change in changes:
        hits += scanner.scan_path(change.path, prefix)
        lines = added.pop(change.raw, [])
        data = None if change.mode == GITLINK else contents[change.blob]
        text = None if data is None else blob_text(data)
        if text is None:
            continue
        if data.startswith(UTF16_BOMS):
            lines = text_lines(text)
        hits += scan_lines(scanner, change.path, lines, prefix)
    # a patch path that matched no change (a label that unquotes differently) is scanned, never dropped
    for raw, lines in added.items():
        hits += scan_lines(scanner, decode(raw), lines, prefix)
    return hits


def scan_lines(scanner: Scanner, path: str, lines: list[str], prefix: str) -> list[Hit]:
    where = f"{prefix}{scanner.shown(path)}"
    return [hit for line in lines for hit in scanner.scan_line(path, line, where)]


def scan_commit(scanner: Scanner, repo: Path, sha: str, identities: set[str] | None) -> list[Finding]:
    short = sha[:12]
    metadata = metadata_of(repo, sha)
    hits: list[Finding] = [Hit(f"{short} commit metadata", entry) for entry in scanner.entries_in(metadata.text)]
    if identities is not None:
        hits += [IdentityProblem(short, role) for role in metadata.unallowed_roles(identities)]
    return hits + scan_changes(scanner, repo, sha, f"{short} ")


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
        if metadata_of(repo, sha).has_allowed(identities):
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
    # the scan keys a path by its bytes decoded as UTF-8 (U+FFFD for an invalid byte)
    return line_key(decode(os.fsencode(path)), lines[int(number) - 1])


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
        try:
            revlist = shlex.split(spec)
        except ValueError:
            raise UsageError("--commits: the range has an unbalanced quote") from None
        hits += scan_commits(scanner, args.repo, revlist, identities, boundary)
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
    for hit in hits:
        print(hit.render())
    # piped, stdout is block-buffered: flush it so the count never lands inside a finding's line
    sys.stdout.flush()
    if hits:
        print(f"{len(hits)} finding(s)", file=sys.stderr)
        return EXIT_HIT
    print("denylist: clean")
    return EXIT_CLEAN


if __name__ == "__main__":
    sys.exit(main())
