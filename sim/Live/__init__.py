# ruff: noqa: N999 - Live's own module name
"""SimLive: a fake of Ableton Live's embedded ``Live`` Python module.

It models the part of the Live Object Model (LOM) that FohMixer and its tests
use, shaped like the real module:

- classes live in submodules (``Live.Track.Track``, ``Live.Base.Timer``, ...);
- lists are ``Live.Base`` vectors, enums are Boost-like ``int`` subclasses
  with ``names`` / ``values`` dicts;
- every observable property has ``add_<p>_listener``, ``remove_<p>_listener``
  and ``<p>_has_listener``; listeners run synchronously, and a change made
  while a notification runs raises ``RuntimeError`` (Live's rule);
- a deleted object compares equal to ``None`` and raises on access.

Names starting with ``_sim`` are test helpers; they are not part of the LOM.
``Live.Base.Timer`` runs on the ``main_thread.MainThread`` installed as
``_sim_main_thread``; with none installed a timer is inert.
"""

import itertools
import math
import sys
import types

_ptr_seq = itertools.count(0x7F3A00001000, 0x40)
_notify_depth = 0
_sim_main_thread = None

NOTIFICATION_ERROR = (
    "Changes cannot be triggered by notifications. You will need to defer your response."
)
_SIGNATURE_ERROR = "Python argument types did not match C++ signature"


def _submodule(name):
    module = types.ModuleType(f"Live.{name}")
    sys.modules[module.__name__] = module
    return module


Base = _submodule("Base")
Application = _submodule("Application")
Song = _submodule("Song")
Track = _submodule("Track")
MixerDevice = _submodule("MixerDevice")
DeviceParameter = _submodule("DeviceParameter")
Device = _submodule("Device")
RackDevice = _submodule("RackDevice")
Chain = _submodule("Chain")
ChainMixerDevice = _submodule("ChainMixerDevice")


def _export(module, cls, name):
    cls.__name__ = name
    cls.__qualname__ = name
    cls.__module__ = module.__name__
    setattr(module, name, cls)
    return cls


def _guard():
    if _notify_depth:
        raise RuntimeError(NOTIFICATION_ERROR)


# --- enums and vectors -----------------------------------------------------------------


def _enum(module, qualname, members):
    """A Boost.Python-like enum: an ``int`` subclass with ``names`` and ``values``."""
    cls = type(qualname.rsplit(".", 1)[-1], (int,), {"names": {}, "values": {}})
    cls.__module__ = module.__name__
    cls.__qualname__ = qualname
    for name, value in members.items():
        member = int.__new__(cls, value)
        member.name = name
        cls.names[name] = member
        cls.values[value] = member
        setattr(cls, name, member)
    cls.__repr__ = lambda self: f"{module.__name__}.{qualname}.{self.name}"
    cls.__str__ = lambda self: str(int(self))
    return cls


class _Vector:
    """A read-only LOM list (``Live.Base.Vector``): indexable, iterable, sized."""

    __slots__ = ("_items",)

    def __init__(self, items=()):
        self._items = tuple(items)

    def __len__(self):
        return len(self._items)

    def __getitem__(self, index):
        return self._items[index]

    def __iter__(self):
        return iter(self._items)

    def __contains__(self, item):
        return item in self._items

    def __eq__(self, other):
        if isinstance(other, _Vector):
            return self._items == other._items
        if isinstance(other, (tuple, list)):
            return self._items == tuple(other)
        return NotImplemented

    __hash__ = None

    def __repr__(self):
        return f"{type(self).__name__}({list(self._items)!r})"


class _StringVector(_Vector):
    __slots__ = ()


_export(Base, _Vector, "Vector")
_export(Base, _StringVector, "StringVector")
# Live's vector classes report the module "Base", not "Live.Base": repr of
# song.tracks on the PC is "<Base.Vector object at ...>" (#58).
_Vector.__module__ = "Base"
_StringVector.__module__ = "Base"


