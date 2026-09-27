"""FohMixer: fohmixer's Ableton Live remote script (spec §2.2).

Live loads this folder as a package in its own Python interpreter and calls
`create_instance(c_instance)` when a set loads. Imports inside the package are
relative and the logger is the package's own, so FohMixer never collides with
AbleSet or the AbletonOSC copies in Live's shared interpreter (spec R7).
"""


def create_instance(c_instance):
    """Live's entry point for a remote script."""
    from .surface import FohMixer

    return FohMixer(c_instance)
