"""FohMixer: fohmixer's Ableton Live remote script (spec §2.2).

Live loads this folder as a package in its own Python interpreter and calls
`create_instance(c_instance)` when a set loads. S0 is a skeleton: the
surface only logs its start and its disconnect; the LOM proxy and its
localhost transport arrive in S2.

Imports are relative (except Live's own modules) and the logger is the
package's own, so FohMixer never collides with AbleSet or the AbletonOSC
copies in Live's shared interpreter (spec R7).
"""

import logging

import Live
from _Framework.ControlSurface import ControlSurface

from . import version
from .version import VERSION

__all__ = ["FohMixer", "create_instance", "version"]

logger = logging.getLogger(__name__)


def live_version() -> str:
    """The running Live's version, e.g. "12.1.5"."""
    app = Live.Application.get_application()
    return f"{app.get_major_version()}.{app.get_minor_version()}.{app.get_bugfix_version()}"


class FohMixer(ControlSurface):
    """The control surface Live instantiates for this script's slot."""

    def __init__(self, c_instance):
        super().__init__(c_instance)
        # WARNING level: Live's log gets start and stop only, never per
        # message (spec I2).
        logger.warning("FohMixer %s started on Live %s", VERSION, live_version())

    def disconnect(self):
        logger.warning("FohMixer %s disconnected", VERSION)
        super().disconnect()


def create_instance(c_instance):
    """Live's entry point for a remote script."""
    return FohMixer(c_instance)
