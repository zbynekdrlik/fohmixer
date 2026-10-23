"""SimLive's locators and arrangement clips (#140): the fixture keys, the
lists in time order, their listeners, and the host's ``cue`` and ``clip``
lines. Names are invented (public repository)."""

import unittest

import _simpath
import host
import site_builder


class _Inline:
    """Stands in for the host's main thread: runs a call at once."""

    def call(self, fn):
        return fn()


def _site():
    return site_builder.build_from_spec(
        {
            "name": "Song Map Site",
            "tracks": [
                {"name": "Click"},
                {
                    "name": "Sections",
                    "arrangement_clips": [
                        {"name": "Key {G}", "start_time": 20.0, "end_time": 22.0},
                        {"name": "Verse", "start_time": 4.0, "end_time": 20.0},
                    ],
                },
            ],
            "cue_points": [
                {"name": "SONG END", "time": 100.0},
                {"name": "001 Test One {C 120 4/4}", "time": 0.0},
                {"name": "> Chorus", "time": 32.5},
            ],
        }
    )


class _Counter:
    def __init__(self):
        self.count = 0

    def __call__(self):
        self.count += 1


def _names(song):
    return [(c.name, c.time) for c in song.cue_points]


class FixtureTest(unittest.TestCase):
    def test_a_fixture_builds_its_cue_points_and_clips(self):
        song = _site()
        self.assertEqual(
            _names(song),
            [("001 Test One {C 120 4/4}", 0.0), ("> Chorus", 32.5), ("SONG END", 100.0)],
        )
        clips = song.tracks[1].arrangement_clips
        self.assertEqual(
            [(c.name, c.start_time, c.end_time, c.is_midi_clip) for c in clips],
            [("Verse", 4.0, 20.0, True), ("Key {G}", 20.0, 22.0, True)],
        )
        self.assertEqual(len(song.tracks[0].arrangement_clips), 0)

    def test_the_lists_are_live_vectors_of_live_classes(self):
        song = _site()
        self.assertEqual(type(song.cue_points).__module__, "Base")
        self.assertEqual(type(song.cue_points[0]).__name__, "CuePoint")
        self.assertEqual(type(song.cue_points[0]).__module__, "Live.Song")
        clip = song.tracks[1].arrangement_clips[0]
        self.assertEqual(type(clip).__name__, "Clip")
        self.assertEqual(type(clip).__module__, "Live.Clip")

    def test_a_set_without_cue_points_has_none(self):
        song = site_builder.build(_simpath.FIXTURE)
        self.assertEqual(len(song.cue_points), 0)
        self.assertTrue(all(len(t.arrangement_clips) == 0 for t in song.tracks))

    def test_the_names_and_times_are_observable_and_read_only_where_live_is(self):
        song = _site()
        point = song.cue_points[0]
        clip = song.tracks[1].arrangement_clips[0]
        for obj, prop in [
            (song, "cue_points"),
            (song.tracks[1], "arrangement_clips"),
            (point, "name"),
            (point, "time"),
            (clip, "name"),
            (clip, "start_time"),
            (clip, "end_time"),
        ]:
            self.assertTrue(hasattr(obj, f"add_{prop}_listener"), prop)
        self.assertFalse(hasattr(clip, "add_is_midi_clip_listener"))
        with self.assertRaises(AttributeError):
            point.time = 3.0
        with self.assertRaises(AttributeError):
            clip.start_time = 3.0

    def test_a_jump_moves_the_song_to_the_locator(self):
        song = _site()
        song.cue_points[2].jump()
        self.assertEqual(song._sim_jumps, [100.0])

    def test_deleting_a_track_deletes_its_clips(self):
        song = _site()
        clip = song.tracks[1].arrangement_clips[0]
        song.delete_track(1)
        self.assertTrue(clip == None)  # noqa: E711 - Live's deleted-object idiom


