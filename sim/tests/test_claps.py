"""SimLive for claps from the surface (#220): a clip's ``muted`` (Live's
"deactivated"), the time signature, their fixture keys and host lines.
Names are invented (public repository)."""

import unittest

import _simpath  # noqa: F401 - puts sim/ on the path
import host
import site_builder


class _Inline:
    """Stands in for the host's main thread: runs a call at once."""

    def call(self, fn):
        return fn()


class _Counter:
    def __init__(self):
        self.count = 0

    def __call__(self):
        self.count += 1


def _site(**song):
    spec = {
        "name": "Claps Site",
        "tracks": [
            {
                "name": "Claps #",
                "arrangement_clips": [
                    {"name": "", "start_time": 16.0, "end_time": 24.0},
                    {"name": "", "start_time": 32.0, "end_time": 40.0, "muted": True},
                ],
            },
        ],
    }
    spec.update(song)
    return site_builder.build_from_spec(spec)


class ClapsSimTest(unittest.TestCase):
    def setUp(self):
        self.song = _site()
        self.held = {}
        self.track = self.song.tracks[0]

    def line(self, text):
        return host.control(_Inline(), self.song, text, self.held)

    def test_a_clip_is_active_unless_its_fixture_says_muted(self):
        clips = self.track.arrangement_clips
        self.assertEqual([c.muted for c in clips], [False, True])

    def test_clip_mute_fires_the_clips_own_listener_only(self):
        listed, switched = _Counter(), _Counter()
        clip = self.track.arrangement_clips[0]
        self.track.add_arrangement_clips_listener(listed)
        clip.add_muted_listener(switched)
        self.assertEqual(self.line("clip mute 0 0 1"), "CLIP 2")
        self.assertTrue(clip.muted)
        self.assertEqual((listed.count, switched.count), (0, 1))
        self.assertEqual(self.line("clip mute 0 0 0"), "CLIP 2")
        self.assertFalse(clip.muted)
        for text in ["clip mute 0 5 1", "clip mute 0 0 2", "clip mute 0 0", "clip mute 9 0 1"]:
            self.assertEqual(self.line(text), "CLIP -1", text)

    def test_a_script_may_switch_a_clip_as_live_does(self):
        clip = self.track.arrangement_clips[1]
        clip.muted = False
        self.assertFalse(clip.muted)

    def test_the_signature_is_four_four_unless_the_fixture_or_a_line_says_otherwise(self):
        self.assertEqual((self.song.signature_numerator, self.song.signature_denominator), (4, 4))
        song = _site(signature=[6, 8])
        self.assertEqual((song.signature_numerator, song.signature_denominator), (6, 8))
        fired = _Counter()
        self.song.add_signature_numerator_listener(fired)
        self.assertEqual(self.line("song signature 3 4"), "SONG 3")
        self.assertEqual((self.song.signature_numerator, self.song.signature_denominator), (3, 4))
        self.assertEqual(fired.count, 1)
        for text in [
            "song signature 0 4",
            "song signature 3 5",
            "song signature x 4",
            "song signature 3",
        ]:
            self.assertEqual(self.line(text), "SONG -1", text)
        self.assertEqual(self.line("song time 8"), "SONG 8.0")


if __name__ == "__main__":
    unittest.main()
