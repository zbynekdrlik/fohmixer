"""One command: ``{"target", "name", "args"}`` -> result data (design §3.4).

``name`` is ``get_prop``, ``set_prop``, ``add_listener``, ``remove_listener``,
``describe``, or any function of the target, called with positional (list) or
keyword (dict) ``args``. Names starting with ``_`` are rejected. A failure
raises a ``FohError``; an exception raised by Live is wrapped as
``OpError("live error")`` that keeps the original type in ``error_type``.
Runs on Live's main thread only.
"""

from . import codec
from . import path as lom_path
from .errors import FohError, OpError


class Context:
    """What a command may touch: the song, the app, ids, subscriptions, its connection."""

    __slots__ = ("app", "connection", "registry", "song", "subscriptions")

    def __init__(self, song, app, registry, subscriptions, connection):
        self.song = song
        self.app = app
        self.registry = registry
        self.subscriptions = subscriptions
        self.connection = connection


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
    ctx.subscriptions.remove(ctx.registry.put(obj, where), prop, ctx.connection)


def _describe(obj, where, args, ctx):
    _check_keys(args, "describe", ())
    return describe(obj)


_HANDLERS = {
    "get_prop": _get_prop,
    "set_prop": _set_prop,
    "add_listener": _add_listener,
    "remove_listener": _remove_listener,
    "describe": _describe,
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
