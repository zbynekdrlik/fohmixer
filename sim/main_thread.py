"""SimLive's stand-in for Live's single main thread.

One thread runs every ``Live.Base.Timer`` callback, every ``schedule_message``
callback and every ``call``/``post`` request, one at a time, like Live's main
thread. ``stall(ms)`` blocks that thread, which is how tests simulate a Live
hiccup. ``request_stop`` only sets a flag, so a signal handler may call it.
"""

import collections
import heapq
import itertools
import sys
import threading
import time
import traceback

import Live

MAX_IDLE_WAIT_S = 0.05


class MainThread:
    SCHEDULE_TICK_MS = 100

    def __init__(self):
        self._cv = threading.Condition()
        self._calls = collections.deque()
        self._timers = {}
        self._scheduled = []
        self._seq = itertools.count()
        self._stopping = False
        self._stop_requested = False
        self._thread = None
        self.ident = None
        self.errors = []

    # --- lifecycle ---

    def install(self):
        """Make ``Live.Base.Timer`` and ``schedule_message`` run on this thread."""
        Live._sim_main_thread = self
        return self

    def uninstall(self):
        if Live._sim_main_thread is self:
            Live._sim_main_thread = None

    def start(self):
        """Run the loop on a new background thread (tests)."""
        self.install()
        started = threading.Event()
        self._thread = threading.Thread(
            target=self.run_forever, kwargs={"started": started}, name="sim-main", daemon=True
        )
        self._thread.start()
        started.wait(2.0)
        return self

    def stop(self, timeout=2.0):
        with self._cv:
            self._stopping = True
            self._cv.notify_all()
        if self._thread is not None and self._thread is not threading.current_thread():
            self._thread.join(timeout)
        self.uninstall()

    def request_stop(self):
        """Signal-handler safe: the loop runs ``on_stop`` and exits at its next turn."""
        self._stop_requested = True

    def run_forever(self, on_stop=None, started=None):
        """Run the loop on the calling thread until ``stop`` or ``request_stop``."""
        self.ident = threading.get_ident()
        if started is not None:
            started.set()
        while True:
            if self._stop_requested:
                self._stop_requested = False
                if on_stop is not None:
                    self._run(on_stop)
                with self._cv:
                    self._stopping = True
            with self._cv:
                if self._stopping:
                    return
                now = time.monotonic()
                calls = list(self._calls)
                self._calls.clear()
                timers = []
                for timer, due in list(self._timers.items()):
                    if due <= now:
                        timers.append(timer)
                        if timer.repeat:
                            self._timers[timer] = now + timer.interval / 1000.0
                        else:
                            del self._timers[timer]
                scheduled = []
                while self._scheduled and self._scheduled[0][0] <= now:
                    scheduled.append(heapq.heappop(self._scheduled)[2])
                if not (calls or timers or scheduled):
                    self._cv.wait(self._idle_wait(now))
                    continue
            for fn in calls:
                self._run(fn)
            for timer in timers:
                if timer.running and timer.callback is not None:
                    self._run(timer.callback)
            for fn in scheduled:
                self._run(fn)

    def _idle_wait(self, now):
        dues = list(self._timers.values())
        if self._scheduled:
            dues.append(self._scheduled[0][0])
        if not dues:
            return MAX_IDLE_WAIT_S
        return max(0.0, min(MAX_IDLE_WAIT_S, min(dues) - now))

    def _run(self, fn):
        try:
            fn()
        except Exception:  # noqa: BLE001 - a host loop survives callback errors, like Live
            self.errors.append(traceback.format_exc())
            traceback.print_exc(file=sys.stderr)

    # --- requests from other threads ---

    def post(self, fn):
        with self._cv:
            self._calls.append(fn)
            self._cv.notify_all()

    def call(self, fn, timeout=5.0):
        """Run ``fn`` on the main thread and return its result (or raise its exception)."""
        if threading.get_ident() == self.ident:
            return fn()
        done = threading.Event()
        box = {}

        def run():
            try:
                box["result"] = fn()
            except BaseException as e:  # noqa: BLE001 - re-raised in the caller's thread
                box["error"] = e
            finally:
                done.set()

        self.post(run)
        if not done.wait(timeout):
            raise TimeoutError("main thread did not run the call in time")
        if "error" in box:
            raise box["error"]
        return box.get("result")

    def stall(self, ms):
        """Block the main thread for ``ms`` milliseconds, without waiting for it."""
        self.post(lambda: time.sleep(ms / 1000.0))

    def schedule(self, delay_ms, fn):
        with self._cv:
            due = time.monotonic() + delay_ms / 1000.0
            heapq.heappush(self._scheduled, (due, next(self._seq), fn))
            self._cv.notify_all()

    # --- Live.Base.Timer hooks ---

    def _add_timer(self, timer):
        with self._cv:
            self._timers[timer] = time.monotonic() + timer.interval / 1000.0
            self._cv.notify_all()

    def _remove_timer(self, timer):
        with self._cv:
            self._timers.pop(timer, None)
