"""Tests for scripts/check_lufs_device.py (#151)."""

from __future__ import annotations

import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_lufs_device as cl  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
LUFS, METER = cl.DEVICES


class TreeTests(unittest.TestCase):
    """The real files, copied into a scratch root and changed one at a time."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        files = [cl.WIRE]
        for device in cl.DEVICES:
            files += [device.maxpat, device.version_txt]
            if device.genexpr is not None:
                files.append(device.genexpr)
        for rel in files:
            (self.root / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(REPO / rel, self.root / rel)

    def tearDown(self) -> None:
        shutil.rmtree(self.root)

    def edit(self, rel: Path, old: str, new: str) -> None:
        path = self.root / rel
        text = path.read_text(encoding="utf-8")
        self.assertIn(old, text)
        path.write_text(text.replace(old, new, 1), encoding="utf-8")

    def patcher(self, device: cl.Device = LUFS) -> dict:
        return json.loads((self.root / device.maxpat).read_text(encoding="utf-8"))

    def save(self, patch: dict, device: cl.Device = LUFS) -> None:
        (self.root / device.maxpat).write_text(json.dumps(patch, indent="\t"), encoding="utf-8")

    def box(self, patch: dict, first_word: str) -> dict:
        return next(b for b in cl.boxes(patch["patcher"]) if cl.words(b)[:1] == [first_word])

    def restore(self, device: cl.Device) -> None:
        shutil.copyfile(REPO / device.maxpat, self.root / device.maxpat)

    def errors(self) -> list[str]:
        return cl.check(self.root)

    def test_the_repository_passes(self) -> None:
        self.assertEqual(cl.check(REPO), [])
        self.assertEqual(cl.main(["--root", str(REPO)]), 0)

    def test_both_patches_pass(self) -> None:
        self.assertEqual(self.errors(), [])
        self.assertEqual([d.address for d in cl.DEVICES], ["/fohmixer/lufs", "/fohmixer/meter"])

    def test_the_hubs_port_is_read_from_the_wire_module(self) -> None:
        self.assertEqual(cl.default_port("x\npub const DEFAULT_PORT: u16 = 39110;\ny"), 39110)
        self.assertIsNone(cl.default_port("pub const DEFAULT_PORT: u32 = 39110;"))
        self.edit(cl.WIRE, "pub const DEFAULT_PORT: u16 = 39110;", "pub const PORT: u16 = 1;")
        self.assertEqual(len(self.errors()), 1)
        self.assertIn("DEFAULT_PORT", self.errors()[0])

    def test_a_parameter_object_fails(self) -> None:
        for device in cl.DEVICES:
            patch = self.patcher(device)
            patch["patcher"]["boxes"].append(
                {"box": {"id": "obj-x", "maxclass": "live.numbox", "numinlets": 1}}
            )
            self.save(patch, device)
            self.assertIn(
                f"{device.maxpat.as_posix()}: obj-x is a parameter (the device has none)",
                self.errors(),
            )
        patch = self.patcher(METER)
        self.box(patch, "plugin~")["parameter_enable"] = 1
        patch["patcher"]["parameters"] = {}
        self.save(patch, METER)
        errors = self.errors()
        self.assertIn(
            f"{METER.maxpat.as_posix()}: obj-plugin is a parameter (the device has none)", errors
        )
        self.assertTrue(any("parameters table" in e for e in errors), errors)
        self.assertFalse(cl.is_parameter({"maxclass": "flonum", "parameter_enable": 0}))
        self.assertFalse(cl.is_parameter({"maxclass": "newobj", "text": "live.path"}))

    def test_udpsend_must_be_localhost_and_the_hubs_port(self) -> None:
        for text in ("udpsend 0.0.0.0 39110", "udpsend 127.0.0.1 39111", "udpsend 127.0.0.1"):
            patch = self.patcher()
            self.box(patch, "udpsend")["text"] = text
            self.save(patch)
            self.assertTrue(any("udpsend 127.0.0.1 39110" in e for e in self.errors()), text)
        patch = self.patcher(METER)
        patch["patcher"]["boxes"].append(
            {"box": {"id": "obj-x", "maxclass": "newobj", "text": "udpsend 127.0.0.1 39110"}}
        )
        self.save(patch, METER)
        self.assertTrue(any("exactly one 'udpsend" in e for e in self.errors()))

    def test_the_audio_clock_sets_each_devices_rate(self) -> None:
        # The owner's ruling of 27.10: the LUFS device sends every 200 ms,
        # the meter every 33 ms, each on its audio object's own interval.
        self.assertEqual([d.send_ms for d in cl.DEVICES], [200, 33])
        self.assertEqual([d.clock for d in cl.DEVICES], ["snapshot~", "peakamp~"])
        for device, old, wrong in (
            (LUFS, '"snapshot~ 200"', "snapshot~ 100"),
            (METER, '"peakamp~ 33"', "peakamp~ 200"),
        ):
            self.edit(device.maxpat, old, f'"{wrong}"')
            errors = self.errors()
            self.assertTrue(
                any(
                    f"every {device.clock} must be '{device.clock} {device.send_ms}'" in e
                    for e in errors
                ),
                errors,
            )
            self.restore(device)
        self.assertEqual(self.errors(), [])

    def test_no_metro_bangs_the_pack(self) -> None:
        # The review of PR 2: a metro sent a frozen value with the DSP off.
        for device in cl.DEVICES:
            patch = self.patcher(device)
            patch["patcher"]["boxes"].append(
                {
                    "box": {
                        "id": "obj-send-metro",
                        "maxclass": "newobj",
                        "text": "metro 200",
                        "numinlets": 2,
                        "numoutlets": 1,
                    }
                }
            )
            patch["patcher"]["lines"].append(
                {"patchline": {"source": ["obj-send-metro", 0], "destination": ["obj-pack", 0]}}
            )
            self.save(patch, device)
            errors = self.errors()
            self.assertTrue(
                any(
                    "the pack must be banged by obj-send-trigger alone (no metro)" in e
                    for e in errors
                ),
                errors,
            )
            self.restore(device)

    def test_the_send_leaves_the_audio_thread_through_a_deferlow(self) -> None:
        # #240: the audio side's clock (peakamp~, snapshot~) runs in Live's
        # audio thread; the socket write goes through obj-send-defer.
        for device in cl.DEVICES:
            patch = self.patcher(device)
            patch["patcher"]["lines"] = [
                ln
                for ln in patch["patcher"]["lines"]
                if ln["patchline"]["destination"] != ["obj-udpsend", 0]
            ]
            patch["patcher"]["lines"].append(
                {"patchline": {"source": ["obj-pack", 0], "destination": ["obj-udpsend", 0]}}
            )
            self.save(patch, device)
            errors = self.errors()
            where = device.maxpat.as_posix()
            self.assertIn(
                f"{where}: the udpsend must be fed by obj-send-defer alone, has [['obj-pack', 0]]",
                errors,
            )
            self.assertTrue(
                any(
                    f"('{device.clock} {device.send_ms}') reaches the udpsend obj-udpsend "
                    "on the audio thread" in e
                    for e in errors
                ),
                errors,
            )
            self.restore(device)
        self.assertEqual(self.errors(), [])

    def test_the_deferlow_is_fed_by_the_pack_alone(self) -> None:
        for device in cl.DEVICES:
            patch = self.patcher(device)
            self.box(patch, "deferlow")
            for b in cl.boxes(patch["patcher"]):
                if b.get("id") == cl.SEND_DEFER:
                    b["text"] = "defer"
            patch["patcher"]["lines"].append(
                {
                    "patchline": {
                        "source": ["obj-send-trigger", 0],
                        "destination": ["obj-send-defer", 0],
                    }
                }
            )
            self.save(patch, device)
            errors = self.errors()
            where = device.maxpat.as_posix()
            self.assertIn(f"{where}: obj-send-defer must be 'deferlow', has [['defer']]", errors)
            self.assertTrue(
                any("obj-send-defer must be fed by the pack alone" in e for e in errors), errors
            )
            self.restore(device)

    def test_a_timed_box_reaching_a_network_hot_inlet_is_named(self) -> None:
        def box_(i: str, text: str, inlets: int = 1, message: bool = False) -> dict:
            maxclass = "message" if message else "newobj"
            return {"id": i, "maxclass": maxclass, "text": text, "numinlets": inlets}

        def line(src: str, out: int, dst: str, inlet: int) -> dict:
            return {"patchline": {"source": [src, out], "destination": [dst, inlet]}}

        def run(boxes: list[dict], lines: list[dict]) -> list[str]:
            patcher = {"boxes": [{"box": b} for b in boxes], "lines": lines}
            return cl.thread_errors("p", patcher, cl.boxes(patcher))

        net = box_("n", "udpsend 127.0.0.1 1")
        clock = box_("m", "metro 100", 2)
        pack = box_("k", "pack a 0", 2)
        self.assertEqual(
            run([net, clock, pack], [line("m", 0, "k", 0), line("k", 0, "n", 0)]),
            [
                "p: m ('metro 100') reaches the udpsend n on the audio thread "
                "(a deferlow or a qmetro goes between)"
            ],
        )
        # A cold inlet only stores; a deferlow or a qmetro hands it over.
        self.assertEqual(run([net, clock, pack], [line("m", 0, "k", 1), line("k", 0, "n", 0)]), [])
        for low in ("deferlow", "defer", "qmetro 100"):
            hop = box_("d", low)
            self.assertEqual(
                run(
                    [net, clock, pack, hop],
                    [line("m", 0, "d", 0), line("d", 0, "k", 0), line("k", 0, "n", 0)],
                ),
                [],
            )
        # A message box named like a clock is no clock; a gate passes its
        # right inlet; jit.net.send sends from both inlets.
        self.assertEqual(run([net, box_("m", "metro", message=True)], [line("m", 0, "n", 0)]), [])
        gate = box_("g", "gate", 2)
        self.assertEqual(
            len(run([net, clock, gate], [line("m", 0, "g", 1), line("g", 0, "n", 0)])), 1
        )
        self.assertEqual(run([net, clock, gate], [line("m", 0, "g", 0), line("g", 0, "n", 0)]), [])
        jit = box_("j", "jit.net.send @port 1", 2)
        self.assertEqual(len(run([jit, clock], [line("m", 0, "j", 1)])), 1)
        for timed in ("delay 5", "snapshot~ 200", "peakamp~ 33", "transport"):
            self.assertEqual(len(run([net, box_("m", timed, 2)], [line("m", 0, "n", 0)])), 1, timed)

    def test_the_trigger_is_fed_by_the_last_value_alone(self) -> None:
        for device in cl.DEVICES:
            patch = self.patcher(device)
            for item in patch["patcher"]["lines"]:
                line = item["patchline"]
                if line["destination"] == [cl.SEND_TRIGGER, 0]:
                    line["destination"] = ["obj-pack", 9]
            self.save(patch, device)
            errors = self.errors()
            self.assertTrue(
                any(
                    f"obj-send-trigger must be fed by {device.send_from} alone, has []" in e
                    for e in errors
                ),
                errors,
            )
            self.restore(device)
        patch = self.patcher(LUFS)
        next(b for b in cl.boxes(patch["patcher"]) if b.get("id") == cl.SEND_TRIGGER)["text"] = (
            "t b i"
        )
        self.save(patch, LUFS)
        self.assertTrue(any("obj-send-trigger must be 't b f'" in e for e in self.errors()))

    def test_the_port_follows_the_wire_module(self) -> None:
        self.edit(cl.WIRE, "DEFAULT_PORT: u16 = 39110;", "DEFAULT_PORT: u16 = 39111;")
        errors = self.errors()
        self.assertEqual(len(errors), 2, errors)
        self.assertTrue(all("'udpsend 127.0.0.1 39111'" in e for e in errors), errors)

    def test_the_address_box_names_the_kind(self) -> None:
        patch = self.patcher()
        pack = self.box(patch, "pack")
        pack["text"] = pack["text"].replace("/fohmixer/lufs", "/fohmixer/meter")
        self.save(patch)
        self.assertIn(
            f"{LUFS.maxpat.as_posix()}: the pack sends to /fohmixer/meter, expected /fohmixer/lufs",
            self.errors(),
        )
        patch = self.patcher(METER)
        pack = self.box(patch, "pack")
        pack["text"] = pack["text"] + " 0"
        self.save(patch, METER)
        self.assertTrue(any("one pack of 10 arguments" in e for e in self.errors()))

    def test_the_pack_sends_the_version(self) -> None:
        patch = self.patcher(METER)
        pack = self.box(patch, "pack")
        version = (REPO / METER.version_txt).read_text(encoding="utf-8").strip()
        pack["text"] = pack["text"].replace(f"v{version}", "v0.0.1")
        self.save(patch, METER)
        self.assertIn(
            f"{METER.maxpat.as_posix()}: the pack sends v0.0.1, expected v{version}",
            self.errors(),
        )

    def test_the_output_is_silence_and_nothing_else_feeds_plugout(self) -> None:
        # The input straight through (the probe's pass-through), or the gen~.
        for source in (["obj-plugin", 0], ["obj-gen", 2]):
            self.restore(LUFS)
            patch = self.patcher()
            patch["patcher"]["lines"].append(
                {"patchline": {"source": source, "destination": ["obj-plugout", 1]}}
            )
            self.save(patch)
            errors = self.errors()
            self.assertEqual(
                errors,
                [
                    f"{LUFS.maxpat.as_posix()}: plugout~ must take silence only (a sig~ 0 "
                    f"nothing feeds; the device sits in a parallel chain), it takes "
                    f"['{source[0]}']"
                ],
            )
        # A sig~ with another value, or one the input feeds.
        self.restore(LUFS)
        patch = self.patcher(METER)
        self.box(patch, "sig~")["text"] = "sig~ 1."
        self.save(patch, METER)
        self.assertTrue(any("silence only" in e for e in self.errors()))
        self.restore(METER)
        patch = self.patcher(METER)
        patch["patcher"]["lines"].append(
            {"patchline": {"source": ["obj-plugin", 0], "destination": ["obj-silence", 0]}}
        )
        self.save(patch, METER)
        self.assertTrue(any("it takes ['obj-silence']" in e for e in self.errors()))
        # `sig~ 0` and no audio into plugout~ at all are silence too.
        self.restore(METER)
        patch = self.patcher(METER)
        self.box(patch, "sig~")["text"] = "sig~ 0"
        self.save(patch, METER)
        self.assertEqual(self.errors(), [])
        patch["patcher"]["lines"] = [
            ln
            for ln in patch["patcher"]["lines"]
            if ln["patchline"]["destination"][0] != "obj-plugout"
        ]
        self.save(patch, METER)
        self.assertEqual(self.errors(), [])

    def test_a_patch_without_plugout_fails(self) -> None:
        patch = self.patcher()
        self.box(patch, "plugout~")["text"] = "dac~"
        self.save(patch)
        self.assertIn(
            f"{LUFS.maxpat.as_posix()}: needs exactly one plugin~ and one plugout~", self.errors()
        )

    def test_a_line_past_a_boxs_ports_fails(self) -> None:
        patch = self.patcher(METER)
        patch["patcher"]["lines"].append(
            {"patchline": {"source": ["obj-plugin", 5], "destination": ["obj-pack", 0]}}
        )
        patch["patcher"]["lines"].append(
            {"patchline": {"source": ["obj-gone", 0], "destination": ["obj-pack", 0]}}
        )
        self.save(patch, METER)
        errors = self.errors()
        self.assertTrue(any("obj-plugin:5 -> obj-pack:0 is past" in e for e in errors), errors)
        self.assertTrue(any("not there: obj-gone" in e for e in errors), errors)

    def test_the_identity_messages_are_there(self) -> None:
        patch = self.patcher()
        box = next(
            b
            for b in cl.boxes(patch["patcher"])
            if b.get("text") == "path this_device canonical_parent canonical_parent"
        )
        box["text"] = "getpath"
        self.save(patch)
        self.assertIn(
            f"{LUFS.maxpat.as_posix()}: no 'path this_device canonical_parent canonical_parent' "
            "message",
            self.errors(),
        )
        self.assertEqual(len(cl.IDENTITY), 5)

    def line(self, patch: dict, src: list, dst: list) -> None:
        patch["patcher"]["lines"].append({"patchline": {"source": src, "destination": dst}})

    def node(self, patch: dict, box_id: str, text: str, inlets: int = 2) -> None:
        patch["patcher"]["boxes"].append(
            {
                "box": {
                    "id": box_id,
                    "maxclass": "newobj",
                    "text": text,
                    "numinlets": inlets,
                    "numoutlets": 1,
                }
            }
        )

    def test_the_identity_is_never_read_by_another_clock(self) -> None:
        # #240: a `metro 1000` re-read every name each second (9 LOM calls
        # per device) on Live's main thread.
        for device in cl.DEVICES:
            patch = self.patcher(device)
            self.node(patch, "obj-id-metro", "metro 1000")
            self.line(patch, ["obj-thisdevice", 0], ["obj-id-metro", 0])
            self.line(patch, ["obj-id-metro", 0], ["obj-id-defer", 0])
            self.save(patch, device)
            errors = self.errors()
            where = device.maxpat.as_posix()
            self.assertIn(
                f"{where}: obj-id-metro ('metro 1000') drives the live.observer "
                "obj-parent1-observer (the identity is read at load, on a change and by its "
                "own 10 s clock only)",
                errors,
            )
            self.assertIn(
                f"{where}: obj-id-defer:0 takes [['obj-thisdevice', 0]] alone, "
                "has [['obj-id-metro', 0], ['obj-thisdevice', 0]]",
                errors,
            )
            self.assertIn(
                f"{where}: obj-id-metro ('metro 1000') runs on a clock (the device has none "
                "but the identity's)",
                errors,
            )
            self.restore(device)
        self.assertEqual(self.errors(), [])

    def test_the_identity_is_read_again_every_10_s_by_its_own_staggered_clock(self) -> None:
        # #240, the PC proof's attempt 2: `live.path`'s follow missed a move
        # into a chain for over 30 s. The DECIDED: the four parents' paths
        # again every 10 s, from a qmetro (the low-priority queue) started
        # after a random offset drawn at load.
        for device in cl.DEVICES:
            where = device.maxpat.as_posix()
            patch = self.patcher(device)
            ids = {b.get("id"): cl.words(b) for b in cl.boxes(patch["patcher"])}
            self.assertEqual(ids.get(cl.ID_CLOCK), ["qmetro", "10000"])
            self.assertEqual(ids.get(cl.ID_OFFSET), ["random", "10000"])
            for level in range(1, cl.PARENTS + 1):
                self.assertEqual(
                    cl.feeds(patch["patcher"], f"obj-parent{level}-path", 0),
                    sorted([[cl.ID_LOAD, cl.PARENTS - level], [cl.ID_TICK, cl.PARENTS - level]]),
                )
            for text in ("qmetro 1000", "qmetro 9999", "metro 10000", "qmetro 10000 @interval 10"):
                patch = self.patcher(device)
                for b in cl.boxes(patch["patcher"]):
                    if b.get("id") == cl.ID_CLOCK:
                        b["text"] = text
                self.save(patch, device)
                errors = self.errors()
                self.assertIn(
                    f"{where}: obj-id-clock must be 'qmetro 10000', has {text.split()}", errors
                )
                self.assertIn(
                    f"{where}: obj-id-clock ('{text}') drives the live.path "
                    "obj-parent1-live-path (the identity is read at load, on a change and by "
                    "its own 10 s clock only)",
                    errors,
                )
                self.assertIn(
                    f"{where}: obj-id-clock ('{text}') runs on a clock (the device has none "
                    "but the identity's)",
                    errors,
                )
            self.restore(device)
        self.assertEqual(self.errors(), [])

    def test_the_identity_clock_drives_the_parents_paths_alone(self) -> None:
        # The review of the 10 s re-read: the clock is exempt from
        # lom_clock_errors, so a LOM read fed from it passed every check.
        tick = sorted(
            [cl.PARENTS - level, f"obj-parent{level}-path", 0] for level in range(1, cl.PARENTS + 1)
        )
        for device in cl.DEVICES:
            where = device.maxpat.as_posix()
            for src in (["obj-id-clock", 0], ["obj-id-tick", 0], ["obj-id-offset-delay", 0]):
                patch = self.patcher(device)
                self.node(patch, "obj-x-get", "get value")
                patch["patcher"]["boxes"][-1]["box"]["maxclass"] = "message"
                self.node(patch, "obj-x-object", "live.object")
                self.line(patch, src, ["obj-x-get", 0])
                self.line(patch, ["obj-x-get", 0], ["obj-x-object", 0])
                self.save(patch, device)
                errors = self.errors()
                want = {
                    "obj-id-clock": [[0, "obj-id-tick", 0]],
                    "obj-id-tick": tick,
                    "obj-id-offset-delay": [[0, "obj-id-clock-defer", 0]],
                }[src[0]]
                has = sorted(want + [[0, "obj-x-get", 0]])
                self.assertIn(f"{where}: {src[0]} outputs into {want} alone, has {has}", errors)
            patch = self.patcher(device)
            self.node(patch, "obj-id-clock", "qmetro 10000")
            self.save(patch, device)
            self.assertIn(f"{where}: obj-id-clock must be one box, has 2", self.errors())
            self.restore(device)
        self.assertEqual(self.errors(), [])

    def test_the_identity_clock_is_deferred_staggered_and_alone(self) -> None:
        cases = (
            # The offset's bang straight into the start: no deferlow.
            (
                [["obj-id-offset-delay", 0], ["obj-id-clock-start", 0]],
                "obj-id-clock-start:0 takes [['obj-id-clock-defer', 0]] alone, "
                "has [['obj-id-clock-defer', 0], ['obj-id-offset-delay', 0]]",
            ),
            # The load straight into the start: no offset.
            (
                [["obj-thisdevice", 0], ["obj-id-clock-defer", 0]],
                "obj-id-clock-defer:0 takes [['obj-id-offset-delay', 0]] alone, "
                "has [['obj-id-offset-delay', 0], ['obj-thisdevice', 0]]",
            ),
            # The interval set from elsewhere.
            (
                [["obj-thisdevice", 1], ["obj-id-clock", 1]],
                "obj-id-clock:1 takes [] alone, has [['obj-thisdevice', 1]]",
            ),
            # The delay's time set from elsewhere.
            (
                [["obj-thisdevice", 1], ["obj-id-offset-delay", 1]],
                "obj-id-offset-delay:1 takes [] alone, has [['obj-thisdevice', 1]]",
            ),
            # The tick banged by something else too.
            (
                [["obj-id-load", 0], ["obj-id-tick", 0]],
                "obj-id-tick:0 takes [['obj-id-clock', 0]] alone, "
                "has [['obj-id-clock', 0], ['obj-id-load', 0]]",
            ),
        )
        for device in cl.DEVICES:
            where = device.maxpat.as_posix()
            for (src, dst), want in cases:
                patch = self.patcher(device)
                self.line(patch, src, dst)
                self.save(patch, device)
                self.assertIn(f"{where}: {want}", self.errors())
            # A second clock beside the identity's, into a parent's path.
            patch = self.patcher(device)
            self.node(patch, "obj-x", "qmetro 10000")
            self.line(patch, ["obj-x", 0], ["obj-parent2-path", 0])
            self.save(patch, device)
            errors = self.errors()
            self.assertIn(
                f"{where}: obj-x ('qmetro 10000') drives the live.path obj-parent2-live-path "
                "(the identity is read at load, on a change and by its own 10 s clock only)",
                errors,
            )
            self.restore(device)
        self.assertEqual(self.errors(), [])

    def test_each_parent_takes_its_id_from_both_live_path_outlets(self) -> None:
        # #240: the follow (the middle outlet) missed a move into a chain on
        # the PC; the left outlet answers every `path` message (the load's,
        # the 10 s re-read's), as the devices before #240 read it for months.
        for device in cl.DEVICES:
            where = device.maxpat.as_posix()
            for level in range(1, cl.PARENTS + 1):
                path = f"obj-parent{level}-live-path"
                both = f"[['{path}', 0], ['{path}', 1]]"
                for outlet in (0, 1):
                    patch = self.patcher(device)
                    patch["patcher"]["lines"] = [
                        ln
                        for ln in patch["patcher"]["lines"]
                        if ln["patchline"]["source"] != [path, outlet]
                    ]
                    self.save(patch, device)
                    errors = self.errors()
                    self.assertIn(
                        f"{where}: obj-parent{level}-route-id:0 takes {both} alone, "
                        f"has [['{path}', {1 - outlet}]]",
                        errors,
                    )
                    self.assertIn(
                        f"{where}: {path}'s outlet {outlet} goes into its route id alone, has []",
                        errors,
                    )
            patch = self.patcher(device)
            self.line(patch, ["obj-parent2-live-path", 0], ["obj-parent2-object", 1])
            self.save(patch, device)
            self.assertIn(
                f"{where}: obj-parent2-live-path's outlet 0 goes into its route id alone, "
                "has [['obj-parent2-object', 1], ['obj-parent2-route-id', 0]]",
                self.errors(),
            )
            self.restore(device)
        self.assertEqual(self.errors(), [])

    def test_an_observers_output_never_reaches_a_name_slot(self) -> None:
        # The FINDING of 2026-11-17 (the PC rollout of #240): through
        # `live.observer` -> `tosymbol` a name with a space kept literal
        # quotes and the set's name read `-`, so no device matched. The
        # observer only triggers the read; the value is `live.object`'s.
        for device in cl.DEVICES:
            where = device.maxpat.as_posix()
            patch = self.patcher(device)
            self.line(patch, ["obj-parent2-observer", 0], ["obj-parent2-symbol", 0])
            self.save(patch, device)
            errors = self.errors()
            self.assertIn(
                f"{where}: the observer obj-parent2-observer reaches obj-pack:5 by its "
                "output (an observer only triggers the read)",
                errors,
            )
            self.assertIn(
                f"{where}: the observer obj-parent2-observer outputs into one 't b' alone "
                "(a trigger of the read), has [['obj-parent2-changed', 0], "
                "['obj-parent2-symbol', 0]]",
                errors,
            )
            self.assertIn(
                f"{where}: obj-parent2-symbol:0 takes [['obj-parent2-reg', 0]] alone, "
                "has [['obj-parent2-observer', 0], ['obj-parent2-reg', 0]]",
                errors,
            )
            self.restore(device)
        self.assertEqual(self.errors(), [])

    def test_the_value_path_is_live_object_route_name_zl_reg_tosymbol(self) -> None:
        for device in cl.DEVICES:
            where = device.maxpat.as_posix()
            patch = self.patcher(device)
            for b in cl.boxes(patch["patcher"]):
                if b.get("id") == "obj-parent3-object":
                    b["text"] = "live.observer"
            self.save(patch, device)
            errors = self.errors()
            self.assertIn(
                f"{where}: obj-pack:6 takes its value from live.object -> route name -> "
                "zl reg -> tosymbol (obj-parent3-object -> obj-parent3-route-name -> "
                "obj-parent3-reg -> obj-parent3-symbol), has "
                "['live.observer', 'route', 'zl', 'tosymbol']",
                errors,
            )
            self.assertIn(
                f"{where}: the observer obj-parent3-object reaches obj-pack:6 by its "
                "output (an observer only triggers the read)",
                errors,
            )
            self.restore(device)

    def test_the_set_is_read_without_an_observer(self) -> None:
        # The set's level read `-` on the PC through an observer: it is read
        # at load and on its path's follow only.
        patch = self.patcher(METER)
        self.node(patch, "obj-parent4-observer", "live.observer")
        self.line(patch, ["obj-id-property", 0], ["obj-parent4-observer", 0])
        self.line(patch, ["obj-parent4-observer", 0], ["obj-parent4-read", 0])
        self.save(patch, METER)
        where = METER.maxpat.as_posix()
        errors = self.errors()
        self.assertIn(
            f"{where}: the set (parent 4) has no observer (it reads at load and follow), "
            "has obj-parent4-observer",
            errors,
        )
        self.assertIn(
            f"{where}: obj-parent4-read:0 takes [['obj-parent4-sel', 1]] alone, "
            "has [['obj-parent4-observer', 0], ['obj-parent4-sel', 1]]",
            errors,
        )
        self.assertEqual(cl.identity_boxes()["obj-parent4-t"], ["t", "i", "i"])
        self.assertEqual(cl.identity_boxes()["obj-parent1-t"], ["t", "i", "i", "i"])

    def test_the_set_takes_its_id_from_both_live_path_outlets(self) -> None:
        # The review of the regression fix: the set's `-` on the PC may as
        # well be a middle outlet that never gave an id for the set's path;
        # the left outlet gives it at the load's `path` message, as the
        # devices before #240 read it (since the PC proof's attempt 2 every
        # parent does).
        for device in cl.DEVICES:
            where = device.maxpat.as_posix()
            patch = self.patcher(device)
            patch["patcher"]["lines"] = [
                ln
                for ln in patch["patcher"]["lines"]
                if ln["patchline"]["source"] != ["obj-parent4-live-path", 0]
            ]
            self.save(patch, device)
            self.assertIn(
                f"{where}: obj-parent4-route-id:0 takes [['obj-parent4-live-path', 0], "
                "['obj-parent4-live-path', 1]] alone, has [['obj-parent4-live-path', 1]]",
                self.errors(),
            )
            self.restore(device)
        self.assertEqual(self.errors(), [])

    def test_a_bang_only_box_ends_the_value_walk(self) -> None:
        def run(middle: str, maxclass: str = "newobj") -> list[str]:
            patcher = {
                "boxes": [
                    {"box": {"id": "o", "maxclass": "newobj", "text": "live.observer"}},
                    {"box": {"id": "k", "maxclass": maxclass, "text": middle}},
                    {"box": {"id": "s", "maxclass": "newobj", "text": "tosymbol"}},
                ],
                "lines": [
                    {"patchline": {"source": ["o", 0], "destination": ["k", 0]}},
                    {"patchline": {"source": ["k", 0], "destination": ["s", 0]}},
                    {"patchline": {"source": ["s", 0], "destination": ["p", 4]}},
                ],
            }
            return [
                e for e in cl.value_errors("p", patcher, cl.boxes(patcher), "p") if "reaches" in e
            ]

        self.assertEqual(run("t b"), [])
        self.assertEqual(run("-", "message"), [])
        self.assertEqual(len(run("$1", "message")), 1)
        self.assertEqual(len(run("set $1", "message")), 1)
        self.assertEqual(len(run("t l")), 1)
        self.assertEqual(len(run("t b s")), 1)
        self.assertEqual(len(run("zl reg")), 1)
        # A message's right inlet sets the text its next bang sends.
        patcher = {
            "boxes": [
                {"box": {"id": "o", "maxclass": "newobj", "text": "live.observer"}},
                {"box": {"id": "m", "maxclass": "message", "text": "-"}},
                {"box": {"id": "s", "maxclass": "newobj", "text": "tosymbol"}},
            ],
            "lines": [
                {"patchline": {"source": ["o", 0], "destination": ["m", 1]}},
                {"patchline": {"source": ["m", 0], "destination": ["s", 0]}},
                {"patchline": {"source": ["s", 0], "destination": ["p", 4]}},
            ],
        }
        found = [e for e in cl.value_errors("p", patcher, cl.boxes(patcher), "p") if "reaches" in e]
        self.assertEqual(
            found,
            ["p: the observer o reaches p:4 by its output (an observer only triggers the read)"],
        )

    def test_every_identity_line_is_held(self) -> None:
        for (box, inlet), sources in cl.identity_wiring("obj-pack").items():
            for src in sources:
                patch = self.patcher(METER)
                patch["patcher"]["lines"] = [
                    ln
                    for ln in patch["patcher"]["lines"]
                    if (ln["patchline"]["source"], ln["patchline"]["destination"])
                    != ([src[0], src[1]], [box, inlet])
                ]
                self.save(patch, METER)
                self.assertTrue(
                    any(f"{box}:{inlet} takes" in e for e in self.errors()), (box, inlet, src)
                )
        self.restore(METER)
        self.assertEqual(self.errors(), [])

    def test_each_pack_slot_has_its_one_source(self) -> None:
        # The review of #240: the device's id and the two values were unguarded.
        for device in cl.DEVICES:
            where = device.maxpat.as_posix()
            patch = self.patcher(device)
            patch["patcher"]["lines"] = [
                ln
                for ln in patch["patcher"]["lines"]
                if ln["patchline"]["destination"] not in (["obj-pack", 2], ["obj-pack", 8])
            ]
            self.line(patch, ["obj-id-route", 1], ["obj-pack", 2])
            self.line(patch, [device.send_from, 0], ["obj-pack", 8])
            self.line(patch, ["obj-parent1-symbol", 0], ["obj-pack", 9])
            self.save(patch, device)
            errors = self.errors()
            self.assertIn(
                f"{where}: obj-pack:2 takes [['obj-id-route', 0]] alone, has [['obj-id-route', 1]]",
                errors,
            )
            self.assertIn(
                f"{where}: obj-pack:8 takes the first value's clip alone, "
                f"has [['{device.send_from}', 0]]",
                errors,
            )
            self.assertTrue(
                any("obj-pack:9 takes [['obj-send-trigger', 1]] alone" in e for e in errors)
            )
            self.restore(device)
        self.assertEqual(self.errors(), [])

    def test_the_walk_takes_unknown_classes_hot_on_every_inlet(self) -> None:
        def box_(i: str, text: str, inlets: int = 2) -> dict:
            return {"id": i, "maxclass": "newobj", "text": text, "numinlets": inlets}

        def line(src: str, dst: str, inlet: int) -> dict:
            return {"patchline": {"source": [src, 0], "destination": [dst, inlet]}}

        def run(middle: str, inlet: int) -> list[str]:
            patcher = {
                "boxes": [
                    {"box": box_("n", "udpsend 127.0.0.1 1", 1)},
                    {"box": box_("m", "del 5")},
                    {"box": box_("k", middle)},
                ],
                "lines": [line("m", "k", inlet), line("k", "n", 0)],
            }
            return cl.thread_errors("p", patcher, cl.boxes(patcher))

        # `pak` and `join` fire from every inlet; `pack` and `zl reg` store
        # on the right. `del` is `delay`.
        self.assertEqual(len(run("pak 0 0", 1)), 1)
        self.assertEqual(len(run("join 2 @triggers 0 1", 1)), 1)
        self.assertEqual(run("pack 0 0", 1), [])
        self.assertEqual(run("zl reg", 1), [])
        self.assertEqual(len(run("pack 0 0", 0)), 1)
        for timed in ("midiin", "notein 1", "ctlin", "edge~", "plugsync~", "makenote 100 150"):
            patcher = {
                "boxes": [
                    {"box": box_("n", "udpsend 127.0.0.1 1", 1)},
                    {"box": box_("m", timed)},
                ],
                "lines": [line("m", "n", 0)],
            }
            self.assertEqual(len(cl.thread_errors("p", patcher, cl.boxes(patcher))), 1, timed)

    def test_a_gone_parent_reads_a_dash(self) -> None:
        patch = self.patcher()
        for b in cl.boxes(patch["patcher"]):
            if b.get("id") == "obj-parent4-none":
                b["text"] = "none"
        self.save(patch)
        self.assertIn(
            f"{LUFS.maxpat.as_posix()}: obj-parent4-none must be '-', has ['none']",
            self.errors(),
        )

    def test_the_meter_patch_needs_no_gen(self) -> None:
        patch = self.patcher(METER)
        self.assertEqual([b for b in cl.boxes(patch["patcher"]) if cl.words(b)[:1] == ["gen~"]], [])
        self.assertEqual(self.errors(), [])
        # The LUFS patch does.
        patch = self.patcher()
        self.box(patch, "gen~")["text"] = "mc.gen~"
        self.save(patch)
        self.assertTrue(any("exactly one gen~" in e for e in self.errors()))

    def test_the_embedded_code_must_equal_the_genexpr(self) -> None:
        self.edit(LUFS.genexpr, "out1 = 0;", "out1 = in1;")
        self.assertEqual(
            self.errors(), [f"{LUFS.maxpat.as_posix()}: the gen~ code differs from its .genexpr"]
        )

    def test_an_absolute_path_fails(self) -> None:
        for path in ("C:\\\\Users\\\\x\\\\a.amxd", "/Users/x/a.amxd", "/home/x/a", "D:/x"):
            patch = self.patcher(METER)
            patch["patcher"]["filename"] = path
            self.save(patch, METER)
            self.assertTrue(any("absolute file path" in e for e in self.errors()), path)

    def test_the_lufs_face_shows_its_version_txt(self) -> None:
        (self.root / LUFS.version_txt).write_text("9.9.9\n", encoding="utf-8")
        errors = self.errors()
        self.assertTrue(any("expected ['v9.9.9']" in e for e in errors), errors)
        self.assertTrue(all(LUFS.maxpat.as_posix() in e for e in errors), errors)

    def test_the_meter_face_shows_its_version_txt(self) -> None:
        (self.root / METER.version_txt).write_text("9.9.9\n", encoding="utf-8")
        errors = self.errors()
        self.assertTrue(any("expected ['v9.9.9']" in e for e in errors), errors)
        self.assertTrue(all(METER.maxpat.as_posix() in e for e in errors), errors)

    def test_text_that_is_no_patcher_fails(self) -> None:
        (self.root / METER.maxpat).write_text("not json", encoding="utf-8")
        self.assertTrue(any("is not a Max patcher" in e for e in self.errors()))
        self.assertEqual(cl.main(["--root", str(self.root)]), 1)


if __name__ == "__main__":
    unittest.main()
