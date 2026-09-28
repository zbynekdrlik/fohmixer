"""Tests for scripts/check_integrity.py (adapted from iemmixer @ 22372bc).

The skip and focus fixtures are assembled from pieces (`SKIP`, `ONLY`, ...):
they are what the integrity scan must find, and written out whole they would
read as real skips to every other scanner of this repository.
"""
from __future__ import annotations

import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_integrity as ci  # noqa: E402

PINNED = "      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1\n"
SKIP, ONLY, FIXME, FAIL = ".skip", ".only", ".fixme", ".fail"
IGNORE = "#[" + "ignore]"
UNITTEST = "unittest"
PYTEST = "pytest"
KILL = "I7"


class IntegrityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.put("crates/a/src/lib.rs", "#[test]\nfn ok() { assert_eq!(1 + 1, 2); }\n")
        self.put("e2e/tests/a.spec.ts", 'test("ok", async ({ page }) => { await page.goto("/"); });\n')
        self.put("live-script/tests/test_a.py", "def test_ok():\n    assert 1 + 1 == 2\n")
        self.put(".github/workflows/ci.yml", "jobs:\n  a:\n    steps:\n" + PINNED)

    def tearDown(self) -> None:
        shutil.rmtree(self.root)

    def put(self, rel: str, text: str) -> None:
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def test_clean_tree(self) -> None:
        self.assertEqual(ci.violations(self.root), [])

    def test_ignored_rust_test(self) -> None:
        self.put("crates/a/src/lib.rs", "#[test]\n" + IGNORE + "\nfn skipped() {}\n")
        self.assertEqual(ci.violations(self.root), [f"crates/a/src/lib.rs:2: {IGNORE} test"])

    def test_skipped_or_focused_e2e(self) -> None:
        for body in (f'test{SKIP}("x", async () => {{}});', f'test{ONLY}("x", async () => {{}});',
                     f'test.describe{SKIP}("x", () => {{}});', f'test{FIXME}("x", async () => {{}});',
                     f'test{FAIL}("x", async () => {{}});', f'describe{ONLY}("x", () => {{}});'):
            self.put("e2e/tests/a.spec.ts", body + "\n")
            self.assertEqual(ci.violations(self.root), ["e2e/tests/a.spec.ts:1: skipped or focused E2E test"], body)

    def test_skipped_python_tests(self) -> None:
        for line in (f"@{UNITTEST}{SKIP}('later')", f"@{UNITTEST}{SKIP}If(True, 'x')",
                     f"@{UNITTEST}{SKIP}Unless(False, 'x')", f"        self{SKIP}Test('x')",
                     f"@{UNITTEST}.expectedFailure", f"@{PYTEST}.mark{SKIP}", f"@{PYTEST}.mark.xfail",
                     f"    {PYTEST}{SKIP}('x')", f"@skip{'If'}(True, 'x')"):
            for rel in ("live-script/tests/test_a.py", "sim/tests/test_b.py", "scripts/test_c.py",
                        "tools/import-tosc/test_d.py"):
                with tempfile.TemporaryDirectory() as d:
                    root = Path(d)
                    (root / rel).parent.mkdir(parents=True)
                    (root / rel).write_text(line + "\ndef test_x():\n    pass\n", encoding="utf-8")
                    self.assertEqual(ci.violations(root), [f"{rel}:1: skipped Python test"], (rel, line))

    def test_ordinary_python_words_pass(self) -> None:
        for line in ("skipped = 0", "def skip_blank(lines): return lines", "# a unittest suite",
                     "self.skipped_frames = 3", "expected_failure_count = 0"):
            self.put("live-script/FohMixer/a.py", line + "\n")
            self.assertEqual(ci.violations(self.root), [], line)

    def test_forbidden_workflow_constructs(self) -> None:
        for line in ("    continue-on-error: true\n", "    runs-on: [self-hosted, x]\n", "on: pull_request_target\n"):
            self.put(".github/workflows/ci.yml", "jobs:\n" + line + PINNED)
            self.assertEqual(ci.violations(self.root), [".github/workflows/ci.yml:2: forbidden workflow construct"], line)

    def test_unpinned_action(self) -> None:
        self.put(".github/workflows/ci.yml", "jobs:\n  a:\n    steps:\n      - uses: actions/checkout@v7\n")
        self.assertEqual(ci.violations(self.root),
                         [".github/workflows/ci.yml:4: action not pinned to a full commit SHA: actions/checkout@v7"])

    def test_a_pin_needs_its_version_comment(self) -> None:
        sha = "3d3c42e5aac5ba805825da76410c181273ba90b1"
        for line, ok in ((f"      - uses: actions/checkout@{sha} # v7.0.1\n", True),
                         (f"      - uses: actions/checkout@{sha}  #v7\n", True),
                         (f"      - uses: actions/checkout@{sha}\n", False),
                         (f"      - uses: actions/checkout@{sha} # latest\n", False),
                         (f"        uses: actions/checkout@{sha} # v7.0.1 pinned\n", False),
                         ("      - uses: ./.github/actions/local\n", True)):
            self.put(".github/workflows/ci.yml", "jobs:\n  a:\n    steps:\n" + line)
            want = [] if ok else [f".github/workflows/ci.yml:4: pinned action without its version comment (# vX.Y.Z): actions/checkout@{sha}"]
            self.assertEqual(ci.violations(self.root), want, line)

    def test_force_kill_command(self) -> None:
        self.put("scripts/stop.ps1", "taskkill /F /IM fohmixer-hub.exe\n")
        self.assertEqual(ci.violations(self.root), [f"scripts/stop.ps1:1: force-kill command (spec {KILL})"])

    def test_force_kill_words_in_comments_are_refused(self) -> None:
        # Code, scripts, the Live script and the workflows: prose included.
        cases = {
            "crates/fohmixer-hub/src/lib.rs": "/// never TerminateProcess the app\nfn f() {}\n",
            "scripts/pc/Pc.psm1": "# Stop-Process is never used\nfunction X { }\n",
            "live-script/FohMixer/a.py": "# os.kill is not how Live stops\nx = proc.kill()\n",
            "sim/Live/a.py": "p.terminate()\n",
            "tools/import-tosc/a.py": "p.terminate()\n",
            ".github/workflows/ci.yml": "jobs:\n  a:\n    steps:\n" + PINNED + "      # taskkill\n",
        }
        for rel, text in cases.items():
            with tempfile.TemporaryDirectory() as d:
                root = Path(d)
                (root / rel).parent.mkdir(parents=True)
                (root / rel).write_text(text, encoding="utf-8")
                line = {".yml": 5, ".py": 2 if rel.startswith("live-script") else 1}.get(Path(rel).suffix, 1)
                self.assertEqual(ci.violations(root), [f"{rel}:{line}: force-kill command (spec {KILL})"], rel)

    def test_force_end_verbs_of_rust_python_powershell_and_jobs_are_refused(self) -> None:
        for body in ("child.kill()", "child.kill ()", "child.start_kill()", "cmd.kill_on_drop(true)",
                     "TerminateJobObject(job, 1)", "JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE", "$p.Kill()",
                     "proc.terminate()", "Invoke-CimMethod -InputObject $p -MethodName Terminate",
                     "shutdown.exe /f /r", "nt::NtTerminateProcess(h, 0)",
                     "Invoke-WmiMethod -Path $w -Name Terminate", "wmic process where processid=1 call terminate",
                     "tskill 1234", "pskill -t live", "Get-Process x | ForEach-Object Kill", "Get-Process x | % Kill"):
            self.put("crates/a/src/lib.rs", f"fn f() {{ {body}; }}\n")
            self.assertEqual(ci.violations(self.root), [f"crates/a/src/lib.rs:1: force-kill command (spec {KILL})"], body)

    def test_graceful_stops_and_ordinary_words_pass(self) -> None:
        for body in ('Command::new("kill").args(["-s", "TERM", &pid])', "signal::kill(pid, Signal::SIGTERM)",
                     "self.killed = true", "skill(x)", "let force_ended = false", "console::ctrl_break(pid)",
                     "// the hub never force-ends a process", "a skill and a skills list", "the job Terminated",
                     "wmic os get caption", "delete the temp file", "ForEach-Object Killed", "$exit = 'terminated'"):
            self.put("crates/a/src/lib.rs", f"fn f() {{ {body}; }}\n")
            self.assertEqual(ci.violations(self.root), [], body)

    def test_node_modules_are_not_scanned(self) -> None:
        self.put("e2e/node_modules/pkg/index.js", "child.kill()\n")
        self.put("e2e/node_modules/pkg/a.spec.ts", f'test{ONLY}("x", async () => {{}});\n')
        self.assertEqual(ci.violations(self.root), [])

    def test_the_scan_reports_in_path_order(self) -> None:
        self.put("crates/b/src/lib.rs", "#[test]\n" + IGNORE + "\nfn skipped() {}\n")
        self.put("scripts/stop.cmd", "taskkill /im fohmixer-hub.exe\n")
        self.assertEqual(ci.violations(self.root), [
            f"crates/b/src/lib.rs:2: {IGNORE} test",
            f"scripts/stop.cmd:1: force-kill command (spec {KILL})",
        ])

    def test_this_repository_is_clean(self) -> None:
        self.assertEqual(ci.violations(Path(__file__).resolve().parent.parent), [])


if __name__ == "__main__":
    unittest.main()
