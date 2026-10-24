"""SimLive for the autotune keys (#140 PR 2): the song clock and a jump of
``current_song_time``, Live's bar (``root_note``, ``scale_name``), a
parameter a Max map holds (disabled: Live refuses its writes), a plug-in's
parameter keeping a written value as Live's float32 (PR 2b), their fixture
keys and the host's ``song`` and ``param`` lines. Names are invented
(public repository)."""

import sys
import unittest

import _simpath  # noqa: F401 - puts SimLive on sys.path first
import host
import Live
import site_builder

if host.SCRIPT_DIR not in sys.path:
    sys.path.insert(0, host.SCRIPT_DIR)

REFUSED = "Value cannot be set, the parameter is disabled"
SLOT_0 = "live_set tracks 0 devices 0 chains 0 devices 0 parameters 1"
SLOT_1 = "live_set tracks 0 devices 0 chains 0 devices 0 parameters 2"


class _Inline:
    """Stands in for the host's main thread: runs a call at once."""

    def call(self, fn):
        return fn()


class _Counter:
    def __init__(self):
        self.count = 0

    def __call__(self):
        self.count += 1


class _Clock:
    """A hand-moved clock (seconds)."""

    def __init__(self, now=100.0):
        self.now = now

    def __call__(self):
        return self.now


def _site():
    return site_builder.build_from_spec(
        {
            "name": "Keys Site",
            "current_song_time": 12.0,
            "root_note": 5,
            "scale_name": "Minor",
            "tracks": [
                {
                    "name": "Stream A #",
                    "devices": [
                        {
                            "template": "rack",
                            "name": "Latencies",
                            "chains": [
                                {
                                    "name": "Auto",
                                    "devices": [
                                        {
                                            "template": "plugin",
                                            "name": "AudioGridder",
                                            "parameters": [
                                                {"name": "0:(unused):(unassigned)", "value": 0.25},
                                                {
                                                    "name": "1:(unused):(unassigned)",
                                                    "enabled": False,
                                                },
                                            ],
                                        }
                                    ],
                                },
                                {"name": "Dry"},
                            ],
                        }
                    ],
                }
            ],
        }
    )


def _slot(song, n):
    return song.tracks[0].devices[0].chains[0].devices[0].parameters[n]


class SongClockTest(unittest.TestCase):
    def test_a_set_of_the_song_time_is_a_jump_that_fires_its_listener(self):
        song = _site()
        self.assertEqual(song.current_song_time, 12.0, "the fixture's")
        fired = _Counter()
        song.add_current_song_time_listener(fired)
        song.current_song_time = 40.5
        self.assertEqual((song.current_song_time, fired.count), (40.5, 1))
        with self.assertRaises(TypeError):
            song.current_song_time = "later"
        self.assertEqual(song.current_song_time, 40.5)

    def test_the_clock_advances_the_song_time_at_the_tempo_while_playing(self):
        song = _site()
        clock = _Clock()
        song_clock = host.SongClock(song, 30.0, clock)
        fired = _Counter()
        song.add_current_song_time_listener(fired)
        song.start_playing()
        for _ in range(2):
            clock.now += 0.5
            song_clock._tick()
        self.assertAlmostEqual(song.current_song_time, 14.0, places=12)
        self.assertEqual(fired.count, 2)
        # Stopped: it stands, and a later start does not jump by the pause.
        song.stop_playing()
        clock.now += 3.0
        song_clock._tick()
        self.assertAlmostEqual(song.current_song_time, 14.0, places=12)
        song.start_playing()
        clock.now += 0.25
        song_clock._tick()
        self.assertAlmostEqual(song.current_song_time, 14.5, places=12)
        self.assertEqual(fired.count, 3)
        # Another tempo: 90 bpm is 1.5 beats a second.
        song.tempo = 90.0
        clock.now += 1.0
        song_clock._tick()
        self.assertAlmostEqual(song.current_song_time, 16.0, places=12)

    def test_the_clock_runs_on_a_live_timer_at_its_rate(self):
        song = _site()
        song_clock = host.SongClock(song, 30.0, _Clock())
        timer = song_clock._timer
        self.assertIsInstance(timer, Live.Base.Timer)
        self.assertAlmostEqual(timer.interval, 1000.0 / 30.0)
        self.assertTrue(timer.repeat)
        song_clock.start()
        self.assertTrue(timer.running)
        song_clock.stop()
        self.assertFalse(timer.running)