# --- timer -------------------------------------------------------------------------------


class _Timer:
    """``Live.Base.Timer``: calls ``callback`` every ``interval`` ms on Live's main thread."""

    def __init__(self, callback=None, interval=1, repeat=False, start=False):
        self.callback = callback
        self.interval = interval
        self.repeat = repeat
        self.running = False
        if start:
            self.start()

    def start(self):
        self.running = True
        if _sim_main_thread is not None:
            _sim_main_thread._add_timer(self)

    def stop(self):
        self.running = False
        if _sim_main_thread is not None:
            _sim_main_thread._remove_timer(self)


_export(Base, _Timer, "Timer")


# --- object and property machinery ---------------------------------------------------


class _Prop:
    """A LOM property backed by ``obj._v[name]``; observable ones get the listener API."""

    def __init__(self, *, readonly=False, observable=True, coerce=None):
        self.readonly = readonly
        self.observable = observable
        self.coerce = coerce
        self.name = None

    def __set_name__(self, owner, name):
        self.name = name

    def __get__(self, obj, owner=None):
        if obj is None:
            return self
        return obj._v[self.name]

    def __set__(self, obj, value):
        if self.readonly:
            raise AttributeError(f"can't set attribute '{self.name}'")
        _guard()
        if self.coerce is not None:
            value = self.coerce(obj, value)
        obj._sim_set(self.name, value)


def _install_listener_api(cls, prop):
    def add(self, listener):
        listeners = self._listeners.setdefault(prop, [])
        if listener in listeners:
            raise RuntimeError("Listener already connected")
        listeners.append(listener)

    def remove(self, listener):
        listeners = self._listeners.get(prop, [])
        if listener not in listeners:
            raise RuntimeError("Listener not connected")
        listeners.remove(listener)

    def has(self, listener):
        return listener in self._listeners.get(prop, ())

    add.__name__ = f"add_{prop}_listener"
    remove.__name__ = f"remove_{prop}_listener"
    has.__name__ = f"{prop}_has_listener"
    setattr(cls, add.__name__, add)
    setattr(cls, remove.__name__, remove)
    setattr(cls, has.__name__, has)


class _LiveObject:
    """Base of every LOM object: pointer, values, listeners, deletion."""

    _listened = ()

    def __init_subclass__(cls, **kwargs):
        super().__init_subclass__(**kwargs)
        for name, attr in list(cls.__dict__.items()):
            if isinstance(attr, _Prop) and attr.observable:
                _install_listener_api(cls, name)
        for name in cls.__dict__.get("_listened", ()):
            _install_listener_api(cls, name)

    def __init__(self):
        object.__setattr__(self, "_deleted", False)
        object.__setattr__(self, "_live_ptr", next(_ptr_seq))
        self._v = {}
        self._listeners = {}

    def __getattribute__(self, name):
        if name[:1] != "_" and object.__getattribute__(self, "_deleted"):
            raise RuntimeError(f"{type(self).__name__} has been deleted")
        return object.__getattribute__(self, name)

    def __setattr__(self, name, value):
        if name[:1] != "_" and not hasattr(getattr(type(self), name, None), "__set__"):
            raise AttributeError(f"'{type(self).__name__}' object has no attribute '{name}'")
        object.__setattr__(self, name, value)

    def __eq__(self, other):
        if other is None:
            return self._deleted
        return self is other

    __hash__ = object.__hash__

    def __repr__(self):
        return f"<{type(self).__module__}.{type(self).__name__} {self._live_ptr:#x}>"

    # --- sim helpers (not LOM) ---

    def _sim_set(self, name, value):
        """Set a value as Live itself would (also read-only ones) and notify."""
        if name in self._v and self._v[name] == value:
            return
        self._v[name] = value
        self._fire(name)

    def _fire(self, prop):
        global _notify_depth
        listeners = list(self._listeners.get(prop, ()))
        if not listeners:
            return
        _notify_depth += 1
        try:
            for listener in listeners:
                listener()
        finally:
            _notify_depth -= 1

    def _sim_listener_count(self, prop):
        return len(self._listeners.get(prop, ()))

    def _sim_children(self):
        return []

    def _sim_delete(self):
        for child in self._sim_children():
            child._sim_delete()
        self._listeners.clear()
        object.__setattr__(self, "_deleted", True)


