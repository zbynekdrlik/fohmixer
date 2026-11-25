"""Tests for scripts/lufs_dsp_check.py (#264): the LUFS device's DSP without gen~."""

from __future__ import annotations

import cmath
import math
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_lufs_device as cl  # noqa: E402
import lufs_dsp_check as ld  # noqa: E402
from test_check_lufs_device import REPO, TreeCase  # noqa: E402

LUFS, METER = cl.DEVICES

# ITU-R BS.1770-4, Tables 1 and 2: the 48 kHz coefficients.
PUBLISHED_48K = {
    "b0": 1.53512485958697,
    "b1": -2.69169618940638,
    "b2": 1.19839281085285,
    "a1": -1.69065929318241,
    "a2": 0.73248077421585,
    "ra1": -1.99004745483398,
    "ra2": 0.99007225036621,
}


def direct(rate: float) -> dict[str, float]:
    """The analog prototypes written out once more in plain Python
    (libebur128's form), independent of the `expr` texts."""
    k = math.tan(math.pi * ld.SHELF_F0 / rate)
    vh = 10 ** (ld.SHELF_GAIN_DB / 20)
    vb = vh**ld.SHELF_VB_EXP
    a0 = 1 + k / ld.SHELF_Q + k * k
    kh = math.tan(math.pi * ld.HIGHPASS_F0 / rate)
    a0h = 1 + kh / ld.HIGHPASS_Q + kh * kh
    return {
        "b0": (vh + vb * k / ld.SHELF_Q + k * k) / a0,
        "b1": 2 * (k * k - vh) / a0,
        "b2": (vh - vb * k / ld.SHELF_Q + k * k) / a0,
        "a1": 2 * (k * k - 1) / a0,
        "a2": (1 - k / ld.SHELF_Q + k * k) / a0,
        "ra1": 2 * (kh * kh - 1) / a0h,
        "ra2": (1 - kh / ld.HIGHPASS_Q + kh * kh) / a0h,
    }


def max_biquad_gain(coeffs: list[float], freq: float, rate: float) -> complex:
    """Max's `biquad~` (y = a0 x + a1 x1 + a2 x2 - b1 y1 - b2 y2) at a
    frequency, its list `a0 a1 a2 b1 b2`."""
    a0, a1, a2, b1, b2 = coeffs
    z1 = cmath.exp(-2j * math.pi * freq / rate)
    return (a0 + a1 * z1 + a2 * z1 * z1) / (1 + b1 * z1 + b2 * z1 * z1)


