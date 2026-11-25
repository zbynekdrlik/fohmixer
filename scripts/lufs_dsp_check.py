"""The FohMixer LUFS device's DSP without `gen~` (#264), for `check_lufs_device.py`.

The DECIDED of 25.11.2026 ("FohMixer LUFS without gen~"): the `gen~` ran a
clock of its own (a 4 Hz grid) that Max for Live services inside Live's
audio thread, and each tick posted a `PostMessageW` from there (dasio's
stacks: `sched_setqueuethrottle -> gen~.mxe64 -> qelem_set ->
PostMessageW`). So the device measures with plain MSP objects, none with a
clock of its own:

- per channel two `biquad~`, BS.1770's high shelf, then the RLB high-pass
  (b = 1, -2, 1); their coefficients come from `expr` boxes fed by
  `dspstate~`'s sample rate (at DSP start, and banged at load from the
  identity's `deferlow`), never from a clock;
- each K-weighted channel squared (`*~` with the signal in both inlets),
  the two summed (`+~`);
- short-term: the sum's 3 s mean (`average~ <max> bipolar`, its window set
  to round(3 sr)), read by `snapshot~ 0` on the hub's poll;
- Voice, the continuous form of the block gate: g = the sum's 400 ms mean
  >= the gate as a mean square (`>=~`); Voice is the 3 s mean of sum x g
  over the 3 s mean of g, valid when that share reaches
  MIN_VOICED / WINDOW_BLOCKS;
- the levels (`-0.691 + 10 log10(max(mean, 1e-30))`) in `expr`, then the
  existing `* 10.` -> `round 1.` -> `clip -700 0` into the pack; the face
  shows the clipped tenths (`* 0.1`).

This module holds the one reference of the formulas: the constants (the
ones the gen~ code used), the `expr` texts the patch must carry (generated
here from the constants), a Python evaluator of those texts (so a test can
compare their values with BS.1770's published 48 kHz coefficients), and
`dsp_errors`, which pins every box and line of the chain.
"""

from __future__ import annotations

import math
import re

from check_lufs_device import feeds, kind, words

# The constants (spec §3.3; the analog prototypes in libebur128's form,
# the values the gen~ code carried until #264).
SHELF_F0 = 1681.974450955533
SHELF_GAIN_DB = 3.999843853973347
SHELF_Q = 0.7071752369554196
SHELF_VB_EXP = 0.4996667741545416
HIGHPASS_F0 = 38.13547087602444
HIGHPASS_Q = 0.5003270373238773
LUFS_OFFSET = -0.691
BLOCKS_PER_SECOND = 10
WINDOW_BLOCKS = 30
MOMENTARY_BLOCKS = 4
VOICE_GATE_LUFS = -40
MIN_VOICED = 10
FLOOR_LUFS = -70
TOP_LUFS = 0
# The highest rate the averages are sized for (their buffers are fixed at
# load: Max's `average~` takes its maximum interval as an argument).
MAX_RATE = 192000
PI = "3.141592653589793"
TINY = "pow(0.1\\, 30.)"


def lit(value: float) -> str:
    """A constant as the `expr` texts write it: Python's shortest round
    trip, with a decimal point (a float in `expr`), never an exponent."""
    text = repr(float(value))
    if "e" in text or "inf" in text or "nan" in text:
        raise ValueError(f"{value} has no plain decimal form")
    return text


def window_samples(seconds: float, rate: float) -> int:
    """The samples of a window at a rate, rounded as the patch's `expr`
    rounds (`int(x + 0.5)`)."""
    return int(seconds * rate + 0.5)


WINDOW_S = WINDOW_BLOCKS / BLOCKS_PER_SECOND
MOMENTARY_S = MOMENTARY_BLOCKS / BLOCKS_PER_SECOND
WINDOW_MAX = window_samples(WINDOW_S, MAX_RATE)
MOMENTARY_MAX = window_samples(MOMENTARY_S, MAX_RATE)
# The gate as a mean square: the block form compared the sum of 4
# sub-blocks' mean squares with 4 x 10^((gate - offset) / 10), i.e. the
# 400 ms mean with 10^((gate - offset) / 10).
GATE_MEAN_SQUARE = 10 ** ((VOICE_GATE_LUFS - LUFS_OFFSET) / 10)

