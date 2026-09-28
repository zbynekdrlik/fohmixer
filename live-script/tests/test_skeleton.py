"""Skeleton tests: the FohMixer remote script loads on SimLive (spec §5.3).

Live imports a remote script as a package from its folder and calls
`create_instance(c_instance)`; here SimLive (`sim/`) stands in for Live's own
`Live` and `_Framework` modules.
"""

from __future__ import annotations

import shutil
import tempfile
import tomllib
import unittest
from pathlib import Path

import _paths
import FohMixer
import Live
import site_builder
from c_instance import CInstance
from FohMixer import Config
from FohMixer.version import VERSION
from main_thread import MainThread

ROOT = Path(_paths.REPO_DIR)


class SkeletonTests(unittest.TestCase):
    def setUp(self) -> None:
        log_dir = tempfile.mkdtemp(prefix="fohmixer-skeleton-")
        self.addCleanup(shutil.rmtree, log_dir, True)
        saved = (Config.PORT, Config.LOG_DIR, Config.INSTANCE)
        self.addCleanup(self._restore, saved)
        Config.PORT = 0
        Config.LOG_DIR = log_dir
        Config.INSTANCE = "skeleton"

    @staticmethod
    def _restore(saved) -> None:
        Config.PORT, Config.LOG_DIR, Config.INSTANCE = saved

    def test_create_instance_returns_a_framework_surface_that_logs_start_and_stop(self) -> None:
        from _Framework.ControlSurface import ControlSurface

        song = site_builder.build(_paths.FIXTURE)
        mt = MainThread().start()
        self.addCleanup(mt.stop)
        with self.assertLogs("fohmixer.skeleton", level="WARNING") as logs:
            surface = mt.call(lambda: FohMixer.create_instance(CInstance(song)))
            self.assertTrue(surface.server.wait_bound(2.0))
            self.assertIsInstance(surface, ControlSurface)
            self.assertEqual(type(surface).__name__, "FohMixer")
            mt.call(surface.disconnect)
        self.assertTrue(
            any(f"FohMixer {VERSION} started: instance skeleton" in line for line in logs.output),
            logs.output,
        )
        self.assertTrue(
            any("FohMixer stopped: instance skeleton" in line for line in logs.output),
            logs.output,
        )

    def test_the_script_version_is_the_workspace_version(self) -> None:
        cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        self.assertEqual(VERSION, cargo["workspace"]["package"]["version"])

    def test_live_is_simlive(self) -> None:
        # The tests must never run against a real `Live` module by accident.
        self.assertTrue(Path(Live.__file__).resolve().is_relative_to(ROOT / "sim"))
        app = Live.Application.get_application()
        self.assertIs(app, Live.Application.get_application())
        self.assertEqual(app.get_major_version(), 12)


if __name__ == "__main__":
    unittest.main()
