"""Puts SimLive (``sim/``) on ``sys.path`` for the sim tests.

Imported first by every test module, so ``import Live`` resolves to the fake.
"""

import os
import sys

SIM_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REPO_DIR = os.path.dirname(SIM_DIR)
FIXTURE = os.path.join(SIM_DIR, "fixtures", "test-site.json")

if SIM_DIR not in sys.path:
    sys.path.insert(0, SIM_DIR)