def _bool(obj, value):
    if isinstance(value, bool):
        return value
    if isinstance(value, (int, float)):
        return bool(value)
    raise TypeError(_SIGNATURE_ERROR)


def _int(obj, value):
    if isinstance(value, int) and not isinstance(value, bool):
        return value
    raise TypeError(_SIGNATURE_ERROR)


def _str(obj, value):
    if isinstance(value, str):
        return value
    raise TypeError(_SIGNATURE_ERROR)


def _float(value):
    if isinstance(value, (int, float)):
        return float(value)
    raise TypeError(_SIGNATURE_ERROR)


# --- DeviceParameter -------------------------------------------------------------------

_AutomationState = _enum(
    DeviceParameter, "AutomationState", {"none": 0, "playing": 1, "overridden": 2}
)
_ParameterState = _enum(
    DeviceParameter, "ParameterState", {"enabled": 0, "irrelevant": 1, "disabled": 2}
)
DeviceParameter.AutomationState = _AutomationState
DeviceParameter.ParameterState = _ParameterState


def volume_to_db(value):
    """SimLive's volume law (the TouchOSC ``value2db`` curve); -70 dB or less is -inf."""
    if value >= 0.4:
        return 40.0 * value - 34.0
    if value >= 0.15:
        return -((399.751894 * value - 201.871345) ** 2 + 12630.61132) / 799.503788
    return 118.426374 * value ** (5567 / 7504) - 70.0


def _live_number(value):
    """A number as Live writes a dB value: 3 significant digits, at most 3
    decimals, and 0 as ``0.00`` ("-0.811", "-6.00", "-14.0", "12.3").

    From the one string captured on the real Live (#9: "-0.811 dB") and
    Live's three-significant-digit display elsewhere; to be checked against
    more captured strings (S2 design note §5.3).
    """
    for decimals in (3, 2, 1, 0):
        text = f"{value:.{decimals}f}"
        digits = len(text.lstrip("-").replace(".", "").lstrip("0"))
        if digits <= 3 or decimals == 0:
            break
    if float(text) == 0.0:
        return "0.00"
    return text


def _db_text(db):
    if db <= -70.0 or math.isinf(db):
        return "-inf dB"
    return f"{_live_number(db)} dB"


def _format(kind, value, items):
    if kind == "volume":
        return _db_text(volume_to_db(value))
    if kind == "pan":
        amount = round(abs(value) * 50)
        if amount == 0:
            return "C"
        return f"{amount}{'L' if value < 0 else 'R'}"
    if kind == "items":
        index = round(value)
        return items[index] if 0 <= index < len(items) else str(index)
    if kind == "db":
        return f"{_live_number(value)} dB"
    if kind == "hz":
        if value >= 1000.0:
            return f"{value / 1000.0:.2f} kHz"
        return f"{value:.0f} Hz"
    if kind == "percent":
        return f"{value:.0f} %"
    if kind == "int":
        return str(round(value))
    return f"{value:.2f}"


def _param_value(param, value):
    value = _float(value)
    if not param._v["min"] <= value <= param._v["max"]:
        raise RuntimeError("Invalid value")
    return value


