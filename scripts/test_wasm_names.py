"""Tests of wasm_names.py (#43): the function names of a module's name section."""

import io
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import wasm_names  # noqa: E402


def leb(value: int) -> bytes:
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def text(s: str) -> bytes:
    raw = s.encode("utf-8")
    return leb(len(raw)) + raw


def section(kind: int, payload: bytes) -> bytes:
    return bytes([kind]) + leb(len(payload)) + payload


def module(names: dict[int, str], with_module_name: bool = True) -> bytes:
    """A module of a type section, a name section (a module name subsection
    first, then the function names) and a producers section."""
    entries = b"".join(leb(i) + text(n) for i, n in sorted(names.items()))
    subsections = b""
    if with_module_name:
        subsections += section(0, text("app"))
    subsections += section(1, leb(len(names)) + entries)
    return (
        wasm_names.MAGIC
        + b"\x01\0\0\0"
        + section(1, b"\x00")
        + section(0, text("name") + subsections)
        + section(0, text("producers") + b"\x00")
    )


class FunctionNames(unittest.TestCase):
    def test_the_names_by_index_past_other_sections(self):
        names = {0: "main", 300: "fohmixer_ui::app::App", 70000: "x"}
        self.assertEqual(wasm_names.function_names(module(names)), names)
        self.assertEqual(wasm_names.function_names(module(names, with_module_name=False)), names)

    def test_a_module_without_a_name_section_names_nothing(self):
        bare = wasm_names.MAGIC + b"\x01\0\0\0" + section(1, b"\x00")
        self.assertEqual(wasm_names.function_names(bare), {})

    def test_bytes_that_are_no_module_are_refused(self):
        with self.assertRaises(wasm_names.NotAModule):
            wasm_names.function_names(b"\x7fELF\x02\x01\x01\x00")
        cut = module({0: "main"})[:-3]
        with self.assertRaises(wasm_names.NotAModule):
            wasm_names.function_names(cut)
        with self.assertRaises(wasm_names.NotAModule):
            wasm_names.leb128(b"\x80\x80", 0)

    def test_a_number_ends_at_its_first_byte_below_0x80(self):
        self.assertEqual(wasm_names.leb128(b"\xe5\x8e\x26\x01", 0), (624485, 3))
        self.assertEqual(wasm_names.leb128(b"\x00\x7f", 1), (127, 2))


class Check(unittest.TestCase):
    def test_enough_names_with_the_crates_own(self):
        names = {i: f"fohmixer_ui::f{i}" for i in range(3)}
        self.assertIsNone(wasm_names.check(names, 3, "fohmixer_ui::"))
        self.assertEqual(
            wasm_names.check(names, 4, "fohmixer_ui::"),
            "3 function names, want at least 4",
        )
        self.assertEqual(
            wasm_names.check({0: "core::fmt"}, 1, "fohmixer_ui::"),
            "no function name holds 'fohmixer_ui::'",
        )


class Main(unittest.TestCase):
    def run_main(self, argv):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            code = wasm_names.main(argv)
        return code, out.getvalue(), err.getvalue()

    def write(self, data: bytes) -> str:
        f = tempfile.NamedTemporaryFile(suffix=".wasm", delete=False)
        f.write(data)
        f.close()
        self.addCleanup(os.unlink, f.name)
        return f.name

    def test_check_and_lookup(self):
        path = self.write(module({0: "main", 2: "fohmixer_ui::app::App"}))
        code, out, _ = self.run_main(["check", path, "--min", "2", "--name", "fohmixer_ui::"])
        self.assertEqual(code, 0)
        self.assertIn("2 function names", out)
        code, _, err = self.run_main(["check", path, "--min", "3", "--name", "fohmixer_ui::"])
        self.assertEqual(code, 1)
        self.assertIn("want at least 3", err)
        code, out, _ = self.run_main(["lookup", path, "2", "1"])
        self.assertEqual(code, 0)
        self.assertEqual(out, "2 fohmixer_ui::app::App\n1 ?\n")

    def test_no_module_exits_2(self):
        path = self.write(b"not wasm")
        code, _, err = self.run_main(["check", path, "--min", "1", "--name", "x"])
        self.assertEqual(code, 2)
        self.assertIn("no WebAssembly magic", err)


if __name__ == "__main__":
    unittest.main()
