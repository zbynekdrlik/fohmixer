"""Listener subscriptions with coalesced flushes (design §3.5).

One Live listener per key ``"<id>.<prop>"``, shared by every subscribed
connection; the first subscriber adds it and the last one to leave removes it.
The Live callback only marks the key dirty: Live forbids changes inside a
notification, and reading/sending there would cost Live's main thread per
callback. ``flush`` (once per timer call) reads each dirty key once and hands
the latest item to every subscriber in one ``push_values`` call per connection.
Meter keys are flushed at most every ``meter_min_interval_ms`` and stay dirty
in between. A key whose object was deleted (found by a failed read, or by the
liveness sweep every ``LIVENESS_INTERVAL_MS``) pushes ``{"key", "error": "gone"}``
and is dropped. Everything here runs on Live's main thread.
"""

import math

from . import log
from .lom import codec
from .lom.errors import OpError
from .lom.registry import is_alive

METER_PROPS = frozenset({"output_meter_left", "output_meter_right", "output_meter_level"})


class _Sub:
    __slots__ = (
        "callback",
        "conns",
        "dirty",
        "display",
        "key",
        "last_flush_ms",
        "obj",
        "path",
        "prop",
    )

    def __init__(self, key, obj, prop, path):
        self.key = key
        self.obj = obj
        self.prop = prop
        self.path = codec.child_path(path, prop)
        self.display = False
        self.conns = set()
        self.dirty = False
        self.last_flush_ms = -math.inf
        self.callback = None


class Subscriptions:
    LIVENESS_INTERVAL_MS = 500.0

    def __init__(self, registry, meter_min_interval_ms=33):
        self._registry = registry
        self._meter_min_interval_ms = meter_min_interval_ms
        self._subs = {}
        self._dirty = set()
        self._last_sweep_ms = -math.inf

    def __len__(self):
        return len(self._subs)

    def key_for(self, obj, prop, path=None):
        return f"{self._registry.put(obj, path)}.{prop}"

    def add(self, obj, prop, display, conn, path=None):
        """Subscribe ``conn`` to ``obj.prop``; return ``(key, item)`` with the current value."""
        key = self.key_for(obj, prop, path)
        sub = self._subs.get(key)
        created = sub is None
        if created:
            sub = _Sub(key, obj, prop, path)
        display = sub.display or bool(display)
        item = self._read(sub, display)
        if created:
            sub.callback = self._make_callback(sub)
            try:
                getattr(obj, f"add_{prop}_listener")(sub.callback)
            except Exception as e:
                raise OpError("live error", str(e) or type(e).__name__, type(e).__name__) from e
            self._subs[key] = sub
        sub.display = display
        sub.conns.add(conn)
        return key, item

    def remove(self, key, conn):
        sub = self._subs.get(key)
        if sub is None or conn not in sub.conns:
            raise OpError("not subscribed", key)
        sub.conns.discard(conn)
        if not sub.conns:
            self._drop(sub)

    def drop_connection(self, conn):
        for sub in list(self._subs.values()):
            sub.conns.discard(conn)
            if not sub.conns:
                self._drop(sub)

    def clear(self):
        for sub in list(self._subs.values()):
            self._drop(sub)
        self._dirty.clear()

    def flush(self, now_ms):
        """Push the latest value of every dirty key that is due."""
        if now_ms - self._last_sweep_ms >= self.LIVENESS_INTERVAL_MS:
            self._last_sweep_ms = now_ms
            for sub in self._subs.values():
                if not is_alive(sub.obj):
                    self._mark(sub)
        if not self._dirty:
            return
        per_conn = {}
        gone = []
        for key in list(self._dirty):
            sub = self._subs.get(key)
            if sub is None:
                self._dirty.discard(key)
                continue
            if sub.prop in METER_PROPS and now_ms - sub.last_flush_ms < self._meter_min_interval_ms:
                continue
            self._dirty.discard(key)
            sub.dirty = False
            sub.last_flush_ms = now_ms
            try:
                item = self._read(sub, sub.display)
            except Exception:  # noqa: BLE001 - any failed read means the object is gone
                item = {"key": key, "error": "gone"}
                gone.append(sub)
            for conn in sub.conns:
                per_conn.setdefault(conn, {})[key] = item
        for sub in gone:
            self._drop(sub)
        for conn, items in per_conn.items():
            conn.push_values(items)

    def _read(self, sub, display):
        if not is_alive(sub.obj):
            raise OpError("live error", "object was deleted", "RuntimeError")
        value = getattr(sub.obj, sub.prop)
        item = {"key": sub.key, "value": codec.encode(value, self._registry, sub.path)}
        if display:
            item["display"] = str(sub.obj)
        return item

    def _make_callback(self, sub):
        def on_change():
            self._mark(sub)

        return on_change

    def _mark(self, sub):
        if not sub.dirty:
            sub.dirty = True
            self._dirty.add(sub.key)

    def _drop(self, sub):
        self._subs.pop(sub.key, None)
        self._dirty.discard(sub.key)
        try:
            getattr(sub.obj, f"remove_{sub.prop}_listener")(sub.callback)
        except Exception as e:  # noqa: BLE001 - logged; a deleted object has no listener left
            if is_alive(sub.obj):
                log.get_logger().warning("cannot remove listener %s: %s", sub.key, e)
            else:
                log.get_logger().debug("listener %s: object already deleted", sub.key)
