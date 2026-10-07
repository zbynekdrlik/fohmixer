"""The generic LOM layer: path grammar and resolution, the id registry, the codec."""

import unittest

import _paths
import Live
import site_builder
from FohMixer.lom import codec, path
from FohMixer.lom.errors import CodecError, PathError, StaleRef, UnknownRef
from FohMixer.lom.registry import Registry


class ParseTest(unittest.TestCase):
    def test_index_and_attribute_steps(self):
        root, steps = path.parse("live_set tracks 3 mixer_device volume")
        self.assertEqual(root, "live_set")
        self.assertEqual(
            [(s.attr, s.index, s.name) for s in steps],
            [("tracks", 3, None), ("mixer_device", None, None), ("volume", None, None)],
        )

    def test_name_selector_with_spaces_and_escapes(self):
        root, steps = path.parse(r"live_set tracks[name=Vocal 1 repro#] devices[name=a\]b\\c]")
        self.assertEqual(root, "live_set")
        self.assertEqual(steps[0].name, "Vocal 1 repro#")
        self.assertEqual(steps[1].name, "a]b\\c")

    def test_root_only(self):
        self.assertEqual(path.parse("live_app"), ("live_app", []))

    def test_malformed_paths(self):
        cases = {
            "": "syntax",
            "live_sets": "bad root",
            "song tracks": "bad root",
            "live_set  tracks": "syntax",
            "live_set tracks ": "syntax",
            "live_set 3": "syntax",
            "live_set tracks 1 2": "syntax",
            "live_set tracks[name=abc": "syntax",
            "live_set tracks[nam=abc]": "syntax",
            "live_set tracks[name=a]x": "syntax",
            r"live_set tracks[name=a\x]": "syntax",
            "live_set tracks -1": "syntax",
            "live_set _live_ptr": "forbidden",
            "live_set tracks __class__": "forbidden",
        }
        for text, kind in cases.items():
            with self.subTest(text=text):
                with self.assertRaises(PathError) as ctx:
                    path.parse(text)
                self.assertEqual(ctx.exception.kind, kind)

    def test_format_round_trips(self):
        text = r"live_set tracks[name=x\]y] devices 0 parameters[name=1 Gain A]"
        root, steps = path.parse(text)
        self.assertEqual(path.format_path(root, steps), text)


class ResolveTest(unittest.TestCase):
    def setUp(self):
        self.song = site_builder.build(_paths.FIXTURE)
        self.app = Live.Application.get_application()
        self.registry = Registry()

    def resolve(self, target):
        return path.resolve(target, self.song, self.app, self.registry)

    def test_index(self):
        obj, where = self.resolve("live_set tracks 0")
        self.assertEqual(obj.name, "Hand1 #")
        self.assertEqual(where, "live_set tracks 0")

    def test_name_selector_resolves_and_reports_index_path(self):
        obj, where = self.resolve("live_set tracks[name=Vocal 2 repro#] mixer_device volume")
        self.assertIs(obj, self.song.tracks[6].mixer_device.volume)
        self.assertEqual(where, "live_set tracks 6 mixer_device volume")

    def test_duplicate_name_is_ambiguous_never_first_match(self):
        with self.assertRaises(PathError) as ctx:
            self.resolve("live_set tracks[name=Keys 1] mute")
        self.assertEqual(ctx.exception.kind, "ambiguous")
        self.assertEqual(ctx.exception.detail, "tracks[name=Keys 1]")

    def test_missing_things_are_not_found(self):
        for target in (
            "live_set tracks[name=Nope]",
            "live_set tracks 99",
            "live_set no_such_attr",
            "live_set tracks 0 mute 1",
            "live_set tracks 0 mixer_device volume[name=x]",
            "live_set start_playing",
        ):
            with self.subTest(target=target):
                with self.assertRaises(PathError) as ctx:
                    self.resolve(target)
                self.assertEqual(ctx.exception.kind, "not found")

    def test_escaped_name(self):
        self.song.tracks[0].name = "a]b"
        obj, where = self.resolve(r"live_set tracks[name=a\]b]")
        self.assertIs(obj, self.song.tracks[0])
        self.assertEqual(where, "live_set tracks 0")

    def test_live_app(self):
        obj, where = self.resolve("live_app")
        self.assertIs(obj, self.app)
        self.assertEqual(where, "live_app")

    def test_deep_rack_path(self):
        obj, where = self.resolve(
            "live_set tracks[name=Mics Stage #] devices 0 chains[name=Wet] mixer_device volume"
        )
        self.assertEqual(type(obj).__name__, "DeviceParameter")
        self.assertEqual(where, "live_set tracks 13 devices 0 chains 1 mixer_device volume")

    def test_ref_and_path_dict_targets(self):
        track = self.song.tracks[2]
        ref = self.registry.put(track, "live_set tracks 2")
        self.assertEqual(self.resolve({"$ref": ref}), (track, "live_set tracks 2"))
        self.assertEqual(self.resolve({"path": "live_set tracks 2"}), (track, "live_set tracks 2"))

    def test_bad_target_types(self):
        for target in (3, None, ["live_set"], {"x": 1}):
            with self.subTest(target=target):
                with self.assertRaises(PathError) as ctx:
                    self.resolve(target)
                self.assertEqual(ctx.exception.kind, "bad target")


