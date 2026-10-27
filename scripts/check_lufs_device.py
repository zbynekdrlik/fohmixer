#!/usr/bin/env python3
"""Keep the FohMixer LUFS device's source in shape (#151).

Spec docs/superpowers/specs/2026-10-26-lufs-design.md §4.2 and §9:

- the gen~ code embedded in `FohMixer LUFS.maxpat` equals the
  `fohmixer_lufs.genexpr` (the device has no file dependencies);
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
DEVICE = Path("live-device") / "FohMixer-LUFS"
GENEXPR = DEVICE / "fohmixer_lufs.genexpr"
MAXPAT = DEVICE / "FohMixer LUFS.maxpat"
VERSION_TXT = DEVICE / "version.txt"
PARAMETERS = ("Short-term", "Voice")
# A drive letter path (raw or JSON-escaped), a UNC path, a Unix home.
ABSOLUTE_PATH = re.compile(r"\b[A-Za-z]:(?:\\\\|\\|/)|\\\\\\\\|/(?:Users|home)/")
VISIBILITIES = (0, 1)


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
    code = (root / GENEXPR).read_text(encoding="utf-8")
    version = (root / VERSION_TXT).read_text(encoding="utf-8").strip()
    return patch_errors((root / MAXPAT).read_text(encoding="utf-8"), code, version)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="The LUFS device's patch, its gen~ code and its version."
    )
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args(argv)
    errors = check(args.root)
    for error in errors:
        print(f"::error::{error}")
    if errors:
        return 1
    print("FohMixer LUFS: the embedded gen~ code and the patch agree")
    return 0


if __name__ == "__main__":
    sys.exit(main())
