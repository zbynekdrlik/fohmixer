"""SimLive's Listen devices (#210): the announce, the polls, the stream."""

import http.server
import socket
import struct
import threading
import unittest
import urllib.parse

import _simpath
import host
import listendev
import site_builder


class _Inline:
    def call(self, fn):
        return fn()


class _Clock:
    def __init__(self):
        self.now = 100.0

    def __call__(self):
        return self.now


def _site():
    return site_builder.build(_simpath.FIXTURE)


class _FakeHub:
    """The hub's select route: ``on`` names the selected devices; every
    question is kept (its path and query)."""

    def __init__(self):
        self.on = set()
        self.asked = []
        hub = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):  # noqa: N802 (http.server's name)
                url = urllib.parse.urlsplit(self.path)
                hub.asked.append(self.path)
                device = urllib.parse.parse_qs(url.query).get("device", ["-1"])[0]
                body = b'{"on":true}' if int(device) in hub.on else b'{"on":false}'
                if url.path != listendev.SELECT_PATH:
                    self.send_response(404)
                    body = b'{"code":"NOT_FOUND"}'
                else:
                    self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args):
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.port = self.server.server_address[1]
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()


class EncodeTest(unittest.TestCase):
    def test_the_golden_announce(self):
        data = listendev.encode_announce(
            9, "v1.50.0", ["Listen", "Listen", "Vox 1", "FOH Band"], 48000, True
        )
        self.assertEqual(data.hex(), listendev.GOLDEN_ANNOUNCE_HEX)

    def test_only_an_on_answer_selects_the_device(self):
        self.assertIs(listendev.answers_on(b'{"on":true}'), True)
        self.assertIs(listendev.answers_on(b'{"on": true}'), True)
        self.assertIs(listendev.answers_on(b'{"on":false}'), False)
        self.assertIs(listendev.answers_on(b'{"on":"yes"}'), False)
        # No `on`: no answer (the patch's dict.unpack on: outputs nothing).
        self.assertIsNone(listendev.answers_on(b""))
        self.assertIsNone(listendev.answers_on(b"<html>true</html>"))
        self.assertIsNone(listendev.answers_on(b"[true]"))
        self.assertIsNone(listendev.answers_on(b'{"code":"LOOPBACK_ONLY"}'))
        self.assertEqual(
            listendev.select_url(8480, 7), "http://127.0.0.1:8480/api/listen/select?device=7"
        )
        self.assertEqual((listendev.POLL_S, listendev.SELECT_HOLD_S), (0.25, 1.0))

    def test_a_matrix_chunk_is_laid_out_as_jitters(self):
        data = listendev.encode_matrix([0.5, 0.25] * 4, 2, 2, 12.5)
        self.assertEqual(data[:8], b"JMTX" + struct.pack(">i", 288))
        header = data[8 : 8 + 288]
        self.assertEqual(header[:4], b"JMTX")
        self.assertEqual(struct.unpack(">iiii", header[4:20]), (288, 2, 2, 2))
        self.assertEqual(struct.unpack(">ii", header[20:28]), (2, 2))
        self.assertEqual(struct.unpack(">ii", header[148:156]), (8, 16))
        self.assertEqual(struct.unpack(">i", header[276:280]), (32,))
        self.assertEqual(struct.unpack(">d", header[280:288]), (12.5,))
        self.assertEqual(struct.unpack(">2f", data[296:304]), (0.5, 0.25))
        self.assertEqual(len(data), 8 + 288 + 32)

    def test_a_message_chunk_carries_its_atoms(self):
        data = listendev.encode_message(["fohmixer_listen", 1, 9])
        self.assertEqual(data[:4], b"JMMP")
        size = struct.unpack(">i", data[4:8])[0]
        self.assertEqual(len(data), 8 + size)
        self.assertEqual(data[16:32], b"sfohmixer_listen")
        self.assertEqual(data[-5:], b"l" + struct.pack(">i", 9))