# The `expr` bodies (after the word `expr`), as the patch carries them: a
# comma inside a call is escaped (`\,`, Max's box text), and no long
# number stands alone between spaces (a lone number becomes a float atom;
# glued to an operator it stays text, so `expr` reads every digit).
_VH = f"pow(10.\\, {lit(SHELF_GAIN_DB)}/20.)"
_VB = f"pow({_VH}\\, {lit(SHELF_VB_EXP)})"
_Q = lit(SHELF_Q)
_A0 = f"(1.+$f1/{_Q}+$f1*$f1)"
_QH = lit(HIGHPASS_Q)
_A0H = f"(1.+$f1/{_QH}+$f1*$f1)"
_LEVEL = f"10.*log10(max($f1\\, {TINY}))-{lit(-LUFS_OFFSET)}"
_SHARE = f"{lit(MIN_VOICED)}/{lit(WINDOW_BLOCKS)}"
_VOICED_LEVEL = f"10.*log10(max($f1/max($f2\\, {TINY})\\, {TINY}))-{lit(-LUFS_OFFSET)}"

EXPRS = {
    # $f1 = the sample rate.
    "obj-shelf-k": f"tan({PI}*{lit(SHELF_F0)}/$f1)",
    "obj-hp-k": f"tan({PI}*{lit(HIGHPASS_F0)}/$f1)",
    "obj-window-short": f"int({lit(WINDOW_S)}*$f1+0.5)",
    "obj-window-momentary": f"int({lit(MOMENTARY_S)}*$f1+0.5)",
    # $f1 = k of the shelf.
    "obj-shelf-b0": f"({_VH}+{_VB}*$f1/{_Q}+$f1*$f1)/{_A0}",
    "obj-shelf-b1": f"2.*($f1*$f1-{_VH})/{_A0}",
    "obj-shelf-b2": f"({_VH}-{_VB}*$f1/{_Q}+$f1*$f1)/{_A0}",
    "obj-shelf-a1": f"2.*($f1*$f1-1.)/{_A0}",
    "obj-shelf-a2": f"(1.-$f1/{_Q}+$f1*$f1)/{_A0}",
    # $f1 = k of the high-pass.
    "obj-hp-a1": f"2.*($f1*$f1-1.)/{_A0H}",
    "obj-hp-a2": f"(1.-$f1/{_QH}+$f1*$f1)/{_A0H}",
    # $f1 = the 3 s mean square.
    "obj-level-s": _LEVEL,
    # $f1 = the 3 s mean of sum x g, $f2 = the 3 s mean of g (cold).
    "obj-level-v": (f"($f2>={_SHARE})*({_VOICED_LEVEL})-{lit(-FLOOR_LUFS)}*($f2<{_SHARE})"),
}


def evaluate(body: str, f1: float, f2: float = 0.0) -> float:
    """An `expr` body's value, computed by Python (the functions it uses
    are C's, as Max's `expr` names them)."""
    source = body.replace("\\,", ",").replace("$f1", "f1").replace("$f2", "f2")
    if re.search(r"[^\w\s.,+\-*/()<>=]", source):
        raise ValueError(f"unexpected text in {body!r}")
    names = {
        "__builtins__": {},
        "f1": f1,
        "f2": f2,
        "tan": math.tan,
        "pow": math.pow,
        "log10": math.log10,
        "max": max,
        "int": int,
    }
    return float(eval(source, names))  # noqa: S307 (the texts above, checked)


def coefficients(rate: float) -> dict[str, float]:
    """The K-weighting coefficients at a rate, from the `expr` texts:
    the shelf's b0 b1 b2 a1 a2 and the high-pass's a1 a2."""
    shelf_k = evaluate(EXPRS["obj-shelf-k"], rate)
    hp_k = evaluate(EXPRS["obj-hp-k"], rate)
    found = {
        name: evaluate(EXPRS[f"obj-shelf-{name}"], shelf_k)
        for name in ("b0", "b1", "b2", "a1", "a2")
    }
    found["ra1"] = evaluate(EXPRS["obj-hp-a1"], hp_k)
    found["ra2"] = evaluate(EXPRS["obj-hp-a2"], hp_k)
    return found


# The chain's boxes (id: words). The readers (`obj-snap-*`) and the pull
# are pinned by `lufs_pull_check`, the clips and the pack by the main check.
AVERAGE = "average~"


