"""LOM paths: ``<root> <step> <step> ...`` with single spaces between tokens.

- root: ``live_set`` (the Song) or ``live_app`` (the Application);
- step: ``<attr>``, ``<attr> <index>`` or ``<attr>[name=<text>]``;
- the ``[name=...]`` text runs to the matching ``]``; escapes are ``\\]`` and ``\\\\``.

A name step selects the one element whose ``name`` equals the text exactly;
none or several is a ``PathError`` ("not found" / "ambiguous"), never a guess.
Resolution returns the object and the path it was reached by, in index form.
"""

import re
from typing import NamedTuple

from .errors import PathError

ROOTS = ("live_set", "live_app")
_IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
_INDEX = re.compile(r"[0-9]+")
_NAME_OPEN = "[name="


class Step(NamedTuple):
    attr: str
    index: int | None = None
    name: str | None = None

    def text(self):
        if self.index is not None:
            return f"{self.attr} {self.index}"
        if self.name is not None:
            escaped = self.name.replace("\\", "\\\\").replace("]", "\\]")
            return f"{self.attr}{_NAME_OPEN}{escaped}]"
        return self.attr


def format_path(root, steps):
    return " ".join([root, *(step.text() for step in steps)])


def parse(text):
    """Parse ``text`` into ``(root, [Step, ...])`` or raise ``PathError``."""
    if not isinstance(text, str):
        raise PathError("bad target", repr(text))
    match = _IDENT.match(text)
    if match is None:
        raise PathError("syntax", text)
    root = match.group()
    if root not in ROOTS:
        raise PathError("bad root", root)
    pos = match.end()
    steps = []
    while pos < len(text):
        if text[pos] != " ":
            raise PathError("syntax", text[pos:])
        pos += 1
        index = _INDEX.match(text, pos)
        if index is not None:
            last = steps[-1] if steps else None
            if last is None or last.index is not None or last.name is not None:
                raise PathError("syntax", f"index {index.group()} has no list before it")
            steps[-1] = last._replace(index=int(index.group()))
            pos = index.end()
            continue
        ident = _IDENT.match(text, pos)
        if ident is None:
            raise PathError("syntax", text[pos:] or "empty step")
        attr = ident.group()
        if attr.startswith("_"):
            raise PathError("forbidden", attr)
        pos = ident.end()
        name = None
        if text.startswith("[", pos):
            if not text.startswith(_NAME_OPEN, pos):
                raise PathError("syntax", text[pos:])
            name, pos = _read_name(text, pos + len(_NAME_OPEN))
        steps.append(Step(attr, None, name))
    return root, steps


def _read_name(text, pos):
    out = []
    while pos < len(text):
        char = text[pos]
        if char == "\\":
            nxt = text[pos + 1 : pos + 2]
            if nxt not in ("]", "\\"):
                raise PathError("syntax", f"bad escape in {text}")
            out.append(nxt)
            pos += 2
        elif char == "]":
            return "".join(out), pos + 1
        else:
            out.append(char)
            pos += 1
    raise PathError("syntax", f"unterminated [name= in {text}")


def resolve(target, song, app, registry):
    """Resolve a command target to ``(obj, path)``.

    ``target`` is a path string, ``{"$ref": id, "class"?: str}`` or ``{"path": str}``.
    """
    if isinstance(target, str):
        return resolve_path(target, song, app)
    if isinstance(target, dict):
        if "$ref" in target:
            return registry.lookup(target["$ref"], target.get("class"))
        if "path" in target:
            return resolve_path(target["path"], song, app)
    raise PathError("bad target", repr(target))


def resolve_path(text, song, app):
    root, steps = parse(text)
    obj = song if root == "live_set" else app
    where = [root]
    for step in steps:
        try:
            value = getattr(obj, step.attr)
        except AttributeError:
            raise PathError("not found", step.text()) from None
        if callable(value):
            raise PathError("not found", f"{step.text()} is a function")
        where.append(step.attr)
        if step.index is not None:
            try:
                value = value[step.index]
            except (IndexError, TypeError, KeyError):
                raise PathError("not found", step.text()) from None
            where.append(str(step.index))
        elif step.name is not None:
            position, value = _select_by_name(value, step)
            where.append(str(position))
        obj = value
    return obj, " ".join(where)


def _select_by_name(items, step):
    try:
        candidates = list(items)
    except TypeError:
        raise PathError("not found", f"{step.text()}: {step.attr} is not a list") from None
    matches = [(i, item) for i, item in enumerate(candidates) if _name_of(item) == step.name]
    if not matches:
        raise PathError("not found", step.text())
    if len(matches) > 1:
        raise PathError("ambiguous", step.text())
    return matches[0]


def _name_of(item):
    try:
        return getattr(item, "name", None)
    except RuntimeError:
        return None
