"""One command: ``{"target", "name", "args"}`` -> result data (design §3.4).

``name`` is ``get_prop``, ``set_prop``, ``add_listener``, ``remove_listener``,
``describe``, ``safe_writes`` (#140 PR 3a: the hub's DRY writes for its loss,
``safe.py``), or any function of the target, called with positional (list) or
keyword (dict) ``args``. Names starting with ``_`` are rejected. A failure
raises a ``FohError``; an exception raised by Live is wrapped as
``OpError("live error")`` that keeps the original type in ``error_type``.
Runs on Live's main thread only.
"""

import math

from . import codec
from . import path as lom_path
from .errors import FohError, OpError
from .registry import id_of

# A rack's first macro: the chain selector's mapping (LOM parameter 1; 0 is
# ``Device On``), the one parameter a DRY write may name (#140 PR 3a).
RACK_MACRO = 1
# A DRY write's target: a ``$ref`` of this class, never a path.
SAFE_CLASS = "DeviceParameter"
SAFE_SHAPE = "a safe write is {target: {$ref, class: DeviceParameter}, value}"


class Context:
    """What a command may touch: the song, the app, ids, subscriptions, its
    connection, and the connections' DRY writes (``safe``, a ``SafeWrites``)."""

    __slots__ = ("app", "connection", "registry", "safe", "song", "subscriptions")

    def __init__(self, song, app, registry, subscriptions, connection, safe=None):
        self.song = song
        self.app = app
        self.registry = registry
        self.subscriptions = subscriptions
        self.connection = connection
        self.safe = safe


def execute(command, ctx):
    if not isinstance(command, dict):
        raise OpError("malformed command", "a command must be an object")
    name = command.get("name")
    if not isinstance(name, str) or not name:
        raise OpError("malformed command", "name must be a non-empty string")
    if name.startswith("_"):
        raise OpError("forbidden", name)
    if "target" not in command:
        raise OpError("malformed command", "target is missing")
    obj, where = lom_path.resolve(command["target"], ctx.song, ctx.app, ctx.registry)
    args = command.get("args")
    if args is None:
        args = {}
    handler = _HANDLERS.get(name)
    if handler is not None:
        if not isinstance(args, dict):
            raise OpError("bad args", f"{name} takes an object")
        return handler(obj, where, args, ctx)
    return _call(obj, where, name, args, ctx)


def _live(fn, *args, **kwargs):
    """Run a LOM access; wrap anything Live raises as ``OpError("live error")``."""
    try:
        return fn(*args, **kwargs)
    except FohError:
        raise
    except Exception as e:
        raise OpError("live error", str(e) or type(e).__name__, type(e).__name__) from e


def _check_keys(args, name, required, optional=()):
    for key in required:
        if key not in args:
            raise OpError("bad args", f"{name} needs {key}")
    unexpected = set(args) - set(required) - set(optional)
    if unexpected:
        raise OpError("bad args", f"{name} does not take {', '.join(sorted(unexpected))}")


def prop_name(args):
    prop = args.get("prop")
    if not isinstance(prop, str) or not prop:
        raise OpError("bad args", "prop must be a non-empty string")
    if prop.startswith("_"):
        raise OpError("forbidden", prop)
    return prop


def read_prop(obj, prop):
    try:
        value = _live(getattr, obj, prop)
    except OpError as e:
        if e.error_type == "AttributeError":
            raise OpError("no such property", prop) from None
        raise
    if callable(value):
        raise OpError("not a property", prop)
    return value


def display_of(obj):
    return _live(str, obj)


def _get_prop(obj, where, args, ctx):
    _check_keys(args, "get_prop", ("prop",), ("display",))
    prop = prop_name(args)
    value = codec.encode(read_prop(obj, prop), ctx.registry, codec.child_path(where, prop))
    if args.get("display"):
        return {"value": value, "display": display_of(obj)}
    return value


def _set_prop(obj, where, args, ctx):
    _check_keys(args, "set_prop", ("prop", "value"))
    prop = prop_name(args)
    current = read_prop(obj, prop)
    value = codec.decode(args["value"], ctx.registry, ctx.song, ctx.app, expected=current)
    _live(setattr, obj, prop, value)


def _add_listener(obj, where, args, ctx):
    _check_keys(args, "add_listener", ("prop",), ("display",))
    prop = prop_name(args)
    if not hasattr(obj, f"add_{prop}_listener"):
        raise OpError("not observable", prop)
    key, item = ctx.subscriptions.add(obj, prop, bool(args.get("display")), ctx.connection, where)
    result = {"key": key}
    result.update((k, v) for k, v in item.items() if k != "key")
    return result


def _remove_listener(obj, where, args, ctx):
    _check_keys(args, "remove_listener", ("prop",))
    prop = prop_name(args)
    ctx.subscriptions.remove(ctx.subscriptions.key_for(obj, prop, where), ctx.connection)


