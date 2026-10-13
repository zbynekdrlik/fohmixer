"""A harness tool in a process of its own (#43): the impair proxy, the fake
Companion and the fake AbleSet each run as their file's own tool, so the work
of the process that drives them never holds their sockets. As threads of the
harness they shared the interpreter's lock, and the harness's ``GET
/hub/events`` (~250 ms of Python, one ~90 ms ``json.dumps`` that holds the
lock throughout) held every connection of theirs that long (master run
38340390496: a page's ping waited 150 ms in the impair proxy).

The contract, both sides (``Child`` drives it, ``serve`` is the tool's):

- the tool prints ``<READY> <port>`` once it listens, or one JSON line with
  its ``error`` and exits 1 when it cannot;
- then it answers each control line on its stdin with one JSON line, in
  order (an ``error`` for a line it does not know or a value it refuses);
- the end of its stdin stops it: a harness that dies leaves no tool behind.

Its log goes to stderr, the harness's log.
"""

import json
import queue
import signal
import subprocess
import threading

# How long a control call waits for the tool's answer, and its end.
CALL_S = 5.0
# How long start() waits for the tool's first line.
START_S = 5.0


class Child:
    """The tool ``command_line`` (``python3 <tool> ...``) in a process of its
    own, named ``what`` in errors, ready at its ``<ready> <port>`` line. A
    refused control line raises ``ValueError``; a tool that ended or does not
    answer raises ``RuntimeError``."""

    # The end of the tool's stdout, in the queue of its lines.
    ENDED = object()

    def __init__(self, what, ready, command_line):
        self.what = what
        self.ready = ready
        self.command_line = command_line
        self.port = None
        self.proc = None
        self.pump = None
        self.lines = queue.Queue()
        self.lock = threading.Lock()
        # Answers still owed to calls that gave up waiting (``ask``).
        self.owed = 0

    def start(self):
        """Starts the tool and waits for its port; raises when it cannot listen."""
        self.proc = subprocess.Popen(
            self.command_line, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True
        )
        self.pump = threading.Thread(target=self._pump, name=f"{self.ready.lower()}-child")
        self.pump.daemon = True
        self.pump.start()
        first = self._next(START_S)
        if isinstance(first, str) and first.startswith(self.ready + " "):
            self.port = int(first.split()[1])
            return
        if first is None:
            reason = f"no port line within {START_S} s"
        elif first is self.ENDED:
            reason = "it ended without a port line"
        else:
            try:
                reason = json.loads(first)["error"]
            except (ValueError, KeyError, TypeError):
                reason = f"its first line was {first!r}"
        failure = RuntimeError(f"{self.what} did not start: {reason}")
        try:
            self._end()
        except RuntimeError as ending:
            raise failure from ending
        raise failure

    def _pump(self):
        """The tool's stdout lines into the queue, then ``ENDED``; closes it at its end."""
        with self.proc.stdout:
            for line in self.proc.stdout:
                self.lines.put(line.rstrip("\n"))
        self.lines.put(self.ENDED)

    def _next(self, timeout):
        """The tool's next line, ``ENDED`` after its last one, or None after ``timeout``."""
        try:
            return self.lines.get(timeout=timeout)
        except queue.Empty:
            return None

    def ask(self, line):
        """One control line and the tool's answer (a dict). The tool answers
        every line once, in order, so an answer still owed to a call that
        gave up waiting comes first and is read and dropped here: no call
        ever takes another's answer for its own."""
        with self.lock:
            if self.proc is None or self.proc.poll() is not None:
                code = None if self.proc is None else self.proc.returncode
                raise RuntimeError(f"{self.what} is not running (exit code {code}): {line!r}")
            try:
                self.proc.stdin.write(line + "\n")
                self.proc.stdin.flush()
            except BrokenPipeError:
                raise RuntimeError(f"{self.what} ended before {line!r}") from None
            answer = self._next(CALL_S)
            while self.owed and answer is not None and answer is not self.ENDED:
                self.owed -= 1
                answer = self._next(CALL_S)
            if answer is None:
                self.owed += 1
        if answer is None or answer is self.ENDED:
            raise RuntimeError(f"{self.what} gave no answer to {line!r} within {CALL_S} s")
        answer = json.loads(answer)
        if "error" in answer:
            raise ValueError(answer["error"])
        return answer

    def stop(self):
        """Ends the tool through the end of its stdin, SIGTERM if it does not end."""
        if self.proc is None or self.proc.poll() is not None:
            return
        with self.lock:
            self._end()

    def _end(self):
        self.proc.stdin.close()
        try:
            self.proc.wait(CALL_S)
        except subprocess.TimeoutExpired:
            self.proc.send_signal(signal.SIGTERM)
            self.proc.wait(CALL_S)
            raise RuntimeError(
                f"{self.what} did not end within {CALL_S} s of its stdin's end"
            ) from None
        finally:
            # Its stdout ends with it: the pump closes it.
            self.pump.join(CALL_S)


def serve(tool, ready, command, stdin, stdout, log):
    """The tool's side: ``tool.start()`` (a ``RuntimeError`` when it cannot
    listen: one JSON ``error`` line, 1), ``<ready> <tool.port>``, then one JSON
    answer per control line (``command(tool, line)``) until the end of
    ``stdin``, which stops the tool: 0."""
    try:
        tool.start()
    except RuntimeError as e:
        log(f"not started: {e}")
        print(json.dumps({"error": str(e)}), file=stdout, flush=True)
        return 1
    print(f"{ready} {tool.port}", file=stdout, flush=True)
    try:
        for line in stdin:
            if line.strip():
                print(json.dumps(command(tool, line)), file=stdout, flush=True)
    finally:
        tool.stop()
    return 0
