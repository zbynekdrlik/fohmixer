"""Object ids: ``live_<ptr>`` (``id_<id>`` without a pointer), checked on lookup.

Live reuses pointers, so an entry keeps the class seen at ``put``; a lookup
fails with ``StaleRef`` when the object was deleted or when the caller expects
another class (a ``$ref`` echoed back carries its ``class``). The registry keeps
strong references: Live hands out a fresh wrapper per access, so a weakref would
die at once. It is cleared on ``disconnect``.
"""

import operator

from .errors import StaleRef, UnknownRef


def is_alive(obj):
    """Live's idiom: a deleted LOM object compares equal to ``None``."""
    try:
        return not operator.eq(obj, None)
    except Exception:  # noqa: BLE001 - any failure to compare means the object is gone
        return False


def id_of(obj):
    ptr = getattr(obj, "_live_ptr", None)
    if ptr is None:
        return f"id_{id(obj)}"
    return f"live_{ptr}"


class Registry:
    def __init__(self):
        self._entries = {}

    def __len__(self):
        return len(self._entries)

    def put(self, obj, path=None):
        """Register ``obj`` (reached by ``path``, if known) and return its id."""
        ref = id_of(obj)
        cls = type(obj).__name__
        old = self._entries.get(ref)
        if path is None and old is not None and old[1] == cls:
            path = old[2]
        self._entries[ref] = (obj, cls, path)
        return ref

    def lookup(self, ref, expected_class=None):
        """Return ``(obj, path)`` for ``ref``; raise ``UnknownRef`` or ``StaleRef``."""
        if not isinstance(ref, str):
            raise UnknownRef("bad id", repr(ref))
        entry = self._entries.get(ref)
        if entry is None:
            raise UnknownRef("unknown id", ref)
        obj, cls, path = entry
        if type(obj).__name__ != cls:
            raise StaleRef("class changed", f"{ref} was {cls}, is {type(obj).__name__}")
        if expected_class is not None and expected_class != cls:
            raise StaleRef("class changed", f"{ref} is {cls}, not {expected_class}")
        if not is_alive(obj):
            raise StaleRef("deleted", ref)
        return obj, path

    def get(self, ref, expected_class=None):
        return self.lookup(ref, expected_class)[0]

    def path_of(self, ref):
        entry = self._entries.get(ref)
        return None if entry is None else entry[2]

    def clear(self):
        self._entries.clear()
