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
from FohMixer.version import VERSION  # noqa: E402


class FakeCInstance:
    """The `c_instance` Live hands a control surface: the skeleton uses none of it."""


def sim_live_version() -> str:
    app = Live.Application.get_application()
    return f"{app.get_major_version()}.{app.get_minor_version()}.{app.get_bugfix_version()}"


class SkeletonTests(unittest.TestCase):
    def test_create_instance_returns_a_surface_that_logs_start_and_disconnect(self) -> None:
        with self.assertLogs("FohMixer", level="WARNING") as logs:
            surface = FohMixer.create_instance(FakeCInstance())
            self.assertEqual(type(surface).__name__, "FohMixer")
            surface.disconnect()
        self.assertEqual(
            logs.output,
            [
                f"WARNING:FohMixer:FohMixer {VERSION} started on Live {sim_live_version()}",
                f"WARNING:FohMixer:FohMixer {VERSION} disconnected",
            ],
        )

    def test_the_surface_is_a_framework_control_surface(self) -> None:
        from _Framework.ControlSurface import ControlSurface

        with self.assertLogs("FohMixer", level="WARNING"):
            surface = FohMixer.create_instance(FakeCInstance())
        self.assertIsInstance(surface, ControlSurface)

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
