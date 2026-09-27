"""SimLive: a fake of Ableton Live's `Live` Python module for CI (spec §5.3).

Live itself is an external system; the real FohMixer script runs on top of
this fake in the tests. S0 holds only what the skeleton touches
(`Live.Application.get_application()` and its version); S2 completes it
(tracks, returns, mixer devices, parameters, listeners, the timer).
"""

from . import Application

__all__ = ["Application", "SIMLIVE"]

# Marks the fake: a test can assert it never runs against a real Live.
SIMLIVE = True
