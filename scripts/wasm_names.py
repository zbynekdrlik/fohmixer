#!/usr/bin/env python3
"""The function names of a WebAssembly module (#43).

WebKit on the CI runner traps now and then inside a wasm-bindgen closure at
page load (`access to a null reference`, `unreachable`; three times by
0.1.0-dev.45, `.claude/rules/e2e.md`). A stripped build names only
wasm-bindgen's adapters, so a trace's `wasm-function[N]` frames could not be
named. The `wasm` job now builds the UI with the name section kept
(`CARGO_PROFILE_RELEASE_STRIP=debuginfo`) and checks it with this script; the
same script names a trace's frames from the run's `wasm-dist` artifact. The
names more than double the module (1.6 to 3.3 MB), so the shipped bundle
leaves them out (`strip`, in the `bundle` job): the code and every function's
index stay the same, so a frame of the shipped module is named from the
`wasm-dist` of the run that built it.

    wasm_names.py check <module.wasm> --min N --name SUBSTRING
        exit 0 when the module names at least N functions and one name holds
        SUBSTRING, else 1 (2: not a module)
    wasm_names.py lookup <module.wasm> N [N ...]
        prints `N name` for each index (`?` when unnamed)
    wasm_names.py strip <module.wasm or a glob of one> [--to PATH]
        the module without its name section, in place or at PATH
"""

import argparse
import glob
import os
import sys

MAGIC = b"\0asm"
NAME_SECTION = "name"
FUNCTION_NAMES = 1


class NotAModule(ValueError):
    """The bytes are no WebAssembly module, or a section runs past the end."""


def leb128(data: bytes, at: int) -> tuple[int, int]:
    """An unsigned LEB128 number at `at`: (value, the index after it)."""
    value = 0
    # A u32 takes at most 5 bytes; the input may end first.
    for shift, index in zip(range(0, 35, 7), range(at, len(data)), strict=False):
        byte = data[index]
        value |= (byte & 0x7F) << shift
        if byte < 0x80:
            return value, index + 1
    raise NotAModule(f"a number at {at} does not end")


def sections(module: bytes):
    """Each section as (its first byte's index, its payload's start, its end,
    whether it is the name section)."""
    if module[:4] != MAGIC or len(module) < 8:
        raise NotAModule("no WebAssembly magic")
    at = 8
    while at < len(module):
        kind = module[at]
        size, start = leb128(module, at + 1)
        end = start + size
        if end > len(module):
            raise NotAModule(f"section {kind} at {at} runs past the end")
        named = False
        if kind == 0:
            length, text = leb128(module, start)
            named = module[text : text + length] == NAME_SECTION.encode()
        yield at, start, end, named
        at = end


def function_names(module: bytes) -> dict[int, str]:
    """The function names of the module's name section (none without one)."""
    names: dict[int, str] = {}
    for _at, start, end, named in sections(module):
        if named:
            length, text = leb128(module, start)
            names.update(name_subsections(module, text + length, end))
    return names


def name_subsections(module: bytes, at: int, end: int) -> dict[int, str]:
    """The function names subsection of a name section's payload."""
    names: dict[int, str] = {}
    while at < end:
        kind = module[at]
        size, start = leb128(module, at + 1)
        if kind == FUNCTION_NAMES:
            count, pos = leb128(module, start)
            for _ in range(count):
                index, pos = leb128(module, pos)
                length, pos = leb128(module, pos)
                names[index] = module[pos : pos + length].decode("utf-8", "replace")
                pos += length
        at = start + size
    return names


def strip_names(module: bytes) -> bytes:
    """The module without its name section; every other byte as it was."""
    kept = [module[at:end] for at, _start, end, named in sections(module) if not named]
    return module[:8] + b"".join(kept)


def check(names: dict[int, str], minimum: int, needle: str) -> str | None:
    """Why the names fall short, or None when they do not."""
    if len(names) < minimum:
        return f"{len(names)} function names, want at least {minimum}"
    if not any(needle in name for name in names.values()):
        sample = [names[i] for i in sorted(names)[:: max(1, len(names) // 5)][:5]]
        return f"no function name holds {needle!r} (some: {sample})"
    return None


def one_path(pattern: str) -> str:
    """The one file `pattern` names (a path, or a glob of exactly one)."""
    paths = sorted(glob.glob(pattern))
    if len(paths) != 1:
        raise NotAModule(f"{pattern!r} matches {len(paths)} files, want one")
    return paths[0]


def strip_file(source: str, target: str) -> str:
    """Writes `source` without its names to `target` (through a temporary
    file beside it); the line to log."""
    with open(source, "rb") as f:
        module = f.read()
    stripped = strip_names(module)
    if function_names(stripped):
        raise NotAModule("names left after the strip")
    temporary = target + ".tmp"
    with open(temporary, "wb") as f:
        f.write(stripped)
    os.replace(temporary, target)
    return f"{source}: {len(module)} bytes, {len(stripped)} without the names, at {target}"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    checker = sub.add_parser("check")
    checker.add_argument("module")
    checker.add_argument("--min", type=int, required=True)
    checker.add_argument("--name", required=True)
    looker = sub.add_parser("lookup")
    looker.add_argument("module")
    looker.add_argument("index", type=int, nargs="+")
    stripper = sub.add_parser("strip")
    stripper.add_argument("module")
    stripper.add_argument("--to")
    args = parser.parse_args(argv)
    try:
        if args.command == "strip":
            source = one_path(args.module)
            print(f"wasm_names: {strip_file(source, args.to or source)}")
            return 0
        with open(args.module, "rb") as f:
            names = function_names(f.read())
    except NotAModule as e:
        print(f"wasm_names: {args.module}: {e}", file=sys.stderr)
        return 2
    if args.command == "lookup":
        for index in args.index:
            print(index, names.get(index, "?"))
        return 0
    why = check(names, args.min, args.name)
    if why is not None:
        print(f"wasm_names: {args.module}: {why}", file=sys.stderr)
        return 1
    print(f"wasm_names: {args.module}: {len(names)} function names")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