class LiveBarTest(unittest.TestCase):
    def test_root_note_takes_an_index_on_lives_scale(self):
        song = _site()
        self.assertEqual((song.root_note, song.scale_name), (5, "Minor"), "the fixture's")
        fired = _Counter()
        song.add_root_note_listener(fired)
        song.root_note = 9
        self.assertEqual((song.root_note, fired.count), (9, 1))
        for bad in (12, -1, 1.5, "A", True):
            with self.assertRaises(RuntimeError, msg=repr(bad)) as raised:
                song.root_note = bad
            self.assertEqual(str(raised.exception), "Invalid value")
        self.assertEqual((song.root_note, fired.count), (9, 1))

    def test_scale_name_is_an_observable_string(self):
        song = _site()
        fired = _Counter()
        song.add_scale_name_listener(fired)
        song.scale_name = "Major"
        self.assertEqual((song.scale_name, fired.count), ("Major", 1))
        with self.assertRaises(TypeError):
            song.scale_name = 1
        self.assertEqual(song.scale_name, "Major")

    def test_a_new_song_holds_c_major_at_the_start(self):
        song = Live.Song.Song("New")
        self.assertEqual(
            (song.current_song_time, song.root_note, song.scale_name), (0.0, 0, "Major")
        )


class HeldParameterTest(unittest.TestCase):
    def test_a_disabled_parameter_refuses_a_write_and_keeps_its_value(self):
        song = _site()
        param = _slot(song, 1)
        param._sim_enable(False)
        fired = _Counter()
        param.add_value_listener(fired)
        with self.assertRaises(RuntimeError) as raised:
            param.value = 0.5
        self.assertEqual(str(raised.exception), REFUSED)
        self.assertEqual((param.value, fired.count), (0.25, 0))
        param._sim_enable(True)
        self.assertTrue(param.is_enabled)
        self.assertEqual(param.state, Live.DeviceParameter.ParameterState.enabled)
        param.value = 0.5
        self.assertEqual((param.value, fired.count), (0.5, 1))

    def test_a_fixtures_enabled_false_builds_a_disabled_parameter(self):
        song = _site()
        held, free = _slot(song, 2), _slot(song, 1)
        self.assertFalse(held.is_enabled)
        self.assertEqual(held.state, Live.DeviceParameter.ParameterState.disabled)
        self.assertTrue(free.is_enabled)
        self.assertEqual(free.state, Live.DeviceParameter.ParameterState.enabled)
        self.assertEqual(free.value, 0.25)
        self.assertEqual(free.name, "0:(unused):(unassigned)")
        self.assertEqual(_slot(song, 0).name, "Device On")


