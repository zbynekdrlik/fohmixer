"""SimLive's fake of `_Framework.ControlSurface`: the base class of a remote
script. S0 needs construction and `disconnect()`; S2 adds what the FohMixer
script uses (`song()`, `schedule_message`, ...)."""


class ControlSurface:
    def __init__(self, c_instance, *args, **kwargs):
        self._c_instance = c_instance

    def log_message(self, *message):
        pass

    def disconnect(self):
        pass
