#!/usr/bin/env python3
"""Version gate: one version for the whole workspace; on PRs to master the head
version must be greater than master's (SemVer 2.0 precedence).

Adapted from iemmixer @ 22372bc. Single source: [workspace.package].version in
Cargo.toml. Every crate uses `version.workspace = true`, Cargo.lock records
that version for every workspace package, and the FohMixer Live script
carries the same string in live-script/FohMixer/version.py (Live loads the
script without Cargo). A base branch without a Cargo.toml (the first PR)
counts as version 0.0.0.
"""
from __future__ import annotations

import argparse
import ast
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATES = ["fohmixer-proto", "fohmixer-hub", "fohmixer-ui", "fohmixer-tray"]
SCRIPT_VERSION = Path("live-script") / "FohMixer" / "version.py"
MISSING_BASE_VERSION = "0.0.0"
SEMVER = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z.-]+))?$")


def parse(version: str) -> tuple[tuple[int, int, int], list[str] | None]:
    match = SEMVER.match(version)
    if not match:
        raise ValueError(f"not a SemVer version: {version!r}")
    core = (int(match[1]), int(match[2]), int(match[3]))
    return core, (match[4].split(".") if match[4] else None)


def compare(a: str, b: str) -> int:
    (core_a, pre_a), (core_b, pre_b) = parse(a), parse(b)
    if core_a != core_b:
        return -1 if core_a < core_b else 1
    if pre_a == pre_b:
        return 0
    if pre_a is None:
        return 1
    if pre_b is None:
        return -1
    for x, y in zip(pre_a, pre_b, strict=False):
        if x == y:
            continue
        x_num, y_num = x.isdigit(), y.isdigit()
        if x_num and y_num:
            return -1 if int(x) < int(y) else 1
        if x_num != y_num:
            return -1 if x_num else 1
        return -1 if x < y else 1
    return -1 if len(pre_a) < len(pre_b) else 1


def workspace_version(cargo_toml: str) -> str:
    return tomllib.loads(cargo_toml)["workspace"]["package"]["version"]


def script_version(source: str) -> str | None:
    """The string assigned to the module-level `VERSION` in `source`, if any."""
    for node in ast.parse(source).body:
        if isinstance(node, ast.Assign):
            targets, value = node.targets, node.value
        elif isinstance(node, ast.AnnAssign) and node.value is not None:
            targets, value = [node.target], node.value
        else:
            continue
        names = [t.id for t in targets if isinstance(t, ast.Name)]
        if "VERSION" in names and isinstance(value, ast.Constant) and isinstance(value.value, str):
            return value.value
    return None


def consistency_errors(root: Path) -> list[str]:
    errors: list[str] = []
    version = workspace_version((root / "Cargo.toml").read_text(encoding="utf-8"))
    parse(version)
    for crate in CRATES:
        manifest = tomllib.loads((root / "crates" / crate / "Cargo.toml").read_text(encoding="utf-8"))
        if manifest["package"].get("version") != {"workspace": True}:
            errors.append(f"crates/{crate}/Cargo.toml must use version.workspace = true")
    lock = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))
    locked = {package["name"]: package["version"] for package in lock["package"] if package["name"] in CRATES}
    for crate in CRATES:
        if locked.get(crate) != version:
            errors.append(f"Cargo.lock has {crate} {locked.get(crate)}, expected {version}")
    script = script_version((root / SCRIPT_VERSION).read_text(encoding="utf-8"))
    if script != version:
        errors.append(f"{SCRIPT_VERSION.as_posix()} has VERSION {script}, expected {version}")
    return errors


def base_version(root: Path, base_ref: str) -> str:
    """The workspace version at `base_ref`; 0.0.0 when it has no Cargo.toml.

    An unknown ref is an error (CalledProcessError), never a pass.
    """
    subprocess.run(["git", "-C", str(root), "rev-parse", "--verify", "--quiet", f"{base_ref}^{{commit}}"],
                   check=True, capture_output=True, text=True)
    exists = subprocess.run(["git", "-C", str(root), "cat-file", "-e", f"{base_ref}:Cargo.toml"],
                            capture_output=True, text=True)
    if exists.returncode != 0:
        print(f"{base_ref} has no Cargo.toml: its version counts as {MISSING_BASE_VERSION}")
        return MISSING_BASE_VERSION
    base_text = subprocess.run(["git", "-C", str(root), "show", f"{base_ref}:Cargo.toml"],
                               check=True, capture_output=True, text=True).stdout
    return workspace_version(base_text)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Workspace version gate.")
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--base-ref", help="e.g. origin/master: the head version must be greater")
    args = parser.parse_args(argv)
    errors = consistency_errors(args.root)
    head = workspace_version((args.root / "Cargo.toml").read_text(encoding="utf-8"))
    if args.base_ref:
        base = base_version(args.root, args.base_ref)
        if compare(head, base) <= 0:
            errors.append(f"version {head} must be greater than {args.base_ref} ({base}): "
                          "bump [workspace.package].version first")
        else:
            print(f"version bump OK: {base} -> {head}")
    for error in errors:
        print(f"::error::{error}")
    if errors:
        return 1
    print(f"version {head}: consistent")
    return 0


if __name__ == "__main__":
    sys.exit(main())
