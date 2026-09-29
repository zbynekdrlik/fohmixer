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

    def test_a_diff_attribute_does_not_hide_history(self) -> None:
        self.commit({".gitattributes": "*.txt -diff\n", "a.txt": "zyxname\n"})
        self.commit({"a.txt": "clean\n"})
        code, out = self.scan("--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn(" a.txt: denylist entry 1", out)

    def test_the_root_commit_is_scanned_whatever_log_showroot_says(self) -> None:
        git(self.repo, "config", "log.showRoot", "false")
        self.commit({"a.txt": "zyxname\n"})
        self.commit({"a.txt": "clean\n"})
        code, out = self.scan("--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn(" a.txt: denylist entry 1", out)

    def test_binary_content_is_skipped_in_commit_mode_too(self) -> None:
        self.commit({"bin.dat": b"\0zyxname\n"})
        self.assertEqual(self.scan("--commits", "HEAD")[0], 0)

    def test_a_line_hit_under_a_quoted_path_prints_the_redacted_path(self) -> None:
        self.commit({"docs/Klávor.txt": "zyxname\n"})
        code, out = self.scan("--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn(" docs/[redacted]: denylist entry 1", out)
        self.assertNotIn("\\", out)
        self.assert_no_term(out)

    def test_a_hit_under_a_quoted_or_spaced_path_keeps_its_label_in_commit_mode(self) -> None:
        self.commit({'Mäso "q"\tx.txt': "zyxname\n", "my notes.txt": "zyxname\n"})
        code, out = self.scan("--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn(' Mäso "q"\\x09x.txt: denylist entry 1', out)
        self.assertIn(" my notes.txt: denylist entry 1", out)
        self.assertEqual(out.count("denylist entry 1"), 2)

    def test_a_quoted_path_has_the_same_allow_key_in_both_modes(self) -> None:
        name = 'Mäso "q"\tx.txt'
        self.commit({name: "keep zyxname here\n"})
        allow = self.write("allow.txt", ds.line_key(name, "keep zyxname here") + "  reviewed\n")
        self.assertEqual(self.scan("--allow", allow, "--tree", "HEAD", "--commits", "HEAD")[0], 0)

    def test_a_merge_is_scanned_against_its_first_parent(self) -> None:
        self.commit({"a.txt": "base\n"})
        git(self.repo, "checkout", "-q", "-b", "side")
        self.commit({"s.txt": "zyxname\n"})
        git(self.repo, "checkout", "-q", "main")
        self.commit({"m.txt": "main\n"})
        git(self.repo, "merge", "-q", "--no-ff", "-m", "merge side", "side")
        # the merge alone: its side commit would hit on its own
        code, out = self.scan("--commits", "HEAD ^HEAD^1 ^HEAD^2")
        self.assertEqual(code, 1)
        self.assertIn(" s.txt: denylist entry 1", out)

    def test_a_renamed_files_lines_are_scanned_under_the_new_name(self) -> None:
        self.commit({"a.txt": "zyxname\n" + "same\n" * 20})
        git(self.repo, "mv", "a.txt", "b.txt")
        git(self.repo, "commit", "-q", "-m", "rename")
        code, out = self.scan("--commits", "HEAD^..HEAD")
        self.assertEqual(code, 1)
        self.assertIn(" b.txt: denylist entry 1", out)

    def test_a_decomposed_letter_does_not_hide_a_term(self) -> None:
        self.commit({"a.txt": "klávor\n"})
        self.assertEqual(self.scan("--tree", "HEAD")[0], 1)

    def test_a_utf16_file_is_scanned_in_both_modes(self) -> None:
        self.commit({"cfg.xml": "<host>zyxname</host>\r\n".encode("utf-16")})
        code, out = self.scan("--tree", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("cfg.xml:1: denylist entry 1", out)
        git(self.repo, "rm", "-q", "cfg.xml")
        git(self.repo, "commit", "-q", "-m", "remove")
        code, out = self.scan("--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn(" cfg.xml: denylist entry 1", out)

    def test_hash_mode_splits_lines_like_the_scan(self) -> None:
        (self.repo / "a.txt").write_bytes(b"a\rzyxname\nb\n")
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = ds.main(["--repo", str(self.repo), "--hash", "a.txt", "1"])
        self.assertEqual((code, out.getvalue().strip()), (0, ds.line_key("a.txt", "a\rzyxname")))

    def test_hash_mode_rejects_a_line_out_of_range(self) -> None:
        (self.repo / "a.txt").write_text("one\ntwo\n", encoding="utf-8")
        for number in ("0", "3", "9", "x"):
            err = io.StringIO()
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(err):
                code = ds.main(["--repo", str(self.repo), "--hash", "a.txt", number])
            self.assertEqual(code, 2, number)

    def test_a_non_utf8_path_is_read_by_its_blob(self) -> None:
        self.commit({os.fsdecode(b"docs/caf\xe9.xml"): "<h>zyxname</h>\n".encode("utf-16"),
                     os.fsdecode(b"docs/caf\xe9.bin"): b"\0x"})
        for mode in ("--tree", "--commits"):
            code, out = self.scan(mode, "HEAD")
            self.assertEqual(code, 1, mode)
            self.assertEqual(out.count("denylist entry 1"), 1, mode)

    def test_non_utf8_names_that_decode_alike_keep_their_own_lines(self) -> None:
        # d\xe8 and d\xe9 both decode to "d�.txt": lines must stay with their own blob
        self.commit({os.fsdecode(b"d\xe8.txt"): b"\0x", os.fsdecode(b"d\xe9.txt"): b"zyxname\n"})
        self.assertEqual(self.scan("--tree", "HEAD")[0], 1)
        self.assertEqual(self.scan("--commits", "HEAD")[0], 1)
        self.commit({os.fsdecode(b"e\xe8.txt"): b"clean\n", os.fsdecode(b"e\xe9.txt"): b"\0zyxname\n"})
        self.assertEqual(self.scan("--commits", "HEAD^!")[0], 0)

    def test_a_graft_does_not_hide_content(self) -> None:
        base = self.commit({"a.txt": "clean\n"})
        head = self.commit({"a.txt": "zyxname\n"})
        tree = git(self.repo, "rev-parse", f"{head}^{{tree}}")
        same_tree = git(self.repo, "commit-tree", tree, "-p", base, "-m", "same tree")
        (self.repo / ".git" / "info" / "grafts").write_text(f"{head} {same_tree}\n", encoding="utf-8")
        code, out = self.scan("--commits", "HEAD^!")
        self.assertEqual(code, 1)
        self.assertIn(" a.txt: denylist entry 1", out)

    def test_no_output_line_can_start_a_workflow_command(self) -> None:
        self.commit({"::error title=x::fake/notes.md": "zyxname\n", " ::warning::y.md": "zyxname\n",
                     "::notice x::/zyxname.md": "x\n"})
        code, out = self.scan("--tree", "HEAD", "--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("tree ::notice x::/[redacted]: path: denylist entry 1", out)
        self.assertEqual([line for line in out.splitlines() if line.lstrip().startswith("::")], [])

    def test_non_utf8_names_keep_their_own_lines_with_quotepath_off(self) -> None:
        git(self.repo, "config", "core.quotePath", "false")
        self.commit({os.fsdecode(b"d\xe8.txt"): b"\0x", os.fsdecode(b"d\xe9.txt"): b"zyxname\n"})
        self.assertEqual(self.scan("--commits", "HEAD")[0], 1)

    def raw_commit(self, author: str, encoding: str, message: bytes, email: str = ALLOWED,
                   extra: str = "", zone: str = "+0000") -> None:
        """A commit object written as bytes (an `encoding` header that git would convert from)."""
        head = git(self.repo, "rev-parse", "HEAD")
        tree = git(self.repo, "rev-parse", "HEAD^{tree}")
        body = (f"tree {tree}\nparent {head}\nauthor {author} <{email}> 1700000000 {zone}\n"
                f"committer dev <{email}> 1700000000 {zone}\nencoding {encoding}\n{extra}\n").encode("utf-8") + message
        done = subprocess.run(["git", "-C", str(self.repo), "hash-object", "-t", "commit", "-w", "--stdin"],
                              input=body, check=True, capture_output=True)
        git(self.repo, "update-ref", "refs/heads/main", done.stdout.decode("ascii").strip())

    def test_a_mislabelled_encoding_does_not_hide_a_name(self) -> None:
        self.commit({"a.txt": "x\n"})
        self.raw_commit("Jan Klávor", "ISO-8859-2", b"clean\n")
        code, out = self.scan("--commits", "HEAD^!")
        self.assertEqual(code, 1)
        self.assertIn("commit metadata: denylist entry 4", out)
        self.assert_no_term(out)

    def test_a_crafted_encoding_does_not_hide_a_message(self) -> None:
        self.commit({"a.txt": "x\n"})
        self.raw_commit("dev", "UCS-2", b"hello zyxname!\n")
        code, out = self.scan("--commits", "HEAD^!")
        self.assertEqual(code, 1)
        self.assertIn("commit metadata: denylist entry 1", out)

    def test_identities_are_read_as_stored(self) -> None:
        self.commit({"a.txt": "x\n"})
        self.raw_commit("dev", "UCS-2", b"clean\n")
        ids = self.write("ids.txt", f"{ALLOWED}\n")
        self.assertEqual(self.scan("--identities", ids, "--commits", "HEAD^!"), (0, "denylist: clean\n"))

    def test_an_encoding_cannot_turn_a_stored_email_into_an_allowed_one(self) -> None:
        # UTF-7 renders the stored `dev+AEA-example.org` as the allowed `dev@example.org` (a `+0000`
        # zone would break the conversion: in UTF-7 a `+` opens a base64 run)
        self.commit({"a.txt": "x\n"})
        self.raw_commit("dev", "UTF-7", b"clean\n", email="dev+AEA-example.org", zone="-0100")
        ids = self.write("ids.txt", f"{ALLOWED}\n")
        code, out = self.scan("--identities", ids, "--commits", "HEAD^!")
        self.assertEqual(code, 1)
        self.assertIn("author email is not an allowed identity", out)
        self.assertNotIn("AEA", out)

    def write_commit(self, headers: str, literally: bool = False) -> None:
        """A commit on HEAD with these headers after tree/parent, written as bytes; HEAD moves to it."""
        head = git(self.repo, "rev-parse", "HEAD")
        tree = git(self.repo, "rev-parse", "HEAD^{tree}")
        body = f"tree {tree}\nparent {head}\n{headers}\nclean\n".encode("utf-8")
        command = ["git", "-C", str(self.repo), "hash-object", "-t", "commit", "-w", "--stdin"]
        done = subprocess.run([*command, "--literally"] if literally else command, input=body, check=True,
                              capture_output=True)
        git(self.repo, "update-ref", "refs/heads/main", done.stdout.decode("ascii").strip())

    def test_a_signature_marker_in_a_name_does_not_hide_the_next_header(self) -> None:
        self.commit({"a.txt": "x\n"})
        self.write_commit(f"author -----BEGIN x <{ALLOWED}> 1700000000 +0000\n"
                          f"committer Zyxname Q <{ALLOWED}> 1700000000 +0000\nencoding UCS-2\n")
        code, out = self.scan("--commits", "HEAD^!")
        self.assertEqual(code, 1)
        self.assertIn("commit metadata: denylist entry 1", out)

    def test_an_unterminated_signature_ends_at_the_next_header(self) -> None:
        self.commit({"a.txt": "x\n"})
        self.write_commit(f"author dev <{ALLOWED}> 1700000000 +0000\ncommitter dev <{ALLOWED}> 1700000000 +0000\n"
                          "gpgsig -----BEGIN PGP SIGNATURE-----\n iQIz\nx-note zyxname\n", literally=True)
        code, out = self.scan("--commits", "HEAD^!")
        self.assertEqual(code, 1)
        self.assertIn("commit metadata: denylist entry 1", out)

    def test_free_text_in_a_signature_is_scanned(self) -> None:
        self.commit({"a.txt": "x\n"})
        self.write_commit(f"author dev <{ALLOWED}> 1700000000 +0000\ncommitter dev <{ALLOWED}> 1700000000 +0000\n"
                          "gpgsig -----BEGIN PGP SIGNATURE-----\n Comment: zyxname key\n \n iQIz\n"
                          " -----END PGP SIGNATURE-----\n")
        code, out = self.scan("--commits", "HEAD^!")
        self.assertEqual(code, 1)
        self.assertIn("commit metadata: denylist entry 1", out)

    def test_every_stored_identity_must_be_allowed(self) -> None:
        ids = self.write("ids.txt", f"{ALLOWED}\n")
        self.commit({"a.txt": "x\n"})
        for author in (f"author dev <{ALLOWED}> 1700000000 +0000\nauthor x <{LEGACY}> 1700000000 +0000",
                       f"author a <{LEGACY}> b <{ALLOWED}> 1700000000 +0000"):
            self.write_commit(f"{author}\ncommitter dev <{ALLOWED}> 1700000000 +0000\n", literally=True)
            code, out = self.scan("--identities", ids, "--commits", "HEAD^!")
            self.assertEqual(code, 1, author)
            self.assertIn("author email is not an allowed identity", out)
            self.assertNotIn(LEGACY, out)

    def test_signature_armour_is_not_scanned(self) -> None:
        # a signature's base64 is noise, not site data: a short term can occur in it by chance
        self.commit({"a.txt": "x\n"})
        armour = ("gpgsig -----BEGIN PGP SIGNATURE-----\n \n iQIz+zyxname/AbC\n"
                  " -----END PGP SIGNATURE-----\n")
        self.raw_commit("dev", "UTF-8", b"clean\n", extra=armour)
        self.assertEqual(self.scan("--commits", "HEAD^!"), (0, "denylist: clean\n"))

    def test_hash_mode_keys_a_non_utf8_path_as_the_scan_does(self) -> None:
        name = os.fsdecode(b"caf\xe9.txt")
        self.commit({name: "keep zyxname here\n"})
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = ds.main(["--repo", str(self.repo), "--hash", name, "1"])
        self.assertEqual(code, 0)
        allow = self.write("allow.txt", out.getvalue().strip() + "  reviewed\n")
        self.assertEqual(self.scan("--allow", allow, "--tree", "HEAD", "--commits", "HEAD")[0], 0)

    def test_a_missing_blob_is_a_usage_error(self) -> None:
        self.commit({"a.txt": "x\n"})
        blob = git(self.repo, "rev-parse", "HEAD:a.txt")
        (self.repo / ".git" / "objects" / blob[:2] / blob[2:]).unlink()
        code, out = self.scan("--commits", "HEAD")
        self.assertEqual(code, 2)
        self.assertIn("missing from the repository", out)

    def test_a_changed_binary_file_stays_binary_in_commit_mode(self) -> None:
        self.commit({"bin.dat": b"\0head\nclean\n"})
        self.commit({"bin.dat": b"\0head\nzyxname\n"})
        self.assertEqual(self.scan("--tree", "HEAD")[0], 0)
        self.assertEqual(self.scan("--commits", "HEAD")[0], 0)

    def test_a_textconv_driver_does_not_hide_history(self) -> None:
        git(self.repo, "config", "diff.hide.textconv", "true")
        self.commit({".gitattributes": "*.txt diff=hide\n", "a.txt": "zyxname\n"})
        self.commit({"a.txt": "clean\n"})
        self.assertEqual(self.scan("--commits", "HEAD")[0], 1)

    def test_diff_relative_does_not_narrow_the_commit_scan(self) -> None:
        git(self.repo, "config", "diff.relative", "true")
        self.commit({"a.txt": "zyxname\n", "sub/x.txt": "x\n"})
        self.commit({"a.txt": "clean\n"})
        code, out = self.scan("--repo", str(self.repo / "sub"), "--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn(" a.txt: denylist entry 1", out)

    def test_a_decomposed_term_hits_composed_text(self) -> None:
        self.deny.write_text("klávor\n", encoding="utf-8")
        self.commit({"a.txt": "klávor\n"})
        self.assertEqual(self.scan("--tree", "HEAD")[0], 1)

    def test_an_unbalanced_quote_in_a_range_is_a_usage_error(self) -> None:
        self.commit({"a.txt": "x\n"})
        self.assertEqual(self.scan("--commits", "HEAD 'x")[0], 2)

    def test_control_characters_in_a_path_are_escaped(self) -> None:
        self.commit({"a\n::error::b.txt": "zyxname\n"})
        code, out = self.scan("--tree", "HEAD", "--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertNotIn("\n::error", out)
        self.assertIn("a\\x0a::error::b.txt:1: denylist entry 1", out)

    def test_a_replace_ref_does_not_hide_content(self) -> None:
        base = self.commit({"a.txt": "clean\n"})
        head = self.commit({"a.txt": "zyxname\n"})
        tree = git(self.repo, "rev-parse", f"{base}^{{tree}}")
        fake = git(self.repo, "commit-tree", tree, "-p", base, "-m", "fake")
        git(self.repo, "replace", head, fake)
        code, out = self.scan("--tree", "HEAD", "--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("a.txt:1: denylist entry 1", out)

    def test_a_submodule_path_is_scanned_whatever_diff_ignore_submodules_says(self) -> None:
        git(self.repo, "config", "diff.ignoreSubmodules", "all")
        base = self.commit({"a.txt": "x\n"})
        git(self.repo, "update-index", "--add", "--cacheinfo", f"160000,{base},zyxname-mod")
        git(self.repo, "commit", "-q", "-m", "add a submodule")
        git(self.repo, "rm", "-q", "--cached", "zyxname-mod")
        git(self.repo, "commit", "-q", "-m", "drop it")
        code, out = self.scan("--commits", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("[redacted]: path: denylist entry 1", out)

    def test_a_crlf_line_has_the_lf_allow_key_in_both_modes_and_in_hash(self) -> None:
        self.commit({"a.txt": "keep zyxname here\r\nx\r\n"})
        key = ds.line_key("a.txt", "keep zyxname here")
        allow = self.write("allow.txt", key + "  reviewed\n")
        self.assertEqual(self.scan("--allow", allow, "--tree", "HEAD", "--commits", "HEAD")[0], 0)
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            ds.main(["--repo", str(self.repo), "--hash", "a.txt", "1"])
        self.assertEqual(out.getvalue().strip(), key)

    def test_an_odd_length_utf16_file_is_still_scanned(self) -> None:
        self.commit({"a.xml": "zyxname\n".encode("utf-16") + b"x"})
        self.assertEqual(self.scan("--tree", "HEAD", "--commits", "HEAD")[0], 1)

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


    def test_a_spanning_term_beside_a_component_term_redacts_the_whole_path(self) -> None:
        self.deny.write_text("quim/brel\nzorb\n", encoding="utf-8")
        self.commit({"x/quim/brel-zorb.txt": "x\n"})
        code, out = self.scan("--tree", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("[redacted]: path: denylist entry 1", out)
        self.assertIn("[redacted]: path: denylist entry 2", out)
        for fragment in ("quim", "brel", "zorb"):
            self.assertNotIn(fragment, out.lower())

    def test_a_path_that_forms_a_term_once_redacted_is_redacted_whole(self) -> None:
        self.deny.write_text("zorb\n]/notes\n", encoding="utf-8")
        self.commit({"zorb/notes.md": "x\n"})
        code, out = self.scan("--tree", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("[redacted]: path: denylist entry 1", out)
        self.assertNotIn("notes", out)

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

    def test_a_signature_marker_in_a_name_cannot_pass_as_legacy(self) -> None:
        marked = self.commit({"b.txt": "x\n"}, env=identity("-----BEGIN -----END dev", ALLOWED))
        self.commit({"c.txt": "clean\n"}, env=identity("dev", ALLOWED))
        code, out = self.scan_new(self.write("boundary.txt", f"{marked}\n"))
        self.assertEqual(code, 1)
        self.assertIn(f"{marked[:12]}: hidden by the boundary, but it has an allowed identity", out)

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
        # one log, as in CI: the findings come whole, then the count (never cut into a finding's line)
        log = subprocess.run(command, cwd=self.repo, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        self.assertEqual(log.stdout.splitlines()[-1], "2 finding(s)")


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
