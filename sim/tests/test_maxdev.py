"""SimLive's Max for Live devices (#151): their datagrams, placement and senders."""

import socket
import struct
import time
import unittest

import _simpath
import host
import Live
import maxdev
import site_builder
from main_thread import MainThread


class _Inline:
    """Stands in for the host's main thread: runs a call at once."""

    def call(self, fn):
        return fn()


class _Clock:
    def __init__(self):
        self.now = 100.0

    def __call__(self):
        return self.now


class _Captured(maxdev.MaxDevices):
    """Keeps every datagram instead of sending it."""

    def __init__(self, song, clock):
        super().__init__(song, port=1, clock=clock)
        self.sent_data = []

    def _send(self, data):
        self.sent_data.append(data)


def _site():
    return site_builder.build(_simpath.FIXTURE)


def _decode(data):
    """The datagram's fields (the hub's parser's view, for the tests)."""
    rest = data
    strings = []

    def string():
        nonlocal rest
        end = rest.index(b"\0")
        text = rest[:end].decode("utf-8")
        rest = rest[(end + 4) & ~3 :]
        return text

    address, tags = string(), string()
    wire, device = struct.unpack(">ii", rest[:8])
    rest = rest[8:]
    version = string()
    strings = [string() for _ in range(maxdev.PARENTS)]
    first, second = struct.unpack(">ii", rest)
    return address, tags, wire, device, version, strings, first, second


class EncodeTest(unittest.TestCase):
    def test_encodes_the_golden_datagram(self):
        data = maxdev.encode(
            maxdev.LUFS, 7, "v1.19.0", ["LUFS IN", "CORE Effects", "Vox 1", "FOH Band"], -253, -248
        )
        self.assertEqual(data.hex(), maxdev.GOLDEN_HEX)
        self.assertEqual(len(data), 96)

    def test_a_meter_datagram_has_its_address_and_values(self):
        data = maxdev.encode(maxdev.METER, 2, "v1.19.0", ["Vox 1", "FOH Band", "-", "-"], 120, -700)
        self.assertEqual(
            _decode(data),
            (
                "/fohmixer/meter",
                ",iisssssii",
                1,
                2,
                "v1.19.0",
                ["Vox 1", "FOH Band", "-", "-"],
                120,
                -700,
            ),
        )

    def test_four_parents_or_none(self):
        with self.assertRaises(ValueError):
            maxdev.encode(maxdev.LUFS, 1, "v1", ["a", "b"], 0, 0)


class PlacementTest(unittest.TestCase):
    def setUp(self):
        self.song = _site()
        self.clock = _Clock()
        self.devs = _Captured(self.song, self.clock)
        self.track = self.song.tracks[1]

    def test_a_device_on_a_track_names_track_and_set(self):
        placed = self.devs.add(self.track, maxdev.LUFS, rack=False)
        self.assertIs(self.track.devices[len(self.track.devices) - 1], placed.device)
        self.assertEqual(placed.device.name, "FohMixer LUFS")
        self.assertEqual(placed.device.class_name, "MxDeviceAudioEffect")
        self.assertEqual(placed.parents(self.song), [self.track.name, self.song.name, "-", "-"])

    def test_a_device_in_a_rack_sits_in_its_own_parallel_chain(self):
        placed = self.devs.add(self.track, maxdev.METER, rack=True)
        rack = self.track.devices[len(self.track.devices) - 1]
        self.assertEqual(rack.name, "LUFS")
        self.assertTrue(rack.can_have_chains)
        self.assertEqual([c.name for c in rack.chains], ["Dry", "LUFS"])
        self.assertEqual(len(rack.chains[0].devices), 0)
        self.assertIs(rack.chains[1].devices[0], placed.device)
        self.assertEqual(placed.device.name, "FohMixer Meter")
        self.assertEqual(
            placed.parents(self.song), ["LUFS", "LUFS", self.track.name, self.song.name]
        )

    def test_ids_count_from_one_per_host(self):
        first = self.devs.add(self.track, maxdev.LUFS, rack=False)
        second = self.devs.add(self.song.tracks[2], maxdev.LUFS, rack=True)
        other = _Captured(self.song, self.clock).add(self.track, maxdev.LUFS, rack=False)
        self.assertEqual((first.id, second.id, other.id), (1, 2, 1))

    def test_remove_takes_the_newest_and_its_rack(self):
        before = len(self.track.devices)
        self.devs.add(self.track, maxdev.LUFS, rack=False)
        self.devs.add(self.track, maxdev.LUFS, rack=True)
        self.assertTrue(self.devs.remove(self.track, maxdev.LUFS))
        self.assertEqual(len(self.track.devices), before + 1)
        self.assertEqual(self.track.devices[before].name, "FohMixer LUFS")
        self.assertTrue(self.devs.remove(self.track, maxdev.LUFS))
        self.assertFalse(self.devs.remove(self.track, maxdev.LUFS))
        self.assertEqual(len(self.track.devices), before)


