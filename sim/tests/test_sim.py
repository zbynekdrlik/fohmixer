"""SimLive: the fake ``Live`` module, the main-thread host and the synthetic site."""

import itertools
import operator
import threading
import time
import unittest

import _simpath
import Live
import site_builder
from _Framework.ControlSurface import ControlSurface
from c_instance import CInstance
from main_thread import MainThread


def _all_objects(song):
    """Every Live object reachable from the song (tracks, mixers, params, devices, chains)."""
    seen = []

    def walk(obj):
        if obj is None or any(obj is s for s in seen):
            return
        seen.append(obj)
        for child in obj._sim_children():
            walk(child)

    walk(song)
    return seen


class SiteTest(unittest.TestCase):
    def setUp(self):
        self.song = site_builder.build(_simpath.FIXTURE)

    def test_site_shape(self):
        names = [t.name for t in self.song.tracks]
        self.assertEqual(len(names), 15)
        self.assertEqual(names[:4], ["Hand1 #", "Hand2 #", "Hand3 #", "Hand4 #"])
        self.assertEqual(names.count("Keys 1"), 2)
        groups = [t for t in self.song.tracks if t.is_foldable]
        self.assertEqual([g.name for g in groups], ["Vocals Repro grp#", "Stems grp#"])
        vocal2 = self.song.tracks[names.index("Vocal 2 repro#")]
        self.assertTrue(vocal2.is_grouped)
        self.assertEqual(vocal2.group_track.name, "Vocals Repro grp#")
        self.assertEqual(
            [r.name for r in self.song.return_tracks], ["A-Reverb #", "B-Main repro #"]
        )
        self.assertEqual(self.song.master_track.name, "Main")
        self.assertEqual(len(self.song.tracks[0].mixer_device.sends), 2)

    def test_keys_1_sits_in_two_groups(self):
        keys = [t for t in self.song.tracks if t.name == "Keys 1"]
        self.assertEqual(
            sorted(k.group_track.name for k in keys), ["Stems grp#", "Vocals Repro grp#"]
        )

    def test_eq_and_rack_devices(self):
        by_name = {t.name: t for t in self.song.tracks}
        eq = by_name["Vocal 1 repro#"].devices[0]
        self.assertEqual(eq.name, "EQ Eight")
        self.assertEqual(eq.class_name, "Eq8")
        params = {p.name: p for p in eq.parameters}
        self.assertIn("1 Gain A", params)
        self.assertIn("8 Frequency A", params)
        self.assertEqual(str(params["1 Gain A"]), "0.00 dB")
        rack = by_name["Mics Stage #"].devices[0]
        self.assertTrue(rack.can_have_chains)
        self.assertEqual([c.name for c in rack.chains], ["Dry", "Wet"])
        self.assertIn("Chain Selector", [p.name for p in rack.parameters])
        self.assertEqual(type(rack.chains[0].mixer_device).__name__, "ChainMixerDevice")

    def test_pro_q_4_on_a_track_and_in_a_rack_chain(self):
        # #71: Hand2 # holds a Pro-Q 4, a rack whose first chain holds a renamed
        # one and whose second chain holds another plug-in.
        hand2 = {t.name: t for t in self.song.tracks}["Hand2 #"]
        on_track, rack = hand2.devices
        self.assertEqual(type(on_track).__name__, "PluginDevice")
        self.assertEqual(type(on_track).__module__, "Live.PluginDevice")
        self.assertIs(Live.PluginDevice.PluginDevice, type(on_track))
        self.assertEqual(
            (on_track.name, on_track.class_name, on_track.class_display_name),
            ("Pro-Q 4", "PluginDevice", "Pro-Q 4"),
        )
        self.assertEqual(type(rack).__name__, "RackDevice")
        main, air = rack.chains
        self.assertEqual((main.name, air.name), ("Main", "Air"))
        renamed = main.devices[0]
        self.assertEqual((renamed.name, renamed.class_display_name), ("De-ess", "Pro-Q 4"))
        self.assertEqual(air.devices[0].class_display_name, "Pro-C 2")
        self.assertEqual([p.name for p in on_track.parameters], ["Device On"])

    def test_a_plug_ins_editor_opens_and_closes_and_is_heard(self):
        plugin = {t.name: t for t in self.song.tracks}["Hand2 #"].devices[0]
        self.assertIs(plugin.is_editor_open, False)
        heard = []
        plugin.add_is_editor_open_listener(lambda: heard.append(plugin.is_editor_open))
        plugin.is_editor_open = True
        plugin.is_editor_open = True
        plugin.is_editor_open = 0
        self.assertEqual(heard, [True, False])
        with self.assertRaises(TypeError):
            plugin.is_editor_open = "yes"
        with self.assertRaises(AttributeError):
            plugin.class_display_name = "Pro-Q 3"

    def test_live_ptrs_are_unique(self):
        objs = _all_objects(self.song)
        self.assertGreater(len(objs), 150)
        ptrs = [o._live_ptr for o in objs]
        self.assertEqual(len(ptrs), len(set(ptrs)))

    def test_classes_live_in_live_submodules(self):
        track = self.song.tracks[0]
        self.assertEqual(type(track).__name__, "Track")
        self.assertEqual(type(track).__module__, "Live.Track")
        self.assertIs(Live.Track.Track, type(track))
        # Live.Base.Vector is reached through Live.Base, but its class reports
        # the module "Base" as Live's does ("<Base.Vector object at ...>" on
        # the PC, #58).
        self.assertIs(Live.Base.Vector, type(self.song.tracks))
        self.assertEqual(type(self.song.tracks).__module__, "Base")
        self.assertIs(Live.Application.get_application().get_document(), self.song)


