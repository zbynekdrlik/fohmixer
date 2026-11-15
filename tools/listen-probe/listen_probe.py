#!/usr/bin/env python3
"""Listen probe (#210): stands in for the hub on the Ableton PC, before a hub
with Listen is deployed, to measure what a real ``FohMixer Listen`` device in
Live sends.

    python listen_probe.py [--port 39111] [--select-port 39112]
                           [--idle 30] [--seconds 60] [--device <id>]
                           [--no-latency] [--out <summary.json>]

It binds ``127.0.0.1:<port>`` for the announces (UDP) and the audio (TCP),
listens ``--idle`` seconds selecting nothing, then selects ``--device`` (the
first device heard when none is given) every 500 ms for ``--seconds``, then
sends one select of 0 and listens 3 s more. It answers every matrix with a
latency chunk in the matrix's byte order, unless ``--no-latency`` (probe P2:
does ``jit.net.send`` wait for the answer?). It prints one JSON summary:

- ``devices``: each device heard, its id, version, sample rate, whether it
  said it sends, and its track and set as short hashes (never names: the
  public repository's rule);
- ``connections``: per TCP connection, its hello, the chunk ids as they came
  (which byte order the header used), the matrices' plane count, type and
  dimensions, how many came while selected and after the stop select, the
  samples per second received, the gaps between matrices, and the samples'
  peak and RMS read both ways (``big`` / ``little``: the order whose values
  look like audio is the one Max sends, probe P3);
- ``selects_sent``.

Stdlib only; the PC's Python 3.12. Nothing is written to Live.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import select
import socket
import struct
import sys
import time

PORT = 39111
SELECT_PORT = 39112
SELECT_EVERY_S = 0.5
AFTER_STOP_S = 3.0
CHUNK_HEADER = 8
MATRIX_HEADER = 288
MAX_DATA = 1 << 20
MAX_MESSAGE = 4096
SELECT_ADDRESS = "/fohmixer/listen/select"
ANNOUNCE_ADDRESS = "/fohmixer/listen"


def osc_string(data: bytes, at: int) -> tuple[str, int]:
    end = data.index(b"\0", at)
    return data[at:end].decode("utf-8"), (end + 4) & ~3


def parse_announce(data: bytes) -> dict | None:
    """An announce's fields, or None for another datagram."""
    try:
        address, at = osc_string(data, 0)
        tags, at = osc_string(data, at)
        if address != ANNOUNCE_ADDRESS or tags != ",iisssssii":
            return None
        wire, device = struct.unpack_from(">ii", data, at)
        at += 8
        names = []
        for _ in range(5):
            text, at = osc_string(data, at)
            names.append(text)
        rate, sending = struct.unpack_from(">ii", data, at)
    except (ValueError, struct.error, UnicodeDecodeError):
        return None
    named = [n for n in names[1:] if n != "-"]
    track, set_name = (named[-2], named[-1]) if len(named) >= 2 else ("", "")
    return {
        "wire": wire,
        "device": device,
        "version": names[0],
        "track_hash": short_hash(track),
        "set_hash": short_hash(set_name),
        "sample_rate": rate,
        "sending": sending,
    }