class _DeviceParameterClass(_LiveObject):
    name = _Prop(readonly=True)
    original_name = _Prop(readonly=True, observable=False)
    value = _Prop(coerce=_param_value)
    min = _Prop(readonly=True, observable=False)
    max = _Prop(readonly=True, observable=False)
    is_quantized = _Prop(readonly=True, observable=False)
    default_value = _Prop(readonly=True, observable=False)
    is_enabled = _Prop(readonly=True, observable=False)
    automation_state = _Prop(readonly=True)
    state = _Prop(readonly=True)

    def __init__(self, name, value=0.0, lo=0.0, hi=1.0, display="plain", items=()):
        super().__init__()
        self._display = display
        self._items = tuple(items)
        quantized = display == "items"
        if quantized:
            lo, hi = 0.0, float(len(self._items) - 1)
        self._v.update(
            name=name,
            original_name=name,
            value=float(value),
            min=float(lo),
            max=float(hi),
            is_quantized=quantized,
            default_value=float(value),
            is_enabled=True,
            automation_state=_AutomationState.none,
            state=_ParameterState.enabled,
        )

    @property
    def value_items(self):
        return _StringVector(self._items)

    def __str__(self):
        if self._deleted:
            raise RuntimeError("DeviceParameter has been deleted")
        return _format(self._display, self._v["value"], self._items)

    def str_for_value(self, value):
        return _format(self._display, _float(value), self._items)

    def re_enable_automation(self):
        _guard()
        if self._v["automation_state"] == _AutomationState.overridden:
            self._sim_set("automation_state", _AutomationState.playing)


_export(DeviceParameter, _DeviceParameterClass, "DeviceParameter")


def _on_off(name, on=True):
    return _DeviceParameterClass(
        name, value=1.0 if on else 0.0, display="items", items=("Off", "On")
    )


# --- mixers ----------------------------------------------------------------------------


class _MixerDeviceClass(_LiveObject):
    volume = _Prop(readonly=True, observable=False)
    panning = _Prop(readonly=True, observable=False)
    track_activator = _Prop(readonly=True, observable=False)
    _listened = ("sends",)

    def __init__(self, volume=0.85, panning=0.0, sends=()):
        super().__init__()
        self._v.update(
            volume=_DeviceParameterClass("Track Volume", volume, 0.0, 1.0, "volume"),
            panning=_DeviceParameterClass("Track Panning", panning, -1.0, 1.0, "pan"),
            track_activator=_on_off("Speaker On"),
        )
        self._sends = [
            _DeviceParameterClass(name, value, 0.0, 1.0, "volume") for name, value in sends
        ]

    @property
    def sends(self):
        return _Vector(self._sends)

    def _sim_children(self):
        return [self._v["volume"], self._v["panning"], self._v["track_activator"], *self._sends]


_export(MixerDevice, _MixerDeviceClass, "MixerDevice")


class _ChainMixerDeviceClass(_LiveObject):
    volume = _Prop(readonly=True, observable=False)
    panning = _Prop(readonly=True, observable=False)
    chain_activator = _Prop(readonly=True, observable=False)
    _listened = ("sends",)

    def __init__(self, volume=0.85, panning=0.0):
        super().__init__()
        self._v.update(
            volume=_DeviceParameterClass("Chain Volume", volume, 0.0, 1.0, "volume"),
            panning=_DeviceParameterClass("Chain Pan", panning, -1.0, 1.0, "pan"),
            chain_activator=_on_off("Chain Activator"),
        )

    @property
    def sends(self):
        return _Vector(())

    def _sim_children(self):
        return [self._v["volume"], self._v["panning"], self._v["chain_activator"]]


_export(ChainMixerDevice, _ChainMixerDeviceClass, "ChainMixerDevice")


# --- devices and chains ----------------------------------------------------------------

_DeviceType = _enum(
    Device, "DeviceType", {"undefined": 0, "instrument": 1, "audio_effect": 2, "midi_effect": 4}
)
Device.DeviceType = _DeviceType


