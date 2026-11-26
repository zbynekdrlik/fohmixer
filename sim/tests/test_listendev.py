"""SimLive's Listen devices (#210; #264: polled by the hub): the announce,
the hub's polls, the stream."""

import socket
import struct
import time
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


def _osc(address, tags=",", *ints):
    return listendev._string(address) + listendev._string(tags) + struct.pack(f">{len(ints)}i", *ints)


class EncodeTest(unittest.TestCase):
    def test_the_golden_announce(self):
        data = listendev.encode_announce(
            9, "v1.50.0", ["Listen", "Listen", "Vox 1", "FOH Band"], 48000, True, 40007
        )
        self.assertEqual(data.hex(), listendev.GOLDEN_ANNOUNCE_HEX)

    def test_the_hubs_polls_as_the_rust_wire_pins_them(self):
        # #264: crates/fohmixer-proto/src/listen/wire/tests.rs's goldens.
        self.assertEqual(listendev.parse_poll(bytes.fromhex(listendev.GOLDEN_POLL_ON_HEX)), ("poll", True))
        off = bytearray.fromhex(listendev.GOLDEN_POLL_ON_HEX)
        off[-1] = 0
        self.assertEqual(listendev.parse_poll(bytes(off)), ("poll", False))
        self.assertEqual(listendev.parse_poll(bytes.fromhex(listendev.GOLDEN_FRAME_HEX)), ("frame", None))

    def test_anything_else_is_no_poll(self):
        # The patch's `route` drops another kind's poll (the LUFS listener's
        # start sweep reaches every port of the range) and anything else.
        for data in (
            _osc("/fohmixer/meter/poll"),
            _osc("/fohmixer/lufs/poll"),
            _osc("/fohmixer/listen/poll"),
            _osc("/fohmixer/listen/poll", ",i", 1)[:-1],
            b"junk",
            b"",
        ):
            self.assertIsNone(listendev.parse_poll(data), data)
        self.assertEqual(listendev.parse_poll(_osc("/fohmixer/listen/poll", ",i", 2)), ("poll", False))

    def test_the_wire_and_the_range(self):
        self.assertEqual((listendev.WIRE_VERSION, listendev.ANNOUNCE_TAGS), (2, ",iisssssiii"))
        self.assertEqual((listendev.POLL_PORT_MIN, listendev.POLL_PORT_SPAN), (40000, 10000))

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
        data = listendev.encode_message(["fohmixer_listen", 2, 9])
        self.assertEqual(data[:4], b"JMMP")
        size = struct.unpack(">i", data[4:8])[0]
        self.assertEqual(len(data), 8 + size)
        self.assertEqual(data[16:32], b"sfohmixer_listen")
        self.assertEqual(data[-5:], b"l" + struct.pack(">i", 9))


