#!/usr/bin/env python3
"""The function names of a WebAssembly module (#43).

WebKit on the CI runner traps now and then inside a wasm-bindgen closure at
page load (`access to a null reference`, `unreachable`; three times by
0.1.0-dev.45, `.claude/rules/e2e.md`). A stripped build names only
wasm-bindgen's adapters, so a trace's `wasm-function[N]` frames could not be
named. The `wasm` job now builds the UI with the name section kept
(`CARGO_PROFILE_RELEASE_STRIP=debuginfo`) and checks it with this script; the
same script names a trace's frames from the run's `wasm-dist` artifact.

    wasm_names.py check <module.wasm> --min N --name SUBSTRING
        exit 0 when the module names at least N functions and one name holds
        SUBSTRING, else 1 (2: not a module)
    wasm_names.py lookup <module.wasm> N [N ...]
        prints `N name` for each index (`?` when unnamed)
"""

import argparse
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


def function_names(module: bytes) -> dict[int, str]:
    """The function names of the module's name section (none without one)."""
    if module[:4] != MAGIC or len(module) < 8:
        raise NotAModule("no WebAssembly magic")
    names: dict[int, str] = {}
    at = 8
    while at < len(module):
        section = module[at]
        size, start = leb128(module, at + 1)
        end = start + size
        if end > len(module):
            raise NotAModule(f"section {section} at {at} runs past the end")
        if section == 0:
            length, text = leb128(module, start)
            if module[text : text + length].decode("utf-8") == NAME_SECTION:
                names.update(name_subsections(module, text + length, end))
        at = end
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


def check(names: dict[int, str], minimum: int, needle: str) -> str | None:
    """Why the names fall short, or None when they do not."""
    if len(names) < minimum:
        return f"{len(names)} function names, want at least {minimum}"
    if not any(needle in name for name in names.values()):
        return f"no function name holds {needle!r}"
    return None


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
    args = parser.parse_args(argv)
    with open(args.module, "rb") as f:
        module = f.read()
    try:
        names = function_names(module)
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