def _describe(obj, where, args, ctx):
    _check_keys(args, "describe", ())
    return describe(obj)


def is_rack_macro(param):
    """Whether ``param`` is a rack's first macro: its ``canonical_parent`` can
    have chains (a rack), and that rack's parameter ``RACK_MACRO`` is it (by
    Live's pointer: each access gives a fresh wrapper)."""
    try:
        rack = param.canonical_parent
        if not rack.can_have_chains:
            return False
        macro = rack.parameters[RACK_MACRO]
    except (AttributeError, IndexError, RuntimeError, TypeError):
        # A parameter of no rack, or one Live cannot read now: no macro.
        return False
    return id_of(macro) == id_of(param)


def in_range(param, value):
    """Whether ``value`` is a number within ``param``'s range."""
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return False
    if not math.isfinite(value):
        return False
    try:
        return param.min <= value <= param.max
    except (AttributeError, RuntimeError, TypeError):
        return False


def _is_safe_write(item):
    """A DRY write's shape: ``{"target": {"$ref", "class"}, "value"}``."""
    if not isinstance(item, dict) or set(item) != {"target", "value"}:
        return False
    target = item["target"]
    return (
        isinstance(target, dict)
        and set(target) == {"$ref", "class"}
        and isinstance(target["$ref"], str)
        and target["class"] == SAFE_CLASS
    )


def _safe_writes(obj, where, args, ctx):
    """Keeps this connection's DRY writes (#140 PR 3a, ``safe.py``): each a
    rack's first macro by its ``$ref`` and a value in its range, every target
    resolved now (its object is what a fallback writes). A list with a target
    that does not resolve, one that is no rack's first macro, or a value out
    of its range is refused whole, and the list kept before stays."""
    _check_keys(args, "safe_writes", ("writes",))
    writes = args["writes"]
    if not isinstance(writes, list):
        raise OpError("bad args", "writes must be a list")
    if not all(_is_safe_write(item) for item in writes):
        raise OpError("bad args", SAFE_SHAPE)
    resolved = []
    unresolved = 0
    for item in writes:
        try:
            param, _path = lom_path.resolve(item["target"], ctx.song, ctx.app, ctx.registry)
        except FohError:
            unresolved += 1
            continue
        resolved.append((param, item["value"]))
    if unresolved:
        raise OpError("bad args", f"{unresolved} safe write target(s) do not resolve")
    not_macros = sum(1 for param, _value in resolved if not is_rack_macro(param))
    if not_macros:
        raise OpError("bad args", f"{not_macros} safe write target(s) are no rack's first macro")
    out_of_range = sum(1 for param, value in resolved if not in_range(param, value))
    if out_of_range:
        raise OpError("bad args", f"{out_of_range} safe write value(s) out of the macro's range")
    return {"armed": ctx.safe.arm(ctx.connection, resolved)}


_HANDLERS = {
    "get_prop": _get_prop,
    "set_prop": _set_prop,
    "add_listener": _add_listener,
    "remove_listener": _remove_listener,
    "describe": _describe,
    "safe_writes": _safe_writes,
}


def is_listener_api(name):
    if name.endswith("_has_listener"):
        return True
    return name.endswith("_listener") and name.startswith(("add_", "remove_"))


def describe(obj):
    """Class, observable properties, properties and functions, from ``dir()``."""
    names = [n for n in dir(obj) if not n.startswith("_")]
    observable = sorted(
        {
            n[len("add_") : -len("_listener")]
            for n in names
            if n.startswith("add_") and n.endswith("_listener")
        }
    )
    properties = []
    functions = []
    for name in names:
        if is_listener_api(name):
            continue
        try:
            attr = getattr(obj, name)
        except Exception:  # noqa: BLE001, S112 - an unreadable attribute is not listed
            continue
        if not callable(attr):
            properties.append(name)
        elif not isinstance(attr, type) and not name[:1].isupper():
            functions.append(name)
    return {
        "class": type(obj).__name__,
        "observable": observable,
        "properties": sorted(properties),
        "functions": sorted(functions),
    }


def _call(obj, where, name, args, ctx):
    if is_listener_api(name):
        raise OpError("forbidden", name)
    try:
        fn = _live(getattr, obj, name)
    except OpError as e:
        if e.error_type == "AttributeError":
            raise OpError("no such function", name) from None
        raise
    if not callable(fn) or isinstance(fn, type):
        raise OpError("no such function", name)
    if isinstance(args, list):
        decoded = codec.decode(args, ctx.registry, ctx.song, ctx.app)
        result = _live(fn, *decoded)
    elif isinstance(args, dict):
        decoded = codec.decode(args, ctx.registry, ctx.song, ctx.app)
        result = _live(fn, **decoded)
    else:
        raise OpError("bad args", "args must be a list or an object")
    return codec.encode(result, ctx.registry)