class HostLineTest(unittest.TestCase):
    def setUp(self):
        self.song = _site()
        self.held = {}

    def line(self, text):
        return host.control(_Inline(), self.song, text, self.held)

    def test_song_time_jumps_as_a_user_would(self):
        fired = _Counter()
        self.song.add_current_song_time_listener(fired)
        self.assertEqual(self.line("song time 12.5\n"), "SONG 12.5")
        self.assertEqual((self.song.current_song_time, fired.count), (12.5, 1))
        self.assertEqual(self.line("song time 130"), "SONG 130.0")
        for bad in ("song time x", "song time", "song time inf", "song tempo 90", "song"):
            self.assertEqual(self.line(bad), "SONG -1", bad)
        self.assertEqual((self.song.current_song_time, fired.count), (130.0, 2))

    def test_param_value_writes_as_another_writer_would(self):
        fired = _Counter()
        _slot(self.song, 1).add_value_listener(fired)
        self.assertEqual(self.line(f"param {SLOT_0} value 0.75"), "PARAM 0.75")
        self.assertEqual((_slot(self.song, 1).value, fired.count), (0.75, 1))
        # A plug-in's slot answers Live's float32 of the value written.
        self.assertEqual(self.line(f"param {SLOT_0} value 0.9"), "PARAM 0.8999999761581421")
        # Held by a map: refused, nothing changes.
        self.assertEqual(self.line(f"param {SLOT_1} value 0.5"), "PARAM -1")
        self.assertEqual(_slot(self.song, 2).value, 0.0)
        # Outside the parameter's range, a bad path, a bad number, no
        # parameter at the path.
        for bad in (
            f"param {SLOT_0} value 2",
            "param live_set tracks 9 devices 0 parameters 1 value 0.5",
            f"param {SLOT_0} value x",
            "param live_set tracks 0 value 0.5",
            f"param {SLOT_0} colour 1",
            "param value 1",
        ):
            self.assertEqual(self.line(bad), "PARAM -1", bad)
        self.assertEqual((_slot(self.song, 1).value, fired.count), (0.8999999761581421, 2))

    def test_param_enabled_holds_or_frees_a_parameter(self):
        self.assertEqual(self.line(f"param {SLOT_1} enabled 1"), "PARAM 1")
        self.assertTrue(_slot(self.song, 2).is_enabled)
        self.assertEqual(self.line(f"param {SLOT_1} value 0.5"), "PARAM 0.5")
        self.assertEqual(self.line(f"param {SLOT_0} enabled 0"), "PARAM 0")
        self.assertFalse(_slot(self.song, 1).is_enabled)
        self.assertEqual(self.line(f"param {SLOT_0} value 0.5"), "PARAM -1")
        for bad in (
            f"param {SLOT_0} enabled 2",
            f"param {SLOT_0} enabled yes",
            "param live_set tracks 0 enabled 1",
            "param live_set tracks 9 devices 0 parameters 1 enabled 1",
        ):
            self.assertEqual(self.line(bad), "PARAM -1", bad)


class RackMacroTest(unittest.TestCase):
    """A rack's macros start at a fixture's ``macros`` (#140 PR 3b: a PA
    rack stands on DRY at 127), else at 0; the start is no set."""

    def test_a_fixtures_macros_start_the_racks_first_macros(self):
        song = site_builder.build_from_spec(
            {
                "name": "Macro Site",
                "tracks": [
                    {
                        "name": "Vocal A #",
                        "devices": [
                            {
                                "template": "rack",
                                "name": "Latencies",
                                "macros": [127.0, 5.0],
                                "chains": [{"name": "Auto"}, {"name": "Dry"}],
                            }
                        ],
                    },
                    {"name": "Stream A #", "devices": [{"template": "rack", "name": "Latencies"}]},
                ],
            }
        )
        rack = song.tracks[0].devices[0]
        self.assertEqual(
            [rack.parameters[i].name for i in (1, 2, 8, 9)],
            ["Macro 1", "Macro 2", "Macro 8", "Chain Selector"],
        )
        self.assertEqual([rack.parameters[i].value for i in (1, 2, 3, 8)], [127.0, 5.0, 0.0, 0.0])
        self.assertEqual(rack.parameters[1]._sim_set_count("value"), 0)
        self.assertEqual(song.tracks[1].devices[0].parameters[1].value, 0.0)


class SetCountTest(unittest.TestCase):
    """Every set through Live's setter counts, refused ones too; SimLive's
    own changes never do (#140 PR 2a review M6)."""

    def setUp(self):
        self.song = _site()
        self.held = {}

    def line(self, text):
        return host.control(_Inline(), self.song, text, self.held)

    def test_a_set_through_lives_setter_counts_refused_or_not(self):
        free, held = _slot(self.song, 1), _slot(self.song, 2)
        self.assertEqual((free._sim_set_count("value"), held._sim_set_count("value")), (0, 0))
        free.value = 0.5
        with self.assertRaises(RuntimeError):
            held.value = 0.5
        with self.assertRaises(RuntimeError):
            free.value = 2.0
        self.assertEqual((free._sim_set_count("value"), held._sim_set_count("value")), (2, 1))
        self.assertEqual(free._sim_set_count("name"), 0)
        with self.assertRaises(AttributeError):
            free.name = "x"
        self.assertEqual(free._sim_set_count("name"), 1, "a read-only property's set too")
        # SimLive's own change (a clock, an animation) is no set.
        free._sim_set("value", 0.25)
        self.assertEqual(free._sim_set_count("value"), 2)
        self.song.root_note = 2
        self.song.start_playing()
        self.assertEqual(self.song._sim_set_count("root_note"), 1)
        self.assertEqual(self.song._sim_set_count("is_playing"), 1)
        self.assertEqual(self.song._sim_set_count("scale_name"), 0)

    def test_the_sets_line_answers_the_count_at_a_path(self):
        self.assertEqual(self.line(f"sets value {SLOT_0}"), "SETS 0")
        self.assertEqual(self.line(f"param {SLOT_0} value 0.75"), "PARAM 0.75")
        self.assertEqual(self.line(f"param {SLOT_1} value 0.75"), "PARAM -1")
        self.assertEqual(self.line(f"sets value {SLOT_0}"), "SETS 1")
        self.assertEqual(self.line(f"sets value {SLOT_1}"), "SETS 1", "a refused set counts")
        self.assertEqual(self.line("sets root_note live_set"), "SETS 0")
        self.assertEqual(self.line("sets value live_set tracks 9 devices 0"), "SETS -1")