class StreamTest(unittest.TestCase):
    """A device on the real sockets, ticked by hand on a fake clock."""

    def setUp(self):
        self.song = _site()
        self.hub_tcp = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.hub_tcp.bind(("127.0.0.1", 0))
        self.hub_tcp.listen(4)
        port = self.hub_tcp.getsockname()[1]
        self.hub_udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.hub_udp.bind(("127.0.0.1", port))
        self.hub_udp.settimeout(5.0)
        self.hub = _FakeHub()
        self.clock = _Clock()
        self.devs = listendev.ListenDevices(self.song, port, self.hub.port, self.clock, polls=False)
        self.devs.start()
        self.devs._timer.stop()

    def tearDown(self):
        self.devs.stop()
        self.hub.close()
        for sock in (self.hub_tcp, self.hub_udp):
            sock.close()

    def select(self, *devices):
        """The hub selects ``devices``; every device asks once."""
        self.hub.on = set(devices)
        self.devs.poll_once()

    def tick_until(self, check):
        for _ in range(200):
            self.devs.tick()
            if check():
                return
            self.clock.now += 0.01
        self.fail("not reached")

    def test_announces_streams_while_selected_and_stops(self):
        track = self.song.tracks[1]
        device = self.devs.add(track, rack=True)
        device.tone = 1000.0
        self.devs.tick()
        announce = self.hub_udp.recv(512)
        self.assertTrue(announce.startswith(b"/fohmixer/listen\0"))
        self.assertIn(track.name.encode(), announce)
        self.assertEqual(announce[-8:], struct.pack(">ii", 48000, 0))
        self.select(device.id)
        self.tick_until(lambda: device.conn is not None)
        conn, _ = self.hub_tcp.accept()
        conn.settimeout(5.0)
        self.clock.now += 0.1
        self.devs.tick()
        got = b""
        while len(got) < 16:
            got += conn.recv(65536)
        self.assertEqual(got[:4], b"JMMP")
        self.assertGreater(device.streamed, 0)
        self.assertEqual(device.streamed % listendev.FRAME, 0)
        # Another device selected: this one stops at once.
        self.select(device.id + 100)
        self.assertEqual(self.hub.asked[-1], f"{listendev.SELECT_PATH}?device={device.id}")
        self.tick_until(lambda: not device.sending(self.clock.now))
        before = device.streamed
        self.clock.now += 1.0
        self.devs.tick()
        self.assertEqual(device.streamed, before)
        conn.close()

    def test_a_device_in_a_rack_that_is_off_neither_announces_nor_streams(self):
        # #238: the PC's sets are saved with every Listen rack off, and the
        # hub switches a track's rack on only while a page listens to it.
        track = self.song.tracks[1]
        device = self.devs.add(track, rack=True, on=False)
        self.assertEqual(device.rack.parameters[0].name, "Device On")
        self.assertEqual(device.rack.parameters[0].value, 0.0)
        self.assertFalse(device.active())
        self.hub_udp.settimeout(0.3)
        self.devs.tick()
        with self.assertRaises(socket.timeout):
            self.hub_udp.recv(512)
        self.select(device.id)
        for _ in range(20):
            self.clock.now += 0.01
            self.devs.tick()
        self.assertIsNone(device.conn, "no stream while the rack is off")
        self.assertEqual(device.streamed, 0)
        # The hub switches the rack on: it announces and streams.
        device.rack.parameters[0].value = 1.0
        self.assertTrue(device.active())
        self.clock.now += listendev.ANNOUNCE_S
        self.select(device.id)
        self.devs.tick()
        self.hub_udp.settimeout(5.0)
        self.assertIn(track.name.encode(), self.hub_udp.recv(512))
        self.tick_until(lambda: device.conn is not None)
        conn, _ = self.hub_tcp.accept()
        self.clock.now += 0.1
        self.devs.tick()
        self.assertGreater(device.streamed, 0)
        # Off again: the stream stops at once, though still selected.
        device.rack.parameters[0].value = 0.0
        before = device.streamed
        self.select(device.id)
        self.clock.now += 0.1
        self.devs.tick()
        self.assertEqual(device.streamed, before)
        conn.close()

    def test_a_device_with_no_rack_or_a_rack_on_runs(self):
        self.assertTrue(self.devs.add(self.song.tracks[0], rack=False).active())
        self.assertTrue(self.devs.add(self.song.tracks[1], rack=True).active())

    def test_a_device_stops_after_the_hold_with_no_answer(self):
        device = self.devs.add(self.song.tracks[0], rack=False)
        self.select(device.id)
        self.tick_until(lambda: device.sending(self.clock.now))
        self.clock.now += listendev.SELECT_HOLD_S
        self.assertFalse(device.sending(self.clock.now))

    def test_two_devices_of_one_host_are_each_told_on_their_own(self):
        # #210, the PC: one UDP select port reached only one device of a
        # Live. Each device asks for itself now.
        first = self.devs.add(self.song.tracks[0], rack=True)
        second = self.devs.add(self.song.tracks[1], rack=True)
        self.select(second.id)
        self.devs.tick()
        self.assertEqual(
            (first.sending(self.clock.now), second.sending(self.clock.now)), (False, True)
        )
        self.select(first.id)
        self.devs.tick()
        self.assertEqual(
            (first.sending(self.clock.now), second.sending(self.clock.now)), (True, False)
        )
        asked = {q.rsplit("=", 1)[1] for q in self.hub.asked}
        self.assertEqual(asked, {str(first.id), str(second.id)})

    def test_a_failed_request_is_no_answer_and_the_hold_ends_it(self):
        device = self.devs.add(self.song.tracks[0], rack=False)
        self.select(device.id)
        self.devs.tick()
        self.assertTrue(device.sending(self.clock.now))
        self.hub.close()
        self.devs.poll_once()
        self.devs.tick()
        self.assertTrue(device.sending(self.clock.now), "within the hold")
        self.clock.now += listendev.SELECT_HOLD_S
        self.devs.tick()
        self.assertFalse(device.sending(self.clock.now))

    def test_no_hub_port_asks_nothing(self):
        port = self.hub_udp.getsockname()[1]
        devs = listendev.ListenDevices(self.song, port, None, self.clock, polls=False)
        devs.start()
        devs._timer.stop()
        self.addCleanup(devs.stop)
        device = devs.add(self.song.tracks[0], rack=False)
        devs.poll_once()
        devs.tick()
        self.assertFalse(device.sending(self.clock.now))
        self.assertEqual(self.hub.asked, [])

    def test_the_owed_frames_are_whole_and_bounded(self):
        device = listendev.ListenDevice(1, None, None)
        device.started = 0.0
        self.assertEqual(device.owed(0.019), 0)
        self.assertEqual(device.owed(0.02), 1)
        self.assertEqual(device.owed(10.0), listendev.MAX_FRAMES)
        device.sent = listendev.FRAME
        self.assertEqual(device.owed(0.05), 1)
        self.assertEqual(listendev.MAX_FRAMES, 50)

    def test_a_96_khz_device_owes_a_frame_every_10_ms_of_its_tone(self):
        device = listendev.ListenDevice(1, None, None, rate=96000)
        device.started = 0.0
        self.assertEqual(device.owed(0.0099), 0)
        self.assertEqual(device.owed(0.01), 1)
        self.assertEqual(device.owed(0.02), 2)
        device.tone = 1000.0
        samples = device.samples(1)
        self.assertEqual(len(samples), 2 * listendev.FRAME)
        # 1 kHz at 96 kHz: a quarter period is 24 samples.
        self.assertAlmostEqual(samples[2 * 24], listendev.LEFT, places=6)


