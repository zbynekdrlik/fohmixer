"""Tests for the wasm stress run's legs and input checks (#43, scripts/wasm_stress_legs.py)."""
from __future__ import annotations

import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import wasm_stress_legs as wl  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent


def run(*argv: str) -> tuple[int, str]:
    """main's exit status and what it printed."""
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        status = wl.main(list(argv))
    return status, out.getvalue()


class SettingsTests(unittest.TestCase):
    def test_settings_are_jsc_words(self) -> None:
        self.assertEqual(wl.settings(""), [])
        self.assertEqual(wl.settings("  "), [])
        self.assertEqual(wl.settings("JSC_useOMGJIT=0"), ["JSC_useOMGJIT=0"])
        self.assertEqual(wl.settings(" JSC_a=1\tJSC_jitPolicyScale=0.1 JSC_b=x:y-z_w "),
                         ["JSC_a=1", "JSC_jitPolicyScale=0.1", "JSC_b=x:y-z_w"])

    def test_anything_a_shell_could_read_as_more_is_refused(self) -> None:
        for text in ("useOMGJIT=0", "FOO=1", "JSC_=1", "JSC_1a=1", "JSC_a", "JSC_a=", "JSC_a=1;b",
                     "JSC_a=$(id)", "JSC_a=`id`", "JSC_a=1&&b", "JSC_a=1|b", "JSC_a=*", "JSC_a=?",
                     "JSC_a=[x]", "JSC_a='1'", 'JSC_a="1"', "JSC_a=1>f", "JSC_a-b=1", "JSC_a=1=2",
                     "jsc_a=1", "JSC_a=1 LD_PRELOAD=x"):
            with self.assertRaises(ValueError, msg=text):
                wl.settings(text)

    def test_a_newline_only_separates_settings(self) -> None:
        # Whitespace (a newline too) splits words: each one is checked whole.
        self.assertEqual(wl.settings("JSC_a=1\nJSC_b=2"), ["JSC_a=1", "JSC_b=2"])
        with self.assertRaises(ValueError):
            wl.settings("JSC_a=1\nPATH=/tmp")

    def test_an_option_set_twice_is_refused(self) -> None:
        with self.assertRaisesRegex(ValueError, "JSC_a is set twice"):
            wl.settings("JSC_a=1 JSC_b=2 JSC_a=3")

    def test_loads_are_a_bounded_whole_number(self) -> None:
        self.assertEqual(wl.loads("1"), 1)
        self.assertEqual(wl.loads(" 1500 "), 1500)
        self.assertEqual(wl.loads(str(wl.MAX_LOADS)), wl.MAX_LOADS)
        for text in ("0", str(wl.MAX_LOADS + 1), "-5", "1.5", "1e3", "", "abc", "15 00"):
            with self.assertRaises(ValueError, msg=text):
                wl.loads(text)


class LegsTests(unittest.TestCase):
    def test_the_fixed_legs(self) -> None:
        matrix = wl.legs("")
        self.assertEqual([leg["leg"] for leg in matrix], [
            "baseline", "no-omg", "no-bbq", "no-wasm-osr", "no-concurrent-jit", "no-fast-memory",
            "early-tier-up"])
        by_leg = {leg["leg"]: leg["jsc"] for leg in matrix}
        self.assertEqual(by_leg["baseline"], "")
        self.assertEqual(by_leg["no-omg"], "JSC_useOMGJIT=0")
        self.assertEqual(by_leg["no-bbq"], "JSC_useBBQJIT=0")
        self.assertEqual(by_leg["no-wasm-osr"], "JSC_useWasmOSR=0")
        self.assertEqual(by_leg["no-concurrent-jit"], "JSC_useConcurrentJIT=0")
        self.assertEqual(by_leg["no-fast-memory"], "JSC_useWasmFastMemory=0")
        self.assertEqual(wl.settings(by_leg["early-tier-up"]), [
            "JSC_thresholdForBBQOptimizeAfterWarmUp=15", "JSC_thresholdForBBQOptimizeSoon=5",
            "JSC_thresholdForOMGOptimizeAfterWarmUp=5000", "JSC_thresholdForOMGOptimizeSoon=50"])

    def test_the_extra_settings_are_one_more_leg(self) -> None:
        matrix = wl.legs(" JSC_useWasmIPInt=0   JSC_freeRetiredWasmCode=0 ")
        self.assertEqual(len(matrix), len(wl.LEGS) + 1)
        self.assertEqual(matrix[-1], {"leg": "custom", "jsc": "JSC_useWasmIPInt=0 JSC_freeRetiredWasmCode=0"})
        with self.assertRaises(ValueError):
            wl.legs("JSC_a=1 rm")

    def test_the_cli_prints_the_matrix_and_refuses_bad_input(self) -> None:
        status, out = run("legs", "--loads", "1500")
        self.assertEqual(status, 0)
        self.assertEqual(json.loads(out), wl.legs(""))
        status, out = run("legs", "--loads", "1500", "--extra", "JSC_x=1")
        self.assertEqual((status, json.loads(out)[-1]["leg"]), (0, "custom"))
        status, out = run("legs", "--loads", "0")
        self.assertEqual(status, 1)
        self.assertTrue(out.startswith("::error::loads must be from 1"), out)
        status, out = run("legs", "--loads", "10", "--extra", "JSC_a=$(id)")
        self.assertEqual(status, 1)
        self.assertIn("::error::not a JSC_<option>=<value> setting", out)

    def test_env_prints_one_setting_a_line(self) -> None:
        self.assertEqual(run("env", ""), (0, ""))
        self.assertEqual(run("env", "JSC_a=1 JSC_b=2"), (0, "JSC_a=1\nJSC_b=2\n"))
        status, out = run("env", "JSC_a=1 PATH=/x")
        self.assertEqual(status, 1)
        self.assertNotIn("JSC_a=1\n", out)


