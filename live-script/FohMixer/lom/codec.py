"""Values between Live and JSON.

Out: primitives as-is; an enum as ``{"$enum": name, "value": int}``; a Live
vector or a tuple/list as a list; a Live object as
``{"$ref", "path", "class", "name"?}`` (registered for later ``$ref`` use);
anything else as ``str()``. ``list()`` is applied only to vectors (a generic
``list(obj)`` on a Live object can hang Live: ableton-js issue #115).

In: ``{"$ref"}`` / ``{"path"}`` resolve to the object; ``{"$enum": name}``
becomes a member of the expected value's enum type (``set_prop``) or of a
qualified ``Live.X.Y.name`` (calls); lists and dicts decode element-wise.
"""

import math

import Live

from . import path as lom_path
from .errors import CodecError


def is_enum(value):
    """A Boost.Python enum member: an ``int`` whose class has ``names`` and ``values``."""
    if not isinstance(value, int) or isinstance(value, bool):
        return False
    cls = type(value)
    return isinstance(getattr(cls, "names", None), dict) and isinstance(
        getattr(cls, "values", None), dict
    )


def enum_name(member):
    name = getattr(member, "name", None)
    if isinstance(name, str):
        return name
    for candidate, value in type(member).names.items():
        if int(value) == int(member):
            return candidate
    return str(int(member))


def _is_live_type(cls):
    module = getattr(cls, "__module__", None) or ""
    return module == "Live" or module.startswith("Live.")


def is_vector(value):
    if isinstance(value, (list, tuple)):
        return True
    cls = type(value)
    return cls.__name__.endswith("Vector") and _is_live_type(cls)


def is_live_object(value):
    return _is_live_type(type(value)) or hasattr(value, "_live_ptr")


def child_path(parent, step):
    return None if parent is None else f"{parent} {step}"


def encode(value, registry, path=None):
    """Encode a LOM value as JSON-able data; Live objects are registered."""
    if value is None or isinstance(value, (bool, str)):
        return value
    if is_enum(value):
        return {"$enum": enum_name(value), "value": int(value)}
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        return value if math.isfinite(value) else str(value)
    if is_vector(value):
        return [encode(item, registry, child_path(path, i)) for i, item in enumerate(value)]
    if is_live_object(value):
        return encode_object(value, registry, path)
    return str(value)


def encode_object(obj, registry, path=None):
    ref = registry.put(obj, path)
    out = {"$ref": ref, "path": registry.path_of(ref), "class": type(obj).__name__}
    try:
        name = getattr(obj, "name", None)
    except RuntimeError:
        name = None
    if isinstance(name, str):
        out["name"] = name
    return out


def decode(value, registry, song, app, expected=None):
    """Decode JSON data into LOM values; ``expected`` is the current value (``set_prop``)."""
    if isinstance(value, dict):
        if "$ref" in value or "path" in value:
            return lom_path.resolve(value, song, app, registry)[0]
        if "$enum" in value:
            return decode_enum(value["$enum"], expected)
        return {key: decode(item, registry, song, app) for key, item in value.items()}
    if isinstance(value, list):
        return [decode(item, registry, song, app) for item in value]
    return value


def decode_enum(name, expected=None):
    if not isinstance(name, str) or not name:
        raise CodecError("bad enum", repr(name))
    if "." in name:
        return _qualified_enum(name)
    if not is_enum(expected):
        raise CodecError("enum needs a qualified name", name)
    member = type(expected).names.get(name)
    if member is None:
        raise CodecError("unknown enum member", f"{type(expected).__qualname__}.{name}")
    return member


def _qualified_enum(name):
    parts = name.split(".")
    if parts[0] != "Live":
        raise CodecError("unknown enum member", name)
    obj = Live
    for part in parts[1:]:
        if not part or part.startswith("_"):
            raise CodecError("forbidden", name)
        try:
            obj = getattr(obj, part)
        except AttributeError:
            raise CodecError("unknown enum member", name) from None
    if not is_enum(obj):
        raise CodecError("not an enum member", name)
    return obj
