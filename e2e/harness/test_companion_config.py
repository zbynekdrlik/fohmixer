"""The synthetic Companion export of the ``companion`` CI job (#52): the
committed ``e2e/companion/test.companionconfig`` is exactly what
``companion_config.py`` writes, and it holds only the three synthetic keys
and their custom variables (invented names, internal actions only)."""

import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import companion_config  # noqa: E402

E2E = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXPORT = os.path.join(E2E, "companion", "test.companionconfig")


class CompanionConfig(unittest.TestCase):
    def test_the_committed_export_is_the_builders(self):
        with open(EXPORT, encoding="utf-8") as f:
            self.assertEqual(
                f.read(),
                companion_config.text(),
                "run: python3 e2e/harness/companion_config.py > e2e/companion/test.companionconfig",
            )

    def test_it_holds_the_three_synthetic_keys_and_nothing_of_a_site(self):
        config = json.loads(companion_config.text())
        self.assertEqual((config["version"], config["type"]), (12, "full"))
        controls = config["pages"]["1"]["controls"]
        self.assertEqual(list(controls), ["0"])
        self.assertEqual(list(controls["0"]), ["0", "1", "2"])
        texts = [c["style"]["layers"][2]["text"]["value"] for c in controls["0"].values()]
        self.assertEqual(
            texts,
            [
                "Light A\n$(custom:light_a)",
                "Scene 1\n$(custom:scene_1)",
                "Hold C\n$(custom:hold_c)",
            ],
        )
        self.assertEqual(sorted(config["custom_variables"]), ["hold_c", "light_a", "scene_1"])
        for empty in (
            "instances",
            "surfaces",
            "surfaceInstances",
            "surfacesRemote",
            "triggers",
            "imageLibrary",
        ):
            self.assertFalse(config[empty], empty)
        actions = [
            action
            for control in controls["0"].values()
            for group in control["steps"]["0"]["action_sets"].values()
            for action in group
        ]
        self.assertEqual(len(actions), 5)
        self.assertEqual(
            {(a["connectionId"], a["definitionId"]) for a in actions},
            {("internal", "custom_variable_set_value")},
        )
        scene = controls["0"]["1"]["steps"]["0"]
        self.assertEqual(list(scene["action_sets"]), ["down", "up", "1000"])
        self.assertEqual(scene["options"]["runWhileHeld"], [], "the 1 s group runs on the release")


if __name__ == "__main__":
    unittest.main()
