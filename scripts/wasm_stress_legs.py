#!/usr/bin/env python3
"""The legs of the on-demand WebKit wasm stress run (#43, `.github/workflows/wasm-stress.yml`).

Each leg runs `e2e/stress/` (Playwright's WebKit, the iPad project) under one
set of JavaScriptCore options, given as `JSC_<option>=<value>` environment
variables (JavaScriptCore reads them at start on Linux; the names are its
`OptionsList.h`'s). The settings come from a workflow input too, so they are
checked here before any shell sees them: only `JSC_<letters and digits>=<value>`
words, the value of letters, digits and `. _ : -`.

    python3 scripts/wasm_stress_legs.py legs --loads N [--extra "JSC_a=1 JSC_b=2"]
        the matrix (JSON): the fixed legs, and a `custom` leg of --extra when given
    python3 scripts/wasm_stress_legs.py env "JSC_a=1 JSC_b=2"
        one `JSC_<option>=<value>` per line (nothing for no settings)
    python3 scripts/wasm_stress_legs.py summary <report.json> --leg NAME
        the leg's numbers as Markdown, from the report the stress spec writes

Exit 1 with an `::error::` line on a setting or a load count it refuses, or on
a missing or unreadable report.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

# One leg per tier or mechanism the trap could come from (#43 design comment).
LEGS: tuple[tuple[str, str], ...] = (
    ("baseline", ""),
    ("no-omg", "JSC_useOMGJIT=0"),
    ("no-bbq", "JSC_useBBQJIT=0"),
    ("no-wasm-osr", "JSC_useWasmOSR=0"),
    # The wasm tier-up plans wait for their compile (WasmOperations.cpp).
    ("no-concurrent-jit", "JSC_useConcurrentJIT=0"),
    ("no-fast-memory", "JSC_useWasmFastMemory=0"),
    # Ten times earlier tier-up to BBQ and to OMG: more tier-ups during the
    # strips' first render (the defaults: 150, 50, 50000, 500).
    (
        "early-tier-up",
        "JSC_thresholdForBBQOptimizeAfterWarmUp=15 JSC_thresholdForBBQOptimizeSoon=5 "
        "JSC_thresholdForOMGOptimizeAfterWarmUp=5000 JSC_thresholdForOMGOptimizeSoon=50",
    ),
)
CUSTOM = "custom"
SETTING = re.compile(r"JSC_[A-Za-z][A-Za-z0-9]*=[A-Za-z0-9._:-]+")
# A leg runs at most ~4 s a load inside a 300-minute job (wasm-stress.yml).
MAX_LOADS = 4000


def settings(text: str) -> list[str]:
    """The `JSC_<option>=<value>` words of `text` (whitespace-separated), each
    checked; ValueError names the first one refused, or an option set twice."""
    words = text.split()
    seen: set[str] = set()
    for word in words:
        if not SETTING.fullmatch(word):
            raise ValueError(f"not a JSC_<option>=<value> setting: {word!r}")
        name = word.split("=", 1)[0]
        if name in seen:
            raise ValueError(f"{name} is set twice")
        seen.add(name)
    return words


def loads(text: str) -> int:
    """A load count: a whole number from 1 to MAX_LOADS."""
    if not re.fullmatch(r"[0-9]+", text.strip()):
        raise ValueError(f"loads must be a whole number, not {text!r}")
    count = int(text)
    if not 1 <= count <= MAX_LOADS:
        raise ValueError(f"loads must be from 1 to {MAX_LOADS}, not {count}")
    return count


def legs(extra: str, only: bool = False) -> list[dict[str, str]]:
    """The matrix: the fixed legs (none with `only`), then the custom legs of
    `extra`: its `;`-separated groups of settings, `custom` for one group,
    `custom-1`, `custom-2`, ... for several (#43: an amplifier with each tier
    off, without the fixed legs again). `only` with no custom leg is refused."""
    groups = [settings(part) for part in extra.split(";")]
    customs = [" ".join(group) for group in groups if group]
    if only and not customs:
        raise ValueError("only the custom legs, but the jsc input has none")
    matrix = [] if only else [{"leg": name, "jsc": " ".join(settings(jsc))} for name, jsc in LEGS]
    if len(customs) == 1:
        matrix.append({"leg": CUSTOM, "jsc": customs[0]})
    else:
        matrix.extend({"leg": f"{CUSTOM}-{i}", "jsc": jsc} for i, jsc in enumerate(customs, 1))
    return matrix


def rate(traps: int, loads_done: int) -> str:
    """`1 in N` loads (rounded), `0 in N` when none trapped."""
    if traps == 0:
        return f"0 in {loads_done}"
    return f"1 in {round(loads_done / traps)}"


def shown(value: object) -> str:
    """A report value for the table: `?` for one not measured (null)."""
    return "?" if value is None else str(value)


def summary(report: dict, leg: str) -> str:
    """The leg's numbers as Markdown: a table row (the report's `counts`: its
    lists keep only the first faults of a kind), then each trap's load and message."""
    counts = report.get("counts", {})
    strips = report.get("strips", {})
    done = report.get("loads", 0)
    traps = counts.get("traps", 0)
    jsc = " ".join(f"{k}={v}" for k, v in sorted(report.get("jsc", {}).items())) or "none"
    finished = "" if report.get("finished") else " (unfinished)"
    lines = [
        f"### wasm stress: {leg}",
        "",
        "| JavaScriptCore options | loads | traps | rate | other page errors | console | crashes "
        "| stalls | ms per load | strips |",
        "|---|---|---|---|---|---|---|---|---|---|",
        f"| `{jsc}` | {done} of {report.get('loads_requested', '?')}{finished} | {traps} "
        f"| {rate(traps, done)} | {counts.get('page_errors', 0)} | {counts.get('console', 0)} "
        f"| {counts.get('crashes', 0)} | {counts.get('stalls', 0)} | {shown(report.get('ms_per_load'))} "
        f"| {shown(strips.get('min'))}-{shown(strips.get('max'))} |",
    ]
    for trap in report.get("traps", []):
        message = (trap.get("message") or "").splitlines()
        first = message[0] if message else ""
        lines.append(f"- load {trap.get('load')}, {trap.get('ms')} ms after goto: {trap.get('name')}: {first}")
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    p_legs = sub.add_parser("legs")
    p_legs.add_argument("--loads", required=True)
    p_legs.add_argument("--extra", default="")
    p_legs.add_argument("--only", action="store_true")
    p_env = sub.add_parser("env")
    p_env.add_argument("settings")
    p_summary = sub.add_parser("summary")
    p_summary.add_argument("report", type=Path)
    p_summary.add_argument("--leg", required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "legs":
            loads(args.loads)
            print(json.dumps(legs(args.extra, args.only)))
        elif args.command == "env":
            for word in settings(args.settings):
                print(word)
        else:
            try:
                report = json.loads(args.report.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError) as error:
                raise ValueError(f"no readable report at {args.report}: {error}") from error
            sys.stdout.write(summary(report, args.leg))
    except ValueError as error:
        print(f"::error::{error}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