class LineTest(unittest.TestCase):
    def setUp(self):
        self.song = _site()
        self.mt = _Inline()
        self.devs = listendev.ListenDevices(self.song)

    def line(self, text):
        return host.control(self.mt, self.song, text, {}, None, self.devs)

    def test_listenhub_sets_the_hubs_port(self):
        self.assertEqual(self.line("listenhub 8480"), "LISTENHUB 8480")
        self.assertEqual(self.devs.hub_port, 8480)
        for bad in ("listenhub", "listenhub 0", "listenhub 65536", "listenhub x", "listenhub 1 2"):
            self.assertEqual(self.line(bad), "LISTENHUB -1", bad)
        self.assertEqual(self.devs.hub_port, 8480)

    def test_listendev_tone_and_streamed(self):
        self.assertEqual(self.line("listendev track 1 add rack"), "LISTENDEV 1")
        self.assertEqual(self.line("listendev track 1 add"), "LISTENDEV 2")
        self.assertEqual(self.line("tone track 1 440"), "TONE 440")
        self.assertEqual(self.devs.on(self.song.tracks[1])[-1].tone, 440.0)
        self.assertEqual(self.line("streamed track 1"), "STREAMED 0")
        self.assertEqual(self.line("listendev track 1 remove"), "LISTENDEV 1")
        names = [d.name for d in self.song.tracks[1].devices]
        self.assertIn(listendev.RACK_NAME, names)

    def test_a_rack_added_off_and_its_device_on(self):
        self.assertEqual(self.line("listendev track 1 add rack off"), "LISTENDEV 1")
        self.assertEqual(self.line("listenrack track 1"), "LISTENRACK 0")
        self.assertEqual(self.line("listendev track 2 add rack off 96000"), "LISTENDEV 1")
        off = self.devs.on(self.song.tracks[2])[-1]
        self.assertEqual((off.rate, off.active()), (96000, False))
        self.assertEqual(self.line("listendev track 3 add rack"), "LISTENDEV 1")
        self.assertEqual(self.line("listenrack track 3"), "LISTENRACK 1")
        self.assertEqual(self.line("listendev track 4 add"), "LISTENDEV 1")
        self.assertEqual(self.line("listenrack track 4"), "LISTENRACK -1")
        self.assertEqual(self.line("listenrack track 5"), "LISTENRACK -1")

    def test_a_device_at_96_khz(self):
        self.assertEqual(self.line("listendev track 1 add rack 96000"), "LISTENDEV 1")
        self.assertEqual(self.line("listendev track 2 add 96000"), "LISTENDEV 1")
        self.assertEqual(self.line("listendev track 3 add rack"), "LISTENDEV 1")
        rates = [self.devs.on(self.song.tracks[i])[-1].rate for i in (1, 2, 3)]
        self.assertEqual(rates, [96000, 96000, 48000])
        racked = self.devs.on(self.song.tracks[1])[-1]
        self.assertIsNotNone(racked.rack)
        self.assertIsNone(self.devs.on(self.song.tracks[2])[-1].rack)

    def test_bad_lines_answer_minus_one(self):
        for text, word in (
            ("listendev track 1 add rack 1000", "LISTENDEV"),
            ("listendev track 1 add rack 96k", "LISTENDEV"),
            ("listendev track 1 add 96000 rack", "LISTENDEV"),
            ("listendev track 1 add rack 96000 x", "LISTENDEV"),
            ("listendev track 1 remove 96000", "LISTENDEV"),
            ("listendev track 1 add off", "LISTENDEV"),
            ("listendev track 1 add rack off x", "LISTENDEV"),
            ("listendev track 1 add rack 96000 off", "LISTENDEV"),
            ("listenrack track x", "LISTENRACK"),
            ("listenrack track 1 1", "LISTENRACK"),
            ("listendev track 99 add", "LISTENDEV"),
            ("listendev track 1 remove", "LISTENDEV"),
            ("listendev track 1 remove rack", "LISTENDEV"),
            ("listendev bus 1 add", "LISTENDEV"),
            ("tone track 1 440", "TONE"),
            ("tone track 1 30000", "TONE"),
            ("tone track 1 x", "TONE"),
            ("streamed track x", "STREAMED"),
        ):
            self.assertEqual(self.line(text), f"{word} -1", text)

    def test_a_host_without_listen_devices_answers_minus_one(self):
        answer = host.control(self.mt, self.song, "listendev track 1 add", {}, None, None)
        self.assertEqual(answer, "LISTENDEV -1")


if __name__ == "__main__":
    unittest.main()
