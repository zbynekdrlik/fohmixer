"""The fake Companion (#52, ``fake_companion.py``) as the hub sees it: BEGIN
and CAPS first; ADD-DEVICE answered OK, BRIGHTNESS and one KEY-STATE per key
(a PNG data URL); a press answered OK and then the key's new state (ERROR
while failing), and recorded; PING answered PONG; an unknown command a bare
ERROR; every line ending with a space, as Companion 5.0.7 writes them. down
closes every connection and new ones at once; up lets them in again."""

import base64
import os
import socket
import struct
import sys
import time
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import fake_companion  # noqa: E402

# How long a test waits for a line that must come.
COME_S = 3.0
ADD = (
    'ADD-DEVICE DEVICEID="fohmixer-1" SERIAL="fohmixer" PRODUCT_NAME="fohmixer" '
    "KEYS_TOTAL=32 KEYS_PER_ROW=8 BITMAPS=144 BITMAP_FORMAT=webp COLORS=hex TEXT=0 BRIGHTNESS=0"
)


class Conn:
    """A client of the fake: lines in and out."""

    def __init__(self, port):
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=COME_S)
        self.file = self.sock.makefile("rb")

    def read(self):
        """The next line, its line end kept; None once the fake closed it."""
        raw = self.file.readline()
        return raw.decode() if raw else None

    def send(self, text):
        self.sock.sendall((text + "\n").encode())

    def close(self):
        self.file.close()
        self.sock.close()