class SummaryTests(unittest.TestCase):
    # The shape e2e/stress/wasm-load.spec.ts writes: the counts, and the first faults of each kind.
    REPORT = {
        "leg": "baseline", "jsc": {}, "webkit": "26.0", "loads_requested": 1500, "loads": 1500,
        "finished": True, "ms_per_load": 1180, "strips": {"min": 28, "max": 28},
        "counts": {"traps": 2, "page_errors": 250, "console": 3, "crashes": 1, "stalls": 4},
        "traps": [{"load": 734, "ms": 212, "name": "RuntimeError",
                   "message": "access to a null reference (evaluating 'wasm.x()')\nmore", "stack": "s"},
                  {"load": 1201, "ms": 230, "name": "RuntimeError", "message": "", "stack": ""}],
        "page_errors": [{"load": 9}], "console": [], "crashes": [400], "stalls": [],
        "page_process_jsc": [{}],
    }

    def test_the_rate(self) -> None:
        self.assertEqual(wl.rate(0, 1500), "0 in 1500")
        self.assertEqual(wl.rate(2, 1500), "1 in 750")
        self.assertEqual(wl.rate(3, 1000), "1 in 333")

    def test_the_summary_lists_the_counts_and_each_trap(self) -> None:
        text = wl.summary(self.REPORT, "baseline")
        self.assertIn("### wasm stress: baseline", text)
        # The counts, not the kept lists' lengths (250 page errors, 1 kept).
        self.assertIn("| `none` | 1500 of 1500 | 2 | 1 in 750 | 250 | 3 | 1 | 4 | 1180 | 28-28 |", text)
        self.assertIn("- load 734, 212 ms after goto: RuntimeError: access to a null reference (evaluating 'wasm.x()')\n",
                      text)
        self.assertIn("- load 1201, 230 ms after goto: RuntimeError: \n", text)

    def test_an_unfinished_leg_and_its_options_are_shown(self) -> None:
        report = dict(self.REPORT, jsc={"JSC_useOMGJIT": "0", "JSC_a": "1"}, loads=600, finished=False,
                      counts={"traps": 0}, traps=[], strips={"min": None, "max": None}, ms_per_load=None)
        text = wl.summary(report, "custom")
        self.assertIn("| `JSC_a=1 JSC_useOMGJIT=0` | 600 of 1500 (unfinished) | 0 | 0 in 600 | 0 | 0 | 0 | 0 "
                      "| ? | ?-? |", text)
        self.assertNotIn("- load", text)

    def test_the_cli_reads_the_report_and_fails_without_one(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "report.json"
            path.write_text(json.dumps(self.REPORT), encoding="utf-8")
            status, out = run("summary", str(path), "--leg", "baseline")
            self.assertEqual((status, out), (0, wl.summary(self.REPORT, "baseline")))
            status, out = run("summary", str(Path(d) / "missing.json"), "--leg", "baseline")
            self.assertEqual(status, 1)
            self.assertTrue(out.startswith("::error::no readable report at"), out)
            path.write_text("{", encoding="utf-8")
            self.assertEqual(run("summary", str(path), "--leg", "baseline")[0], 1)


class WorkflowTests(unittest.TestCase):
    def test_the_workflow_runs_this_script_and_the_stress_config(self) -> None:
        text = (ROOT / ".github" / "workflows" / "wasm-stress.yml").read_text(encoding="utf-8")
        self.assertIn("scripts/wasm_stress_legs.py legs", text)
        self.assertIn("wasm_stress_legs.py env", text)
        self.assertIn("--config playwright.stress.config.ts", text)
        self.assertIn("workflow_dispatch:", text)
        self.assertNotIn("push:", text)
        self.assertNotIn("pull_request", text)
        # The normal CI never runs the stress specs.
        self.assertNotIn("stress", (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