class _DeviceClass(_LiveObject):
    name = _Prop(coerce=_str)
    class_name = _Prop(readonly=True, observable=False)
    class_display_name = _Prop(readonly=True, observable=False)
    type = _Prop(readonly=True, observable=False)
    is_active = _Prop(readonly=True)
    can_have_chains = _Prop(readonly=True, observable=False)
    can_have_drum_pads = _Prop(readonly=True, observable=False)
    _listened = ("parameters",)

    def __init__(self, name, class_name, parameters=(), display_name=None):
        super().__init__()
        self._v.update(
            name=name,
            class_name=class_name,
            class_display_name=display_name or class_name,
            type=_DeviceType.audio_effect,
            is_active=True,
            can_have_chains=False,
            can_have_drum_pads=False,
        )
        self._parameters = [_on_off("Device On"), *parameters]

    @property
    def parameters(self):
        return _Vector(self._parameters)

    def _sim_children(self):
        return list(self._parameters)


_export(Device, _DeviceClass, "Device")


class _ChainClass(_LiveObject):
    name = _Prop(coerce=_str)
    color = _Prop(coerce=_int)
    mute = _Prop(coerce=_bool)
    solo = _Prop(coerce=_bool)
    mixer_device = _Prop(readonly=True, observable=False)
    _listened = ("devices",)

    def __init__(self, name, devices=()):
        super().__init__()
        self._v.update(
            name=name, color=0x8C8C8C, mute=False, solo=False, mixer_device=_ChainMixerDeviceClass()
        )
        self._devices = list(devices)

    @property
    def devices(self):
        return _Vector(self._devices)

    def delete_device(self, index):
        _guard()
        _delete_from(self, self._devices, index, "devices")

    def _sim_children(self):
        return [self._v["mixer_device"], *self._devices]


_export(Chain, _ChainClass, "Chain")


class _RackDeviceClass(_DeviceClass):
    _listened = ("chains", "return_chains")

    def __init__(self, name, class_name, chains=(), parameters=()):
        macros = [_DeviceParameterClass(f"Macro {i}", 0.0, 0.0, 127.0, "int") for i in range(1, 9)]
        selector = _DeviceParameterClass("Chain Selector", 0.0, 0.0, 127.0, "int")
        super().__init__(name, class_name, [*macros, selector, *parameters])
        self._v["can_have_chains"] = True
        self._chains = list(chains)

    @property
    def chains(self):
        return _Vector(self._chains)

    @property
    def return_chains(self):
        return _Vector(())

    def _sim_children(self):
        return [*self._parameters, *self._chains]


_export(RackDevice, _RackDeviceClass, "RackDevice")


def _delete_from(owner, items, index, prop):
    if not isinstance(index, int) or not 0 <= index < len(items):
        raise IndexError("Index out of range")
    victim = items.pop(index)
    victim._sim_delete()
    owner._fire(prop)


# --- tracks ----------------------------------------------------------------------------

_MonitoringStates = _enum(Track, "Track.monitoring_states", {"IN": 0, "AUTO": 1, "OFF": 2})


def _monitoring(track, value):
    valid = isinstance(value, int) and not isinstance(value, bool)
    if valid and int(value) in _MonitoringStates.values:
        return _MonitoringStates.values[int(value)]
    raise TypeError(_SIGNATURE_ERROR)


