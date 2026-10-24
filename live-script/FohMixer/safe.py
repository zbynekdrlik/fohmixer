"""The hub's DRY writes, made when the hub is gone (#140 PR 3a, spec §8).

The hub registers with each of its connections the autotune selectors it
found (each a rack's first macro, the object resolved at the registration)
with their DRY (off) values (``safe_writes``, ``lom/ops.py``). When that
connection ends, or the hub has said nothing on it for
``Config.HUB_SILENCE_MS`` (then the connection is closed too), the script
writes each of them once and logs it:
a hub that stops while autotune is selected must not leave speech tuned.
Live unloading the script (a set load, Live closing) writes nothing: the
selectors are saved with the set.

Nothing here writes anything but a registered selector's registered value.
Runs on Live's main thread only.
"""

from .lom.registry import is_alive


class SafeWrites:
    """Each connection's DRY writes: ``(parameter, value)`` pairs."""

    def __init__(self, log, silence_s):
        self._log = log
        self._silence_s = silence_s
        self._lists = {}

    def arm(self, conn, items):
        """``items`` replace ``conn``'s list (an empty one disarms it); its length."""
        items = list(items)
        if items:
            self._lists[conn] = items
        else:
            self._lists.pop(conn, None)
        return len(items)

    def armed(self, conn):
        return conn in self._lists

    def fire(self, conn, why):
        """Writes each of ``conn``'s registered values once (a deleted object
        is skipped), logs it and forgets the list; how many were written. An
        unarmed connection writes and logs nothing."""
        items = self._lists.pop(conn, None)
        if items is None:
            return 0
        written = 0
        for param, value in items:
            if not is_alive(param):
                continue
            try:
                param.value = value
            except Exception as e:  # noqa: BLE001 - logged; the other selectors still go
                self._log.warning(
                    "hub lost (%s): an autotune selector could not be set to DRY: %s", why, e
                )
                continue
            written += 1
        self._log.warning("hub lost (%s): set %d autotune selectors to DRY", why, written)
        return written

    def check_silence(self, now, connections):
        """Fires every armed one of ``connections`` the hub has said nothing
        on for more than the silence (``now`` and ``last_heard_s`` are
        ``time.monotonic()`` seconds); the connections fired, each with how
        many it wrote. Run after the tick's reads: a Live stall that held the
        hub's pings back is never silence."""
        fired = []
        for conn in connections:
            if self.armed(conn) and now - conn.last_heard_s > self._silence_s:
                fired.append((conn, self.fire(conn, "silence")))
        return fired

    def clear(self):
        """Forgets every list, writing nothing (Live unloads the script)."""
        self._lists.clear()