class CueLineTest(unittest.TestCase):
    def setUp(self):
        self.song = _site()
        self.held = {}

    def line(self, text):
        return host.control(_Inline(), self.song, text, self.held)

    def test_cue_add_fires_the_list_once_and_keeps_time_order(self):
        fired = _Counter()
        self.song.add_cue_points_listener(fired)
        self.assertEqual(self.line("cue add 50 002 Test Two {F#m 90}\n"), "CUE 4")
        self.assertEqual(fired.count, 1)
        self.assertEqual(
            [n for n, _ in _names(self.song)],
            ["001 Test One {C 120 4/4}", "> Chorus", "002 Test Two {F#m 90}", "SONG END"],
        )
        self.assertEqual(self.song.cue_points[2].time, 50.0)

    def test_cue_rename_fires_the_cues_name_and_not_the_list(self):
        listed, named, timed = _Counter(), _Counter(), _Counter()
        point = self.song.cue_points[0]
        self.song.add_cue_points_listener(listed)
        point.add_name_listener(named)
        point.add_time_listener(timed)
        self.assertEqual(self.line("cue rename 0 001 Test One {Eb 120}"), "CUE 3")
        self.assertEqual((listed.count, named.count, timed.count), (0, 1, 0))
        self.assertEqual(point.name, "001 Test One {Eb 120}")

    def test_cue_time_fires_the_cues_time_and_keeps_the_list_in_order(self):
        listed, timed = _Counter(), _Counter()
        point = self.song.cue_points[0]
        self.song.add_cue_points_listener(listed)
        point.add_time_listener(timed)
        self.assertEqual(self.line("cue time 0 200.5"), "CUE 3")
        self.assertEqual((listed.count, timed.count), (0, 1))
        self.assertEqual(self.song.cue_points[2].name, "001 Test One {C 120 4/4}")
        self.assertEqual(self.song.cue_points[2].time, 200.5)

    def test_cue_remove_fires_the_list_and_deletes_the_cue(self):
        fired = _Counter()
        point = self.song.cue_points[1]
        self.song.add_cue_points_listener(fired)
        self.assertEqual(self.line("cue remove 1"), "CUE 2")
        self.assertEqual(fired.count, 1)
        self.assertTrue(point == None)  # noqa: E711 - Live's deleted-object idiom
        self.assertEqual(
            [n for n, _ in _names(self.song)], ["001 Test One {C 120 4/4}", "SONG END"]
        )

    def test_a_bad_cue_line_answers_an_error_and_changes_nothing(self):
        fired, named = _Counter(), _Counter()
        self.song.add_cue_points_listener(fired)
        self.song.cue_points[0].add_name_listener(named)
        before = _names(self.song)
        for text in [
            "cue rename 99 x",
            "cue time 3 1",
            "cue remove 3",
            "cue add x 001 T",
            "cue add 5",
            "cue add inf 001 T",
            "cue rename 0",
            "cue rename x y",
            "cue time 0 x",
            "cue remove 0 x",
            "cue move 0 1",
            "cue",
        ]:
            self.assertEqual(self.line(text), "CUE -1", text)
        self.assertEqual(_names(self.song), before)
        self.assertEqual((fired.count, named.count), (0, 0))


class ClipLineTest(unittest.TestCase):
    def setUp(self):
        self.song = _site()
        self.held = {}
        self.track = self.song.tracks[1]

    def line(self, text):
        return host.control(_Inline(), self.song, text, self.held)

    def clips(self):
        return [(c.name, c.start_time, c.end_time) for c in self.track.arrangement_clips]

    def test_clip_add_fires_the_tracks_clips_and_keeps_start_order(self):
        fired = _Counter()
        self.track.add_arrangement_clips_listener(fired)
        self.assertEqual(self.line("clip add 1 10 12 Key {D}"), "CLIP 3")
        self.assertEqual(fired.count, 1)
        self.assertEqual(
            self.clips(),
            [("Verse", 4.0, 20.0), ("Key {D}", 10.0, 12.0), ("Key {G}", 20.0, 22.0)],
        )

    def test_clip_rename_fires_the_clips_name_only(self):
        listed, named = _Counter(), _Counter()
        clip = self.track.arrangement_clips[0]
        self.track.add_arrangement_clips_listener(listed)
        clip.add_name_listener(named)
        self.assertEqual(self.line("clip rename 1 0 Chorus {A}"), "CLIP 2")
        self.assertEqual((listed.count, named.count), (0, 1))
        self.assertEqual(clip.name, "Chorus {A}")

    def test_clip_remove_fires_the_tracks_clips(self):
        fired = _Counter()
        self.track.add_arrangement_clips_listener(fired)
        self.assertEqual(self.line("clip remove 1 1"), "CLIP 1")
        self.assertEqual(fired.count, 1)
        self.assertEqual(self.clips(), [("Verse", 4.0, 20.0)])

    def test_a_bad_clip_line_answers_an_error_and_changes_nothing(self):
        fired = _Counter()
        self.track.add_arrangement_clips_listener(fired)
        before = self.clips()
        for text in [
            "clip add 9 1 2 Key {D}",
            "clip add 1 2 1 Key {D}",
            "clip add 1 x 2 Key {D}",
            "clip add 1 1 2",
            "clip rename 1 5 X",
            "clip rename 1 0",
            "clip remove 1 5",
            "clip remove 9 0",
            "clip remove 1 x",
            "clip remove 1 0 x",
            "clip move 1 0",
            "clip",
        ]:
            self.assertEqual(self.line(text), "CLIP -1", text)
        self.assertEqual(self.clips(), before)
        self.assertEqual(fired.count, 0)


if __name__ == "__main__":
    unittest.main()
