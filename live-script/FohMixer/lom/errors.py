"""Errors that become ``{"ok": false, "error", "errorType"}`` result slots."""


class FohError(Exception):
    """A command failure: ``kind`` is a short fixed word, ``detail`` says where."""

    def __init__(self, kind, detail="", error_type=None):
        self.kind = kind
        self.detail = detail
        self.error_type = error_type or type(self).__name__
        super().__init__(f"{kind}: {detail}" if detail else kind)


class PathError(FohError):
    """A target path that does not parse or does not resolve to exactly one object."""


class UnknownRef(FohError):
    """An object id the registry never issued (or cleared)."""


class StaleRef(FohError):
    """An object id whose object was deleted, or whose pointer now holds another class."""


class CodecError(FohError):
    """A value that cannot be decoded (e.g. an unknown enum member)."""


class OpError(FohError):
    """A command that cannot run; ``error_type`` keeps a wrapped LOM exception's type."""
