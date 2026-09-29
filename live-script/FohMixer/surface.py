"""FohMixer: the control surface Live instantiates (design §3.6, §3.8, §3.9).

Live's main thread runs ``_on_timer`` every ``TIMER_INTERVAL_MS`` from
``Live.Base.Timer`` (and ``_tick`` from ``schedule_message`` as a ~100 ms
fallback). Each call does all of the script's work, its socket I/O included:
the script runs no threads (#5, K1: in Live they got Python only around this
tick). A call records ``last_main_tick``, reads the sockets (``poll_in``),
runs queued commands within the work budget (``Drain``), flushes dirty
subscriptions, makes the heartbeat when one is due (``Heartbeats``), and
writes the sockets (``poll_out``).
"""

import collections
import queue
import time

import Live
from _Framework.ControlSurface import ControlSurface

from . import Config, log
from .lom import ops
from .lom.errors import FohError
from .lom.registry import Registry
from .subscriptions import Subscriptions
from .transport.server import Server
from .version import VERSION

BIND_HOST = "127.0.0.1"
PROTO = 1
STALL_LOG_MS = 200.0
SHUTDOWN_GRACE_S = 0.3
ERROR_LOG_INTERVAL_S = 60.0


def heartbeat_data(age_ms, max_cmd_ms, gap_s):
    """A heartbeat's data. ``gap_ms`` is the time since the previous heartbeat
    was made (nominally ``HEARTBEAT_INTERVAL_MS``): far longer means the main
    thread did not tick, not that a sent heartbeat was held up (#9)."""
    return {
        "main_tick_age_ms": round(age_ms, 1),
        "max_cmd_ms": round(max_cmd_ms, 3),
        "gap_ms": round(gap_s * 1000.0, 1),
    }


class Heartbeats:
    """Which ticks make a heartbeat, and what it says (#5: made in the tick).

    One every ``interval_s`` on a fixed grid (with Live's ~33 ms ticks: every
    100 ms on average, not every fourth tick). ``main_tick_age_ms`` is how long
    the main thread was away before this tick: any gap of an interval or more
    ends in a tick that makes a heartbeat, so a stall shows on the first one
    after it; while it lasts none goes out, and the hub reports Live busy once
    one is overdue. After a stall the grid restarts (no burst of the skipped
    ones). Times are ``time.monotonic()`` seconds, passed in.
    """

    def __init__(self, interval_s, now):
        self._interval_s = interval_s
        self._previous = now
        self._due = now + interval_s

    def on_tick(self, now, tick_gap_s, max_cmd_ms):
        """The heartbeat's data when one is due at this tick, else None."""
        if now < self._due:
            return None
        data = heartbeat_data(tick_gap_s * 1000.0, max_cmd_ms, now - self._previous)
        self._previous = now
        self._due += self._interval_s
        if self._due <= now:
            self._due = now + self._interval_s
        return data


class Drain:
    """Runs queued envelopes command by command within a time budget.

    A call stops before the next command once ``budget_ms`` has passed, or once
    the commands of the last ``window_ms`` add up to ``window_budget_ms``; a
    single command cannot be interrupted, so a call lasts at most the budget plus
    one command. A partly run envelope resumes on the next call; its result goes
    out whole. ``(conn, None)`` in the inbox means the connection is gone.
    Time is integer nanoseconds (``clock``), so the limits compare exactly.
    """

    def __init__(
        self,
        inbox,
        run_command,
        on_result,
        on_disconnect,
        budget_ms,
        window_budget_ms,
        window_ms,
        clock=time.perf_counter_ns,
    ):
        self._inbox = inbox
        self._run_command = run_command
        self._on_result = on_result
        self._on_disconnect = on_disconnect
        self._budget_ns = round(budget_ms * 1_000_000)
        self._window_budget_ns = round(window_budget_ms * 1_000_000)
        self._window_ns = round(window_ms * 1_000_000)
        self._clock = clock
        self._window = collections.deque()
        self._window_used_ns = 0
        self._current = None
        self.max_cmd_ms = 0.0

    def run(self):
        started = self._clock()
        while True:
            now = self._clock()
            if now - started >= self._budget_ns or self._used(now) >= self._window_budget_ns:
                return
            if self._current is None:
                try:
                    conn, payload = self._inbox.get_nowait()
                except queue.Empty:
                    return
                if payload is None:
                    self._timed(self._on_disconnect, conn)
                    continue
                self._current = (conn, payload.get("uuid"), payload["commands"], [])
            conn, uuid, commands, results = self._current
            if not conn.is_open:
                self._current = None
                continue
            if len(results) < len(commands):
                results.append(self._timed(self._run_command, commands[len(results)], conn))
            if len(results) == len(commands):
                self._current = None
                self._on_result(conn, uuid, results)

    def _timed(self, fn, *args):
        started = self._clock()
        try:
            return fn(*args)
        finally:
            ended = self._clock()
            self._window.append((ended, ended - started))
            self._window_used_ns += ended - started
            self.max_cmd_ms = max(self.max_cmd_ms, (ended - started) / 1_000_000)

    def _used(self, now):
        horizon = now - self._window_ns
        while self._window and self._window[0][0] <= horizon:
            self._window_used_ns -= self._window.popleft()[1]
        return self._window_used_ns


