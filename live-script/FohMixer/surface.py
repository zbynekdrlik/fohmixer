"""The FohMixer control surface. S0 is a skeleton: it logs its start and its
disconnect; the LOM proxy and its localhost transport arrive in S2."""

import logging

import Live
from _Framework.ControlSurface import ControlSurface

from .version import VERSION

logger = logging.getLogger(__package__)


def live_version():
    """The running Live's version, e.g. "12.2.5"."""
    app = Live.Application.get_application()
    return f"{app.get_major_version()}.{app.get_minor_version()}.{app.get_bugfix_version()}"


class FohMixer(ControlSurface):
    """The control surface Live instantiates for this script's slot."""

    def __init__(self, c_instance):
        super().__init__(c_instance)
        # WARNING level: Live's log gets start and stop only, never one line
        # per message (spec I2).
        logger.warning("FohMixer %s started on Live %s", VERSION, live_version())

    def disconnect(self):
        logger.warning("FohMixer %s disconnected", VERSION)
        super().disconnect()