class DisplayTest(unittest.TestCase):
    def setUp(self):
        self.vol = site_builder.build(_simpath.FIXTURE).tracks[0].mixer_device.volume

    def _display(self, value):
        self.vol.value = value
        return str(self.vol)

    def test_volume_display_law(self):
        self.assertEqual(self._display(0.85), "0.00 dB")
        self.assertEqual(self._display(0.0), "-inf dB")
        self.assertEqual(self._display(1.0), "6.00 dB")
        self.assertEqual(self._display(0.5), "-14.0 dB")
        self.assertEqual(self._display(0.4), "-18.0 dB")
        self.assertEqual(self._display(0.2), "-34.4 dB")
        self.assertEqual(self._display(0.15), "-41.0 dB")
        self.assertEqual(self._display(0.1), "-48.5 dB")
        self.assertEqual(self._display(0.001), "-69.3 dB")

    def test_db_strings_are_live_s_three_significant_digits(self):
        # Real Live showed "-0.811 dB" on the PC (#9, finding 4): Live writes
        # dB values with 3 significant digits, at most 3 decimals, 0 as 0.00.
        # SimLive writes what Live would, so tests see real strings.
        self.assertEqual(self._display(0.829725), "-0.811 dB")
        self.assertEqual(self._display(0.85), "0.00 dB")
        self.assertEqual(self._display(0.84), "-0.400 dB")
        self.assertEqual(self._display(0.7), "-6.00 dB")
        self.assertEqual(self._display(0.6001), "-10.0 dB")  # -9.996 rounds up a digit
        self.assertEqual(self._display(0.5), "-14.0 dB")
        self.assertEqual(self._display(1.0), "6.00 dB")
        self.assertEqual(self._display(0.8499), "-0.004 dB")
        self.assertEqual(self._display(0.84999), "0.00 dB")
        self.assertEqual(self._display(0.0), "-inf dB")
        gain = next(
            p
            for p in site_builder.build(_simpath.FIXTURE).tracks[5].devices[0].parameters
            if p.name == "1 Gain A"
        )
        self.assertEqual(gain.str_for_value(-3.5), "-3.50 dB")
        self.assertEqual(gain.str_for_value(12.26), "12.3 dB")
        self.assertEqual(gain.str_for_value(0.0), "0.00 dB")

    def test_str_for_value_does_not_change_value(self):
        self.vol.value = 0.85
        self.assertEqual(self.vol.str_for_value(0.5), "-14.0 dB")
        self.assertEqual(self.vol.value, 0.85)

    def test_out_of_range_set_raises_and_keeps_value(self):
        self.vol.value = 0.5
        with self.assertRaises(RuntimeError):
            self.vol.value = 1.5
        self.assertEqual(self.vol.value, 0.5)


