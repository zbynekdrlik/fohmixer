"""Puts the script (``live-script/``) and SimLive (``sim/``) on ``sys.path``.

Imported first by every test module: ``import FohMixer`` then loads the real
script package, and ``import Live`` loads SimLive, as inside Live.
"""

import os
import sys

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
SCRIPT_DIR = os.path.dirname(TESTS_DIR)
REPO_DIR = os.path.dirname(SCRIPT_DIR)
SIM_DIR = os.path.join(REPO_DIR, "sim")
FIXTURE = os.path.join(SIM_DIR, "fixtures", "test-site.json")
HOST = os.path.join(SIM_DIR, "host.py")

for _dir in (SIM_DIR, SCRIPT_DIR):
    if _dir not in sys.path:
        sys.path.insert(0, _dir)
