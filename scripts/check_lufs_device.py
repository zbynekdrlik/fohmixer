#!/usr/bin/env python3
"""Keep the FohMixer LUFS device's source equal to the reference DSP (#151).

Spec docs/superpowers/specs/2026-10-26-lufs-design.md §4.2 and §9:

- the constants table between `lufs-table begin` / `lufs-table end` in
  `crates/fohmixer-proto/src/lufs/dsp.rs` and in the device's
  `fohmixer_lufs.genexpr`: the same names with the same values;
- the gen~ code embedded in `FohMixer LUFS.maxpat` equals the `.genexpr` (the
  device has no file dependencies), and its rings are WINDOW_BLOCKS long;
- the patch: the two `live.numbox` parameters `Short-term` and `Voice`
  (float, -70..0, LUFS, the same visibility, which probe P1 decides), the
  version on its face equal to `version.txt`, and no absolute file path.

Exit 0 when all hold, else 1 with one `::error::` line per problem.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RUST = Path("crates") / "fohmixer-proto" / "src" / "lufs" / "dsp.rs"
DEVICE = Path("live-device") / "FohMixer-LUFS"
GENEXPR = DEVICE / "fohmixer_lufs.genexpr"
MAXPAT = DEVICE / "FohMixer LUFS.maxpat"
VERSION_TXT = DEVICE / "version.txt"
BEGIN, END = "lufs-table begin", "lufs-table end"
NUMBER = r"(-?\d+(?:\.\d+)?(?:[eE][-+]?\d+)?)"
RUST_CONST = re.compile(rf"^pub const (\w+): (?:f64|usize) = {NUMBER};$")
GEN_CONST = re.compile(rf"^(\w+) = {NUMBER};$")
GEN_DATA = re.compile(r"^Data (\w+)\((\d+)\);$")
PARAMETERS = ("Short-term", "Voice")
# A drive letter path (raw or JSON-escaped), a UNC path, a Unix home.
ABSOLUTE_PATH = re.compile(r"\b[A-Za-z]:(?:\\\\|\\|/)|\\\\\\\\|/(?:Users|home)/")
VISIBILITIES = (0, 1)


def table(text: str, pattern: re.Pattern[str], what: str) -> tuple[dict[str, float], list[str]]:
    """The constants between the markers, and the problems reading them."""
    lines = [line.strip() for line in text.splitlines()]
    begins = [i for i, line in enumerate(lines) if line.endswith(BEGIN)]
    ends = [i for i, line in enumerate(lines) if line.endswith(END)]
    if len(begins) != 1 or len(ends) != 1 or ends[0] < begins[0]:
        return {}, [f"{what}: needs exactly one '{BEGIN}' line before one '{END}' line"]
    found: dict[str, float] = {}
    errors: list[str] = []
    for line in lines[begins[0] + 1 : ends[0]]:
        match = pattern.match(line)
        if match is None:
            continue
        name, value = match[1], float(match[2])
        if name in found:
            errors.append(f"{what}: {name} is in the table twice")
        found[name] = value
    if not found:
        errors.append(f"{what}: the table is empty")
    return found, errors


def compare_tables(rust: dict[str, float], gen: dict[str, float]) -> list[str]:
    errors = [f"{GENEXPR.as_posix()} lacks {name}" for name in sorted(rust.keys() - gen.keys())]
    errors += [
        f"{GENEXPR.as_posix()} has {name}, which the Rust table lacks"
        for name in sorted(gen.keys() - rust.keys())
    ]
    errors += [
        f"{name}: {RUST.as_posix()} has {rust[name]!r}, {GENEXPR.as_posix()} {gen[name]!r}"
        for name in sorted(rust.keys() & gen.keys())
        if rust[name] != gen[name]
    ]
    return errors


def data_sizes(code: str, window: float | None) -> list[str]:
    sizes = {
        m[1]: int(m[2]) for m in (GEN_DATA.match(line.strip()) for line in code.splitlines()) if m
    }
    if not sizes:
        return [f"{GENEXPR.as_posix()}: no Data ring"]
    return [
        f"{GENEXPR.as_posix()}: Data {name}({size}) is not WINDOW_BLOCKS ({window})"
        for name, size in sorted(sizes.items())
        if size != window
    ]


def boxes(patcher: dict) -> list[dict]:
    """Every box of a patcher and of the patchers inside it."""
    found = []
    for item in patcher.get("boxes", []):
        box = item.get("box", {})
        found.append(box)
        if isinstance(box.get("patcher"), dict):
            found += boxes(box["patcher"])
    return found


def patch_errors(maxpat_text: str, code: str, version: str) -> list[str]:
    where = MAXPAT.as_posix()
    errors: list[str] = []
    if ABSOLUTE_PATH.search(maxpat_text):
        errors.append(f"{where} holds an absolute file path")
    try:
        patcher = json.loads(maxpat_text)["patcher"]
    except (ValueError, KeyError, TypeError) as error:
        return errors + [f"{where} is not a Max patcher: {error}"]
    every = boxes(patcher)
    gens = [b for b in boxes(patcher) if str(b.get("text", "")).split()[:1] == ["gen~"]]
    if len(gens) != 1 or not isinstance(gens[0].get("patcher"), dict):
        errors.append(f"{where}: needs exactly one gen~ with its patcher embedded")
    else:
        codeboxes = [b for b in boxes(gens[0]["patcher"]) if b.get("maxclass") == "codebox"]
        if len(codeboxes) != 1:
            errors.append(f"{where}: the gen~ needs exactly one codebox")
        elif codeboxes[0].get("code") != code:
            errors.append(f"{where}: the gen~ code differs from {GENEXPR.as_posix()}")
    names = {"plugin~", "plugout~"}
    present = {str(b.get("text", "")).split()[0] for b in every if str(b.get("text", "")).strip()}
    errors += [f"{where}: no {name}" for name in sorted(names - present)]
    numboxes = {b.get("varname"): b for b in every if b.get("maxclass") == "live.numbox"}
    if sorted(numboxes) != sorted(PARAMETERS):
        errors.append(
            f"{where}: the live.numbox parameters are {sorted(map(str, numboxes))}, "
            f"expected {list(PARAMETERS)}"
        )
    visibilities = set()
    for name in PARAMETERS:
        box = numboxes.get(name)
        if box is None:
            continue
        values = box.get("saved_attribute_attributes", {}).get("valueof", {})
        want = {
            "parameter_longname": name,
            "parameter_type": 0,
            "parameter_mmin": -70.0,
            "parameter_mmax": 0.0,
        }
        errors += [
            f"{where}: {name} has {key} {values.get(key)!r}, expected {value!r}"
            for key, value in want.items()
            if values.get(key) != value
        ]
        if "LUFS" not in str(values.get("parameter_units", "")):
            errors.append(f"{where}: {name}'s unit is not LUFS")
        visibilities.add(values.get("parameter_invisible", 0))
    if len(visibilities) > 1 or not visibilities <= set(VISIBILITIES):
        errors.append(
            f"{where}: the parameters' visibility must be one of {VISIBILITIES} "
            f"and the same on both, found {sorted(map(str, visibilities))}"
        )
    shown = [b.get("text") for b in every if b.get("varname") == "version"]
    if shown != [f"v{version}"]:
        errors.append(
            f"{where}: its face shows {shown}, expected ['v{version}'] ({VERSION_TXT.as_posix()})"
        )
    return errors


def check(root: Path) -> list[str]:
    rust, errors = table((root / RUST).read_text(encoding="utf-8"), RUST_CONST, RUST.as_posix())
    code = (root / GENEXPR).read_text(encoding="utf-8")
    gen, gen_errors = table(code, GEN_CONST, GENEXPR.as_posix())
    errors += gen_errors
    if rust and gen:
        errors += compare_tables(rust, gen)
    errors += data_sizes(code, rust.get("WINDOW_BLOCKS"))
    version = (root / VERSION_TXT).read_text(encoding="utf-8").strip()
    errors += patch_errors((root / MAXPAT).read_text(encoding="utf-8"), code, version)
    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="The LUFS device's source against the Rust reference."
    )
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args(argv)
    errors = check(args.root)
    for error in errors:
        print(f"::error::{error}")
    if errors:
        return 1
    print("FohMixer LUFS: the table, the embedded gen~ code and the patch agree")
    return 0


if __name__ == "__main__":
    sys.exit(main())
