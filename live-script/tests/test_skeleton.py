"""S0 skeleton tests: the FohMixer remote script loads on SimLive (spec §5.3).

Live imports a remote script as a package from its folder and calls
`create_instance(c_instance)`; here SimLive (`sim/`) stands in for Live's own
`Live` and `_Framework` modules.
"""
from __future__ import annotations

import sys
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
for path in (ROOT / "sim", ROOT / "live-script"):
    if str(path) not in sys.path:
        sys.path.insert(0, str(path))

import FohMixer  # noqa: E402
import Live  # noqa: E402


class FakeCInstance:
    """The `c_instance` Live hands a control surface: S0 needs none of it."""


class SkeletonTests(unittest.TestCase):
    def test_create_instance_returns_a_surface_that_disconnects(self) -> None:
        with self.assertLogs("FohMixer", level="WARNING") as logs:
            surface = FohMixer.create_instance(FakeCInstance())
            self.assertIsInstance(surface, FohMixer.FohMixer)
            surface.disconnect()
        self.assertTrue(surface.disconnected, "FohMixer.disconnect() calls ControlSurface's")
        self.assertEqual(
            logs.output,
            [
                f"WARNING:FohMixer:FohMixer {FohMixer.version.VERSION} started on Live 12.1.5",
                f"WARNING:FohMixer:FohMixer {FohMixer.version.VERSION} disconnected",
            ],
        )

    def test_the_surface_keeps_its_c_instance(self) -> None:
        c_instance = FakeCInstance()
        with self.assertLogs("FohMixer", level="WARNING"):
            surface = FohMixer.create_instance(c_instance)
        self.assertIs(surface.c_instance, c_instance)

    def test_the_script_version_is_the_workspace_version(self) -> None:
        cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        self.assertEqual(FohMixer.version.VERSION, cargo["workspace"]["package"]["version"])

    def test_live_is_simlive(self) -> None:
        # The tests must never run against a real `Live` module by accident.
        self.assertTrue(Live.SIMLIVE)
        app = Live.Application.get_application()
        self.assertIs(app, Live.Application.get_application())
        self.assertEqual(
            (app.get_major_version(), app.get_minor_version(), app.get_bugfix_version()),
            (12, 1, 5),
        )


if __name__ == "__main__":
    unittest.main()