class FakeCompanionTest(unittest.TestCase):
    def setUp(self):
        self.fake = fake_companion.FakeCompanion(0)
        self.fake.start()
        self.addCleanup(self.fake.stop)

    def connect(self):
        conn = Conn(self.fake.port)
        self.addCleanup(conn.close)
        return conn

    def registered(self):
        """A connection past ADD-DEVICE and its 32 key states."""
        conn = self.connect()
        conn.read()
        conn.read()
        conn.send(ADD)
        conn.read()
        conn.read()
        for _ in range(32):
            conn.read()
        return conn

    def test_the_handshake_and_the_keys(self):
        conn = self.connect()
        self.assertEqual(conn.read(), 'BEGIN CompanionVersion="5.0.7+fake" ApiVersion="1.12.0" \n')
        self.assertEqual(
            conn.read(), 'CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS="rgb,png,webp" \n'
        )
        conn.send(ADD)
        self.assertEqual(conn.read(), 'ADD-DEVICE OK DEVICEID="fohmixer-1" \n')
        self.assertEqual(conn.read(), 'BRIGHTNESS DEVICEID="fohmixer-1" VALUE=100 \n')
        states = [conn.read() for _ in range(32)]
        self.assertTrue(all(s.startswith("KEY-STATE ") and s.endswith(" \n") for s in states))
        params = fake_companion.params(states[9].rstrip("\n"))
        self.assertEqual(
            (params["DEVICEID"], params["KEY"], params["LOCATION"], params["PRESSED"]),
            ("fohmixer-1", "9", "1/1/1", "0"),
        )
        header, data = params["BITMAP"].split(",", 1)
        self.assertEqual(header, "data:image/png;base64")
        image = base64.b64decode(data)
        self.assertEqual(image[:8], b"\x89PNG\r\n\x1a\n")
        self.assertEqual(struct.unpack(">II", image[16:24]), (8, 8))
        self.assertEqual(params["COLOR"], "#%02x%02x%02x" % fake_companion.colour(9, False))
        self.assertEqual(self.fake.state()["connections"], 1)

    def test_a_press_is_answered_recorded_and_redrawn(self):
        conn = self.registered()
        conn.send('KEY-PRESS DEVICEID="fohmixer-1" KEY=3 PRESSED=1')
        self.assertEqual(conn.read(), 'KEY-PRESS OK DEVICEID="fohmixer-1" \n')
        state = fake_companion.params(conn.read().rstrip("\n"))
        self.assertEqual((state["KEY"], state["PRESSED"]), ("3", "1"))
        self.assertEqual(state["COLOR"], "#%02x%02x%02x" % fake_companion.colour(3, True))
        self.assertNotEqual(fake_companion.colour(3, True), fake_companion.colour(3, False))
        self.assertEqual(
            [(p["key"], p["pressed"]) for p in self.fake.state()["presses"]], [(3, True)]
        )
        self.assertEqual(self.fake.fail(True)["failing"], True)
        conn.send('KEY-PRESS DEVICEID="fohmixer-1" KEY=3 PRESSED=0')
        self.assertEqual(
            conn.read(), 'KEY-PRESS ERROR DEVICEID="fohmixer-1" MESSAGE="test refusal" \n'
        )
        self.assertEqual(len(self.fake.state()["presses"]), 2, "a refused press is recorded too")
        self.assertEqual(self.fake.clear()["presses"], [])
        conn.send("PING 7")
        self.assertEqual(conn.read(), "PONG 7 \n")
        conn.send("FOO")
        self.assertEqual(conn.read(), 'ERROR MESSAGE="Unknown command: FOO" \n')

    def test_down_closes_every_connection_and_up_lets_them_in(self):
        conn = self.registered()
        self.assertEqual(self.fake.down()["down"], True)
        self.assertIsNone(conn.read(), "closed")
        self.assertIsNone(self.connect().read(), "a new connection is closed at once")
        self.assertEqual(self.fake.up()["down"], False)
        self.assertTrue(self.connect().read().startswith("BEGIN "))

    def test_a_png_is_a_solid_square(self):
        image = fake_companion.png((10, 20, 30), side=4)
        self.assertEqual(image[:8], b"\x89PNG\r\n\x1a\n")
        self.assertEqual(struct.unpack(">II", image[16:24]), (4, 4))
        self.assertEqual(fake_companion.params('X A=1 B="x y" C'), {"A": "1", "B": "x y"})

    def add(self, conn, device):
        """Registers ``device`` on ``conn`` and reads its states back, parsed."""
        conn.send(ADD.replace("fohmixer-1", device))
        conn.read()
        conn.read()
        return [fake_companion.params(conn.read().rstrip("\n")) for _ in range(32)]

    def test_a_silent_client_is_closed_and_a_key_held_stays_held_for_any_device(self):
        fake = fake_companion.FakeCompanion(0, idle_s=0.4)
        fake.start()
        self.addCleanup(fake.stop)

        def open_conn():
            conn = Conn(fake.port)
            self.addCleanup(conn.close)
            conn.read()
            conn.read()
            return conn

        conn = open_conn()
        self.add(conn, "fohmixer-1")
        conn.send('KEY-PRESS DEVICEID="fohmixer-1" KEY=5 PRESSED=1')
        conn.read()
        conn.read()
        for n in range(3):
            time.sleep(0.3)
            conn.send(f"PING {n}")
            self.assertEqual(conn.read(), f"PONG {n} \n", "a byte from the client keeps it open")
        self.assertIsNone(conn.read(), "closed after the idle time without a byte")
        # The hub's device id changes with every connection: the key is held per key.
        again = open_conn()
        states = self.add(again, "fohmixer-2")
        self.assertEqual([s["PRESSED"] for s in states if s["KEY"] == "5"], ["1"])
        self.assertEqual(sum(s["PRESSED"] == "1" for s in states), 1)
        again.send('KEY-PRESS DEVICEID="fohmixer-2" KEY=5 PRESSED=0')
        again.read()
        again.read()
        again.close()
        third = open_conn()
        states = self.add(third, "fohmixer-3")
        self.assertEqual(sum(s["PRESSED"] == "1" for s in states), 0, "released by the new device")

    def test_a_press_companion_refuses_is_refused(self):
        conn = self.connect()
        conn.read()
        conn.read()
        conn.send('KEY-PRESS DEVICEID="fohmixer-1" KEY=3 PRESSED=1')
        self.assertEqual(
            conn.read(), 'KEY-PRESS ERROR DEVICEID="fohmixer-1" MESSAGE="Device not found" \n'
        )
        conn.send("KEY-PRESS KEY=3 PRESSED=1")
        self.assertEqual(conn.read(), 'KEY-PRESS ERROR DEVICEID="" MESSAGE="Device not found" \n')
        self.add(conn, "fohmixer-1")
        conn.send('KEY-PRESS DEVICEID="other" KEY=3 PRESSED=1')
        self.assertEqual(
            conn.read(), 'KEY-PRESS ERROR DEVICEID="other" MESSAGE="Device not found" \n'
        )
        for key in ("32", "-1", "x", "", "3.5", "\u0663"):
            conn.send(f'KEY-PRESS DEVICEID="fohmixer-1" KEY={key} PRESSED=1')
            self.assertEqual(
                conn.read(), 'KEY-PRESS ERROR DEVICEID="fohmixer-1" MESSAGE="Invalid KEY" \n', key
            )
        conn.send('KEY-PRESS DEVICEID="fohmixer-1" PRESSED=1')
        self.assertEqual(
            conn.read(), 'KEY-PRESS ERROR DEVICEID="fohmixer-1" MESSAGE="Invalid KEY" \n'
        )
        conn.send('KEY-PRESS DEVICEID="fohmixer-1" KEY=31 PRESSED=1')
        self.assertEqual(conn.read(), 'KEY-PRESS OK DEVICEID="fohmixer-1" \n')
        self.assertEqual(
            [(p["key"], p["pressed"]) for p in self.fake.state()["presses"]], [(31, True)]
        )

    def test_garbage_in_add_device_is_an_error_line_not_a_closed_socket(self):
        conn = self.connect()
        conn.read()
        conn.read()
        for bad in ("KEYS_TOTAL=x", "KEYS_TOTAL=32 KEYS_PER_ROW=0", "KEYS_TOTAL=32 KEYS_PER_ROW=y"):
            conn.send(f'ADD-DEVICE DEVICEID="fohmixer-1" {bad}')
            self.assertTrue(conn.read().startswith("ERROR MESSAGE="), bad)
        conn.send("PING 1")
        self.assertEqual(conn.read(), "PONG 1 \n")
        self.assertEqual(self.fake.state()["devices"], [])

    def test_the_add_device_parameters_are_in_the_state(self):
        conn = self.registered()
        devices = self.fake.state()["devices"]
        self.assertEqual(len(devices), 1)
        self.assertEqual(
            {k: devices[0][k] for k in ("DEVICEID", "SERIAL", "KEYS_TOTAL", "KEYS_PER_ROW")},
            {
                "DEVICEID": "fohmixer-1",
                "SERIAL": "fohmixer",
                "KEYS_TOTAL": "32",
                "KEYS_PER_ROW": "8",
            },
        )
        self.assertEqual((devices[0]["BITMAP_FORMAT"], devices[0]["COLORS"]), ("webp", "hex"))
        conn.close()
        for _ in range(50):
            if not self.fake.state()["devices"]:
                break
            time.sleep(0.05)
        self.assertEqual(self.fake.state()["devices"], [], "gone with its connection")

    def test_down_answers_with_no_connections_left(self):
        for _ in range(3):
            self.registered()
        self.assertEqual(self.fake.state()["connections"], 3)
        self.assertEqual(self.fake.down()["connections"], 0)

    def test_reset_puts_everything_back(self):
        conn = self.registered()
        conn.send('KEY-PRESS DEVICEID="fohmixer-1" KEY=3 PRESSED=1')
        conn.read()
        conn.read()
        self.fake.fail(True)
        self.fake.down()
        answer = self.fake.reset()
        self.assertEqual((answer["down"], answer["failing"], answer["presses"]), (False, False, []))
        again = self.add(self.connect_past_handshake(), "fohmixer-2")
        self.assertEqual(sum(s["PRESSED"] == "1" for s in again), 0, "held keys forgotten")

    def connect_past_handshake(self):
        conn = self.connect()
        conn.read()
        conn.read()
        return conn

    def test_stop_ends_with_a_client_still_connected(self):
        fake = fake_companion.FakeCompanion(0)
        fake.start()
        conn = Conn(fake.port)
        self.addCleanup(conn.close)
        conn.read()
        fake.stop()
        self.assertFalse(fake.thread.is_alive())


if __name__ == "__main__":
    unittest.main()