def dsp_boxes() -> dict[str, list[str]]:
    found = {box: ["expr", *body.split()] for box, body in EXPRS.items()}
    found |= {
        "obj-plugin": ["plugin~"],
        "obj-sr-state": ["dspstate~"],
        "obj-sr": ["t", "f", "f", "f", "f"],
        "obj-shelf-t": ["t", "f", "f", "f", "f", "f"],
        "obj-shelf-pack": ["pack", "0.", "0.", "0.", "0.", "0."],
        "obj-hp-t": ["t", "b", "f", "f"],
        "obj-hp-pack": ["pack", "1.", "-2.", "1.", "0.", "0."],
        "obj-shelf-l": ["biquad~"],
        "obj-shelf-r": ["biquad~"],
        "obj-hp-l": ["biquad~"],
        "obj-hp-r": ["biquad~"],
        "obj-square-l": ["*~"],
        "obj-square-r": ["*~"],
        "obj-sum": ["+~"],
        "obj-avg-short": [AVERAGE, str(WINDOW_MAX), "bipolar"],
        "obj-avg-momentary": [AVERAGE, str(MOMENTARY_MAX), "bipolar"],
        "obj-gate": [">=~", lit(GATE_MEAN_SQUARE)],
        "obj-avg-share": [AVERAGE, str(WINDOW_MAX), "bipolar"],
        "obj-voiced": ["*~"],
        "obj-avg-voiced": [AVERAGE, str(WINDOW_MAX), "bipolar"],
        "obj-face-s": ["*", "0.1"],
        "obj-face-v": ["*", "0.1"],
    }
    return found


SHELF_ORDER = ("b0", "b1", "b2", "a1", "a2")


def dsp_wiring() -> dict[tuple[str, int], list[tuple[str, int]]]:
    """Each inlet of the chain and the only sources it takes."""
    found: dict[tuple[str, int], list[tuple[str, int]]] = {
        # The rate: at DSP start, and banged once at load (main thread).
        ("obj-sr-state", 0): [("obj-id-defer", 0)],
        ("obj-sr", 0): [("obj-sr-state", 1)],
        ("obj-shelf-k", 0): [("obj-sr", 3)],
        ("obj-hp-k", 0): [("obj-sr", 2)],
        ("obj-window-short", 0): [("obj-sr", 1)],
        ("obj-window-momentary", 0): [("obj-sr", 0)],
        ("obj-shelf-t", 0): [("obj-shelf-k", 0)],
        ("obj-hp-t", 0): [("obj-hp-k", 0)],
        ("obj-hp-a2", 0): [("obj-hp-t", 2)],
        ("obj-hp-a1", 0): [("obj-hp-t", 1)],
        # pack's left inlet is hot: b0 last, the high-pass's bang last.
        ("obj-hp-pack", 0): [("obj-hp-t", 0)],
        ("obj-hp-pack", 1): [],
        ("obj-hp-pack", 2): [],
        ("obj-hp-pack", 3): [("obj-hp-a1", 0)],
        ("obj-hp-pack", 4): [("obj-hp-a2", 0)],
        # The filters: the input, then the coefficients' list (a0 a1 a2 b1
        # b2, Max's order) into the left inlet; nothing in inlets 1-5 (a
        # signal there would override the list).
        ("obj-shelf-l", 0): [("obj-plugin", 0), ("obj-shelf-pack", 0)],
        ("obj-shelf-r", 0): [("obj-plugin", 1), ("obj-shelf-pack", 0)],
        ("obj-hp-l", 0): [("obj-shelf-l", 0), ("obj-hp-pack", 0)],
        ("obj-hp-r", 0): [("obj-shelf-r", 0), ("obj-hp-pack", 0)],
        ("obj-square-l", 0): [("obj-hp-l", 0)],
        ("obj-square-l", 1): [("obj-hp-l", 0)],
        ("obj-square-r", 0): [("obj-hp-r", 0)],
        ("obj-square-r", 1): [("obj-hp-r", 0)],
        ("obj-sum", 0): [("obj-square-l", 0)],
        ("obj-sum", 1): [("obj-square-r", 0)],
        ("obj-avg-short", 0): [("obj-sum", 0), ("obj-window-short", 0)],
        ("obj-avg-momentary", 0): [("obj-sum", 0), ("obj-window-momentary", 0)],
        ("obj-gate", 0): [("obj-avg-momentary", 0)],
        ("obj-gate", 1): [],
        ("obj-avg-share", 0): [("obj-gate", 0), ("obj-window-short", 0)],
        ("obj-voiced", 0): [("obj-sum", 0)],
        ("obj-voiced", 1): [("obj-gate", 0)],
        ("obj-avg-voiced", 0): [("obj-voiced", 0), ("obj-window-short", 0)],
        # The readers: their signal and the pull's bang.
        ("obj-snap-s", 0): [("obj-avg-short", 0), ("obj-pull", 2)],
        ("obj-snap-g", 0): [("obj-avg-share", 0), ("obj-pull", 1)],
        ("obj-snap-v", 0): [("obj-avg-voiced", 0), ("obj-pull", 0)],
        ("obj-level-s", 0): [("obj-snap-s", 0)],
        ("obj-level-v", 0): [("obj-snap-v", 0)],
        ("obj-level-v", 1): [("obj-snap-g", 0)],
        ("obj-times-s", 0): [("obj-level-s", 0)],
        ("obj-times-s", 1): [],
        ("obj-times-v", 0): [("obj-level-v", 0)],
        ("obj-times-v", 1): [],
        # The face: the clipped tenths, from the poll only.
        ("obj-face-s", 0): [("obj-clip-s", 0)],
        ("obj-face-s", 1): [],
        ("obj-face-v", 0): [("obj-clip-v", 0)],
        ("obj-face-v", 1): [],
        ("obj-short", 0): [("obj-face-s", 0)],
        ("obj-voice", 0): [("obj-face-v", 0)],
    }
    for index, name in enumerate(SHELF_ORDER):
        found[(f"obj-shelf-{name}", 0)] = [("obj-shelf-t", len(SHELF_ORDER) - 1 - index)]
        found[("obj-shelf-pack", index)] = [(f"obj-shelf-{name}", 0)]
    for box in ("obj-shelf-l", "obj-shelf-r", "obj-hp-l", "obj-hp-r"):
        for inlet in range(1, 6):
            found[(box, inlet)] = []
    return found