class SenderTest(unittest.TestCase):
    def setUp(self):
        self.song = _site()
        self.clock = _Clock()
        self.devs = _Captured(self.song, self.clock)
        self.track = self.song.tracks[1]
        self.devs.add(self.track, maxdev.LUFS, rack=False)

    def run_for(self, seconds, step=0.005):
        end = self.clock.now + seconds
        while self.clock.now < end - 1e-9:
            self.devs.tick()
            self.clock.now += step

    def test_sends_its_values_at_its_rate(self):
        self.assertTrue(self.devs.send(self.track, maxdev.LUFS, -300, -253))
        self.run_for(1.0)
        self.assertEqual(len(self.devs.sent_data), 5)
        fields = _decode(self.devs.sent_data[0])
        self.assertEqual(fields[0], "/fohmixer/lufs")
        self.assertEqual((fields[3], fields[6], fields[7]), (1, -300, -253))

    def test_off_stops_sending(self):
        self.devs.send(self.track, maxdev.LUFS, -300, -253)
        self.run_for(0.5)
        sent = len(self.devs.sent_data)
        self.assertTrue(self.devs.send(self.track, maxdev.LUFS, None, None))
        self.run_for(1.0)
        self.assertEqual(len(self.devs.sent_data), sent)
        self.assertEqual(self.devs.sending(), 0)

    def test_a_rename_changes_the_name_field(self):
        self.devs.send(self.track, maxdev.LUFS, -300, -253)
        self.run_for(0.1)
        self.track._sim_set("name", "Vox 9")
        self.run_for(0.2)
        names = [_decode(d)[5][0] for d in self.devs.sent_data]
        self.assertNotEqual(names[0], "Vox 9")
        self.assertEqual(names[-1], "Vox 9")

    def test_a_flood_sends_at_its_rate(self):
        self.devs.send(self.track, maxdev.LUFS, -300, -300, rate=200.0)
        self.run_for(1.0)
        self.assertEqual(len(self.devs.sent_data), 200)

    def test_a_late_tick_sends_what_it_owes_at_most_a_seconds_worth(self):
        self.devs.send(self.track, maxdev.LUFS, -300, -300)
        self.devs.tick()
        self.clock.now += 0.35
        self.devs.tick()
        self.assertEqual(len(self.devs.sent_data), 2)
        self.clock.now += 5.0
        self.devs.tick()
        self.assertEqual(len(self.devs.sent_data), 7)

    def test_a_meter_sends_thirty_per_second(self):
        self.devs.add(self.track, maxdev.METER, rack=True)
        self.assertTrue(self.devs.send(self.track, maxdev.METER, 120, -700))
        self.run_for(1.0)
        self.assertEqual(len(self.devs.sent_data), 30)
        self.assertEqual(_decode(self.devs.sent_data[0])[5][:2], ["LUFS", "LUFS"])

    def test_a_track_without_the_device_sends_nothing(self):
        self.assertFalse(self.devs.send(self.track, maxdev.METER, 0, 0))


