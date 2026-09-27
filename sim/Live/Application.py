"""SimLive's `Live.Application`: the application singleton and its version."""

# The Live version SimLive reports (Live 12, as on the Ableton PC).
SIM_VERSION = (12, 1, 5)


class Application:
    """The fake of `Live.Application.Application`."""

    def get_major_version(self) -> int:
        return SIM_VERSION[0]

    def get_minor_version(self) -> int:
        return SIM_VERSION[1]

    def get_bugfix_version(self) -> int:
        return SIM_VERSION[2]


_APPLICATION = Application()


def get_application() -> Application:
    """The one application object, as in Live."""
    return _APPLICATION
