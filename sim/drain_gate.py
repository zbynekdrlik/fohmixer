"""The script's command drain held and stepped by hand (#243).

A hub test makes a timing window certain with it: a subscription batch run
over several ticks with a change in Live between them. ``hold`` stops the
drain between ticks (requests wait in the script's inbox), ``step`` runs
exactly one queued command through the drain's own loop, ``resume`` lets it
run again. ``ticks`` counts the surface's timer ticks that ended in their
socket writes, so a test can wait until what a change pushed has gone out.

Every method runs on Live's main thread (``MainThread.call``), except
``ticks``, which only reads a counter.
"""

import itertools

# A budget the drain's own loop spends after one command on the step's clock
# (one tick per reading): the check before the command reads 1, the one after
# it reads 4.
STEP_BUDGET_NS = 2


class DrainGate:
    def __init__(self, drain, server):
        self._drain = drain
        self._run = drain.run
        self._command = drain._run_command
        self._poll_out = server.poll_out
        self._held = False
        self._last = None
        self._ticks = 0
        # The surface looks these up at every tick, so the instance's own
        # attributes take their place.
        drain.run = self._gated_run
        drain._run_command = self._seen_command
        server.poll_out = self._counted_poll_out

    @property
    def ticks(self):
        return self._ticks

    def hold(self):
        self._held = True

    def resume(self):
        self._held = False

    def step(self):
        """Runs one queued command; the command, or None when none waited."""
        drain = self._drain
        real_clock, budget = drain._clock, drain._budget_ns
        readings = itertools.count(real_clock())
        drain._clock = lambda: next(readings)
        drain._budget_ns = STEP_BUDGET_NS
        self._last = None
        try:
            self._run()
        finally:
            drain._clock, drain._budget_ns = real_clock, budget
        return self._last

    def _gated_run(self):
        if not self._held:
            self._run()

    def _seen_command(self, command, conn):
        self._last = command
        return self._command(command, conn)

    def _counted_poll_out(self, *args, **kwargs):
        try:
            return self._poll_out(*args, **kwargs)
        finally:
            self._ticks += 1


def step_words(command):
    """A stepped command in a control line's answer: its name and prop."""
    if command is None:
        return "-"
    args = command.get("args") if isinstance(command.get("args"), dict) else {}
    return f"{command.get('name', '-')} {args.get('prop', '-')}"
