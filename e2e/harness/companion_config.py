"""The synthetic Companion 5.0.7 configuration of the real-Companion CI job
(#52): ``e2e/companion/test.companionconfig``, Companion's own export format
(``version`` 12, ``button-layered`` buttons, ``custom_variable_set_value``
actions), written by this file so the committed export is reviewable and
reproducible:

    python3 e2e/harness/companion_config.py > e2e/companion/test.companionconfig

Three keys on page 1, invented names only, internal actions only (no
connection, no module):

- key 0, **Light A**: a press toggles the custom variable ``light_a``
  (``off``/``on``); its text shows it and its background turns green while
  ``on``.
- key 1, **Scene 1**: a release after less than 1 s sets ``scene_1`` to
  ``short``; a release after a hold of 1 s or more runs the 1000 ms duration
  group instead and sets ``long``.
- key 2, **Hold C**: down sets ``hold_c`` to ``down``, up to ``up``; its
  background is orange while held.

The tests read Companion's state through its HTTP API
(``GET /api/custom-variable/<name>/value``), never through the hub.
``test_companion_config.py`` keeps the committed file equal to ``text()``.
"""

import json

# The build the file says it comes from (Companion writes it into exports).
BUILD = "5.0.7+9763-stable-cec2f88e6f"
# Colours as Companion stores them (0xRRGGBB): black, green, orange, white.
BLACK = 0
GREEN = 0x00AA00
ORANGE = 0xFFA500
WHITE = 0xFFFFFF


def prop(value, expression=False):
    """A layer property as Companion 5 stores it."""
    return {"value": value, "isExpression": expression}


def layers(text, colour, colour_is_expression=False):
    """A button's style: the canvas, a background box, a centred text."""
    return [
        {
            "id": "canvas",
            "name": "Canvas",
            "usage": "auto",
            "type": "canvas",
            "decoration": prop("default"),
            "showStatusIcons": prop("default"),
        },
        {
            "id": "box0",
            "name": "Background",
            "usage": "auto",
            "type": "box",
            "enabled": prop(True),
            "opacity": prop(100),
            "x": prop(0),
            "y": prop(0),
            "width": prop(100),
            "height": prop(100),
            "rotation": prop(0),
            "color": prop(colour, colour_is_expression),
            "borderWidth": prop(0),
            "borderColor": prop(0),
            "borderPosition": prop("inside"),
        },
        {
            "id": "text0",
            "name": "Text",
            "usage": "auto",
            "type": "text",
            "enabled": prop(True),
            "opacity": prop(100),
            "x": prop(0),
            "y": prop(0),
            "width": prop(100),
            "height": prop(100),
            "rotation": prop(0),
            "text": prop(text),
            "color": prop(WHITE),
            "halign": prop("center"),
            "valign": prop("center"),
            "fontsize": prop(100),
            "fontsizeAllowShrink": prop(True),
            "font": prop("companion-sans"),
            "outlineColor": prop(4278190080),
        },
    ]


def set_variable(action_id, name, value, expression=False):
    """Companion's internal action "Custom Variable: Set value"."""
    return {
        "id": action_id,
        "definitionId": "custom_variable_set_value",
        "connectionId": "internal",
        "options": {
            "name": prop(name),
            "create": prop(False),
            "value": prop(value, expression),
        },
        "type": "action",
        "children": {},
    }


def button(text, colour, colour_is_expression, action_sets):
    """A regular button of one step."""
    return {
        "type": "button-layered",
        "style": {"layers": layers(text, colour, colour_is_expression)},
        "options": {
            "stepProgression": "auto",
            "stepExpression": "",
            "rotaryActions": False,
            "canModifyStyleInApis": False,
            "notes": "",
        },
        "feedbacks": [],
        "steps": {"0": {"action_sets": action_sets, "options": {"runWhileHeld": []}}},
        "localVariables": [],
    }


def variable(description, default, order):
    """A custom variable's definition."""
    return {
        "description": description,
        "defaultValue": default,
        "persistCurrentValue": False,
        "sortOrder": order,
    }


def config():
    """The whole export."""
    light_a = button(
        "Light A\n$(custom:light_a)",
        f"$(custom:light_a) == 'on' ? {GREEN} : {BLACK}",
        True,
        {
            "down": [
                set_variable(
                    "lightA0toggle0000001",
                    "light_a",
                    "$(this:current) == 'on' ? 'off' : 'on'",
                    expression=True,
                )
            ],
            "up": [],
        },
    )
    scene_1 = button(
        "Scene 1\n$(custom:scene_1)",
        BLACK,
        False,
        {
            "down": [],
            "up": [set_variable("scene1short000000001", "scene_1", "short")],
            "1000": [set_variable("scene1long0000000001", "scene_1", "long")],
        },
    )
    hold_c = button(
        "Hold C\n$(custom:hold_c)",
        f"$(custom:hold_c) == 'down' ? {ORANGE} : {BLACK}",
        True,
        {
            "down": [set_variable("holdC0down0000000001", "hold_c", "down")],
            "up": [set_variable("holdC0up000000000001", "hold_c", "up")],
        },
    )
    return {
        "version": 12,
        "type": "full",
        "companionBuild": BUILD,
        "pages": {
            "1": {
                "id": "fohmixerTestPage0001",
                "name": "TEST",
                "controls": {"0": {"0": light_a, "1": scene_1, "2": hold_c}},
                "gridSize": {"minColumn": 0, "maxColumn": 7, "minRow": 0, "maxRow": 3},
            }
        },
        "triggers": {},
        "triggerCollections": [],
        "custom_variables": {
            "light_a": variable("Light A (synthetic test)", "off", 0),
            "scene_1": variable("Scene 1's last release (synthetic test)", "idle", 1),
            "hold_c": variable("Hold C (synthetic test)", "up", 2),
        },
        "customVariablesCollections": [],
        "expressionVariables": {},
        "expressionVariablesCollections": [],
        "instances": {},
        "connectionCollections": [],
        "surfaces": {},
        "surfaceGroups": {},
        "surfacesRemote": {},
        "surfaceInstances": {},
        "surfaceInstanceCollections": [],
        "imageLibrary": [],
        "imageLibraryCollections": [],
    }


def text():
    """The file's text: the export as JSON, one space of indent, a final newline."""
    return json.dumps(config(), indent=1) + "\n"


if __name__ == "__main__":
    print(text(), end="")
