"""SimLive's Listen devices (#210): the announce, the selects, the stream."""

import socket
import struct
import unittest

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


def _select(device, turn=1):
    head = listendev._string(listendev.SELECT_ADDRESS) + listendev._string(",ii")
    return head + struct.pack(">ii", device, turn)


class EncodeTest(unittest.TestCase):
    def test_the_golden_announce(self):
        data = listendev.encode_announce(
            9, "v1.50.0", ["Listen", "Listen", "Vox 1", "FOH Band"], 48000, True
        )
        self.assertEqual(data.hex(), listendev.GOLDEN_ANNOUNCE_HEX)

    def test_a_select_reads_and_anything_else_does_not(self):
        self.assertEqual(listendev.parse_select(_select(7)), 7)
        self.assertEqual(listendev.parse_select(_select(0, 3)), 0)
        self.assertIsNone(listendev.parse_select(_select(7) + b"\0\0\0\0"))
        self.assertIsNone(listendev.parse_select(b"/fohmixer/lufs\0\0"))

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
        probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        probe.bind(("127.0.0.1", 0))
        self.select_port = probe.getsockname()[1]
        probe.close()
        self.clock = _Clock()
        self.devs = listendev.ListenDevices(self.song, port, self.select_port, self.clock)
        self.devs.start()
        self.devs._timer.stop()
        self.sender = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)

    def tearDown(self):
        self.devs.stop()
        for sock in (self.hub_tcp, self.hub_udp, self.sender):
            sock.close()

    def select(self, device):
        self.sender.sendto(_select(device), ("127.0.0.1", self.select_port))

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
        self.tick_until(lambda: not device.sending(self.clock.now))
        before = device.streamed
        self.clock.now += 1.0
        self.devs.tick()
        self.assertEqual(device.streamed, before)
        conn.close()

    def test_a_device_stops_after_the_hold_with_no_select(self):
        device = self.devs.add(self.song.tracks[0], rack=False)
        self.select(device.id)
        self.tick_until(lambda: device.sending(self.clock.now))
        self.clock.now += listendev.SELECT_HOLD_S
        self.assertFalse(device.sending(self.clock.now))

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

    def test_listendev_tone_and_streamed(self):
        self.assertEqual(self.line("listendev track 1 add rack"), "LISTENDEV 1")
        self.assertEqual(self.line("listendev track 1 add"), "LISTENDEV 2")
        self.assertEqual(self.line("tone track 1 440"), "TONE 440")
        self.assertEqual(self.devs.on(self.song.tracks[1])[-1].tone, 440.0)
        self.assertEqual(self.line("streamed track 1"), "STREAMED 0")
        self.assertEqual(self.line("listendev track 1 remove"), "LISTENDEV 1")
        names = [d.name for d in self.song.tracks[1].devices]
        self.assertIn(listendev.RACK_NAME, names)

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