class StreamTest(unittest.TestCase):
    """Devices on the real sockets, ticked by hand on a fake clock; the test
    is the hub (its UDP announce port and TCP audio port, and a poll
    socket of its own)."""

    def setUp(self):
        self.song = _site()
        self.hub_tcp = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.hub_tcp.bind(("127.0.0.1", 0))
        self.hub_tcp.listen(4)
        port = self.hub_tcp.getsockname()[1]
        self.hub_udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.hub_udp.bind(("127.0.0.1", port))
        self.hub_udp.settimeout(5.0)
        self.poller = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.clock = _Clock()
        self.devs = listendev.ListenDevices(self.song, port, self.clock)
        self.devs.start()
        self.devs._timer.stop()

    def tearDown(self):
        self.devs.stop()
        for sock in (self.hub_tcp, self.hub_udp, self.poller):
            sock.close()

    def poll(self, device, on):
        self.poller.sendto(listendev.encode_poll(on), ("127.0.0.1", device.port))

    def frame(self, device):
        self.poller.sendto(listendev.encode_frame(), ("127.0.0.1", device.port))

    def announce(self):
        return self.hub_udp.recv(512)

    def silent(self):
        self.hub_udp.settimeout(0.2)
        try:
            with self.assertRaises(socket.timeout):
                self.hub_udp.recv(512)
        finally:
            self.hub_udp.settimeout(5.0)

    def tick(self, seconds=0.0):
        """SimLive's main thread after ``seconds``: a few ticks 5 ms apart
        (a datagram sent just before may not be in its socket yet)."""
        self.clock.now += seconds
        for _ in range(5):
            self.devs.tick()
            time.sleep(0.005)

    def until(self, check):
        """Ticks (10 ms of the fake clock each) until ``check`` holds."""
        for _ in range(200):
            self.devs.tick()
            if check():
                return
            self.clock.now += 0.01
            time.sleep(0.002)
        self.fail("not reached")

    def test_a_device_announces_its_port_at_load_and_answers_each_poll(self):
        track = self.song.tracks[1]
        device = self.devs.add(track, rack=True)
        self.assertTrue(listendev.POLL_PORT_MIN <= device.port < listendev.POLL_PORT_MIN + listendev.POLL_PORT_SPAN)
        announce = self.announce()
        self.assertTrue(announce.startswith(b"/fohmixer/listen\0"))
        self.assertIn(track.name.encode(), announce)
        self.assertEqual(announce[-12:], struct.pack(">iii", 48000, 0, device.port))
        self.tick(1.0)
        self.silent()
        self.poll(device, False)
        self.tick()
        self.assertEqual(self.announce()[-12:], struct.pack(">iii", 48000, 0, device.port))
        self.silent()

    def test_on_streams_only_at_the_hubs_frame_polls_and_off_stops_at_once(self):
        track = self.song.tracks[1]
        device = self.devs.add(track, rack=True)
        device.tone = 1000.0
        self.announce()
        self.poll(device, True)
        self.until(lambda: device.conn is not None)
        self.assertEqual(self.announce()[-8:-4], struct.pack(">i", 1), "sending")
        conn, _ = self.hub_tcp.accept()
        conn.settimeout(5.0)
        hello = conn.recv(65536)
        self.assertEqual(hello[:4], b"JMMP")
        self.assertIn(b"sfohmixer_listen", hello)
        # No frame poll: nothing streams, whatever the time (no own clock).
        self.tick(1.0)
        self.assertEqual(device.streamed, 0)
        self.frame(device)
        self.until(lambda: device.streamed > 0)
        self.assertEqual(device.streamed % listendev.FRAME, 0)
        got = b""
        while b"JMTX" not in got:
            got += conn.recv(65536)
        # Off: the next frame poll streams nothing.
        self.poll(device, False)
        self.until(lambda: not device.on)
        before = device.streamed
        self.clock.now += 1.0
        self.frame(device)
        self.tick()
        self.assertEqual(device.streamed, before)
        conn.close()

    def test_another_kinds_poll_is_dropped(self):
        device = self.devs.add(self.song.tracks[0], rack=False)
        self.announce()
        self.poller.sendto(_osc("/fohmixer/meter/poll"), ("127.0.0.1", device.port))
        self.tick()
        self.silent()

    def test_a_device_in_a_rack_that_is_off_answers_nothing_until_it_is_on(self):
        # #238: the PC's sets are saved with every Listen rack off, and the
        # hub switches a track's rack on only while a page listens to it;
        # #264: the patch's poll gate is its Device On (`is_active`).
        track = self.song.tracks[1]
        device = self.devs.add(track, rack=True, on=False)
        self.assertEqual(device.rack.parameters[0].name, "Device On")
        self.assertFalse(device.active())
        self.silent()
        self.poll(device, True)
        self.frame(device)
        self.tick(0.1)
        self.silent()
        self.assertIsNone(device.conn, "no stream while the rack is off")
        self.assertEqual(device.streamed, 0)
        # The hub switches the rack on: it announces at once, still off.
        device.rack.parameters[0].value = 1.0
        self.tick()
        self.assertIn(track.name.encode(), self.announce())
        self.assertFalse(device.on, "the poll before is gone")
        self.poll(device, True)
        self.until(lambda: device.conn is not None)
        conn, _ = self.hub_tcp.accept()
        self.frame(device)
        self.until(lambda: device.streamed > 0)
        # Off again: the next frame poll streams nothing, though still on.
        device.rack.parameters[0].value = 0.0
        before = device.streamed
        self.clock.now += 0.5
        self.frame(device)
        self.tick()
        self.assertEqual(device.streamed, before)
        conn.close()

    def test_a_device_with_no_rack_or_a_rack_on_runs(self):
        self.assertTrue(self.devs.add(self.song.tracks[0], rack=False).active())
        self.assertTrue(self.devs.add(self.song.tracks[1], rack=True).active())

    def test_two_devices_of_one_host_have_a_port_each_and_are_told_on_their_own(self):
        # #210, the PC: one UDP select port reached only one device of a
        # Live. Each device listens on a port of its own (#264).
        first = self.devs.add(self.song.tracks[0], rack=True)
        second = self.devs.add(self.song.tracks[1], rack=True)
        self.assertNotEqual(first.port, second.port)
        self.poll(second, True)
        self.poll(first, False)
        self.until(lambda: second.on)
        self.assertEqual((first.on, second.on), (False, True))
        self.poll(first, True)
        self.poll(second, False)
        self.until(lambda: first.on and not second.on)

    def test_a_removed_device_closes_its_poll_socket(self):
        track = self.song.tracks[0]
        device = self.devs.add(track, rack=False)
        self.assertTrue(self.devs.remove(track))
        self.assertIsNone(device.socket)

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


class BindTest(unittest.TestCase):
    def test_a_port_taken_is_drawn_again(self):
        taken = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        taken.bind(("127.0.0.1", 0))
        self.addCleanup(taken.close)
        busy = taken.getsockname()[1]
        draws = iter([busy, 40001])
        sock = listendev.bind_poll_socket(lambda: next(draws))
        self.addCleanup(sock.close)
        self.assertEqual(sock.getsockname()[1], 40001)

    def test_no_free_port_fails_loudly(self):
        taken = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        taken.bind(("127.0.0.1", 0))
        self.addCleanup(taken.close)
        busy = taken.getsockname()[1]
        with self.assertRaises(OSError):
            listendev.bind_poll_socket(lambda: busy)


class LineTest(unittest.TestCase):
    def setUp(self):
        self.song = _site()
        self.mt = _Inline()
        self.devs = listendev.ListenDevices(self.song)

    def tearDown(self):
        self.devs.stop()

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

    def test_the_hub_port_line_is_gone(self):
        # #264: the devices no longer ask the hub's HTTP port.
        self.assertNotEqual(self.line("listenhub 8480"), "LISTENHUB 8480")

    def test_a_host_without_listen_devices_answers_minus_one(self):
        answer = host.control(self.mt, self.song, "listendev track 1 add", {}, None, None)
        self.assertEqual(answer, "LISTENDEV -1")


if __name__ == "__main__":
    unittest.main()
