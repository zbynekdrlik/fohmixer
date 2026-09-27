# ruff: noqa: N999 - Live's own module name
"""SimLive's fake of ``_Framework.ControlSurface``: the base class of a remote script."""

import Live


class ControlSurface:
    def __init__(self, c_instance, *args, **kwargs):
        self._c_instance = c_instance
        self._pending_messages = []

    def song(self):
        return self._c_instance.song()

    def application(self):
        return Live.Application.get_application()

    def schedule_message(self, delay_in_ticks, callback, parameter=None):
        """Run ``callback`` after ``delay_in_ticks`` Live ticks (100 ms each) on the main thread."""
        fn = callback if parameter is None else (lambda: callback(parameter))
        main_thread = Live._sim_main_thread
        if main_thread is None:
            self._pending_messages.append(fn)
            return
        main_thread.schedule(delay_in_ticks * main_thread.SCHEDULE_TICK_MS, fn)

    def show_message(self, message):
        show = getattr(self._c_instance, "show_message", None)
        if show is not None:
            show(message)

    def log_message(self, *message):
        pass

    def request_rebuild_midi_map(self):
        pass

    def build_midi_map(self, midi_map_handle):
        pass

    def disconnect(self):
        self._pending_messages.clear()