class DeviceVectorTest(unittest.TestCase):
    """A device's ``parameters`` is Live's ``Device.ATimeableValueVector``,
    as real Live answers it (#140: the PC with 1.10.0), not a ``Base``
    vector: a script that took only ``Base`` vectors for lists sent it as
    text. A track's devices and a rack's chains stay ``Base`` vectors."""

    def test_a_devices_parameters_are_a_device_vector(self):
        song = _site()
        rack = song.tracks[0].devices[0]
        plugin = rack.chains[0].devices[0]
        for device in (rack, plugin):
            params = device.parameters
            self.assertEqual(
                (type(params).__module__, type(params).__name__),
                ("Device", "ATimeableValueVector"),
            )
            self.assertEqual(params[0].name, "Device On")
            self.assertEqual(list(params), list(device._parameters))
        for vector in (song.tracks[0].devices, rack.chains, song.tracks):
            self.assertEqual((type(vector).__module__, type(vector).__name__), ("Base", "Vector"))
        self.assertIs(Live.Device.ATimeableValueVector, type(rack.parameters))


class Float32Test(unittest.TestCase):
    """A plug-in's parameter keeps a written value as Live's float32 (#140
    PR 2b: the PC read 2/11 written to a slot back as 0.1818181872367859);
    any other parameter keeps the float as written."""

    def test_a_plugins_parameter_keeps_a_float32(self):
        slot = _slot(_site(), 1)
        slot.value = 2 / 11
        self.assertEqual(slot.value, 0.1818181872367859)
        slot.value = 10 / 11
        self.assertEqual(slot.value, 0.9090909361839294)
        slot.value = 0.25
        self.assertEqual(slot.value, 0.25, "a float32 value stays as it is")
        self.assertEqual(slot.str_for_value(slot.value), "0.25")

    def test_the_float32_echo_is_no_change(self):
        slot = _slot(_site(), 1)
        fired = _Counter()
        slot.add_value_listener(fired)
        slot.value = 2 / 11
        slot.value = 0.1818181872367859
        slot.value = 2 / 11
        self.assertEqual(fired.count, 1, "Live's value never changed again")
        self.assertEqual(slot._sim_set_count("value"), 3)

    def test_another_devices_parameter_keeps_the_float_as_written(self):
        song = _site()
        rack = song.tracks[0].devices[0]
        macro = rack.parameters[1]
        self.assertEqual(macro.name, "Macro 1")
        macro.value = 64.1
        self.assertEqual(macro.value, 64.1)
        volume = song.tracks[0].mixer_device.volume
        volume.value = 2 / 11
        self.assertEqual(volume.value, 2 / 11)

    def test_a_fixtures_value_is_kept_as_given(self):
        song = site_builder.build_from_spec(
            {
                "name": "F",
                "tracks": [
                    {
                        "name": "T",
                        "devices": [
                            {
                                "template": "plugin",
                                "name": "P",
                                "parameters": [{"name": "0:x", "value": 1 / 28}],
                            }
                        ],
                    }
                ],
            }
        )
        self.assertEqual(song.tracks[0].devices[0].parameters[1].value, 1 / 28)


if __name__ == "__main__":
    unittest.main()
