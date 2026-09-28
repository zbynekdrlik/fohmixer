# Testing the FohMixer script and SimLive

The real script (`live-script/FohMixer/`) runs on SimLive (`sim/`), a fake of
Live's `Live` and `_Framework` modules. Nothing here needs Live.

## What the CI `python` job must run

Python **3.11** (the version Live 12 embeds), on a Linux runner, from the repo root:

```bash
python3.11 -m venv .venv
.venv/bin/pip install -r requirements-dev.txt        # websockets==13.1 (test client only)
.venv/bin/pip install ruff                           # pin the version the S0 job pins
.venv/bin/ruff check live-script sim
.venv/bin/ruff format --check live-script sim
.venv/bin/python -m unittest discover -s sim/tests -v 2>&1 | tee sim-tests.log
.venv/bin/python -m unittest discover -s live-script/tests -v 2>&1 | tee script-tests.log
```

The job must fail when a test is skipped, so a green run means every test ran.
Check the logs after the runs (the `tee` pipelines need `set -o pipefail`):

```bash
set -euo pipefail
! grep -E "skipped|expected failure" sim-tests.log script-tests.log
```

Expected today: `Ran 21 tests` (sim) and `Ran 97 tests` (script), both `OK`,
about 20 s in total.

## Notes for the runner

- Linux only: `test_transport` reads `/proc/net/tcp` to prove a closed server
  leaves no socket in TIME_WAIT (so the port binds again at once).
- `test_integration` starts `sim/host.py` subprocesses with the same
  interpreter and talks to them over 127.0.0.1 WebSockets. Ports are chosen by
  the OS (`--port 0`), except one test that needs two consecutive free ports.
- Timing assertions are made on typical values (median / 90th percentile)
  plus a generous worst case, so single scheduler hiccups on a shared runner do
  not fail the job, while a real block (seconds, the 3 s send timeout) does.
- Logs go to temporary folders, never into the script folder.
- Ruff: the code is clean under both old (0.6) and new (0.16) default rule sets.

## Running the script on SimLive by hand (and from the hub tests, S3)

```bash
python3 sim/host.py --port 39101 --instance band --site sim/fixtures/test-site.json --meters-hz 30
```

It prints `READY <port>` once the WebSocket server is bound and exits 0 on
SIGTERM after the script's `disconnect()`. Control lines on stdin (the hub's
tests use them; `live-script/tests/test_integration.py` tests them):

- `stall <ms>` blocks the simulated main thread;
- `rename "<old>" "<new>"` renames every track and return named `<old>`, as a
  user in Live would (the `name` listeners fire), and prints `RENAMED <count>`;
- `listeners <prop> <path>` prints `LISTENERS <n>`, the Live listeners on
  `<prop>` of the object at the LOM `<path>` (`-1` when the path does not
  resolve): the hub's tests prove one listener per key with it.

The hub's Rust tests (`crates/fohmixer-hub/tests/`) start it with `python3`
(`FOHMIXER_PYTHON` overrides); it needs nothing beyond the standard library.