class ReferenceTests(unittest.TestCase):
    def test_the_expr_texts_give_bs1770s_48k_coefficients(self) -> None:
        found = ld.coefficients(48000.0)
        self.assertEqual(set(found), set(PUBLISHED_48K))
        for name, value in PUBLISHED_48K.items():
            self.assertAlmostEqual(found[name], value, delta=1e-6, msg=name)

    def test_the_expr_texts_equal_the_analog_prototypes_at_each_rate(self) -> None:
        for rate in (44100.0, 48000.0, 88200.0, 96000.0, 192000.0):
            found, want = ld.coefficients(rate), direct(rate)
            for name, value in want.items():
                self.assertAlmostEqual(found[name], value, delta=1e-12, msg=f"{name} {rate}")
        # 96 kHz is no copy of 48 kHz.
        self.assertGreater(abs(ld.coefficients(96000.0)["b1"] - PUBLISHED_48K["b1"]), 0.1)

    def test_k_weighting_gains_0_691_db_at_997_hz_at_48_and_96_khz(self) -> None:
        # The -0.691 offset is the filters' gain at 997 Hz (BS.1770): a
        # 997 Hz sine at 0 dBFS on one channel reads -3.01 LUFS.
        # The analog prototypes are prewarped at their corner frequencies
        # only, so at 96 kHz the shelf's slope sits a little off the 48 kHz
        # curve (0.674 dB at 997 Hz against 0.691 at 48 kHz); 0.03 dB.
        for rate, delta in ((48000.0, 0.005), (96000.0, 0.03)):
            c = ld.coefficients(rate)
            shelf = [c["b0"], c["b1"], c["b2"], c["a1"], c["a2"]]
            highpass = [1.0, -2.0, 1.0, c["ra1"], c["ra2"]]
            gain = max_biquad_gain(shelf, 997.0, rate) * max_biquad_gain(highpass, 997.0, rate)
            self.assertAlmostEqual(20 * math.log10(abs(gain)), 0.691, delta=delta, msg=rate)

    def test_the_windows_and_the_averages_size(self) -> None:
        self.assertEqual(ld.evaluate(ld.EXPRS["obj-window-short"], 48000.0), 144000)
        self.assertEqual(ld.evaluate(ld.EXPRS["obj-window-short"], 44100.0), 132300)
        self.assertEqual(ld.evaluate(ld.EXPRS["obj-window-momentary"], 48000.0), 19200)
        self.assertEqual(ld.evaluate(ld.EXPRS["obj-window-momentary"], 44100.0), 17640)
        self.assertEqual(ld.evaluate(ld.EXPRS["obj-window-short"], 192000.0), ld.WINDOW_MAX)
        self.assertEqual(ld.evaluate(ld.EXPRS["obj-window-momentary"], 192000.0), ld.MOMENTARY_MAX)
        self.assertEqual((ld.WINDOW_MAX, ld.MOMENTARY_MAX), (576000, 76800))
        self.assertEqual((ld.WINDOW_S, ld.MOMENTARY_S), (3.0, 0.4))

    def test_the_gate_is_the_block_gate_as_a_mean_square(self) -> None:
        # The block form: 4 sub-blocks' mean squares summed >= 4 x 10^((-40 + 0.691) / 10).
        self.assertAlmostEqual(ld.GATE_MEAN_SQUARE, 10 ** (-39.309 / 10), delta=1e-18)
        self.assertEqual(float(ld.lit(ld.GATE_MEAN_SQUARE)), ld.GATE_MEAN_SQUARE)
        self.assertAlmostEqual(
            10 * math.log10(ld.GATE_MEAN_SQUARE) + ld.LUFS_OFFSET, ld.VOICE_GATE_LUFS, delta=1e-9
        )

    def test_the_levels(self) -> None:
        level_s, level_v = ld.EXPRS["obj-level-s"], ld.EXPRS["obj-level-v"]
        self.assertAlmostEqual(ld.evaluate(level_s, 1.0), -0.691, delta=1e-12)
        self.assertAlmostEqual(ld.evaluate(level_s, 0.1), -10.691, delta=1e-12)
        # Silence: the floor (1e-30) and no error, clipped to -70 after.
        self.assertAlmostEqual(ld.evaluate(level_s, 0.0), -300.691, delta=1e-6)
        # Voice: the voiced mean is the mean of sum x g over the share.
        self.assertAlmostEqual(ld.evaluate(level_v, 0.05, 0.5), -10.691, delta=1e-12)
        self.assertAlmostEqual(ld.evaluate(level_v, 0.1, 1.0), -10.691, delta=1e-12)
        # Valid from 10 of 30 voiced on; below that, the floor.
        self.assertAlmostEqual(ld.evaluate(level_v, 0.1 / 3, 1 / 3), -10.691, delta=1e-9)
        self.assertEqual(ld.evaluate(level_v, 0.03, 0.3), -70.0)
        self.assertEqual(ld.evaluate(level_v, 0.0, 0.0), -70.0)

    def test_constants_are_written_without_an_exponent(self) -> None:
        self.assertEqual(ld.lit(3), "3.0")
        self.assertEqual(ld.lit(-0.691), "-0.691")
        with self.assertRaises(ValueError):
            ld.lit(1e-30)
        with self.assertRaises(ValueError):
            ld.evaluate("__import__('os')", 1.0)
        for body in ld.EXPRS.values():
            self.assertNotRegex(body, r"\de-?\d", body)
            # Max's box text: a comma inside a call is escaped.
            self.assertNotRegex(body, r"(?<!\\),", body)


