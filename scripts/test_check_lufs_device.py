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

    def test_the_identity_is_read_at_load_never_on_a_clock(self) -> None:
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
                "obj-parent1-observer (the identity is read at load and on a change only)",
                errors,
            )
            self.assertIn(
                f"{where}: obj-id-defer:0 takes [['obj-thisdevice', 0]] alone, "
                "has [['obj-id-metro', 0], ['obj-thisdevice', 0]]",
                errors,
            )
            self.assertIn(
                f"{where}: obj-id-metro ('metro 1000') runs on a clock (the device has none)",
                errors,
            )
            self.restore(device)
        self.assertEqual(self.errors(), [])

    def test_the_names_come_from_observers_on_the_followed_path(self) -> None:
        for device in cl.DEVICES:
            where = device.maxpat.as_posix()
            patch = self.patcher(device)
            for b in cl.boxes(patch["patcher"]):
                if b.get("id") == "obj-parent2-observer":
                    b["text"] = "live.object"
            self.line(patch, ["obj-parent3-live-path", 0], ["obj-parent3-route-id", 0])
            self.save(patch, device)
            errors = self.errors()
            self.assertIn(
                f"{where}: obj-parent2-observer must be 'live.observer', has ['live.object']",
                errors,
            )
            self.assertIn(
                f"{where}: obj-parent2-observer is a live.object "
                "(the names come from live.observer)",
                errors,
            )
            self.assertIn(
                f"{where}: obj-parent3-live-path's left outlet (it follows the object) "
                "goes nowhere, has [['obj-parent3-route-id', 0]]",
                errors,
            )
            self.assertIn(
                f"{where}: obj-parent3-route-id:0 takes [['obj-parent3-live-path', 1]] alone, "
                "has [['obj-parent3-live-path', 0], ['obj-parent3-live-path', 1]]",
                errors,
            )
            self.restore(device)

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
