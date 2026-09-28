# ruff: noqa: N999 - the folder name is the Live control-surface name
"""FohMixer: a Live remote script exposing the native LOM over a localhost WebSocket."""


def create_instance(c_instance):
    from .surface import FohMixer

    return FohMixer(c_instance)