class RealSocketTest(unittest.TestCase):
    def test_sends_five_per_second(self):
        receiver = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.addCleanup(receiver.close)
        receiver.bind(("127.0.0.1", 0))
        receiver.settimeout(0.05)
        song = _site()
        main = MainThread().start()
        self.addCleanup(main.stop)
        devs = maxdev.MaxDevices(song, receiver.getsockname()[1])

        def start():
            devs.start()
            devs.add(song.tracks[1], maxdev.LUFS, rack=True)
            devs.send(song.tracks[1], maxdev.LUFS, -300, -253)

        main.call(start)
        self.addCleanup(lambda: main.call(devs.stop))
        got = []
        end = time.monotonic() + 1.0
        while time.monotonic() < end:
            try:
                got.append(receiver.recv(1024))
            except TimeoutError:
                continue
        self.assertIn(len(got), range(4, 7))
        self.assertEqual(_decode(got[0])[5], ["LUFS", "LUFS", song.tracks[1].name, song.name])


class LineTest(unittest.TestCase):
    def setUp(self):
        self.song = _site()
        self.devs = _Captured(self.song, _Clock())

    def line(self, text):
        return host.control(_Inline(), self.song, text, {}, self.devs)

    def test_maxdev_adds_and_removes(self):
        self.assertEqual(self.line("maxdev lufs track 1 add rack\n"), "MAXDEV 1")
        self.assertEqual(self.line("maxdev lufs track 1 add"), "MAXDEV 2")
        self.assertEqual(self.line("maxdev meter return 0 add"), "MAXDEV 1")
        self.assertEqual(self.line("maxdev meter master 0 add rack"), "MAXDEV 1")
        self.assertEqual(self.line("maxdev lufs track 1 remove"), "MAXDEV 1")
        self.assertEqual(self.line("maxdev lufs track 2 remove"), "MAXDEV -1")
        self.assertEqual(self.line("maxdev lufs track 99 add"), "MAXDEV -1")

    def test_bad_maxdev_lines_answer_minus_one(self):
        for text in (
            "maxdev",
            "maxdev gain track 1 add",
            "maxdev lufs bus 1 add",
            "maxdev lufs track x add",
            "maxdev lufs track 1 move",
            "maxdev lufs track 1 remove rack",
            "maxdev lufs track 1 add shelf",
        ):
            self.assertEqual(self.line(text), "MAXDEV -1", text)

    def test_readings_start_and_stop(self):
        self.line("maxdev lufs track 1 add")
        self.line("maxdev meter track 1 add rack")
        self.assertEqual(self.line("lufs track 1 -300 -253"), "LUFS 1")
        self.assertEqual(self.line("level track 1 -120 -180"), "LEVEL 2")
        self.assertEqual(self.line("flood track 1 200"), "FLOOD 2")
        self.assertEqual(self.line("lufs track 1 off"), "LUFS 1")
        self.assertEqual(self.line("level track 1 off"), "LEVEL 0")

    def test_bad_reading_lines_answer_minus_one(self):
        self.line("maxdev lufs track 1 add")
        for text, word in (
            ("lufs track 1 1 -253", "LUFS"),
            ("lufs track 1 -701 -253", "LUFS"),
            ("lufs track 1 -300", "LUFS"),
            ("lufs track 2 -300 -300", "LUFS"),
            ("level track 1 121 0", "LEVEL"),
            ("level track 1 0 0", "LEVEL"),
            ("flood track 1 0", "FLOOD"),
            ("flood track 1 fast", "FLOOD"),
        ):
            self.assertEqual(self.line(text), f"{word} -1", text)

    def test_without_a_port_nothing_sends(self):
        devs = maxdev.MaxDevices(self.song, port=None)
        answer = host.control(_Inline(), self.song, "maxdev lufs track 1 add", {}, devs)
        self.assertEqual(answer, "MAXDEV 1")
        answer = host.control(_Inline(), self.song, "lufs track 1 -300 -300", {}, devs)
        self.assertEqual(answer, "LUFS -1")

    def test_a_host_without_max_devices_answers_minus_one(self):
        self.assertEqual(
            host.control(_Inline(), self.song, "maxdev lufs track 1 add", {}), "MAXDEV -1"
        )


class LiveClassTest(unittest.TestCase):
    def test_the_device_is_a_device_of_lives_module(self):
        device = Live.Device.Device(maxdev.NAMES[maxdev.LUFS], maxdev.CLASS_NAME)
        self.assertEqual(type(device).__name__, "Device")


if __name__ == "__main__":
    unittest.main()
