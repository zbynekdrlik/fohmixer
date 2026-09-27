"""SimLive's `_Framework.ControlSurface`: the base class a remote script
subclasses. S0 needs only construction and `disconnect()`; S2 adds what the
FohMixer script uses (`schedule_message`, `song()`, ...)."""


class ControlSurface:
    """The fake of `_Framework.ControlSurface.ControlSurface`."""

    def __init__(self, c_instance):
        self.c_instance = c_instance
        self.disconnected = False

    def disconnect(self):
        self.disconnected = True
