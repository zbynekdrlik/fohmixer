#!/usr/bin/env python3
"""The stepped sine of probe P2 (#151, spec §4.5): Live's meter scale.

    python sine_steps.py OUT.wav

It writes a stereo WAV, 48 kHz, 24-bit: a 1 kHz sine for 2 s at each of
the peaks 0, -3, -6, -12, -18, -24, -36, -48 and -60 dBFS, in that order,
the same on both channels. Played on a track at 0 dB with the meter device
last in its chain, each step pairs the device's reading with Live's
``output_meter_left`` / ``_right``: the pairs are Live's meter scale. Every
step starts and ends at a zero crossing (2000 whole cycles), so a step's
edge adds no click.
"""

import argparse
import math
import struct
import sys
import wave

RATE = 48_000
FREQ = 1_000
STEP_S = 2
STEPS_DB = (0, -3, -6, -12, -18, -24, -36, -48, -60)
FULL_SCALE = 2**23 - 1


def step_frames(peak_db):
    """One step's samples (integers, 24-bit range): ``STEP_S`` seconds of the
    sine at ``peak_db`` dBFS."""
    amplitude = FULL_SCALE * 10 ** (peak_db / 20)
    count = RATE * STEP_S
    return [round(amplitude * math.sin(2 * math.pi * FREQ * n / RATE)) for n in range(count)]


def pcm24(samples):
    """Stereo 24-bit little-endian frames: each sample on both channels."""
    out = bytearray()
    for sample in samples:
        three = struct.pack("<i", sample)[:3]
        out += three + three
    return bytes(out)


def write(path, steps_db=STEPS_DB):
    """Writes the stepped sine to ``path``."""
    with wave.open(path, "wb") as wav:
        wav.setnchannels(2)
        wav.setsampwidth(3)
        wav.setframerate(RATE)
        for peak_db in steps_db:
            wav.writeframes(pcm24(step_frames(peak_db)))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("out", help="the WAV file to write")
    args = parser.parse_args(argv)
    write(args.out)
    print(f"wrote {args.out}: {len(STEPS_DB)} steps of {STEP_S} s at {RATE} Hz, 24-bit")
    return 0


if __name__ == "__main__":
    sys.exit(main())
