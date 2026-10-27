"""The stepped sine of probe P2 (#151): the file's format and each step's
level, read back from the written WAV."""

import math
import os
import struct
import sys
import tempfile
import unittest
import wave

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import sine_steps  # noqa: E402


def read_back(path):
    """The WAV's format and its left and right channels as integers."""
    with wave.open(path, "rb") as wav:
        fmt = (wav.getnchannels(), wav.getsampwidth(), wav.getframerate())
        data = wav.readframes(wav.getnframes())
    left, right = [], []
    for i in range(0, len(data), 6):
        frame = data[i : i + 6]
        left.append(struct.unpack("<i", frame[0:3] + (b"\xff" if frame[2] & 0x80 else b"\x00"))[0])
        right.append(struct.unpack("<i", frame[3:6] + (b"\xff" if frame[5] & 0x80 else b"\x00"))[0])
    return fmt, left, right


def rms_db(samples):
    """The samples' RMS in dB of full scale."""
    mean = sum(s * s for s in samples) / len(samples)
    return 20 * math.log10(math.sqrt(mean) / sine_steps.FULL_SCALE)


class SineStepsTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.dir = tempfile.TemporaryDirectory()
        cls.path = os.path.join(cls.dir.name, "steps.wav")
        sine_steps.main([cls.path])
        cls.fmt, cls.left, cls.right = read_back(cls.path)

    @classmethod
    def tearDownClass(cls):
        cls.dir.cleanup()

    def test_format(self):
        self.assertEqual(self.fmt, (2, 3, 48_000))
        self.assertEqual(len(self.left), 9 * 2 * 48_000)
        self.assertEqual(self.left, self.right)

    def test_steps_have_their_levels(self):
        size = sine_steps.STEP_S * sine_steps.RATE
        self.assertEqual(sine_steps.STEPS_DB, (0, -3, -6, -12, -18, -24, -36, -48, -60))
        for i, peak in enumerate(sine_steps.STEPS_DB):
            step = self.left[i * size : (i + 1) * size]
            # A sine's RMS is its peak less 3.01 dB.
            want = peak + 20 * math.log10(1 / math.sqrt(2))
            self.assertAlmostEqual(rms_db(step), want, delta=0.05, msg=f"step {peak} dBFS")
            top = max(abs(s) for s in step) / sine_steps.FULL_SCALE
            self.assertAlmostEqual(20 * math.log10(top), peak, delta=0.01, msg=f"peak {peak}")

    def test_steps_are_1_khz_and_start_at_zero(self):
        size = sine_steps.STEP_S * sine_steps.RATE
        step = self.left[:size]
        rising = sum(1 for a, b in zip(step, step[1:], strict=False) if a < 0 <= b)
        self.assertIn(rising, (1999, 2000))
        for i in range(len(sine_steps.STEPS_DB)):
            self.assertEqual(self.left[i * size], 0)

    def test_negative_samples_keep_their_sign(self):
        self.assertEqual(sine_steps.pcm24([-1, 1]), b"\xff\xff\xff" * 2 + b"\x01\x00\x00" * 2)


if __name__ == "__main__":
    unittest.main()