class _TrackClass(_LiveObject):
    name = _Prop(coerce=_str)
    color = _Prop(coerce=_int)
    mute = _Prop(coerce=_bool)
    solo = _Prop(coerce=_bool)
    current_monitoring_state = _Prop(coerce=_monitoring)
    output_meter_left = _Prop(readonly=True)
    output_meter_right = _Prop(readonly=True)
    output_meter_level = _Prop(readonly=True)
    is_foldable = _Prop(readonly=True, observable=False)
    is_grouped = _Prop(readonly=True, observable=False)
    group_track = _Prop(readonly=True, observable=False)
    mixer_device = _Prop(readonly=True, observable=False)
    _listened = ("devices", "fold_state")

    def __init__(self, song, name, *, foldable=False, group=None, mixer=None, color=0x5480E4):
        super().__init__()
        self._song = song
        self._devices = []
        self._v.update(
            name=name,
            color=color,
            mute=False,
            solo=False,
            fold_state=0,
            current_monitoring_state=_MonitoringStates.AUTO,
            output_meter_left=0.0,
            output_meter_right=0.0,
            output_meter_level=0.0,
            is_foldable=foldable,
            is_grouped=group is not None,
            group_track=group,
            mixer_device=mixer or _MixerDeviceClass(),
        )

    @property
    def fold_state(self):
        return self._v["fold_state"]

    @fold_state.setter
    def fold_state(self, value):
        _guard()
        if not self._v["is_foldable"]:
            raise RuntimeError("Track cannot be folded")
        value = int(_bool(self, value))
        if value != self._v["fold_state"]:
            self._v["fold_state"] = value
            self._fire("fold_state")
            if self._song is not None:
                self._song._fire("visible_tracks")

    @property
    def devices(self):
        return _Vector(self._devices)

    @property
    def is_visible(self):
        return self._sim_visible()

    def delete_device(self, index):
        _guard()
        _delete_from(self, self._devices, index, "devices")

    def _sim_visible(self):
        group = self._v["group_track"]
        while group is not None:
            if group._v["fold_state"]:
                return False
            group = group._v["group_track"]
        return True

    def _sim_set_meter(self, left, right=None):
        """Simulate Live's meters: set left/right (and level = max) and notify."""
        right = left if right is None else right
        self._sim_set("output_meter_left", float(left))
        self._sim_set("output_meter_right", float(right))
        self._sim_set("output_meter_level", float(max(left, right)))

    def _sim_children(self):
        return [self._v["mixer_device"], *self._devices]


_TrackClass.monitoring_states = _MonitoringStates
_export(Track, _TrackClass, "Track")


# --- song and application ---------------------------------------------------------------


def _tempo(song, value):
    value = _float(value)
    if not 20.0 <= value <= 999.0:
        raise RuntimeError("Invalid value")
    return value


class _SongClass(_LiveObject):
    is_playing = _Prop(coerce=_bool)
    tempo = _Prop(coerce=_tempo)
    name = _Prop(readonly=True, observable=False)
    _listened = ("tracks", "return_tracks", "visible_tracks")

    def __init__(self, name=""):
        super().__init__()
        self._v.update(is_playing=False, tempo=120.0, name=name)
        self._tracks = []
        self._returns = []
        self._master = None

    @property
    def tracks(self):
        return _Vector(self._tracks)

    @property
    def return_tracks(self):
        return _Vector(self._returns)

    @property
    def master_track(self):
        return self._master

    @property
    def visible_tracks(self):
        return _Vector([t for t in self._tracks if t._sim_visible()])

    def start_playing(self):
        self.is_playing = True

    def stop_playing(self):
        self.is_playing = False

    def continue_playing(self):
        self.is_playing = True

    def delete_track(self, index):
        _guard()
        if not isinstance(index, int) or not 0 <= index < len(self._tracks):
            raise IndexError("Index out of range")
        victim = self._tracks[index]
        doomed = [victim]
        for track in self._tracks:
            group = track._v["group_track"]
            while group is not None:
                if group is victim:
                    doomed.append(track)
                    break
                group = group._v["group_track"]
        self._tracks = [t for t in self._tracks if not any(t is d for d in doomed)]
        for track in doomed:
            track._sim_delete()
        self._fire("tracks")
        self._fire("visible_tracks")

    def _sim_children(self):
        return [*self._tracks, *self._returns, self._master]


_export(Song, _SongClass, "Song")


class _ApplicationClass(_LiveObject):
    def __init__(self):
        super().__init__()
        self._version = (12, 2, 5)
        self._document = None

    def get_major_version(self):
        return self._version[0]

    def get_minor_version(self):
        return self._version[1]

    def get_bugfix_version(self):
        return self._version[2]

    def get_document(self):
        return self._document

    def _sim_configure(self, document, version):
        self._document = document
        self._version = tuple(version)


_export(Application, _ApplicationClass, "Application")
_application = _ApplicationClass()


def _get_application():
    return _application


Application.get_application = _get_application