class ListenerTest(unittest.TestCase):
    def setUp(self):
        self.song = site_builder.build(_simpath.FIXTURE)
        self.track = self.song.tracks[0]

    def test_setter_fires_listener_once_per_change(self):
        calls = []
        self.track.add_mute_listener(lambda: calls.append(self.track.mute))
        self.track.mute = True
        self.track.mute = True
        self.assertEqual(calls, [True])

    def test_listener_api_shape(self):
        def cb():
            pass

        self.assertFalse(self.track.mute_has_listener(cb))
        self.track.add_mute_listener(cb)
        self.assertTrue(self.track.mute_has_listener(cb))
        self.assertEqual(self.track._sim_listener_count("mute"), 1)
        self.track.remove_mute_listener(cb)
        self.assertFalse(self.track.mute_has_listener(cb))
        with self.assertRaises(RuntimeError):
            self.track.remove_mute_listener(cb)

    def test_set_inside_notification_raises(self):
        errors = []

        def cb():
            try:
                self.track.solo = True
            except RuntimeError as e:
                errors.append(str(e))

        self.track.add_mute_listener(cb)
        self.track.mute = True
        self.assertEqual(len(errors), 1)
        self.assertIn("notifications", errors[0])
        self.assertFalse(self.track.solo)

    def test_enum_property(self):
        states = Live.Track.Track.monitoring_states
        self.assertEqual(states.names["IN"], 0)
        self.assertIs(states.values[2], states.OFF)
        self.assertIsInstance(self.track.current_monitoring_state, states)
        self.track.current_monitoring_state = states.IN
        self.assertIs(self.track.current_monitoring_state, states.IN)

    def test_fold_state_and_is_visible_are_not_observable_as_in_live(self):
        # Live answers add_listener on a track's fold_state or is_visible
        # with "not observable" (read on the PC, #58); visible_tracks is the
        # list that tells a fold.
        group = next(t for t in self.song.tracks if t.name == "Vocals Repro grp#")
        self.assertFalse(hasattr(group, "add_fold_state_listener"))
        self.assertFalse(hasattr(group, "add_is_visible_listener"))
        self.assertTrue(hasattr(self.song, "add_visible_tracks_listener"))

    def test_fold_state_changes_visible_tracks(self):
        fired = []
        self.song.add_visible_tracks_listener(lambda: fired.append(1))
        group = next(t for t in self.song.tracks if t.name == "Vocals Repro grp#")
        before = len(self.song.visible_tracks)
        group.fold_state = 1
        self.assertEqual(len(self.song.visible_tracks), before - 4)
        self.assertEqual(fired, [1])
        with self.assertRaises(RuntimeError):
            self.track.fold_state = 1

    def test_delete_track(self):
        fired = []
        self.song.add_tracks_listener(lambda: fired.append(1))
        vol = self.track.mixer_device.volume
        self.song.delete_track(0)
        self.assertEqual(len(self.song.tracks), 14)
        self.assertEqual(fired, [1])
        # Live's deleted-object idiom: a deleted object compares equal to None.
        self.assertTrue(operator.eq(self.track, None))
        self.assertTrue(operator.eq(vol, None))
        with self.assertRaises(RuntimeError):
            _ = self.track.name
        with self.assertRaises(RuntimeError):
            str(vol)
        self.assertFalse(operator.eq(self.song.tracks[0], None))

    def test_meter_helper_fires_listeners(self):
        seen = []
        self.track.add_output_meter_left_listener(lambda: seen.append(self.track.output_meter_left))
        self.track._sim_set_meter(0.5, 0.25)
        self.assertEqual(seen, [0.5])
        self.assertEqual(self.track.output_meter_level, 0.5)
        with self.assertRaises(AttributeError):
            self.track.output_meter_left = 0.1

    def test_unknown_attribute_cannot_be_set(self):
        with self.assertRaises(AttributeError):
            self.track.no_such_prop = 1


class MainThreadTest(unittest.TestCase):
    def setUp(self):
        self.mt = MainThread()
        self.mt.start()

    def tearDown(self):
        self.mt.stop()
        self.assertEqual(self.mt.errors, [])

    def test_timer_fires_about_every_interval(self):
        fires = []
        timer = Live.Base.Timer(
            callback=lambda: fires.append(time.monotonic()), interval=10, repeat=True
        )
        timer.start()
        time.sleep(0.3)
        timer.stop()
        gaps_ms = sorted((b - a) * 1000 for a, b in itertools.pairwise(fires))
        self.assertGreaterEqual(len(fires), 15)
        self.assertGreaterEqual(gaps_ms[len(gaps_ms) // 2], 9.0)
        self.assertLess(gaps_ms[len(gaps_ms) // 2], 15.0)

    def test_stall_delays_timer(self):
        fires = []
        timer = Live.Base.Timer(
            callback=lambda: fires.append(time.monotonic()), interval=10, repeat=True
        )
        timer.start()
        time.sleep(0.05)
        self.mt.stall(200)
        time.sleep(0.35)
        timer.stop()
        gaps = [b - a for a, b in itertools.pairwise(fires)]
        self.assertGreaterEqual(max(gaps), 0.2)

    def test_call_runs_on_main_thread(self):
        ident = self.mt.call(threading.get_ident)
        self.assertEqual(ident, self.mt.ident)
        self.assertNotEqual(ident, threading.get_ident())
        with self.assertRaises(ValueError):
            self.mt.call(lambda: int("x"))

    def test_schedule_message_fires_after_one_tick(self):
        song = site_builder.build(_simpath.FIXTURE)
        surface = self.mt.call(lambda: ControlSurface(CInstance(song)))
        self.assertIs(surface.song(), song)
        self.assertIs(surface.application(), Live.Application.get_application())
        done = threading.Event()
        started = time.monotonic()
        fired_at = []

        def cb():
            fired_at.append(time.monotonic())
            done.set()

        self.mt.call(lambda: surface.schedule_message(1, cb))
        self.assertTrue(done.wait(1.0))
        delay = fired_at[0] - started
        self.assertGreaterEqual(delay, 0.09)
        self.assertLess(delay, 0.5)


if __name__ == "__main__":
    unittest.main()
