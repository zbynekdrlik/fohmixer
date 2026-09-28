# ruff: noqa: N999 - Live remote scripts conventionally name this module Config
"""Per-instance settings. Each Windows user's copy edits ``INSTANCE`` and ``PORT``.

Defaults: band 39101, master 39102. Never 39031 (the ableton-js default,
which AbleSet may adopt). The script always binds 127.0.0.1.
"""

INSTANCE = "band"
PORT = 39101

TIMER_INTERVAL_MS = 10
DRAIN_BUDGET_MS = 5
WINDOW_BUDGET_MS = 5
WINDOW_MS = 20
METER_MIN_INTERVAL_MS = 33
HEARTBEAT_INTERVAL_MS = 100
RESULT_QUEUE_MAX = 1000

# Log folder; None means ``logs/`` next to this file (env FOHMIXER_LOG_DIR overrides).
LOG_DIR = None