class PatchTests(TreeCase):
    def test_the_patch_carries_the_reference_chain(self) -> None:
        patch = self.patcher(LUFS)["patcher"]
        every = cl.boxes(patch)
        self.assertEqual(ld.dsp_errors("p", patch, every), [])
        ids = {b.get("id"): b for b in every}
        for box, body in ld.EXPRS.items():
            self.assertEqual(ids[box]["text"], f"expr {body}", box)
            self.assertEqual(ids[box]["numinlets"], 2 if "$f2" in body else 1, box)
        # The coefficients reach Max's biquad~ in its order (a0 a1 a2 b1 b2):
        # the shelf's pack inlets from the b0..a2 exprs, the high-pass's
        # numerator fixed at 1 -2 1; at 48 kHz the cascade gains 0.691 dB.
        c = ld.coefficients(48000.0)
        shelf = [
            c[cl.feeds(patch, "obj-shelf-pack", i)[0][0].removeprefix("obj-shelf-")]
            for i in range(5)
        ]
        hp_args = [float(w) for w in cl.words(ids["obj-hp-pack"])[1:4]]
        hp = [*hp_args, c["ra1"], c["ra2"]]
        self.assertEqual(cl.feeds(patch, "obj-hp-pack", 3), [["obj-hp-a1", 0]])
        self.assertEqual(cl.feeds(patch, "obj-hp-pack", 4), [["obj-hp-a2", 0]])
        gain = max_biquad_gain(shelf, 997.0, 48000.0) * max_biquad_gain(hp, 997.0, 48000.0)
        self.assertAlmostEqual(20 * math.log10(abs(gain)), 0.691, delta=0.01)
        # The four averages: 3 s (three) and 400 ms, sized for 192 kHz.
        averages = sorted(cl.words(b)[1] for b in every if cl.words(b)[:1] == ["average~"])
        self.assertEqual(averages, ["576000", "576000", "576000", "76800"])
        self.assertEqual(cl.words(ids["obj-gate"]), [">=~", ld.lit(ld.GATE_MEAN_SQUARE)])

    def test_each_coefficient_list_leaves_its_pack_complete(self) -> None:
        # `t` fires right to left: the pack's hot inlet must be reached by
        # the trigger's lowest outlet, after every cold one, so the biquad~
        # gets the five new coefficients in one list (the review of #264:
        # b0 went first, so the shelf ran on b0 alone after the load).
        self.assertEqual(self.firing_errors(self.patcher(LUFS)["patcher"]), [])
        # The order the review found: a2 last, b0 first.
        patch = self.patcher(LUFS)
        for index, name in enumerate(ld.SHELF_ORDER):
            self.drop_lines(patch, src=["obj-shelf-t", index], dst=[f"obj-shelf-{name}", 0])
            self.line(patch, ["obj-shelf-t", 4 - index], [f"obj-shelf-{name}", 0])
        self.assertEqual(
            self.firing_errors(patch["patcher"]), ["obj-shelf-pack: hot inlet from outlet 4"]
        )
        self.save(patch, LUFS)
        self.assertTrue(any("obj-shelf-b0:0 takes" in e for e in self.errors()))

    @staticmethod
    def firing_errors(patcher: dict) -> list[str]:
        errors = []
        for trigger, pack in (("obj-shelf-t", "obj-shelf-pack"), ("obj-hp-t", "obj-hp-pack")):
            outlet_of = {}
            for inlet in range(5):
                for src, out in cl.feeds(patcher, pack, inlet):
                    if src == trigger:
                        outlet_of[inlet] = out
                    for up, up_out in cl.feeds(patcher, src, 0):
                        if up == trigger:
                            outlet_of[inlet] = up_out
            hot = outlet_of.get(0)
            if hot is None or any(out <= hot for i, out in outlet_of.items() if i != 0):
                errors.append(f"{pack}: hot inlet from outlet {hot}")
        return errors

    def test_an_expr_other_than_the_reference_fails(self) -> None:
        where = LUFS.maxpat.as_posix()
        self.edit(LUFS.maxpat, "0.7071752369554196", "0.7071752369554197")
        errors = self.errors()
        self.assertEqual(len(errors), 1, errors)
        self.assertTrue(errors[0].startswith(f"{where}: obj-shelf-b0 must be 'expr "), errors)

    def test_a_changed_line_of_the_chain_fails(self) -> None:
        where = LUFS.maxpat.as_posix()
        cases = [
            # The high-pass before the shelf.
            (["obj-plugin", 0], ["obj-shelf-l", 0], ["obj-plugin", 0], ["obj-hp-l", 0]),
            # A coefficient into biquad~'s own inlet (it would override the list).
            (["obj-shelf-a2", 0], ["obj-shelf-pack", 4], ["obj-shelf-a2", 0], ["obj-shelf-l", 5]),
            # The 400 ms window on a 3 s average.
            (
                ["obj-window-short", 0],
                ["obj-avg-share", 0],
                ["obj-window-momentary", 0],
                ["obj-avg-share", 0],
            ),
            # The voiced mean read without the gate.
            (["obj-gate", 0], ["obj-voiced", 1], ["obj-sum", 0], ["obj-voiced", 1]),
            # The face fed from the reader again.
            (["obj-face-s", 0], ["obj-short", 0], ["obj-snap-s", 0], ["obj-short", 0]),
        ]
        for old_src, old_dst, new_src, new_dst in cases:
            self.restore(LUFS)
            patch = self.patcher(LUFS)
            self.drop_lines(patch, src=old_src, dst=old_dst)
            self.line(patch, new_src, new_dst)
            self.save(patch, LUFS)
            errors = self.errors()
            self.assertTrue(
                any(e.startswith(f"{where}: {new_dst[0]}:{new_dst[1]} takes") for e in errors),
                (new_dst, errors),
            )

    def test_an_extra_box_of_the_chains_classes_fails(self) -> None:
        where = LUFS.maxpat.as_posix()
        for text in ("average~ 100", "biquad~", "expr $f1", "dspstate~", ">=~ 0.", "*~", "+~"):
            self.restore(LUFS)
            patch = self.patcher(LUFS)
            self.node(patch, "obj-x", text)
            self.save(patch, LUFS)
            errors = self.errors()
            self.assertTrue(
                any(e.startswith(f"{where}: the {text.split()[0]} boxes must be") for e in errors),
                (text, errors),
            )
        # The input into anything but the shelves.
        self.restore(LUFS)
        patch = self.patcher(LUFS)
        self.node(patch, "obj-x", "peakamp~ 0")
        self.line(patch, ["obj-plugin", 0], ["obj-x", 0])
        self.save(patch, LUFS)
        self.assertIn(
            f"{where}: obj-plugin outputs into the two shelves alone, has "
            "[[0, 'obj-shelf-l', 0], [0, 'obj-x', 0], [1, 'obj-shelf-r', 0]]",
            self.errors(),
        )

    def test_a_changed_average_or_gate_fails(self) -> None:
        where = LUFS.maxpat.as_posix()
        for box, old, new in (
            ("obj-avg-short", "average~ 576000 bipolar", "average~ 576000 rms"),
            ("obj-avg-momentary", "average~ 76800 bipolar", "average~ 19200 bipolar"),
        ):
            self.restore(LUFS)
            self.edit(LUFS.maxpat, f'"{old}"', f'"{new}"')
            self.assertIn(
                f"{where}: {box} must be '{old}', has {new.split()}",
                self.errors(),
            )

    def test_the_meter_has_no_dsp_chain_to_check(self) -> None:
        self.assertEqual([d.kweighted for d in cl.DEVICES], [True, False])
        self.assertEqual(self.errors(), [])
        self.assertTrue((REPO / LUFS.maxpat).exists())


if __name__ == "__main__":
    unittest.main()