def short_hash(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()[:8]


def select_datagram(device: int, turn: int) -> bytes:
    def pad(text: str) -> bytes:
        raw = text.encode()
        return raw + b"\0" * (4 - len(raw) % 4)

    return pad(SELECT_ADDRESS) + pad(",ii") + struct.pack(">ii", device, turn)


def order_of(size: bytes, want: int | None, limit: int) -> str | None:
    """The byte order in which `size` reads `want` (or 1..limit)."""
    for name, fmt in (("big", ">i"), ("little", "<i")):
        value = struct.unpack(fmt, size)[0]
        if (want is None and 1 <= value <= limit) or value == want:
            return name
    return None


def stats(data: bytes) -> dict:
    """The samples' peak and RMS read big- and little-endian."""
    count = len(data) // 4
    out = {}
    for name, fmt in (("big", ">"), ("little", "<")):
        values = struct.unpack(f"{fmt}{count}f", data[: count * 4]) if count else ()
        finite = [v for v in values if math.isfinite(v)]
        peak = max((abs(v) for v in finite), default=0.0)
        rms = math.sqrt(sum(v * v for v in finite) / len(finite)) if finite else 0.0
        out[name] = {"peak": peak, "rms": rms, "not_finite": count - len(finite)}
    return out


def next_chunk(buf: bytearray) -> tuple[dict | None, int]:
    """One chunk from the front of `buf`: its description and its length, or
    (None, 0) while incomplete. A chunk that cannot be framed raises."""
    if len(buf) < CHUNK_HEADER:
        return None, 0
    ident = bytes(buf[:4])
    if ident in (b"JMTX", b"XTMJ"):
        if len(buf) < CHUNK_HEADER + MATRIX_HEADER:
            return None, 0
        header = bytes(buf[CHUNK_HEADER : CHUNK_HEADER + MATRIX_HEADER])
        order = order_of(header[4:8], MATRIX_HEADER, MATRIX_HEADER)
        if order is None:
            raise ValueError("a matrix header whose size is not 288 in either order")
        fmt = ">" if order == "big" else "<"
        planes, kind, dimcount = struct.unpack_from(f"{fmt}iii", header, 8)
        dims = struct.unpack_from(f"{fmt}32i", header, 20)[: max(0, min(dimcount, 32))]
        datasize = struct.unpack_from(f"{fmt}i", header, 276)[0]
        sent = struct.unpack_from(f"{fmt}d", header, 280)[0]
        if not 0 <= datasize <= MAX_DATA:
            raise ValueError(f"a matrix of {datasize} bytes")
        total = CHUNK_HEADER + MATRIX_HEADER + datasize
        if len(buf) < total:
            return None, 0
        return {
            "kind": "matrix",
            "chunk_id": ident.decode(),
            "order": order,
            "chunk_size_order": order_of(bytes(buf[4:8]), MATRIX_HEADER, MATRIX_HEADER),
            "planes": planes,
            "type": kind,
            "dims": list(dims),
            "time": sent,
            "data": bytes(buf[CHUNK_HEADER + MATRIX_HEADER : total]),
        }, total
    if ident in (b"JMMP", b"PMMJ"):
        order = order_of(bytes(buf[4:8]), None, MAX_MESSAGE)
        if order is None:
            raise ValueError("a message chunk with no size in either order")
        fmt = ">" if order == "big" else "<"
        size = struct.unpack(f"{fmt}i", bytes(buf[4:8]))[0]
        if len(buf) < CHUNK_HEADER + size:
            return None, 0
        body = bytes(buf[CHUNK_HEADER : CHUNK_HEADER + size])
        return {"kind": "message", "order": order, "hex": body.hex()}, CHUNK_HEADER + size
    raise ValueError(f"an unknown chunk id {ident.hex()}")


def latency(order: str, client: float, before: float, after: float) -> bytes:
    fmt = ">" if order == "big" else "<"
    ident = b"JMLP" if order == "big" else b"PLMJ"
    return ident + b"\0\0\0\0" + struct.pack(f"{fmt}ddd", client, before, after)


class Conn:
    def __init__(self, sock: socket.socket, start: float):
        self.sock = sock
        self.buf = bytearray()
        self.opened = time.monotonic() - start
        self.messages: list[dict] = []
        self.matrices = 0
        self.ids: dict[str, int] = {}
        self.shapes: dict[str, int] = {}
        self.samples = 0
        self.first = None
        self.last = None
        self.gaps: list[float] = []
        self.after_stop = 0
        self.stats = None
        self.closed = None

    def summary(self) -> dict:
        span = (self.last - self.first) if self.first is not None else 0.0
        gaps = sorted(self.gaps)
        return {
            "opened_s": round(self.opened, 3),
            "closed_s": None if self.closed is None else round(self.closed, 3),
            "messages": self.messages,
            "matrices": self.matrices,
            "chunk_ids": self.ids,
            "shapes": self.shapes,
            "samples_per_s": round(self.samples / span, 1) if span > 0 else None,
            "gap_ms_median": round(gaps[len(gaps) // 2] * 1000, 2) if gaps else None,
            "gap_ms_max": round(gaps[-1] * 1000, 2) if gaps else None,
            "matrices_after_stop": self.after_stop,
            "sample_stats": self.stats,
        }


def run(args: argparse.Namespace) -> dict:
    start = time.monotonic()
    udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    udp.bind(("127.0.0.1", args.port))
    tcp = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    tcp.bind(("127.0.0.1", args.port))
    tcp.listen(8)
    devices: dict[int, dict] = {}
    conns: list[Conn] = []
    selects = 0
    chosen = args.device
    select_from = start + args.idle
    stop_at = select_from + args.seconds
    end_at = stop_at + AFTER_STOP_S
    next_select = select_from
    stopped = False
    while (now := time.monotonic()) < end_at:
        if select_from <= now < stop_at and now >= next_select:
            if chosen is None and devices:
                chosen = sorted(devices)[0]
            if chosen is not None:
                udp.sendto(select_datagram(chosen, 1), ("127.0.0.1", args.select_port))
                selects += 1
            next_select = now + SELECT_EVERY_S
        if now >= stop_at and not stopped:
            udp.sendto(select_datagram(0, 1), ("127.0.0.1", args.select_port))
            selects += 1
            stopped = True
        socks = [udp, tcp] + [c.sock for c in conns if c.closed is None]
        ready, _, _ = select.select(socks, [], [], 0.01)
        for sock in ready:
            if sock is udp:
                announce = parse_announce(udp.recv(512))
                if announce is not None:
                    devices[announce["device"]] = announce
            elif sock is tcp:
                accepted, _ = tcp.accept()
                conns.append(Conn(accepted, start))
            else:
                conn = next(c for c in conns if c.sock is sock)
                data = sock.recv(65536)
                if not data:
                    conn.closed = time.monotonic() - start
                    continue
                conn.buf += data
                read(conn, args, start, stopped)
    summary = {
        "devices": list(devices.values()),
        "selected": chosen,
        "selects_sent": selects,
        "connections": [c.summary() for c in conns],
    }
    for sock in [udp, tcp] + [c.sock for c in conns]:
        sock.close()
    return summary


def read(conn: Conn, args: argparse.Namespace, start: float, stopped: bool) -> None:
    for _ in range(len(conn.buf) // CHUNK_HEADER + 1):
        chunk, length = next_chunk(conn.buf)
        if chunk is None:
            return
        del conn.buf[:length]
        if chunk["kind"] == "message":
            if len(conn.messages) < 8:
                conn.messages.append(chunk)
            continue
        now = time.monotonic()
        conn.matrices += 1
        conn.ids[chunk["chunk_id"] + "/" + chunk["order"]] = (
            conn.ids.get(chunk["chunk_id"] + "/" + chunk["order"], 0) + 1
        )
        shape = f"planes={chunk['planes']} type={chunk['type']} dims={chunk['dims']}"
        conn.shapes[shape] = conn.shapes.get(shape, 0) + 1
        planes = max(1, chunk["planes"])
        conn.samples += len(chunk["data"]) // (4 * planes)
        if conn.last is not None:
            conn.gaps.append(now - conn.last)
        conn.first = now if conn.first is None else conn.first
        conn.last = now
        if stopped:
            conn.after_stop += 1
        if conn.matrices == 50:
            conn.stats = stats(chunk["data"])
        if not args.no_latency:
            ms = (now - start) * 1000.0
            conn.sock.sendall(latency(chunk["order"], chunk["time"], ms, ms))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--port", type=int, default=PORT)
    parser.add_argument("--select-port", type=int, default=SELECT_PORT)
    parser.add_argument("--idle", type=float, default=30.0)
    parser.add_argument("--seconds", type=float, default=60.0)
    parser.add_argument("--device", type=int, default=None)
    parser.add_argument("--no-latency", action="store_true")
    parser.add_argument("--out", default=None)
    args = parser.parse_args(argv)
    try:
        summary = run(args)
    except (OSError, ValueError) as error:
        print(f"listen-probe: {error}", file=sys.stderr)
        return 1
    text = json.dumps(summary, indent=1)
    print(text)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as out:
            out.write(text + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