# The classes of the chain, each exactly as many as the chain has (none
# elsewhere in the device).
COUNTED = ("biquad~", AVERAGE, "dspstate~", "expr", ">=~", "*~", "+~")


def dsp_errors(where: str, patcher: dict, every: list[dict]) -> list[str]:
    """The chain's boxes, each inlet's sources, and no other box of its
    classes."""
    ids = {b.get("id"): b for b in every}
    want = dsp_boxes()
    errors = [
        f"{where}: {box} must be '{' '.join(text)}', has {words(ids.get(box, {}))}"
        for box, text in want.items()
        if words(ids.get(box, {})) != text
    ]
    for (box, inlet), sources in dsp_wiring().items():
        fed = feeds(patcher, box, inlet)
        expected = sorted([list(s) for s in sources])
        if fed != expected:
            errors.append(f"{where}: {box}:{inlet} takes {expected} alone, has {fed}")
    # The input goes into the two shelves alone (the silence check pins
    # what reaches `plugout~`).
    out = sorted(
        [line["source"][1], *line["destination"]]
        for line in (item.get("patchline", {}) for item in patcher.get("lines", []))
        if line.get("source", [None])[0] == "obj-plugin"
    )
    if out != [[0, "obj-shelf-l", 0], [1, "obj-shelf-r", 0]]:
        errors.append(f"{where}: obj-plugin outputs into the two shelves alone, has {out}")
    for cls in COUNTED:
        found = sorted(b.get("id") for b in every if words(b)[:1] == [cls])
        expected = sorted(box for box, text in want.items() if text[0] == cls)
        if found != expected:
            errors.append(f"{where}: the {cls} boxes must be {expected}, has {found}")
    return errors


# Every gen object: a `gen~` (any form) runs a clock of its own (#264).
GEN = ("gen~", "mc.gen~", "gen", "jit.gen", "jit.gl.pix", "jit.pix")


def is_gen(box: dict) -> bool:
    """A `gen~` (or any gen object, a codebox) anywhere in the patch."""
    return kind(box) in GEN or box.get("maxclass") == "codebox"


def gen_errors(where: str, every: list[dict]) -> list[str]:
    """No gen object in the patch, its subpatchers included (#264: a `gen~`
    runs a clock of its own, which Max for Live services in Live's audio
    thread, and each tick posts from there)."""
    return [
        f"{where}: {b.get('id')} ('{b.get('text')}') is a gen~ (#264: a gen~ runs a clock "
        "of its own and posts from Live's audio thread)"
        for b in every
        if is_gen(b)
    ]