class RegistryTest(unittest.TestCase):
    def setUp(self):
        self.song = site_builder.build(_paths.FIXTURE)
        self.registry = Registry()

    def test_put_get_round_trip(self):
        track = self.song.tracks[0]
        ref = self.registry.put(track, "live_set tracks 0")
        self.assertEqual(ref, f"live_{track._live_ptr}")
        self.assertIs(self.registry.get(ref), track)
        self.assertIs(self.registry.get(ref, "Track"), track)
        self.assertEqual(self.registry.put(track), ref)
        self.assertEqual(self.registry.path_of(ref), "live_set tracks 0")

    def test_unknown_and_cleared(self):
        ref = self.registry.put(self.song.tracks[0])
        with self.assertRaises(UnknownRef):
            self.registry.get("live_1")
        with self.assertRaises(UnknownRef):
            self.registry.get(42)
        self.registry.clear()
        with self.assertRaises(UnknownRef):
            self.registry.get(ref)

    def test_pointer_reused_by_another_class_is_stale(self):
        track = self.song.tracks[0]
        ref = self.registry.put(track, "live_set tracks 0")
        device = self.song.tracks[5].devices[0]
        object.__setattr__(device, "_live_ptr", track._live_ptr)  # Live reused the address
        self.assertEqual(self.registry.put(device), ref)
        with self.assertRaises(StaleRef) as ctx:
            self.registry.get(ref, "Track")
        self.assertEqual(ctx.exception.kind, "class changed")
        with self.assertRaises(StaleRef):
            path.resolve({"$ref": ref, "class": "Track"}, self.song, None, self.registry)
        self.assertIs(self.registry.get(ref, "Device"), device)

    def test_deleted_object_is_stale(self):
        ref = self.registry.put(self.song.tracks[0])
        self.song.delete_track(0)
        with self.assertRaises(StaleRef) as ctx:
            self.registry.get(ref)
        self.assertEqual(ctx.exception.kind, "deleted")


