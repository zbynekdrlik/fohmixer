"""The drain gate (#243): the script's real ``Drain`` held, stepped one
command at a time and resumed; the surface's ticks counted."""

import os
import queue
import sys
import unittest

import _simpath
from drain_gate import DrainGate, step_words

SCRIPT_DIR = os.path.join(_simpath.REPO_DIR, "live-script")
if SCRIPT_DIR not in sys.path:
    sys.path.insert(0, SCRIPT_DIR)

from FohMixer.surface import Drain  # noqa: E402 - after the script's path


class Conn:
    is_open = True


class Server:
    def __init__(self):
        self.writes = 0

    def poll_out(self, now=None):
        self.writes += 1


def command(prop):
    return {"target": "live_set", "name": "add_listener", "args": {"prop": prop}}


class DrainGateTest(unittest.TestCase):
    def setUp(self):
        self.inbox = queue.Queue()
        self.ran = []
        self.results = []
        self.conn = Conn()
        # A budget no command reaches (the hand-ticked surface's).
        self.drain = Drain(
            self.inbox,
            lambda cmd, conn: self.ran.append(cmd["args"]["prop"]) or {"ok": True},
            lambda conn, uuid, results: self.results.append((uuid, len(results))),
            lambda conn: None,
            1000,
            1000,
            20,
        )
        self.server = Server()
        self.gate = DrainGate(self.drain, self.server)

    def queue(self, uuid, *props):
        self.inbox.put((self.conn, {"uuid": uuid, "commands": [command(p) for p in props]}))

    def test_an_open_gate_runs_the_drain_as_it_is(self):
        self.queue("a", "x", "y", "z")
        self.drain.run()
        self.assertEqual(self.ran, ["x", "y", "z"])
        self.assertEqual(self.results, [("a", 3)])

    def test_a_held_drain_runs_nothing_until_resumed(self):
        self.gate.hold()
        self.queue("a", "x", "y")
        self.drain.run()
        self.drain.run()
        self.assertEqual(self.ran, [])
        self.gate.resume()
        self.drain.run()
        self.assertEqual(self.ran, ["x", "y"])
        self.assertEqual(self.results, [("a", 2)])

    def test_a_step_runs_exactly_one_command_and_its_batch_answers_at_its_end(self):
        self.gate.hold()
        self.queue("a", "x", "y", "z")
        self.queue("b", "w")
        self.assertEqual(self.gate.step(), command("x"))
        self.assertEqual(self.ran, ["x"])
        self.assertEqual(self.results, [])
        self.assertEqual(self.gate.step(), command("y"))
        self.assertEqual(self.gate.step(), command("z"))
        self.assertEqual(self.results, [("a", 3)])
        self.assertEqual(self.gate.step(), command("w"))
        self.assertEqual(self.results, [("a", 3), ("b", 1)])
        self.assertIsNone(self.gate.step(), "nothing waits")
        # The drain's own budget is back: a resumed run takes the rest.
        self.queue("c", "u", "v")
        self.gate.resume()
        self.drain.run()
        self.assertEqual(self.ran, ["x", "y", "z", "w", "u", "v"])

    def test_the_ticks_count_the_socket_writes(self):
        self.assertEqual(self.gate.ticks, 0)
        self.server.poll_out()
        self.server.poll_out(1.0)
        self.assertEqual(self.gate.ticks, 2)
        self.assertEqual(self.server.writes, 2)

    def test_a_stepped_command_in_words(self):
        self.assertEqual(step_words(command("devices")), "add_listener devices")
        self.assertEqual(step_words({"name": "get_prop"}), "get_prop -")
        self.assertEqual(step_words(None), "-")


if __name__ == "__main__":
    unittest.main()
