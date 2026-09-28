"""The TouchOSC import tool on its synthetic fixtures (S3 plan, Task 7).

The fixtures are built into a temporary folder from ``build_fixtures.py``.
One test compares ``fixtures/expected-layout.json`` with the import of the
fixtures (``FOHMIXER_REGENERATE=1`` rewrites it after a deliberate change),
and the Rust test ``fohmixer-proto``'s ``imported_layout`` parses and
validates it, so the layout the tool writes is the one the hub serves.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import build_fixtures  # noqa: E402
import import_tosc  # noqa: E402

EXPECTED = os.path.join(HERE, "fixtures", "expected-layout.json")


def items(page):
    return page["items"]


def by_kind(item_list, kind):
    return [i for i in item_list if i["kind"] == kind]


def all_items(layout):
    """Every item of every page, nested page and the overlay."""
    found = list(layout["overlay"])

    def walk(page):
        found.extend(page["items"])
        for sub in page.get("pager", {}).get("pages", []):
            walk(sub)

    for page in layout["pages"]:
        walk(page)
    return found


class ImportTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.dir = tempfile.mkdtemp(prefix="import-tosc-test-")
        cls.tosc, cls.als = build_fixtures.build(cls.dir)
        cls.layout = import_tosc.import_files(cls.tosc, cls.als)
        cls.report = cls.layout["report"]
        cls.pages = {p["title"]: p for p in cls.layout["pages"]}
        cls.foh = cls.pages["FOH"]
        cls.sub = {p["title"]: p for p in cls.foh["pager"]["pages"]}

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.dir, ignore_errors=True)

    def strip_named(self, name):
        found = [
            i
            for i in all_items(self.layout)
            if i["kind"] == "strip" and i["binding"]["anchor"].get("name") == name
        ]
        self.assertEqual(len(found), 1, name)
        return found[0]

    def midi(self, label):
        found = [i for i in all_items(self.layout) if i.get("label") == label]
        self.assertEqual(len(found), 1, label)
        return found[0]

    def verdict(self, message):
        found = [m for m in self.report["midi"] if m["message"] == message]
        self.assertEqual(len(found), 1, message)
        return found[0]

    def dropped(self, why):
        return [d for d in self.report["dropped"] if why in d["why"]]

    # --- structure ---

    def test_pages_the_nested_pager_and_the_root_tab_bar(self):
        layout = self.layout
        self.assertEqual(layout["schema"], 1)
        self.assertEqual(layout["canvas"], {"w": 2360.0, "h": 1640.0})
        self.assertEqual([p["title"] for p in layout["pages"]], ["Cue", "FOH", "Conf"])
        self.assertEqual(
            layout["tabbar"], {"orientation": "top", "bar_size": 59.0, "default_page": 1}
        )
        self.assertEqual(len({p["id"] for p in layout["pages"]}), 3)
        self.assertEqual(self.foh["tab"], {"color": "#404040FF", "text_size": 33.0})
        pager = self.foh["pager"]
        self.assertEqual(pager["frame"], {"x": 229.0, "y": 61.0, "w": 1746.0, "h": 773.0})
        self.assertEqual(
            pager["tabbar"], {"orientation": "left", "bar_size": 65.0, "default_page": 0}
        )
        self.assertEqual([p["title"] for p in pager["pages"]], ["STAGE", "OTHERS"])
        self.assertEqual(pager["pages"][0]["tab"]["color"], "#BAFFA657")

    def test_the_root_overlay_holds_the_alert_the_techalert_strip_and_refresh(self):
        overlay = self.layout["overlay"]
        self.assertEqual([i["kind"] for i in overlay], ["strip", "refresh", "alert"])
        self.assertEqual(overlay[0]["strip_kind"], "meter_mute_only")
        self.assertEqual(overlay[1]["label"], "REFRESH ALL")
        self.assertEqual(overlay[1]["style"]["bg"], "#808080FF")
        alert = overlay[2]
        self.assertEqual(alert["binding"], overlay[0]["binding"])
        self.assertEqual(alert["period_ms"], 300)
        # Clipped to the canvas (the box stuck out to the left and right).
        self.assertEqual(alert["frame"], {"x": 0.0, "y": 80.0, "w": 2360.0, "h": 1553.0})
        self.assertEqual(alert["style"]["bg"], "#FF00001F")
        self.assertTrue(self.dropped("battery"), self.report["dropped"])

    def test_the_battery_gauge_is_dropped_whole_and_reported_once(self):
        # The battery script sits on the gauge's fader; the group, its box
        # and its "100%" label go with it (no static fake gauge).
        self.assertEqual(
            [d for d in self.report["dropped"] if "battery" in d["node"] or "X5" in d["why"]],
            [{"node": "root/battery", "why": "the battery gauge (X5)"}],
        )
        self.assertEqual([i["kind"] for i in self.layout["overlay"]], ["strip", "refresh", "alert"])

    def test_an_alert_box_before_its_techalert_strip_is_kept(self):
        # In the real project the hidden alert box is the root's second
        # child, before REFRESH ALL, the battery and the TechAlert strip.
        path = os.path.join(self.dir, "alert-first.tosc")
        with open(path, "wb") as f:
            f.write(build_fixtures.tosc_bytes(build_fixtures.alert_first(build_fixtures.project())))
        layout = import_tosc.import_files(path, self.als)
        overlay = layout["overlay"]
        # Node order stays the z-order: the box blinks under the overlay.
        self.assertEqual([i["kind"] for i in overlay], ["alert", "refresh", "strip"])
        self.assertEqual([i["z"] for i in overlay], sorted(i["z"] for i in overlay))
        self.assertEqual(overlay[0]["binding"], overlay[2]["binding"])
        self.assertEqual(overlay[0]["frame"], {"x": 0.0, "y": 80.0, "w": 2360.0, "h": 1553.0})
        self.assertEqual(overlay[0]["period_ms"], 300)
        self.assertFalse(
            [d for d in layout["report"]["dropped"] if "alert box" in d["why"]],
            layout["report"]["dropped"],
        )

    def test_only_the_refresh_control_refreshes_and_backdrops_stay_inert(self):
        # The sidebar backdrops carry the mute script, which mentions a
        # refresh; only the control that asks for one is REFRESH ALL.
        refresh = [i for i in all_items(self.layout) if i["kind"] == "refresh"]
        self.assertEqual(
            [(i["label"], i["frame"]) for i in refresh],
            [("REFRESH ALL", {"x": 21.0, "y": 1300.0, "w": 209.0, "h": 54.0})],
        )
        backdrops = [
            i for i in items(self.foh) if i["kind"] == "area" and i["style"] == {"bg": "#000000BD"}
        ]
        self.assertEqual(len(backdrops), 5, [i["kind"] for i in items(self.foh)])
        self.assertEqual(backdrops[0]["frame"], {"x": 40.0, "y": 174.0, "w": 101.0, "h": 77.0})

    def test_partly_visible_nodes_are_clipped_not_dropped(self):
        # A return strip 28 px past the canvas's bottom edge (all its parts
        # inside) is kept, clipped; its double-click guard matches it.
        echo = self.strip_named("A-Echo")
        self.assertEqual(echo["strip_kind"], "return")
        self.assertEqual(echo["frame"], {"x": 2180.0, "y": 898.0, "w": 161.0, "h": 742.0})
        self.assertEqual(len(echo["children"]), 8)
        self.assertTrue(echo["mute_guard"])
        self.assertNotIn(
            "double_click_mute 'master_A-Echo': matches no strip", self.report["stale_config"]
        )
        # The bottom row's area and its vertical title, past the bottom edge.
        areas = {a.get("title"): a for a in by_kind(items(self.foh), "area") if "title" in a}
        self.assertEqual(areas[""]["frame"], {"x": 271.0, "y": 901.0, "w": 529.0, "h": 739.0})
        self.assertEqual(areas["EFFECTS"]["frame"], {"x": 247.0, "y": 902.0, "w": 53.0, "h": 738.0})
        # A nested page's backdrop larger than the pager: its part in the page.
        backdrop = self.sub["STAGE"]["items"][0]
        self.assertEqual(backdrop["style"]["bg"], "#9D9DA0FF")
        self.assertEqual(backdrop["frame"], {"x": 294.0, "y": 61.0, "w": 1681.0, "h": 773.0})
        # The TechAlert meter, taller than its group: its part in the group.
        self.assertEqual(
            self.strip_named("TechAlert #")["children"]["meter"],
            {"x": 77.0, "y": 1144.0, "w": 10.0, "h": 12.0},
        )
        # A box starting above its page (under the root tab bar).
        box9 = [a for a in by_kind(items(self.foh), "area") if a["frame"]["x"] == 37.0]
        self.assertEqual(box9[0]["frame"], {"x": 37.0, "y": 59.0, "w": 178.0, "h": 1185.0})
        # Each clipped node is reported.
        for path in (
            "root/pager1/WORSHIP/master_A-Echo",
            "root/pager1/WORSHIP/effects_area",
            "root/pager1/WORSHIP/effects",
            "root/pager1/WORSHIP/FOH /STAGE/backdrop",
            "root/band_TechAlert #/meter",
            "root/pager1/WORSHIP/box9",
            "root/alert",
        ):
            self.assertIn(path, self.report["clipped"])

    def test_frames_are_composed_into_canvas_coordinates(self):
        # A label at (10,10) in a group at (100,100) on a page at (0,59).
        marks = [i for i in items(self.foh) if i["kind"] == "label" and i["text"] == "[]"]
        self.assertEqual(marks[0]["frame"], {"x": 110.0, "y": 169.0, "w": 20.0, "h": 20.0})
        # A strip on a nested page: page (65,0) in the pager (229,2) on FOH (0,59).
        vocal = self.strip_named("Vocal 1 repro#")
        self.assertEqual(vocal["frame"], {"x": 389.0, "y": 101.0, "w": 136.0, "h": 706.0})
        self.assertEqual(vocal["children"]["mute"], {"x": 404.0, "y": 749.0, "w": 106.0, "h": 52.0})

    def test_z_follows_node_order(self):
        zs = [i["z"] for i in items(self.foh)]
        self.assertEqual(zs, sorted(zs))
        self.assertEqual(len(set(zs)), len(zs))

    def test_areas_and_labels_keep_their_colours(self):
        areas = by_kind(items(self.foh), "area")
        effects = [a for a in areas if a.get("title") == "EFFECTS"]
        self.assertEqual(effects[0]["style"]["bg"], "#636363FF")
        self.assertTrue(effects[0]["style"]["vertical"])
        box = [a for a in areas if "title" not in a]
        self.assertEqual(box[0]["style"]["bg"], "#00000080")
        self.assertEqual(box[0]["frame"], {"x": 27.0, "y": 69.0, "w": 198.0, "h": 1240.0})

    # --- strips ---

    def test_strip_kinds_are_recognised(self):
        self.assertEqual(self.strip_named("Vocal 1 repro#")["strip_kind"], "narrow")
        self.assertEqual(self.strip_named("Hand1 #")["strip_kind"], "standard")
        self.assertEqual(self.strip_named("Hand2 #")["strip_kind"], "solid")
        self.assertEqual(self.strip_named("Hand2 #")["style"]["bg"], "#F5FF14FF")
        ret = self.strip_named("B-Main repro #")
        self.assertEqual(ret["strip_kind"], "return")
        self.assertEqual(ret["binding"]["anchor"]["kind"], "return")
        self.assertEqual(self.strip_named("TechAlert #")["strip_kind"], "meter_mute_only")

    def test_the_instance_comes_from_the_group_name_prefix(self):
        self.assertEqual(
            self.strip_named("Hand1 #")["binding"],
            {"instance": "master", "anchor": {"kind": "track", "name": "Hand1 #"}},
        )
        self.assertEqual(
            self.strip_named("Keys 1")["binding"]["instance"], "band", "no prefix: band"
        )
        self.assertEqual(self.strip_named("Vocal 1 repro#")["binding"]["instance"], "band")

    def test_strip_parts_have_their_geometry(self):
        children = self.strip_named("Hand1 #")["children"]
        self.assertEqual(
            sorted(children),
            ["db", "fader", "instance_label", "label", "meter", "mute", "pan", "status"],
        )
        alert = self.strip_named("TechAlert #")["children"]
        self.assertEqual(sorted(alert), ["label", "meter", "mute", "status"])

    def test_decorative_script_carriers_are_dropped(self):
        decoration = self.report["decoration"]
        # Six full strips on the canvas (one of them partly): two backdrop
        # buttons with the mute script each (the off-canvas strip is
        # dropped whole).
        self.assertEqual(decoration["mute script carriers"], 12)
        self.assertEqual(decoration["second meter bars"], 6)
        self.assertEqual(decoration["scale labels"], 6)
        self.assertEqual(decoration["tick lines"], 6)
        self.assertEqual(decoration["meter dBFS labels (D11)"], 6)
        for item in all_items(self.layout):
            self.assertNotIn("button1", json.dumps(item))

    def test_the_script_table_normalises_trailing_newlines(self):
        roles = self.report["scripts"]
        meter = [h for h, r in roles.items() if "meter" in r]
        self.assertEqual(len(meter), 1, roles)
        self.assertIn("second meter bar", roles[meter[0]])

    def test_double_click_mute_becomes_the_mute_guard(self):
        self.assertTrue(self.strip_named("Hand1 #")["mute_guard"])
        self.assertFalse(self.strip_named("Hand2 #")["mute_guard"])

    # --- sidebar ---

    def test_solos_stage_mics_and_stage_aut(self):
        solos = by_kind(items(self.foh), "solo")
        self.assertEqual(
            [s["binding"]["anchor"]["name"] for s in solos], ["Vocals Repro grp#", "Stems grp#"]
        )
        stage = by_kind(items(self.foh), "stage")
        self.assertEqual(len(stage), 1)
        self.assertEqual(stage[0]["binding"]["anchor"]["name"], "Mics Stage #")
        self.assertTrue(stage[0]["aut"])
        self.assertEqual(stage[0]["style"]["bg"], "#000594FF")
        aut = by_kind(items(self.foh), "hub_toggle")
        self.assertEqual(aut[0]["key"], "stage_aut")
        self.assertEqual(aut[0]["label"], "STAGE AUT")

    def test_stage_buttons_take_their_inner_buttons_colour(self):
        # The stage and STAGE AUT groups are transparent; their blue is on
        # the inner buttons.
        stage = by_kind(items(self.foh), "stage")[0]
        aut = by_kind(items(self.foh), "hub_toggle")[0]
        self.assertEqual(stage["style"]["bg"], "#000594FF")
        self.assertEqual(aut["style"]["bg"], "#000594FF")
        self.assertEqual(stage["style"]["text"], "STAGE")

    # --- config ---

    def test_the_config_text_is_read(self):
        self.assertEqual(self.report["instances"], ["band", "master"])
        self.assertEqual(
            self.layout["config"]["unfold"],
            [
                {"instance": "band", "name": "Vocals Repro grp#"},
                {"instance": "band", "name": "Old grp#"},
            ],
        )
        conf = by_kind(items(self.pages["Conf"]), "label")
        self.assertIn("connection_band: 2", conf[0]["text"])

    def test_stale_config_is_reported_never_fixed(self):
        stale = self.report["stale_config"]
        self.assertIn("unfold_band 'Old grp#': no such group track in the set", stale)
        self.assertIn("double_click_mute 'band_Nothing': matches no strip", stale)
        self.assertEqual(len(stale), 2, stale)

    def test_unresolved_and_ambiguous_bindings_are_reported(self):
        unresolved = self.report["unresolved"]
        self.assertIn("band track 'Keys 1': 2 tracks have this name (ambiguous)", unresolved)
        self.assertEqual(len(unresolved), 1, unresolved)

    # --- former MIDI controls ---

    def test_an_on_off_mapping_becomes_a_toggle_on_the_mute(self):
        cue = self.midi("Vox 1 TU")
        self.assertEqual(cue["kind"], "param_toggle")
        self.assertEqual(cue["press"], "toggle")
        self.assertEqual(
            cue["targets"],
            [
                {
                    "binding": {
                        "instance": "band",
                        "anchor": {"kind": "track", "name": "Vocal 3 repro#"},
                    },
                    "prop": "mute",
                    "on": False,
                    "off": True,
                }
            ],
        )
        self.assertEqual(self.verdict("CC20 ch14")["verdict"], "clean")

    def test_a_return_activator_and_press_modes(self):
        reverb = self.midi("REVERB")
        self.assertEqual(
            reverb["targets"][0]["binding"]["anchor"], {"kind": "return", "name": "A-Reverb #"}
        )
        voc = self.midi("VOC MIC")
        self.assertEqual(voc["press"], "double_tap_latch")
        self.assertEqual(
            [t["binding"]["anchor"]["name"] for t in voc["targets"]],
            ["Vocal 1 repro#", "Vocal 2 repro#"],
        )
        self.assertEqual(self.midi("ZVUKAR")["press"], "pulse_and_double_tap_latch")

    def test_a_full_range_continuous_mapping_becomes_a_cc_linear_fader(self):
        fader = self.midi("Podklady All")
        self.assertEqual(fader["kind"], "param_fader")
        self.assertEqual(
            fader["targets"],
            [
                {
                    "binding": {
                        "instance": "band",
                        "anchor": {"kind": "track", "name": name},
                        "path": "mixer_device volume",
                    },
                    "prop": "value",
                    "scale": "cc_linear",
                }
                for name in ("Drums #", "Bass #")
            ],
        )
        self.assertEqual(fader["frame"], {"x": 755.0, "y": 1079.0, "w": 101.0, "h": 399.0})

    def test_a_rack_macro_mapping_is_a_path_through_its_racks(self):
        autotune = self.midi("AUTOTUNE")
        self.assertEqual(
            autotune["targets"],
            [
                {
                    "binding": {
                        "instance": "band",
                        "anchor": {"kind": "track", "name": "Vocal 1 repro#"},
                        "path": "devices[name=Vox Chain] chains[name=Main] "
                        "devices[name=Latencies] parameters 1",
                    },
                    "prop": "value",
                    "on": 127.0,
                    "off": 0.0,
                }
            ],
        )

    def test_a_send_toggle_writes_the_ends_of_its_range(self):
        repro = self.midi("REPRO")
        self.assertEqual(repro["targets"][0]["binding"]["path"], "mixer_device sends 1")
        self.assertEqual((repro["targets"][0]["on"], repro["targets"][0]["off"]), (1.0, 0.0))

    def test_a_standalone_midi_button_keeps_its_place_and_its_sibling_label(self):
        # REPRO is a button outside any group, at (75,909) on the page at
        # (0,59): its frame is composed once, and the label drawn over it
        # (a sibling node) names it.
        repro = [
            i for i in items(self.foh) if i["kind"] == "param_toggle" and i["label"] == "REPRO"
        ]
        self.assertEqual(len(repro), 1, [i.get("label") for i in items(self.foh)])
        self.assertEqual(repro[0]["frame"], {"x": 75.0, "y": 968.0, "w": 102.0, "h": 60.0})
        self.assertEqual(repro[0]["press"], "toggle")
        self.assertEqual(
            self.verdict("CC31 ch14"),
            {"control": "REPRO", "message": "CC31 ch14", "verdict": "clean", "targets": 1},
        )
        # The label is the control's, not a second static text over it.
        self.assertFalse([i for i in all_items(self.layout) if i.get("text") == "REPRO"])
        self.assertFalse(
            [d for d in self.report["dropped"] if "button42" in d["node"]], self.report["dropped"]
        )

    def test_former_midi_controls_are_named_by_their_visible_labels(self):
        # Every label of a former MIDI control carries the restyle script;
        # Podklady's name is two labels; its volume readout and the
        # "ON/OFF" labels are not names.
        self.assertEqual(
            {m["message"]: m["control"] for m in self.report["midi"]},
            {
                "CC20 ch14": "Vox 1 TU",
                "CC28 ch14": "Gitara 2",
                "NOTE29 ch14": "ALERT LOOP",
                "CC55 ch14": "REVERB",
                "CC56 ch14": "VOC MIC",
                "CC36 ch14": "AUTOTUNE",
                "CC30 ch14": "ZVUKAR",
                "CC31 ch14": "REPRO",
                "CC58 ch14": "HALF",
                "CC67 ch14": "SELECT",
                "CC57 ch14": "Podklady All",
            },
        )
        self.assertFalse(
            [i for i in all_items(self.layout) if i.get("text") in ("REPRO", "- 0.0", "All")]
        )

    def test_unmapped_partial_and_unsupported_controls_are_dropped(self):
        self.assertEqual(self.verdict("CC28 ch14")["why"], "no mapping in the set")
        self.assertEqual(self.verdict("NOTE29 ch14")["why"], "no mapping in the set")
        self.assertIn("partial range", self.verdict("CC58 ch14")["why"])
        self.assertIn("not reproduced", self.verdict("CC67 ch14")["why"])
        for label in ("Gitara 2", "ALERT LOOP", "HALF", "SELECT"):
            self.assertFalse([i for i in all_items(self.layout) if i.get("label") == label], label)
        clean = sorted(m["message"] for m in self.report["midi"] if m["verdict"] == "clean")
        self.assertEqual(
            clean,
            [
                "CC20 ch14",
                "CC30 ch14",
                "CC31 ch14",
                "CC36 ch14",
                "CC55 ch14",
                "CC56 ch14",
                "CC57 ch14",
            ],
        )
        self.assertEqual(self.report["macro_bus_links"], 2)

    # --- drops ---

    def test_off_canvas_and_hidden_nodes_are_reported(self):
        self.assertTrue(
            [
                d
                for d in self.report["dropped"]
                if "master_Hand3 #" in d["node"] and d["why"] == "off the canvas"
            ],
            self.report["dropped"],
        )
        self.assertTrue(
            [
                d
                for d in self.report["dropped"]
                if d["node"].endswith("/hidden") and d["why"] == "hidden"
            ],
            self.report["dropped"],
        )
        self.assertTrue(self.dropped("page-change message (X6)"), self.report["dropped"])

    # --- output ---

    def test_the_expected_layout_file_is_this_import(self):
        # The committed file is today's import (the Rust schema test reads
        # it). After a deliberate change to the import, regenerate it with
        # FOHMIXER_REGENERATE=1 and commit the result.
        text = import_tosc.dumps(self.layout)
        if os.environ.get("FOHMIXER_REGENERATE") == "1":
            with open(EXPECTED, "w", encoding="utf-8") as f:
                f.write(text)
        with open(EXPECTED, encoding="utf-8") as f:
            self.assertEqual(f.read(), text)

    def test_every_frame_is_inside_the_canvas(self):
        def check(frame, where):
            self.assertGreaterEqual(frame["x"], 0, where)
            self.assertGreaterEqual(frame["y"], 0, where)
            self.assertLessEqual(frame["x"] + frame["w"], 2360, where)
            self.assertLessEqual(frame["y"] + frame["h"], 1640, where)

        for item in all_items(self.layout):
            check(item["frame"], item)
            for frame in item.get("children", {}).values():
                check(frame, item)

    def test_the_command_line_writes_the_layout_and_the_report(self):
        out = os.path.join(self.dir, "layout.json")
        report = os.path.join(self.dir, "report.md")
        result = subprocess.run(
            [
                sys.executable,
                os.path.join(HERE, "import_tosc.py"),
                self.tosc,
                "--set",
                self.als,
                "--out",
                out,
                "--report",
                report,
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        with open(out, encoding="utf-8") as f:
            self.assertEqual(json.load(f), self.layout)
        with open(report, encoding="utf-8") as f:
            text = f.read()
        self.assertIn("# TouchOSC import report", text)
        self.assertIn("CC28 ch14", text)
        self.assertIn("no mapping in the set", text)
        self.assertIn("unfold_band 'Old grp#'", text)

    def test_the_command_line_refuses_a_file_that_is_no_project(self):
        bad = os.path.join(self.dir, "bad.tosc")
        with open(bad, "wb") as f:
            f.write(b"not zlib")
        result = subprocess.run(
            [
                sys.executable,
                os.path.join(HERE, "import_tosc.py"),
                bad,
                "--out",
                os.path.join(self.dir, "x.json"),
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("import-tosc:", result.stderr)

    def test_without_a_set_every_midi_control_is_dropped(self):
        layout = import_tosc.import_files(self.tosc, None)
        self.assertEqual({m["verdict"] for m in layout["report"]["midi"]}, {"dropped"})
        self.assertEqual({m["why"] for m in layout["report"]["midi"]}, {"no set given (--set)"})
        self.assertEqual(layout["report"]["unresolved"], [])


if __name__ == "__main__":
    unittest.main()
