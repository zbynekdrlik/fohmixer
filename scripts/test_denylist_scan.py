"""Tests for scripts/denylist_scan.py (ported from iemmixer's test_denylist_scan.py;
run: python3 -m unittest discover -s scripts -p 'test_*.py'). Every term here is
invented; a real denylist term never enters this repo."""
from __future__ import annotations

import contextlib
import io
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import denylist_scan as ds  # noqa: E402

SCRIPTS = Path(__file__).resolve().parent
TERMS = ["zyxname", "10.9.", "ghost-host.example", "klávor"]
ALLOWED = "dev@example.org"
LEGACY = "someone.private@example.net"


def git(repo: Path, *args: str, env: dict[str, str] | None = None) -> str:
    done = subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True,
                          env={**os.environ, **env} if env else None)
    return done.stdout.decode("utf-8").strip()


def identity(name: str, email: str) -> dict[str, str]:
    return {"GIT_AUTHOR_NAME": name, "GIT_AUTHOR_EMAIL": email,
            "GIT_COMMITTER_NAME": name, "GIT_COMMITTER_EMAIL": email}


class ScanCase(unittest.TestCase):
    terms = TERMS

    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp())
        self.repo = self.tmp / "repo"
        self.repo.mkdir()
        git(self.repo, "init", "-q", "-b", "main")
        git(self.repo, "config", "user.email", "test@example.org")
        git(self.repo, "config", "user.name", "test")
        git(self.repo, "config", "commit.gpgsign", "false")
        self.deny = self.tmp / "deny.txt"
        self.deny.write_text("# test terms\n" + "\n".join(self.terms) + "\n", encoding="utf-8")

    def tearDown(self) -> None:
        shutil.rmtree(self.tmp)

    def commit(self, files: dict[str, str | bytes], message: str = "change",
               env: dict[str, str] | None = None) -> str:
        for rel, content in files.items():
            path = self.repo / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content if isinstance(content, bytes) else content.encode("utf-8"))
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", message, env=env)
        return git(self.repo, "rev-parse", "HEAD")

    def scan(self, *extra: str) -> tuple[int, str]:
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = ds.main(["--denylist", str(self.deny), "--repo", str(self.repo), *extra])
        return code, out.getvalue() + err.getvalue()

    def write(self, name: str, text: str) -> str:
        path = self.tmp / name
        path.write_text(text, encoding="utf-8")
        return str(path)

    def assert_no_term(self, out: str) -> None:
        for term in self.terms:
            self.assertNotIn(term.lower(), out.lower())


