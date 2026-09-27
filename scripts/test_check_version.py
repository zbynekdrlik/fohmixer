"""Tests for scripts/check_version.py (adapted from iemmixer @ 22372bc)."""
from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_version as cv  # noqa: E402

CRATE = '[package]\nname = "{name}"\nversion.workspace = true\n'


def write_tree(root: Path, version: str, lock_versions: dict[str, str] | None = None,
               script_version: str | None = None) -> None:
    (root / "Cargo.toml").write_text(
        f'[workspace]\nmembers = []\n\n[workspace.package]\nversion = "{version}"\n', encoding="utf-8")
    for name in cv.CRATES:
        (root / "crates" / name).mkdir(parents=True, exist_ok=True)
        (root / "crates" / name / "Cargo.toml").write_text(CRATE.format(name=name), encoding="utf-8")
    locks = lock_versions or {name: version for name in cv.CRATES}
    lock = "version = 4\n" + "".join(f'\n[[package]]\nname = "{n}"\nversion = "{v}"\n' for n, v in locks.items())
    (root / "Cargo.lock").write_text(lock, encoding="utf-8")
    script = root / "live-script" / "FohMixer" / "version.py"
    script.parent.mkdir(parents=True, exist_ok=True)
    script.write_text(f'"""The script version."""\n\nVERSION = "{script_version or version}"\n', encoding="utf-8")


def git(root: Path, *args: str) -> None:
    subprocess.run(["git", "-C", str(root), *args], check=True, capture_output=True)


def init_repo(root: Path) -> None:
    for args in (["init", "-q", "-b", "master"], ["config", "user.email", "t@example.org"],
                 ["config", "user.name", "t"], ["config", "commit.gpgsign", "false"]):
        git(root, *args)


class CompareTests(unittest.TestCase):
    def test_semver_precedence(self) -> None:
        self.assertGreater(cv.compare("0.1.0-dev.2", "0.1.0-dev.1"), 0)
        self.assertGreater(cv.compare("0.1.0-dev.10", "0.1.0-dev.9"), 0)
        self.assertGreater(cv.compare("0.1.0", "0.1.0-dev.5"), 0)
        self.assertGreater(cv.compare("0.1.1-dev.0", "0.1.0"), 0)
        self.assertGreater(cv.compare("0.1.0-dev.1.1", "0.1.0-dev.1"), 0)
        self.assertGreater(cv.compare("0.1.0-dev.a", "0.1.0-dev.1"), 0)
        self.assertGreater(cv.compare("0.1.0-dev.1", "0.0.0"), 0)
        self.assertEqual(cv.compare("0.1.0-dev.3", "0.1.0-dev.3"), 0)
        self.assertLess(cv.compare("0.0.9", "0.1.0-dev.0"), 0)

    def test_rejects_non_semver(self) -> None:
        with self.assertRaises(ValueError):
            cv.parse("0.1")


class ConsistencyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())

    def tearDown(self) -> None:
        shutil.rmtree(self.root)

    def test_consistent_tree_passes(self) -> None:
        write_tree(self.root, "0.1.0-dev.1")
        self.assertEqual(cv.consistency_errors(self.root), [])

    def test_crate_with_its_own_version_fails(self) -> None:
        write_tree(self.root, "0.1.0-dev.1")
        (self.root / "crates" / "fohmixer-ui" / "Cargo.toml").write_text(
            '[package]\nname = "fohmixer-ui"\nversion = "1.0.0"\n', encoding="utf-8")
        self.assertEqual(cv.consistency_errors(self.root),
                         ["crates/fohmixer-ui/Cargo.toml must use version.workspace = true"])

    def test_stale_lockfile_fails(self) -> None:
        write_tree(self.root, "0.1.0-dev.2", lock_versions={n: "0.1.0-dev.1" for n in cv.CRATES})
        self.assertEqual(len(cv.consistency_errors(self.root)), len(cv.CRATES))

    def test_live_script_version_must_match(self) -> None:
        write_tree(self.root, "0.1.0-dev.2", script_version="0.1.0-dev.1")
        self.assertEqual(cv.consistency_errors(self.root), [
            "live-script/FohMixer/version.py has VERSION 0.1.0-dev.1, expected 0.1.0-dev.2"])

    def test_live_script_without_a_version_fails(self) -> None:
        write_tree(self.root, "0.1.0-dev.1")
        (self.root / "live-script" / "FohMixer" / "version.py").write_text("NAME = 'x'\n", encoding="utf-8")
        self.assertEqual(cv.consistency_errors(self.root), [
            "live-script/FohMixer/version.py has VERSION None, expected 0.1.0-dev.1"])

    def test_script_version_reads_the_string_assignment(self) -> None:
        self.assertEqual(cv.script_version('"""Doc."""\n\nVERSION = "1.2.3"\n'), "1.2.3")
        self.assertEqual(cv.script_version('VERSION: str = "1.2.4"\n'), "1.2.4")
        self.assertIsNone(cv.script_version("VERSION = 3\n"))
        self.assertIsNone(cv.script_version("OTHER = '1.2.3'\n"))


class BaseRefTests(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        init_repo(self.root)
        write_tree(self.root, "0.1.0-dev.0")
        git(self.root, "add", "-A")
        git(self.root, "commit", "-qm", "base")

    def tearDown(self) -> None:
        shutil.rmtree(self.root)

    def test_bumped_head_passes(self) -> None:
        write_tree(self.root, "0.1.0-dev.1")
        self.assertEqual(cv.main(["--root", str(self.root), "--base-ref", "master"]), 0)

    def test_unbumped_head_fails(self) -> None:
        self.assertEqual(cv.main(["--root", str(self.root), "--base-ref", "master"]), 1)

    def test_a_lower_head_fails(self) -> None:
        write_tree(self.root, "0.0.9")
        self.assertEqual(cv.main(["--root", str(self.root), "--base-ref", "master"]), 1)

    def test_an_unknown_base_ref_is_an_error(self) -> None:
        write_tree(self.root, "0.1.0-dev.1")
        with self.assertRaises(subprocess.CalledProcessError):
            cv.main(["--root", str(self.root), "--base-ref", "no-such-branch"])


class MissingBaseTests(unittest.TestCase):
    """The first PR: the base branch has no Cargo.toml yet (S0)."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        init_repo(self.root)
        (self.root / "README.md").write_text("docs only\n", encoding="utf-8")
        git(self.root, "add", "-A")
        git(self.root, "commit", "-qm", "docs only")

    def tearDown(self) -> None:
        shutil.rmtree(self.root)

    def test_a_base_without_cargo_toml_counts_as_0_0_0(self) -> None:
        self.assertEqual(cv.base_version(self.root, "master"), "0.0.0")

    def test_the_first_version_passes_against_a_base_without_cargo_toml(self) -> None:
        write_tree(self.root, "0.1.0-dev.1")
        self.assertEqual(cv.main(["--root", str(self.root), "--base-ref", "master"]), 0)

    def test_0_0_0_does_not_pass_against_a_missing_base(self) -> None:
        write_tree(self.root, "0.0.0")
        self.assertEqual(cv.main(["--root", str(self.root), "--base-ref", "master"]), 1)


class WorkspaceMembersTests(unittest.TestCase):
    def test_crates_list_matches_the_workspace_members(self) -> None:
        root = Path(__file__).resolve().parent.parent
        members = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["members"]
        self.assertEqual(sorted(m.split("/")[-1] for m in members), sorted(cv.CRATES))

    def test_this_tree_is_consistent(self) -> None:
        root = Path(__file__).resolve().parent.parent
        self.assertEqual(cv.consistency_errors(root), [])


if __name__ == "__main__":
    unittest.main()
