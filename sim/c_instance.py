"""SimLive's stand-in for the ``c_instance`` handle Live passes to ``create_instance``."""


class CInstance:
    def __init__(self, song):
        self._song = song
        self.messages = []

    def song(self):
        return self._song

    def show_message(self, message):
        self.messages.append(message)

    def handle(self):
        return 0