class DenylistScanTests(ScanCase):
    def test_clean_tree_passes(self) -> None:
        self.commit({"a.txt": "nothing private here\n"})
        self.assertEqual(self.scan("--tree", "HEAD")[0], 0)

    def test_term_in_content_is_reported_without_revealing_it(self) -> None:
        self.commit({"a.txt": "hello ZyxName!\n"})
        code, out = self.scan("--tree", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("a.txt:1", out)
        self.assertIn("denylist entry 1", out)
        self.assertNotIn("zyxname", out.lower())

    def test_term_inside_a_longer_word_is_not_a_hit(self) -> None:
        self.commit({"a.txt": "prezyxnamed zyxnameless\n"})
        self.assertEqual(self.scan("--tree", "HEAD")[0], 0)

    def test_underscore_does_not_hide_a_term(self) -> None:
        self.commit({"a.rs": "let x_zyxname_y = 1;\n"})
        self.assertEqual(self.scan("--tree", "HEAD")[0], 1)

    def test_a_string_escape_does_not_hide_a_term(self) -> None:
        # Fixture strings such as "TRACK\t1\tNAME mic": the `t` of `\t` is
        # an escape, not part of the word.
        self.commit({"a.rs": 'let l = "TRACK\\t1\\tZyxName mic";\n'})
        self.assertEqual(self.scan("--tree", "HEAD")[0], 1)

    def test_letters_with_diacritics_count_as_word_characters(self) -> None:
        self.commit({"a.md": "šzyxname\n"})
        self.assertEqual(self.scan("--tree", "HEAD")[0], 0)

    def test_term_ending_in_a_dot_matches_an_address(self) -> None:
        self.commit({"a.txt": "addr 10.9.3.4\n"})
        self.assertEqual(self.scan("--tree", "HEAD")[0], 1)

    def test_term_starting_with_a_digit_needs_a_left_boundary(self) -> None:
        self.commit({"a.txt": "addr 110.9.3.4\n"})
        self.assertEqual(self.scan("--tree", "HEAD")[0], 0)

    def test_term_in_a_path_is_reported(self) -> None:
        self.commit({"docs/zyxname-notes.md": "x\n"})
        code, out = self.scan("--tree", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("docs/", out)
        self.assertIn("path", out)

    def test_history_hit_is_found_by_commits_mode_only(self) -> None:
        self.commit({"a.txt": "zyxname\n"})
        (self.repo / "a.txt").write_text("clean\n", encoding="utf-8")
        git(self.repo, "commit", "-q", "-am", "clean up")
        self.assertEqual(self.scan("--tree", "HEAD")[0], 0)
        self.assertEqual(self.scan("--commits", "HEAD")[0], 1)

    def test_commit_message_hit(self) -> None:
        self.commit({"a.txt": "clean\n"}, message="fix for ghost-host.example")
        code, out = self.scan("--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("commit metadata", out)

    def test_author_identity_is_scanned_against_the_denylist(self) -> None:
        git(self.repo, "config", "user.email", "zyxname@example.org")
        self.commit({"a.txt": "clean\n"})
        code, out = self.scan("--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("commit metadata", out)
        self.assertNotIn("zyxname", out.lower())

    def test_identity_outside_the_allowed_set_is_rejected_without_printing_it(self) -> None:
        ids = self.write("ids.txt", "# allowed\ntest@example.org\n")
        git(self.repo, "config", "user.email", "someone.private@example.net")
        self.commit({"a.txt": "clean\n"})
        code, out = self.scan("--identities", ids, "--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("author email is not an allowed identity", out)
        self.assertIn("committer email is not an allowed identity", out)
        self.assertNotIn("someone.private", out)

    def test_allowed_identities_pass(self) -> None:
        ids = self.write("ids.txt", "TEST@example.org\n")
        self.commit({"a.txt": "clean\n"})
        self.assertEqual(self.scan("--identities", ids, "--commits", "HEAD")[0], 0)

    def test_allowlisted_line_is_skipped(self) -> None:
        self.commit({"a.txt": "keep zyxname here\n"})
        allow = self.write("allow.txt", ds.line_key("a.txt", "keep zyxname here") + "  a.txt reviewed\n")
        self.assertEqual(self.scan("--allow", allow, "--tree", "HEAD", "--commits", "HEAD")[0], 0)

    def test_a_missing_allow_file_is_a_usage_error(self) -> None:
        self.commit({"a.txt": "x\n"})
        self.assertEqual(self.scan("--allow", str(self.tmp / "absent.txt"), "--tree", "HEAD")[0], 2)

    def test_binary_content_is_skipped_but_its_path_is_scanned(self) -> None:
        self.commit({"bin.dat": b"\0zyxname"})
        self.assertEqual(self.scan("--tree", "HEAD")[0], 0)
        self.commit({"zyxname.bin": b"\0x"})
        self.assertEqual(self.scan("--tree", "HEAD")[0], 1)

    def test_empty_denylist_is_a_usage_error(self) -> None:
        self.commit({"a.txt": "x\n"})
        self.deny.write_text("# nothing\n\n", encoding="utf-8")
        self.assertEqual(self.scan("--tree", "HEAD")[0], 2)

    def test_empty_identity_list_is_a_usage_error(self) -> None:
        self.commit({"a.txt": "x\n"})
        ids = self.write("ids.txt", "# nobody\n")
        self.assertEqual(self.scan("--identities", ids, "--commits", "HEAD")[0], 2)

    def test_a_missing_denylist_is_a_usage_error(self) -> None:
        self.commit({"a.txt": "x\n"})
        self.deny.unlink()
        self.assertEqual(self.scan("--tree", "HEAD")[0], 2)

    def test_a_denylist_that_is_not_utf8_is_a_usage_error(self) -> None:
        self.commit({"a.txt": "x\n"})
        self.deny.write_bytes(b"zyx\xe1name\n")
        self.assertEqual(self.scan("--tree", "HEAD")[0], 2)

    def test_a_missing_identity_list_is_a_usage_error(self) -> None:
        self.commit({"a.txt": "x\n"})
        self.assertEqual(self.scan("--identities", str(self.tmp / "absent.txt"), "--commits", "HEAD")[0], 2)

    def test_a_missing_boundary_file_is_a_usage_error(self) -> None:
        self.commit({"a.txt": "x\n"})
        self.assertEqual(self.scan("--boundary", str(self.tmp / "absent.txt"), "--commits", "HEAD")[0], 2)

    def test_an_unknown_revision_is_a_usage_error(self) -> None:
        self.commit({"a.txt": "x\n"})
        code, out = self.scan("--commits", "no-such-rev")
        self.assertEqual(code, 2)
        self.assertIn("git rev-list failed", out)

    def test_a_term_in_a_path_is_redacted(self) -> None:
        self.commit({"docs/zyxname-notes.md": "addr 10.9.1.1\n"})
        code, out = self.scan("--tree", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("docs/[redacted]: path: denylist entry 1", out)
        self.assertIn("docs/[redacted]:1: denylist entry 2", out)
        self.assert_no_term(out)

    def test_a_term_in_a_path_is_redacted_in_commit_mode(self) -> None:
        self.commit({"logs/10.9.3.4.txt": "zyxname\n"})
        (self.repo / "logs" / "10.9.3.4.txt").unlink()
        git(self.repo, "commit", "-q", "-am", "remove")
        code, out = self.scan("--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn(" logs/[redacted]: path: denylist entry 2", out)
        self.assertIn(" logs/[redacted]: denylist entry 1", out)
        self.assert_no_term(out)

    def test_an_added_line_starting_with_plus_plus_is_content_not_a_header(self) -> None:
        self.commit({"a.txt": "++ zyxname was here\nclean\n"})
        code, out = self.scan("--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn(" a.txt: denylist entry 1", out)
        self.assertNotIn("was here", out)
        self.assert_no_term(out)

    def test_a_vertical_tab_does_not_hide_a_term_in_history(self) -> None:
        self.commit({"a.txt": "x\x0bzyxname\n"})
        self.commit({"a.txt": "clean\n"})
        code, out = self.scan("--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn(" a.txt: denylist entry 1", out)

    def test_a_path_with_diacritics_is_scanned_in_commit_mode(self) -> None:
        self.commit({"docs/Klávor.txt": "x\n"})
        (self.repo / "docs" / "Klávor.txt").unlink()
        git(self.repo, "commit", "-q", "-am", "remove")
        self.assertEqual(self.scan("--tree", "HEAD")[0], 0)
        code, out = self.scan("--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("docs/[redacted]: path: denylist entry 4", out)
        self.assert_no_term(out)

    def test_the_path_of_a_binary_or_empty_file_is_scanned_in_commit_mode(self) -> None:
        self.commit({"zyxname.bin": b"\0x", "ghost-host.example.txt": ""})
        git(self.repo, "rm", "-q", "zyxname.bin", "ghost-host.example.txt")
        git(self.repo, "commit", "-q", "-m", "remove")
        code, out = self.scan("--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("[redacted]: path: denylist entry 1", out)
        self.assertIn("[redacted]: path: denylist entry 3", out)
        self.assert_no_term(out)

    def test_a_path_with_a_space_has_the_same_allow_key_in_both_modes(self) -> None:
        self.commit({"my notes.txt": "keep zyxname here\n"})
        allow = self.write("allow.txt", ds.line_key("my notes.txt", "keep zyxname here") + "  reviewed\n")
        self.assertEqual(self.scan("--allow", allow, "--tree", "HEAD", "--commits", "HEAD")[0], 0)

    def test_hash_mode_prints_the_line_key(self) -> None:
        self.commit({"a.txt": "one\ntwo\n"})
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = ds.main(["--repo", str(self.repo), "--hash", "a.txt", "2"])
        self.assertEqual(code, 0)
        self.assertEqual(out.getvalue().strip(), ds.line_key("a.txt", "two"))


class SyntheticListTests(ScanCase):
    """An invented list planted in the tree, a commit message and an author."""

    terms = ["Zorblax", "quimbrel-pc", "10.77."]

    def test_a_planted_term_in_the_tree_is_a_hit_without_revealing_it(self) -> None:
        self.commit({"docs/notes.md": "mixed by zorblax\n", "cfg.toml": 'host = "QUIMBREL-PC"\n'})
        code, out = self.scan("--tree", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("docs/notes.md:1: denylist entry 1", out)
        self.assertIn("cfg.toml:1: denylist entry 2", out)
        self.assert_no_term(out)

    def test_a_planted_term_in_a_commit_message_is_a_hit_without_revealing_it(self) -> None:
        self.commit({"a.txt": "clean\n"}, message="log from 10.77.0.3")
        code, out = self.scan("--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("commit metadata: denylist entry 3", out)
        self.assert_no_term(out)

    def test_a_planted_term_in_the_author_is_a_hit_without_revealing_it(self) -> None:
        self.commit({"a.txt": "clean\n"}, env={"GIT_AUTHOR_NAME": "Zorblax Q"})
        code, out = self.scan("--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("commit metadata: denylist entry 1", out)
        self.assert_no_term(out)

    def test_an_added_line_in_history_is_a_hit_without_revealing_it(self) -> None:
        self.commit({"a.txt": "zorblax was here\n"})
        self.commit({"a.txt": "clean\n"})
        code, out = self.scan("--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("a.txt: denylist entry 1", out)
        self.assert_no_term(out)


    def test_a_term_spanning_path_components_redacts_the_whole_path(self) -> None:
        self.deny.write_text("quim/brel\n", encoding="utf-8")
        self.commit({"quim/brel.txt": "x\n"})
        code, out = self.scan("--tree", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("[redacted]: path: denylist entry 1", out)
        self.assertNotIn("quim", out.lower())
        self.assertNotIn("brel", out.lower())


class BoundaryTests(ScanCase):
    """The kept legacy history: its commits are left out of the commit scan."""

    terms = ["Zorblax"]

    def setUp(self) -> None:
        super().setUp()
        self.ids = self.write("ids.txt", f"{ALLOWED}\n")
        # Legacy history: a non-allowed identity and a planted term in a message and a line.
        self.commit({"a.txt": "base\n"}, env=identity("Legacy", LEGACY))
        self.legacy = self.commit({"a.txt": "zorblax\n"}, "zorblax notes", env=identity("Legacy", LEGACY))

    def scan_new(self, boundary: str, *extra: str) -> tuple[int, str]:
        return self.scan("--identities", self.ids, "--boundary", boundary, "--commits", "HEAD", *extra)

    def test_a_commit_before_the_boundary_is_not_scanned(self) -> None:
        self.commit({"a.txt": "clean\n"}, env=identity("dev", ALLOWED))
        boundary = self.write("boundary.txt", f"# legacy tip\n{self.legacy}\n")
        self.assertEqual(self.scan_new(boundary), (0, "denylist: clean\n"))
        # The same range without the boundary scans the legacy commits: term and identity hits.
        code, out = self.scan("--identities", self.ids, "--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn(f"{self.legacy[:12]} commit metadata: denylist entry 1", out)
        self.assertIn(f"{self.legacy[:12]}: author email is not an allowed identity", out)
        self.assert_no_term(out)
        self.assertNotIn(LEGACY, out)

    def test_a_commit_after_the_boundary_is_scanned(self) -> None:
        self.commit({"b.txt": "zorblax again\n"}, env=identity("dev", ALLOWED))
        boundary = self.write("boundary.txt", f"{self.legacy}\n")
        code, out = self.scan_new(boundary)
        self.assertEqual(code, 1)
        self.assertIn("b.txt: denylist entry 1", out)
        self.assertNotIn(self.legacy[:12], out)
        self.assert_no_term(out)

    def test_a_new_commit_with_a_legacy_identity_is_rejected(self) -> None:
        self.commit({"a.txt": "clean\n"}, env=identity("Legacy", LEGACY))
        boundary = self.write("boundary.txt", f"{self.legacy}\n")
        code, out = self.scan_new(boundary)
        self.assertEqual(code, 1)
        self.assertIn("author email is not an allowed identity", out)
        self.assertNotIn(LEGACY, out)

    def test_every_legacy_tip_must_be_listed(self) -> None:
        # A second legacy line, merged later by an allowed identity (the shape of this repo's history).
        git(self.repo, "checkout", "-q", "-b", "side", f"{self.legacy}~1")
        side = self.commit({"s.txt": "side\n"}, "side work", env=identity("Legacy", LEGACY))
        git(self.repo, "checkout", "-q", "main")
        git(self.repo, "merge", "-q", "--no-ff", "-m", "merge side", "side", env=identity("dev", ALLOWED))
        one_tip = self.write("one.txt", f"{self.legacy}\n")
        code, out = self.scan_new(one_tip)
        self.assertEqual(code, 1)
        self.assertIn(f"{side[:12]}: author email is not an allowed identity", out)
        both_tips = self.write("both.txt", f"{self.legacy}\n{side}\n")
        self.assertEqual(self.scan_new(both_tips)[0], 0)

    def test_a_boundary_over_an_allowed_commit_is_a_finding(self) -> None:
        new = self.commit({"b.txt": "zorblax in new work\n"}, env=identity("dev", ALLOWED))
        self.commit({"c.txt": "clean\n"}, env=identity("dev", ALLOWED))
        moved = self.write("boundary.txt", f"{new}\n")
        code, out = self.scan_new(moved)
        self.assertEqual(code, 1)
        self.assertIn(f"{new[:12]}: hidden by the boundary, but it has an allowed identity", out)
        self.assertNotIn(self.legacy[:12], out)

    def test_a_boundary_over_a_commit_with_one_allowed_identity_is_a_finding(self) -> None:
        # Legacy work re-committed (a cherry-pick) by the allowed identity is new history.
        picked = self.commit({"b.txt": "picked\n"}, env={
            "GIT_AUTHOR_NAME": "Legacy", "GIT_AUTHOR_EMAIL": LEGACY,
            "GIT_COMMITTER_NAME": "dev", "GIT_COMMITTER_EMAIL": ALLOWED})
        self.commit({"c.txt": "clean\n"}, env=identity("dev", ALLOWED))
        code, out = self.scan_new(self.write("boundary.txt", f"{picked}\n"))
        self.assertEqual(code, 1)
        self.assertIn(f"{picked[:12]}: hidden by the boundary, but it has an allowed identity", out)
        self.assertNotIn(LEGACY, out)

    def test_a_not_in_the_callers_range_cannot_flip_the_boundary(self) -> None:
        # rev-list reads `^tip` after a `--not` as `tip`: the boundary must come first.
        new = self.commit({"b.txt": "clean\n"}, env=identity("dev", ALLOWED))
        git(self.repo, "checkout", "-q", "--orphan", "other")
        other = self.commit({"o.txt": "other\n"}, env=identity("dev", ALLOWED))
        git(self.repo, "checkout", "-q", "main")
        boundary = self.write("boundary.txt", f"{self.legacy}\n")
        code, out = self.scan("--identities", self.ids, "--boundary", boundary, "--commits", f"{new} --not {other}")
        self.assertEqual((code, out), (0, "denylist: clean\n"))

    def test_a_boundary_line_that_is_not_a_sha_is_a_usage_error_without_echoing_it(self) -> None:
        wrong = self.write("boundary.txt", "# a denylist passed by mistake\nZorblax\n")
        code, out = self.scan_new(wrong)
        self.assertEqual(code, 2)
        self.assertIn("line 2 is not a full commit SHA", out)
        self.assert_no_term(out)

    def test_an_unknown_boundary_commit_is_a_usage_error(self) -> None:
        absent = self.write("boundary.txt", "0123456789abcdef0123456789abcdef01234567\n")
        code, out = self.scan_new(absent)
        self.assertEqual(code, 2)
        self.assertIn("fetch-depth: 0", out)

    def test_an_empty_boundary_is_a_usage_error(self) -> None:
        self.assertEqual(self.scan_new(self.write("boundary.txt", "# none\n"))[0], 2)

    def test_the_command_line_as_ci_runs_it(self) -> None:
        self.commit({"a.txt": "clean\n"}, env=identity("dev", ALLOWED))
        boundary = self.write("boundary.txt", f"{self.legacy}\n")
        command = [sys.executable, str(SCRIPTS / "denylist_scan.py"), "--denylist", str(self.deny),
                   "--identities", self.ids, "--boundary", boundary, "--tree", "HEAD", "--commits", "HEAD"]
        clean = subprocess.run(command, cwd=self.repo, capture_output=True, text=True)
        self.assertEqual((clean.returncode, clean.stdout), (0, "denylist: clean\n"))
        self.commit({"b.txt": "Zorblax\n"}, env=identity("dev", ALLOWED))
        hit = subprocess.run(command, cwd=self.repo, capture_output=True, text=True)
        self.assertEqual(hit.returncode, 1)
        self.assertIn("b.txt:1: denylist entry 1", hit.stdout)
        self.assert_no_term(hit.stdout + hit.stderr)


class RepoFilesTests(unittest.TestCase):
    """The committed boundary and identity files parse (their commits are checked in the secrets job)."""

    def test_the_boundary_lists_the_two_legacy_tips(self) -> None:
        self.assertEqual(ds.load_boundary(SCRIPTS / "denylist-boundary.txt"), [
            "b3d26d64f6ee89df8619197142f478a10d2922fa", "aca52f28a4489197d90b9f324043b41cda25c1ac"])

    def test_the_identities_are_the_noreply_account_and_github(self) -> None:
        self.assertEqual(ds.load_identities(SCRIPTS / "allowed-identities.txt"), {
            "26905282+zbynekdrlik@users.noreply.github.com", "noreply@github.com"})


if __name__ == "__main__":
    unittest.main()
