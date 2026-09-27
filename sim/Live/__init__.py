"""SimLive: a fake of Ableton Live's embedded `Live` Python module (spec §5.3).

Live itself is an external system; in the tests the real FohMixer script runs
on top of this fake. It is shaped like the real module: classes live in
submodules (`Live.Application.Application`, ...), created here as module
objects. S0 holds only what the skeleton touches
(`Live.Application.get_application()` and the Live version); S2 completes it
(tracks, returns, mixer devices, parameters, listeners, the timer).
"""

import sys
import types


def _submodule(name):
    module = types.ModuleType(f"Live.{name}")
    sys.modules[module.__name__] = module
    return module


Application = _submodule("Application")


class _ApplicationClass:
    """The fake of `Live.Application.Application`: Live 12, as on the PC."""

    def __init__(self):
        self._version = (12, 2, 5)

    def get_major_version(self):
        return self._version[0]

    def get_minor_version(self):
        return self._version[1]

    def get_bugfix_version(self):
        return self._version[2]


_ApplicationClass.__name__ = _ApplicationClass.__qualname__ = "Application"
_ApplicationClass.__module__ = Application.__name__
Application.Application = _ApplicationClass
_application = _ApplicationClass()


def _get_application():
    """The one application object, as in Live."""
    return _application


Application.get_application = _get_application
