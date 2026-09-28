"""Builds a SimLive Song from a JSON site fixture.

(Named ``site_builder``, not ``site``: Python pre-imports its stdlib ``site``
module, which would shadow a ``sim/site.py``.)

Fixture shape::

    {"name": str, "live_version": [12, 2, 5],
     "tracks": [TRACK...], "returns": [TRACK...], "master": TRACK}

    TRACK  = {"name", "color"?, "mute"?, "solo"?, "volume"?, "panning"?,
              "devices"?: [DEVICE...], "children"?: [TRACK...] (makes a group),
              "fold_state"?: 0|1 (groups)}
    DEVICE = {"template": "plain"|"eq8"|"rack", "name", "class_name"?,
              "parameters"?: [PARAM...], "chains"?: [{"name", "devices"?}]}
    PARAM  = {"name", "value"?, "min"?, "max"?, "display"?, "items"?}

``display`` is one of volume, pan, items, db, hz, percent, int, plain.
"""

import json

import Live

EQ8_FILTER_TYPES = (
    "Low Cut 48",
    "Low Cut 12",
    "Low Shelf",
    "Bell",
    "Notch",
    "High Shelf",
    "High Cut 12",
    "High Cut 48",
)
EQ8_BAND_FREQUENCIES = (30.0, 100.0, 250.0, 800.0, 2000.0, 5000.0, 10000.0, 18000.0)


def build(path):
    """Load the fixture at ``path`` and return the Song (also the app's document)."""
    with open(path, encoding="utf-8") as f:
        spec = json.load(f)
    return build_from_spec(spec)


def build_from_spec(spec):
    song = Live.Song.Song(spec.get("name", ""))
    return_names = [r["name"] for r in spec.get("returns", ())]
    for track_spec in spec.get("tracks", ()):
        _add_track(song, track_spec, None, return_names)
    song._returns = [_make_track(song, r, None, return_names) for r in spec.get("returns", ())]
    song._master = _make_track(song, spec.get("master", {"name": "Main"}), None, ())
    Live.Application.get_application()._sim_configure(song, spec.get("live_version", (12, 2, 5)))
    return song


def _add_track(song, spec, group, return_names):
    track = _make_track(song, spec, group, return_names)
    song._tracks.append(track)
    for child in spec.get("children", ()):
        _add_track(song, child, track, return_names)


def _make_track(song, spec, group, return_names):
    sends = [(name, 0.0) for name in return_names]
    mixer = Live.MixerDevice.MixerDevice(
        volume=spec.get("volume", 0.85), panning=spec.get("panning", 0.0), sends=sends
    )
    track = Live.Track.Track(
        song,
        spec["name"],
        foldable="children" in spec,
        group=group,
        mixer=mixer,
        color=spec.get("color", 0x5480E4),
    )
    track._v["mute"] = bool(spec.get("mute", False))
    track._v["solo"] = bool(spec.get("solo", False))
    track._v["fold_state"] = int(spec.get("fold_state", 0))
    track._devices = [_make_device(d) for d in spec.get("devices", ())]
    return track


def _make_param(spec):
    return Live.DeviceParameter.DeviceParameter(
        spec["name"],
        value=spec.get("value", 0.0),
        lo=spec.get("min", 0.0),
        hi=spec.get("max", 1.0),
        display=spec.get("display", "plain"),
        items=spec.get("items", ()),
    )


def _make_device(spec):
    template = spec.get("template", "plain")
    params = [_make_param(p) for p in spec.get("parameters", ())]
    if template == "eq8":
        params = _eq8_parameters() + params
        return Live.Device.Device(
            spec["name"], spec.get("class_name", "Eq8"), params, display_name="EQ Eight"
        )
    if template == "rack":
        chains = [
            Live.Chain.Chain(c["name"], [_make_device(d) for d in c.get("devices", ())])
            for c in spec.get("chains", ())
        ]
        return Live.RackDevice.RackDevice(
            spec["name"], spec.get("class_name", "AudioEffectGroupDevice"), chains, params
        )
    if template == "plain":
        return Live.Device.Device(spec["name"], spec.get("class_name", spec["name"]), params)
    raise ValueError(f"unknown device template {template!r}")


def _eq8_parameters():
    params = []
    for band, freq in enumerate(EQ8_BAND_FREQUENCIES, start=1):
        on = band <= 4
        params += [
            _make_param(
                {
                    "name": f"{band} Filter On A",
                    "value": 1.0 if on else 0.0,
                    "display": "items",
                    "items": ["Off", "On"],
                }
            ),
            _make_param(
                {
                    "name": f"{band} Filter Type A",
                    "value": 3.0,
                    "display": "items",
                    "items": list(EQ8_FILTER_TYPES),
                }
            ),
            _make_param(
                {
                    "name": f"{band} Frequency A",
                    "value": freq,
                    "min": 10.0,
                    "max": 22000.0,
                    "display": "hz",
                }
            ),
            _make_param(
                {"name": f"{band} Gain A", "value": 0.0, "min": -15.0, "max": 15.0, "display": "db"}
            ),
            _make_param({"name": f"{band} Resonance A", "value": 0.71, "min": 0.1, "max": 18.0}),
        ]
    params += [
        _make_param(
            {"name": "Scale", "value": 100.0, "min": -200.0, "max": 200.0, "display": "percent"}
        ),
        _make_param(
            {"name": "Output Gain", "value": 0.0, "min": -12.0, "max": 12.0, "display": "db"}
        ),
    ]
    return params