class FohMixer(ControlSurface):
    def __init__(self, c_instance):
        super().__init__(c_instance)
        self._log = log.setup()
        self._last_error_log = {}
        self._registry = Registry()
        self._subs = Subscriptions(self._registry, Config.METER_MIN_INTERVAL_MS)
        self.last_main_tick = time.monotonic()
        self.connected = True
        self._shown_bind_error = None
        self._server = Server(
            BIND_HOST, Config.PORT, self._connect_info(), Config.RESULT_QUEUE_MAX, self._log
        )
        self._drain = Drain(
            self._server.inbox,
            self._run_command,
            self._push_result,
            self._subs.drop_connection,
            Config.DRAIN_BUDGET_MS,
            Config.WINDOW_BUDGET_MS,
            Config.WINDOW_MS,
        )
        self._heartbeats = Heartbeats(Config.HEARTBEAT_INTERVAL_MS / 1000.0, self.last_main_tick)
        self._server.start()
        self._timer = Live.Base.Timer(
            callback=self._on_timer, interval=Config.TIMER_INTERVAL_MS, repeat=True
        )
        self._timer.start()
        self.schedule_message(1, self._tick)
        self._log.warning(
            "FohMixer %s started: instance %s on %s:%s",
            VERSION,
            Config.INSTANCE,
            BIND_HOST,
            Config.PORT,
        )

    @property
    def server(self):
        return self._server

    @property
    def registry(self):
        return self._registry

    def _connect_info(self):
        try:
            name = getattr(self.song(), "name", "")
        except Exception as e:  # noqa: BLE001 - reported; connect still goes out
            self._log.warning("cannot read the set name: %s", e)
            name = ""
        try:
            app = self.application()
            live_version = (
                f"{app.get_major_version()}.{app.get_minor_version()}.{app.get_bugfix_version()}"
            )
        except Exception as e:  # noqa: BLE001 - reported; connect still goes out
            self._log.warning("cannot read the Live version: %s", e)
            live_version = ""
        return {
            "instance": Config.INSTANCE,
            "set_name": name if isinstance(name, str) else "",
            "script_version": VERSION,
            "live_version": live_version,
            "proto": PROTO,
        }

    # --- Live's main thread ---

    def _on_timer(self):
        if not self.connected:
            return
        now = time.monotonic()
        gap_s = now - self.last_main_tick
        self.last_main_tick = now
        if gap_s * 1000.0 > STALL_LOG_MS:
            self._log.warning("Live's main thread stalled for %d ms", gap_s * 1000.0)
        try:
            self._server.poll_in()
            self._drain.run()
            self._subs.flush(now * 1000.0)
            heartbeat = self._heartbeats.on_tick(now, gap_s, self._drain.max_cmd_ms)
            if heartbeat is not None:
                self._server.broadcast_heartbeat(heartbeat)
        except Exception:  # noqa: BLE001 - logged (rate-limited); Live's timer must keep running
            self._log_error("timer work failed")
        # Written even when the work above failed: what it queued still goes out.
        try:
            self._server.poll_out()
        except Exception:  # noqa: BLE001 - logged (rate-limited); Live's timer must keep running
            self._log_error("socket writes failed")

    def _tick(self):
        if not self.connected:
            return
        self._on_timer()
        bind_error = self._server.bind_error
        if bind_error != self._shown_bind_error:
            self._shown_bind_error = bind_error
            if bind_error:
                self.show_message(f"FohMixer: {bind_error}")
        self.schedule_message(1, self._tick)

    def _run_command(self, command, conn):
        ctx = ops.Context(self.song(), self.application(), self._registry, self._subs, conn)
        try:
            return {"ok": True, "data": ops.execute(command, ctx)}
        except FohError as e:
            return {"ok": False, "error": str(e), "errorType": e.error_type}
        except Exception as e:  # noqa: BLE001 - becomes an error slot; logged (rate-limited)
            self._log_error("unexpected command failure")
            return {"ok": False, "error": str(e) or type(e).__name__, "errorType": type(e).__name__}

    def _push_result(self, conn, uuid, results):
        conn.push_result(uuid, results)

    def _log_error(self, what):
        """Log with traceback, at most once per ``ERROR_LOG_INTERVAL_S`` per kind."""
        now = time.monotonic()
        if now - self._last_error_log.get(what, -ERROR_LOG_INTERVAL_S) >= ERROR_LOG_INTERVAL_S:
            self._last_error_log[what] = now
            self._log.exception(what)

    # --- lifecycle ---

    def disconnect(self):
        """Live unloads the script (set load/close): stop, say goodbye, release everything."""
        self.connected = False
        self._timer.stop()
        self._server.broadcast("disconnect")
        self._server.shutdown(SHUTDOWN_GRACE_S)
        self._subs.clear()
        self._registry.clear()
        self._log.warning("FohMixer stopped: instance %s", Config.INSTANCE)
        super().disconnect()
