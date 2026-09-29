"""The TouchOSC import tool on its synthetic fixtures (S3 plan, Task 7; the UI
redesign note §3, #21).

The fixtures are built into a temporary folder from ``build_fixtures.py``.
One test compares ``fixtures/expected-layout.json`` with the import of the
fixtures (``FOHMIXER_REGENERATE=1`` rewrites it after a deliberate change),
and the Rust test ``fohmixer-proto``'s ``imported_layout`` parses and
validates it, so the layout the tool writes is the one the hub serves. The
grouping rules are also tested on small projects built here and on the
geometry functions directly.
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
from build_fixtures import N, label, strip  # noqa: E402

EXPECTED = os.path.join(HERE, "fixtures", "expected-layout.json")


def groups_of(page):
    """Every group of a root page: its rows' groups and its sub-pages' groups."""
    found = []
    for row in page.get("rows", []):
        for section in row["sections"]:
            if section["kind"] == "pager":
                for sub in section["pages"]:
                    found.extend(sub["sections"])
            else:
                found.append(section)
    return found


def all_controls(layout):
    """Every control of the layout: rails, groups (sub-pages too), global."""
    found = list(layout["global"])
    for page in layout["pages"]:
        found.extend(page.get("rail", []))
        for group in groups_of(page):
            found.extend(group["controls"])
    return found


def all_ids(layout):
    """Every page, sub-page, pager and group id."""
    found = []
    for page in layout["pages"]:
        found.append(page["id"])
        for row in page.get("rows", []):
            for section in row["sections"]:
                found.append(section["id"])
                for sub in section.get("pages", []):
                    found.append(sub["id"])
                    found.extend(g["id"] for g in sub["sections"])
    return found


def row_ids(page):
    return [[s["id"] for s in row["sections"]] for row in page.get("rows", [])]


def names(group):
    return [import_tosc.display(c) for c in group["controls"]]


def keys_anywhere(value):
    """Every dict key in a JSON value."""
    if isinstance(value, dict):
        return set(value) | set().union(*(keys_anywhere(v) for v in value.values()))
    if isinstance(value, list):
        return set().union(*(keys_anywhere(v) for v in value))
    return set()


# --- small projects for single rules ---------------------------------------------------------


def page_node(title, children):
    """A root page under a 59 px tab bar."""
    return N("GROUP", title, (0, 59, 2360, 1581), children, tabLabel=title)


def pager_node(name, frame, pages, default=0):
    return N("PAGER", name, frame, pages, values={"page": str(default)})


def root_node(pages, overlay=()):
    pager = pager_node("pager1", (0, 0, 2360, 1640), pages)
    return N("GROUP", "root", (0, 0, 2360, 1640), [pager, *overlay])


def solo_node(name, frame, text):
    """A solo button group (a transparent group, the inner ``btn_solo``)."""
    w, h = frame[2], frame[3]
    button = N("BUTTON", "btn_solo", (0, 0, w, h), buttonType=2, background=True)
    return N("GROUP", name, frame, [button, label("label", (0, 0, w, 30), text)])


def box_node(name, frame, color=(0.2, 0.4, 0.6, 1)):
    return N("BOX", name, frame, color=color)


def title_node(name, frame, text, color=(0.5, 0.5, 0.5, 1)):
    return label(name, frame, text, background=True, color=color)


def item(x, y, w, h, z=0, **fields):
    """A placed item for the geometry functions."""
    return {"frame": (float(x), float(y), float(w), float(h)), "z": z, **fields}


class ImportTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.dir = tempfile.mkdtemp(prefix="import-tosc-test-")
        cls.tosc, cls.als = build_fixtures.build(cls.dir)
        cls.layout = import_tosc.import_files(cls.tosc, cls.als)
        cls.report = cls.layout["report"]
        cls.pages = {p["title"]: p for p in cls.layout["pages"]}
        cls.foh = cls.pages["FOH"]
        cls.pager = cls.foh["rows"][0]["sections"][0]
        cls.sub = {p["title"]: p for p in cls.pager["pages"]}
        cls.groups = {g["id"]: g for p in cls.layout["pages"] for g in groups_of(p)}

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.dir, ignore_errors=True)

    def import_root(self, root, name):
        path = os.path.join(self.dir, f"{name}.tosc")
        with open(path, "wb") as f:
            f.write(build_fixtures.tosc_bytes(root))
        return import_tosc.import_files(path, None)

    def strip_named(self, name, instance=None):
        found = [
            c
            for c in all_controls(self.layout)
            if c["kind"] == "strip"
            and c["binding"]["anchor"].get("name") == name
            and instance in (None, c["binding"]["instance"])
        ]
        self.assertEqual(len(found), 1, name)
        return found[0]

    def midi(self, label_text):
        found = [c for c in all_controls(self.layout) if c.get("label") == label_text]
        self.assertEqual(len(found), 1, label_text)
        return found[0]

    def verdict(self, message):
        found = [m for m in self.report["midi"] if m["message"] == message]
        self.assertEqual(len(found), 1, message)
        return found[0]

    def dropped(self, why):
        return [d for d in self.report["dropped"] if why in d["why"]]

    # --- structure ---

    def test_pages_the_default_page_and_the_nested_pager(self):
        layout = self.layout
        self.assertEqual(layout["schema"], 2)
        self.assertEqual([p["title"] for p in layout["pages"]], ["Cue", "FOH", "Conf"])
        self.assertEqual([p["id"] for p in layout["pages"]], ["cue", "foh", "conf"])
        self.assertEqual(layout["default_page"], "foh")
        self.assertEqual(self.pager["kind"], "pager")
        self.assertEqual(self.pager["id"], "foh-pager")
        self.assertEqual(self.pager["default_page"], "stage")
        self.assertEqual(
            [(p["id"], p["title"]) for p in self.pager["pages"]],
            [("stage", "STAGE"), ("others", "OTHERS")],
        )

    def test_no_geometry_and_no_touchosc_look_is_written(self):
        # Schema 2 says what is on a page; the frames only group.
        written = keys_anywhere({k: v for k, v in self.layout.items() if k != "report"})
        for key in (
            "frame",
            "z",
            "style",
            "canvas",
            "tabbar",
            "tab",
            "overlay",
            "items",
            "children",
            "background",
            "pager",
            "width",
            "fader",
        ):
            self.assertNotIn(key, written)
        kinds = {c.get("strip_kind") for c in all_controls(self.layout)}
        self.assertEqual(kinds - {None}, {"standard", "return"})

    def test_the_layout_keys_are_the_contracts(self):
        self.assertEqual(
            list(self.layout), ["schema", "default_page", "pages", "global", "config", "report"]
        )
        self.assertEqual(list(self.foh), ["id", "title", "rail", "rows"])
        # A page without a rail (the cue and Conf pages) writes none.
        self.assertEqual(list(self.pages["Conf"]), ["id", "title", "rows"])

    # --- the global controls (the root overlay) ---

    def test_the_overlay_becomes_one_alert_and_refresh(self):
        # The TechAlert strip and the hidden alert box are one control; the
        # label is the strip's own.
        techalert = {"instance": "band", "anchor": {"kind": "track", "name": "TechAlert #"}}
        self.assertEqual(
            self.layout["global"],
            [
                {"kind": "alert", "binding": techalert, "period_ms": 300, "label": "TechAlert"},
                {"kind": "refresh", "label": "REFRESH ALL"},
            ],
        )
        # The TechAlert strip is no strip of its own anywhere.
        self.assertFalse(
            [
                c
                for c in all_controls(self.layout)
                if c["kind"] == "strip" and c["binding"] == techalert
            ]
        )

    def test_the_battery_gauge_is_dropped_whole_and_reported_once(self):
        # The battery script sits on the gauge's fader; the group, its box
        # and its "100%" label go with it (no static fake gauge).
        self.assertEqual(
            [d for d in self.report["dropped"] if "battery" in d["node"] or "X5" in d["why"]],
            [{"node": "root/battery", "why": "the battery gauge (X5)"}],
        )
        self.assertEqual([c["kind"] for c in self.layout["global"]], ["alert", "refresh"])

    def test_a_guarded_techalert_strip_keeps_its_guard_on_the_alert(self):
        saved = build_fixtures.CONFIG_TEXT
        try:
            build_fixtures.CONFIG_TEXT = saved + "double_click_mute: 'band_TechAlert #'\n"
            project = build_fixtures.project()
        finally:
            build_fixtures.CONFIG_TEXT = saved
        path = os.path.join(self.dir, "guarded-alert.tosc")
        with open(path, "wb") as f:
            f.write(build_fixtures.tosc_bytes(project))
        layout = import_tosc.import_files(path, self.als)
        self.assertIs(layout["global"][0]["mute_guard"], True)
        self.assertNotIn("double_click_mute 'band_TechAlert #'", json.dumps(layout["report"]))
        # Unguarded (the fixture's own Conf text): no field.
        self.assertNotIn("mute_guard", self.layout["global"][0])

    def test_an_alert_box_before_its_techalert_strip_is_kept(self):
        # In the real project the hidden alert box is the root's second
        # child, before REFRESH ALL, the battery and the TechAlert strip.
        path = os.path.join(self.dir, "alert-first.tosc")
        with open(path, "wb") as f:
            f.write(build_fixtures.tosc_bytes(build_fixtures.alert_first(build_fixtures.project())))
        layout = import_tosc.import_files(path, self.als)
        self.assertEqual(layout["global"], self.layout["global"])
        self.assertFalse(
            [d for d in layout["report"]["dropped"] if "alert box" in d["why"]],
            layout["report"]["dropped"],
        )

    def test_other_overlay_labels_are_dropped_and_reported(self):
        self.assertIn(
            {"node": "root/caption", "why": "an overlay label (schema 2 keeps no free text)"},
            self.report["dropped"],
        )
        self.assertNotIn("MAIN MIX", json.dumps(self.layout["pages"] + self.layout["global"]))

    def test_only_the_refresh_control_refreshes_and_backdrops_stay_inert(self):
        # The sidebar backdrops carry the mute script, which mentions a
        # refresh; only the control that asks for one is REFRESH ALL. The
        # backdrops are no controls and no sections (decoration).
        refresh = [c for c in all_controls(self.layout) if c["kind"] == "refresh"]
        self.assertEqual(refresh, [{"kind": "refresh", "label": "REFRESH ALL"}])
        self.assertEqual(self.report["decoration"]["boxes and backdrops without controls"], 8)

    def test_partly_visible_nodes_are_clipped_not_dropped(self):
        # A return strip 28 px past the canvas's bottom edge (all its parts
        # inside) is kept, clipped; its double-click guard matches it.
        echo = self.strip_named("A-Echo")
        self.assertEqual(echo["strip_kind"], "return")
        self.assertTrue(echo["mute_guard"])
        self.assertNotIn(
            "double_click_mute 'master_A-Echo': matches no strip", self.report["stale_config"]
        )
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
        self.assertEqual(len(self.report["clipped"]), 7, self.report["clipped"])

    def test_free_labels_on_pages_are_dropped_and_reported(self):
        # The "[]" mark in a group at (100,100) on FOH: no control, no title.
        self.assertIn(
            {
                "node": "root/pager1/WORSHIP/group100/mark",
                "why": "a free label (schema 2 keeps no free text)",
            },
            self.report["dropped"],
        )

    # --- rail ---

    def test_the_rail_holds_the_pages_non_strip_controls_top_to_bottom(self):
        # FOH has strips: its stage mics, STAGE AUT, solos and toggles, all
        # outside the nested pager, form its rail ordered by y (REPRO, a
        # standalone button, is the lowest).
        self.assertEqual(
            [(c["kind"], import_tosc.display(c)) for c in self.foh["rail"]],
            [
                ("stage", "STAGE"),
                ("hub_toggle", "STAGE AUT"),
                ("solo", "Vocals"),
                ("solo", "Stems"),
                ("param_toggle", "REVERB"),
                ("param_toggle", "VOC MIC"),
                ("param_toggle", "AUTOTUNE"),
                ("param_toggle", "ZVUKAR"),
                ("param_toggle", "REPRO"),
            ],
        )
        # Its groups keep only strips and the param fader.
        self.assertEqual(
            {c["kind"] for g in groups_of(self.foh) for c in g["controls"]},
            {"strip", "param_fader"},
        )

    def test_the_stage_mics_stage_aut_and_solos_keep_their_bindings(self):
        stage, aut, vocals, stems = self.foh["rail"][:4]
        self.assertEqual(
            stage,
            {
                "kind": "stage",
                "binding": {
                    "instance": "band",
                    "anchor": {"kind": "track", "name": "Mics Stage #"},
                },
                "aut": True,
                "label": "STAGE",
            },
        )
        self.assertEqual(aut, {"kind": "hub_toggle", "key": "stage_aut", "label": "STAGE AUT"})
        self.assertEqual(
            [s["binding"]["anchor"]["name"] for s in (vocals, stems)],
            ["Vocals Repro grp#", "Stems grp#"],
        )
        self.assertEqual([s["label"] for s in (vocals, stems)], ["Vocals", "Stems"])

    def test_a_page_without_strips_groups_its_toggles(self):
        cue = self.pages["Cue"]
        self.assertNotIn("rail", cue)
        self.assertEqual(row_ids(cue), [["cue-1"]])
        self.assertEqual(names(self.groups["cue-1"]), ["Vox 1 TU"])

    def test_a_sub_page_with_strips_sends_its_toggles_to_the_pages_rail(self):
        # The rail is the root page's: a sub-page with strips sends its solo
        # there; a sub-page without strips groups its own.
        mixer = N(
            "GROUP",
            "Mixer",
            (65, 0, 1681, 773),
            [strip("Keys 1", 400, 20), solo_node("Stems grp#", (20, 20, 150, 60), "Stems")],
            tabLabel="Mixer",
        )
        cues = N(
            "GROUP",
            "Cues",
            (65, 0, 1681, 773),
            [solo_node("Vocals Repro grp#", (20, 20, 150, 60), "Vocals")],
            tabLabel="Cues",
        )
        page = page_node(
            "Main",
            [
                solo_node("Drums #", (20, 900, 150, 60), "Drums"),
                pager_node("sub", (229, 2, 1746, 773), [mixer, cues]),
            ],
        )
        layout = self.import_root(root_node([page]), "sub-rail")
        main = layout["pages"][0]
        # By y: the sub-page's solo (y 81) above the page's own (y 959).
        self.assertEqual([c["label"] for c in main["rail"]], ["Stems", "Drums"])
        pager = main["rows"][0]["sections"][0]
        mixer_out, cues_out = pager["pages"]
        self.assertEqual([names(g) for g in mixer_out["sections"]], [["Keys 1"]])
        self.assertEqual([names(g) for g in cues_out["sections"]], [["Vocals"]])

    def test_a_solo_whose_label_spells_no_word_shows_its_tracks_name(self):
        # The real layout's solo labels were "[][][]..." placeholders that a
        # TouchOSC script overwrote at run time (#21, the post-deploy check):
        # a text without a letter names nothing, so the surface shows the
        # track's name ("SOLO Stems"), as for a solo without a label.
        page = page_node(
            "Main",
            [
                strip("Keys 1", 400, 20),
                solo_node("Stems grp#", (20, 20, 150, 60), "[][][][][][][][][][]"),
                solo_node("Drums #", (20, 200, 150, 60), "Drums"),
            ],
        )
        layout = self.import_root(root_node([page]), "placeholder-solo")
        rail = layout["pages"][0]["rail"]
        self.assertEqual([c["kind"] for c in rail], ["solo", "solo"])
        self.assertEqual([c.get("label") for c in rail], [None, "Drums"])

    # --- groups ---

    def test_a_control_belongs_to_the_area_holding_its_centre(self):
        # Both reverb returns sit in the EFFECTS box; the Podklady fader's
        # frame overlaps the box's right edge but its centre is 5.5 px
        # outside, so it is not in it.
        self.assertEqual(names(self.groups["foh-3"]), ["A-Reverb #", "A-Reverb #"])
        self.assertEqual(
            [c["binding"]["instance"] for c in self.groups["foh-3"]["controls"]],
            ["band", "master"],
        )
        self.assertEqual(names(self.groups["foh-4"]), ["Podklady All"])

    def test_a_vertical_title_beside_its_box_names_the_group(self):
        effects = self.groups["foh-3"]
        self.assertEqual(effects["title"], "EFFECTS")
        # The box's fill, its alpha dropped.
        self.assertEqual(effects["color"], "#636363")

    def test_a_label_above_its_box_names_the_group(self):
        hands = self.groups["foh-5"]
        self.assertEqual(hands["title"], "HANDS")
        self.assertEqual(hands["color"], "#366B42")
        self.assertEqual(names(hands), ["Hand4 #", "Hand2 #"])

    def test_a_title_inside_its_pages_panel_names_the_group(self):
        # The STAGE sub-page: a grey backdrop and a translucent green panel,
        # both the whole page, and the vertical STAGE title on the panel.
        # The panel is drawn last: it is the strips' box.
        stage = self.sub["STAGE"]["sections"]
        self.assertEqual(len(stage), 1)
        self.assertEqual(stage[0]["id"], "stage-1")
        self.assertEqual(stage[0]["title"], "STAGE")
        self.assertEqual(stage[0]["color"], "#BAFFA6")
        self.assertEqual(names(stage[0]), ["Vocal 1 repro#", "Keys 1"])

    def test_controls_in_no_area_form_untitled_guessed_groups(self):
        # The two top-right strips are one run; the Podklady fader and the
        # A-Echo return are runs of their own (far apart, other rows).
        for gid, members in (
            ("foh-2", ["B-Main repro #", "Hand2 #"]),
            ("foh-4", ["Podklady All"]),
            ("foh-6", ["A-Echo"]),
            ("others-1", ["Hand1 #"]),
            ("cue-1", ["Vox 1 TU"]),
        ):
            group = self.groups[gid]
            self.assertEqual(names(group), members, gid)
            self.assertNotIn("title", group, gid)
            self.assertNotIn("color", group, gid)
        self.assertEqual(
            self.report["guessed"],
            [
                "cue/cue-1: Vox 1 TU",
                "others/others-1: Hand1 #",
                "foh/foh-2: B-Main repro #",
                "foh/foh-2: Hand2 #",
                "foh/foh-4: Podklady All",
                "foh/foh-6: A-Echo",
            ],
        )

    def test_rows_cluster_sections_by_vertical_overlap_and_order_them_by_x(self):
        # Row 1: the nested pager and the fixed top-right strips next to it;
        # row 2: EFFECTS, the Podklady fader, HANDS, A-Echo, left to right.
        # Ids are row-major positions (the pager takes its own).
        self.assertEqual(
            row_ids(self.foh), [["foh-pager", "foh-2"], ["foh-3", "foh-4", "foh-5", "foh-6"]]
        )
        self.assertEqual([list(row) for row in self.foh["rows"]], [["sections"], ["sections"]])

    def test_controls_inside_a_group_are_ordered_by_x(self):
        # Built right to left: the order follows x, not node order.
        page = page_node(
            "Main",
            [
                box_node("box", (100, 100, 900, 800)),
                strip("Hand3 #", 700, 120),
                strip("Hand1 #", 150, 120),
                strip("Hand2 #", 400, 120),
            ],
        )
        layout = self.import_root(root_node([page]), "x-order")
        (group,) = groups_of(layout["pages"][0])
        self.assertEqual(names(group), ["Hand1 #", "Hand2 #", "Hand3 #"])

    def test_a_pager_between_fixed_groups_keeps_its_place(self):
        left = N("GROUP", "Left", (65, 0, 800, 760), [strip("Keys 1", 100, 0)], tabLabel="Left")
        page = page_node(
            "Main",
            [
                box_node("left box", (0, 0, 400, 760)),
                strip("Hand1 #", 50, 20),
                pager_node("sub", (450, 0, 900, 760), [left]),
                strip("Hand2 #", 1500, 20),
                box_node("below", (0, 800, 1200, 760)),
                strip("Hand3 #", 50, 820),
            ],
        )
        layout = self.import_root(root_node([page]), "pager-between")
        main = layout["pages"][0]
        self.assertEqual(row_ids(main), [["main-1", "main-pager", "main-3"], ["main-4"]])
        self.assertEqual(
            [names(g) for g in groups_of(main)],
            [["Hand1 #"], ["Keys 1"], ["Hand2 #"], ["Hand3 #"]],
        )

    def test_a_titled_area_holding_controls_is_its_own_title(self):
        page = page_node(
            "Main",
            [title_node("keys", (100, 100, 400, 800), "KEYS"), strip("Keys 1", 150, 120)],
        )
        layout = self.import_root(root_node([page]), "titled-box")
        (group,) = groups_of(layout["pages"][0])
        self.assertEqual((group["title"], group["color"]), ("KEYS", "#808080"))

    def test_an_inert_backdrop_button_is_no_section(self):
        # A dark button with no function behind a strip (as behind the real
        # sidebar's controls): the strip is in no area, its group a guess.
        page = page_node(
            "Main",
            [build_fixtures.backdrop("button9", (90, 90, 180, 740)), strip("Keys 1", 100, 100)],
        )
        layout = self.import_root(root_node([page]), "backdrop")
        (group,) = groups_of(layout["pages"][0])
        self.assertEqual(group, {"kind": "group", "id": "main-1", "controls": group["controls"]})
        self.assertEqual(layout["report"]["guessed"], ["main/main-1: Keys 1"])
        self.assertEqual(layout["report"]["decoration"]["boxes and backdrops without controls"], 1)

    def test_a_title_without_controls_in_its_box_is_dropped_and_reported(self):
        page = page_node(
            "Main",
            [
                box_node("empty", (100, 100, 400, 400)),
                title_node("orphan", (100, 60, 400, 36), "ORPHAN"),
                strip("Keys 1", 900, 120),
            ],
        )
        layout = self.import_root(root_node([page]), "orphan-title")
        self.assertNotIn("ORPHAN", json.dumps(layout["pages"]))
        self.assertIn(
            {
                "node": "root/pager1/Main/orphan",
                "why": "a section title without controls in its box",
            },
            layout["report"]["dropped"],
        )

    # --- strips ---

    def test_strip_kinds_are_standard_or_return(self):
        self.assertEqual(self.strip_named("Vocal 1 repro#")["strip_kind"], "standard")
        self.assertEqual(self.strip_named("Hand1 #")["strip_kind"], "standard")
        self.assertEqual(self.strip_named("Hand2 #", "band")["strip_kind"], "standard")
        ret = self.strip_named("B-Main repro #")
        self.assertEqual(ret["strip_kind"], "return")
        self.assertEqual(ret["binding"]["anchor"]["kind"], "return")

    def test_a_strip_wider_than_its_pages_median_is_wide(self):
        # FOH's fixed strips: four of 120 px, three of 161 (the median is
        # 120; 161 >= 1.2 x 120). The nested pages compare their own strips.
        for name, instance in (("B-Main repro #", "band"), ("Hand2 #", "band"), ("A-Echo", None)):
            self.assertIs(self.strip_named(name, instance).get("wide"), True, name)
        for name, instance in (
            ("A-Reverb #", "band"),
            ("Hand4 #", None),
            ("Vocal 1 repro#", None),
            ("Hand1 #", None),
        ):
            self.assertNotIn("wide", self.strip_named(name, instance), name)

    def test_defaults_are_omitted(self):
        for control in all_controls(self.layout):
            for key in ("wide", "mute_guard", "aut"):
                self.assertIsNot(control.get(key), False, control)
        self.assertNotIn("weight", keys_anywhere(self.layout["pages"]))
        self.assertEqual(
            self.strip_named("Keys 1"),
            {
                "kind": "strip",
                "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Keys 1"}},
                "strip_kind": "standard",
            },
        )
        # A toggle without its own colour writes none.
        self.assertNotIn("color", self.midi("VOC MIC"))
        self.assertEqual(self.midi("REVERB")["color"], "#875700")

    def test_the_instance_comes_from_the_group_name_prefix(self):
        self.assertEqual(
            self.strip_named("Hand1 #")["binding"],
            {"instance": "master", "anchor": {"kind": "track", "name": "Hand1 #"}},
        )
        self.assertEqual(
            self.strip_named("Keys 1")["binding"]["instance"], "band", "no prefix: band"
        )
        self.assertEqual(self.strip_named("Vocal 1 repro#")["binding"]["instance"], "band")

    def test_decorative_script_carriers_are_dropped(self):
        decoration = self.report["decoration"]
        # Ten full strips on the canvas (one of them partly): two backdrop
        # buttons with the mute script each (the off-canvas strip is
        # dropped whole).
        self.assertEqual(decoration["mute script carriers"], 20)
        self.assertEqual(decoration["second meter bars"], 10)
        self.assertEqual(decoration["scale labels"], 10)
        self.assertEqual(decoration["tick lines"], 10)
        self.assertEqual(decoration["meter dBFS labels (D11)"], 10)
        self.assertNotIn("button1", json.dumps(all_controls(self.layout)))

    def test_the_script_table_normalises_trailing_newlines(self):
        roles = self.report["scripts"]
        meter = [h for h, r in roles.items() if "meter" in r]
        self.assertEqual(len(meter), 1, roles)
        self.assertIn("second meter bar", roles[meter[0]])

    def test_double_click_mute_becomes_the_mute_guard(self):
        self.assertTrue(self.strip_named("Hand1 #")["mute_guard"])
        self.assertNotIn("mute_guard", self.strip_named("Hand2 #", "band"))

    # --- ids and report ---

    def test_ids_are_unique(self):
        ids = all_ids(self.layout)
        self.assertEqual(len(ids), len(set(ids)), ids)
        self.assertEqual(
            sorted(ids),
            sorted(
                [
                    "cue",
                    "cue-1",
                    "foh",
                    "foh-pager",
                    "stage",
                    "stage-1",
                    "others",
                    "others-1",
                    "foh-2",
                    "foh-3",
                    "foh-4",
                    "foh-5",
                    "foh-6",
                    "conf",
                    "conf-1",
                ]
            ),
        )

    def test_ids_stay_unique_when_titles_collide(self):
        # A page titled "FOH pager" takes the id the FOH pager would get; a
        # page titled "Stage 1" the one of the STAGE sub-page's first group.
        stage = N("GROUP", "Stage", (65, 0, 1681, 773), [strip("Keys 1", 100, 0)], tabLabel="Stage")
        foh = page_node("FOH", [pager_node("sub", (229, 2, 1746, 773), [stage])])
        layout = self.import_root(
            root_node([foh, page_node("FOH pager", []), page_node("Stage 1", [])]), "collide"
        )
        ids = all_ids(layout)
        self.assertEqual(len(ids), len(set(ids)), ids)
        self.assertEqual(ids, ["foh", "foh-pager-2", "stage", "stage-1-2", "foh-pager", "stage-1"])

    def test_the_report_lists_the_groups_row_by_row(self):
        conf = self.groups["conf-1"]
        self.assertEqual(
            self.report["groups"],
            {
                "cue": [["cue-1: Vox 1 TU"]],
                "foh": [
                    [
                        "foh-pager (pager): STAGE, OTHERS",
                        "foh-2: B-Main repro #, Hand2 #",
                    ],
                    [
                        "foh-3 EFFECTS: A-Reverb #, A-Reverb #",
                        "foh-4: Podklady All",
                        "foh-5 HANDS: Hand4 #, Hand2 #",
                        "foh-6: A-Echo",
                    ],
                ],
                "stage": [["stage-1 STAGE: Vocal 1 repro#, Keys 1"]],
                "others": [["others-1: Hand1 #"]],
                "conf": [["conf-1: " + ", ".join(names(conf))]],
            },
        )

    # --- config ---

    def test_the_config_text_is_read(self):
        self.assertEqual(self.report["instances"], ["band", "master"])
        self.assertEqual(
            self.layout["config"],
            {
                "unfold": [
                    {"instance": "band", "name": "Vocals Repro grp#"},
                    {"instance": "band", "name": "Old grp#"},
                ]
            },
        )

    def test_the_config_text_becomes_text_controls_in_one_group(self):
        conf = self.pages["Conf"]
        self.assertEqual(row_ids(conf), [["conf-1"]])
        texts = self.groups["conf-1"]["controls"]
        self.assertEqual({c["kind"] for c in texts}, {"text"})
        self.assertEqual(
            [c["text"] for c in texts], build_fixtures.CONFIG_TEXT.rstrip("\n").split("\n")
        )

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
        self.assertEqual(cue["color"], "#75CC26")
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
        self.assertEqual(
            fader,
            {
                "kind": "param_fader",
                "label": "Podklady All",
                "targets": [
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
            },
        )

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
        # REPRO is a button outside any group, at (75,909) on the page: it
        # is the lowest rail control, named by the label drawn over it (a
        # sibling node).
        repro = [c for c in self.foh["rail"] if c.get("label") == "REPRO"]
        self.assertEqual(len(repro), 1, [c.get("label") for c in self.foh["rail"]])
        self.assertIs(self.foh["rail"][-1], repro[0])
        self.assertEqual(repro[0]["press"], "toggle")
        self.assertEqual(
            self.verdict("CC31 ch14"),
            {"control": "REPRO", "message": "CC31 ch14", "verdict": "clean", "targets": 1},
        )
        # The label is the control's, not a dropped free label.
        self.assertFalse([d for d in self.report["dropped"] if "label942" in d["node"]])
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
            [c for c in all_controls(self.layout) if c.get("text") in ("REPRO", "- 0.0", "All")]
        )

    def test_unmapped_partial_and_unsupported_controls_are_dropped(self):
        self.assertEqual(self.verdict("CC28 ch14")["why"], "no mapping in the set")
        self.assertEqual(self.verdict("NOTE29 ch14")["why"], "no mapping in the set")
        self.assertIn("partial range", self.verdict("CC58 ch14")["why"])
        self.assertIn("not reproduced", self.verdict("CC67 ch14")["why"])
        for text in ("Gitara 2", "ALERT LOOP", "HALF", "SELECT"):
            self.assertFalse([c for c in all_controls(self.layout) if c.get("label") == text], text)
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
        self.assertFalse([c for c in all_controls(self.layout) if c.get("text") == "gone"])

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
        self.assertIn("6 controls in guessed groups", result.stdout)
        with open(out, encoding="utf-8") as f:
            self.assertEqual(json.load(f), self.layout)
        with open(report, encoding="utf-8") as f:
            text = f.read()
        self.assertIn("# TouchOSC import report", text)
        self.assertIn("CC28 ch14", text)
        self.assertIn("no mapping in the set", text)
        self.assertIn("unfold_band 'Old grp#'", text)
        self.assertIn(
            "- foh rail: STAGE, STAGE AUT, Vocals, Stems, REVERB, VOC MIC, AUTOTUNE, ZVUKAR, "
            "REPRO\n",
            text,
        )
        self.assertIn(
            "- foh row 1: foh-pager (pager): STAGE, OTHERS | foh-2: B-Main repro #, Hand2 #\n",
            text,
        )
        self.assertIn("- global (every page): TechAlert, REFRESH ALL\n", text)
        self.assertIn("## Guessed groups (their controls are in no area)\n\n- cue/cue-1:", text)

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
        # The rail keeps what needs no set: the stage mics, STAGE AUT, solos.
        self.assertEqual(
            [c["kind"] for c in layout["pages"][1]["rail"]],
            ["stage", "hub_toggle", "solo", "solo"],
        )


class GeometryTest(unittest.TestCase):
    """The grouping's geometry functions on hand-made frames."""

    def test_innermost_takes_the_smallest_area_holding_the_centre(self):
        page = item(0, 0, 1000, 1000, z=1)
        box = item(100, 100, 400, 400, z=2)
        self.assertIs(import_tosc.innermost((150, 150, 100, 100), [page, box]), box)
        # The frame overlaps the box, its centre (550, 300) is outside it.
        self.assertIs(import_tosc.innermost((450, 250, 200, 100), [page, box]), page)
        self.assertIsNone(import_tosc.innermost((1100, 0, 10, 10), [page, box]))
        # Of two equal areas, the one drawn last.
        later = item(100, 100, 400, 400, z=3)
        self.assertIs(import_tosc.innermost((150, 150, 100, 100), [page, box, later]), later)

    def test_a_title_touches_its_box_within_eight_px(self):
        box = item(100, 100, 400, 400, z=1)
        beside = item(52, 100, 40, 400)  # 8 px left of the box
        above = item(100, 50, 400, 42)  # 8 px above
        overlapping = item(80, 100, 40, 400)
        for title in (beside, above, overlapping):
            self.assertIsNotNone(import_tosc.title_key(title, box), title)
        self.assertIsNone(import_tosc.title_key(item(51, 100, 40, 400), box), "9 px left")
        self.assertIsNone(import_tosc.title_key(item(100, 49, 400, 42), box), "9 px above")
        # A corner that only touches diagonally names nothing.
        self.assertIsNone(import_tosc.title_key(item(60, 60, 38, 38), box))

    def test_a_title_names_the_box_on_its_edge_before_a_box_around_it(self):
        # A vertical title beside its box, both on a page-sized panel: the
        # box beside it wins, the panel keeps no title.
        panel = item(0, 0, 2000, 800, z=1)
        box = item(100, 0, 600, 800, z=2)
        title = item(60, 0, 40, 800, z=3)
        other = item(800, 0, 600, 800, z=4)
        named = import_tosc.titles_of([panel, box, other], [title])
        self.assertEqual(named, {id(box): title})
        # Of two titles on one box, the overlapping one before the touching one.
        touching = item(100, -40, 600, 36, z=5)
        overlapping = item(90, 0, 40, 800, z=6)
        named = import_tosc.titles_of([box], [touching, overlapping])
        self.assertEqual(named, {id(box): overlapping})

    def test_rows_cluster_by_vertical_overlap(self):
        a = item(500, 0, 100, 400, name="a")
        b = item(0, 50, 100, 300, name="b")
        # Overlaps the first row by 20 px only (less than half its height).
        c = item(200, 380, 100, 300, name="c")
        d = item(0, 420, 100, 300, name="d")
        found = import_tosc.rows([a, c, d, b])
        self.assertEqual([[s["name"] for s in row] for row in found], [["b", "a"], ["d", "c"]])

    def test_runs_break_on_a_wide_gap_or_another_row(self):
        # Median width 100: a gap of 150 joins, 151 breaks.
        strips = [
            item(0, 0, 100, 500, name="a"),
            item(250, 0, 100, 500, name="b"),
            item(501, 0, 100, 500, name="c"),
            # Below the first run, interleaved in x: its own run.
            item(120, 600, 100, 500, name="d"),
            item(230, 600, 100, 500, name="e"),
        ]
        found = import_tosc.runs(strips)
        self.assertEqual(
            [[s["name"] for s in run] for run in found], [["a", "b"], ["d", "e"], ["c"]]
        )
        self.assertEqual(import_tosc.runs([]), [])

    def test_wide_is_from_one_point_two_times_the_median(self):
        self.assertEqual(
            import_tosc.wide_flags([100, 100, 100, 120, 119.9]), [False, False, False, True, False]
        )
        self.assertEqual(import_tosc.wide_flags([160]), [False])
        self.assertEqual(import_tosc.wide_flags([]), [])

    def test_rgb_drops_the_alpha_and_a_transparent_fill(self):
        self.assertEqual(import_tosc.rgb("#BAFFA657"), "#BAFFA6")
        self.assertIsNone(import_tosc.rgb("#00000000"))
        self.assertIsNone(import_tosc.rgb(None))


if __name__ == "__main__":
    unittest.main()