class CodecTest(unittest.TestCase):
    def setUp(self):
        self.song = site_builder.build(_paths.FIXTURE)
        self.app = Live.Application.get_application()
        self.registry = Registry()

    def encode(self, value, where=None):
        return codec.encode(value, self.registry, where)

    def decode(self, value, expected=None):
        return codec.decode(value, self.registry, self.song, self.app, expected)

    def test_primitives_pass_through(self):
        for value in (None, True, False, 0, 7, 0.85, "x"):
            with self.subTest(value=value):
                self.assertEqual(self.encode(value), value)
                self.assertIs(type(self.encode(value)), type(value))
        self.assertEqual(self.encode(float("inf")), "inf")

    def test_track_becomes_ref(self):
        track = self.song.tracks[1]
        out = self.encode(track, "live_set tracks 1")
        self.assertEqual(
            out,
            {
                "$ref": f"live_{track._live_ptr}",
                "path": "live_set tracks 1",
                "class": "Track",
                "name": "Hand2 #",
            },
        )
        self.assertIs(self.registry.get(out["$ref"]), track)

    def test_vector_becomes_list_of_refs_with_index_paths(self):
        out = self.encode(self.song.tracks, "live_set tracks")
        self.assertEqual(len(out), 15)
        self.assertEqual(out[3]["path"], "live_set tracks 3")
        self.assertEqual(out[3]["name"], "Hand4 #")
        self.assertEqual({o["class"] for o in out}, {"Track"})

    def test_live_vectors_report_the_module_base_as_live_does(self):
        # Live answers repr(song.tracks) "<Base.Vector object at ...>" (read on
        # the PC, #58): its vector classes report the module "Base", and
        # SimLive's must too, or a codec that misses them passes every test.
        self.assertEqual(type(self.song.tracks).__module__, "Base")
        param = self.song.tracks[0].mixer_device.track_activator
        self.assertEqual(type(param.value_items).__module__, "Base")

    def test_a_vector_of_module_base_becomes_a_list(self):
        # Live's own vector class, as the PC showed it (#58): not "Live.*".
        Vector = type("Vector", (), {"__iter__": lambda self: iter((1, "a"))})
        Vector.__module__ = "Base"
        self.assertEqual(self.encode(Vector()), [1, "a"])
        # Another class named like a vector is no vector.
        Other = type(
            "Vector", (), {"__iter__": lambda self: iter(()), "__str__": lambda self: "other"}
        )
        Other.__module__ = "thing"
        self.assertEqual(self.encode(Other()), "other")

    def test_object_without_name_has_no_name_key(self):
        out = self.encode(self.song.tracks[0].mixer_device, "live_set tracks 0 mixer_device")
        self.assertEqual(out["class"], "MixerDevice")
        self.assertNotIn("name", out)

    def test_string_vector_and_tuple(self):
        param = self.song.tracks[0].mixer_device.track_activator
        self.assertEqual(self.encode(param.value_items), ["Off", "On"])
        self.assertEqual(self.encode((1, 2.5, "a")), [1, 2.5, "a"])

    def test_enum(self):
        state = Live.Track.Track.monitoring_states.OFF
        self.assertEqual(self.encode(state), {"$enum": "OFF", "value": 2})

    def test_unknown_object_falls_back_to_str(self):
        class Thing:
            def __str__(self):
                return "thing"

        self.assertEqual(self.encode(Thing()), "thing")
        self.assertEqual(self.encode({"a": 1}), "{'a': 1}")

    def test_decode_refs_and_paths(self):
        track = self.song.tracks[4]
        ref = self.encode(track, "live_set tracks 4")["$ref"]
        self.assertIs(self.decode({"$ref": ref}), track)
        self.assertIs(self.decode({"path": "live_set tracks 4"}), track)
        self.assertIs(self.decode(ref), ref)

    def test_decode_id_inside_call_args(self):
        track = self.song.tracks[4]
        ref = self.encode(track)["$ref"]
        self.assertEqual(self.decode([{"$ref": ref}, 3]), [track, 3])
        self.assertEqual(self.decode({"track": {"$ref": ref}, "n": 1}), {"track": track, "n": 1})

    def test_decode_enum_from_current_value(self):
        current = self.song.tracks[0].current_monitoring_state
        member = self.decode({"$enum": "IN"}, expected=current)
        self.assertIs(member, Live.Track.Track.monitoring_states.IN)

    def test_decode_qualified_enum(self):
        member = self.decode({"$enum": "Live.Track.Track.monitoring_states.OFF"})
        self.assertIs(member, Live.Track.Track.monitoring_states.OFF)

    def test_decode_enum_errors(self):
        current = self.song.tracks[0].current_monitoring_state
        bad = [
            ({"$enum": "LOUD"}, current),
            ({"$enum": "IN"}, None),
            ({"$enum": "IN"}, 3),
            ({"$enum": "Live.Track.Track.nope.X"}, None),
            ({"$enum": "Live.Track._secret"}, None),
            ({"$enum": "os.path"}, None),
            ({"$enum": "Live.Track.Track"}, None),
            ({"$enum": 5}, None),
        ]
        for value, expected in bad:
            with self.subTest(value=value), self.assertRaises(CodecError):
                self.decode(value, expected)


if __name__ == "__main__":
    unittest.main()
