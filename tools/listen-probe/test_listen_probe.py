"""Tests for listen_probe.py (#210): the chunks both ways, the announce, a
whole run against a fake device on real sockets."""

from __future__ import annotations

import argparse
import math
import socket
import struct
import sys
import threading
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import listen_probe as lp  # noqa: E402


def pad(text: str) -> bytes:
    raw = text.encode()
    return raw + b"\0" * (4 - len(raw) % 4)


def announce(device: int) -> bytes:
    out = pad("/fohmixer/listen") + pad(",iisssssii") + struct.pack(">ii", 1, device)
    for text in ("v1.50.0", "Listen", "Listen", "Vox 1", "FOH Band"):
        out += pad(text)
    return out + struct.pack(">ii", 48000, 0)


def matrix(fmt: str, samples: list[float], cols: int, rows: int) -> bytes:
    ident = b"JMTX" if fmt == ">" else b"XTMJ"
    dims = [cols, rows] + [0] * 30
    strides = [8, 8 * cols] + [0] * 30
    data = struct.pack(f"{fmt}{len(samples)}f", *samples)
    header = ident + struct.pack(f"{fmt}iiii", 288, 2, 2, 2)
    header += struct.pack(f"{fmt}32i", *dims) + struct.pack(f"{fmt}32i", *strides)
    header += struct.pack(f"{fmt}id", len(data), 5.0)
    return ident + struct.pack(f"{fmt}i", 288) + header + data


def message(fmt: str, body: bytes) -> bytes:
    ident = b"JMMP" if fmt == ">" else b"PMMJ"
    return ident + struct.pack(f"{fmt}i", len(body)) + body


class ChunkTests(unittest.TestCase):
    def test_a_matrix_reads_in_either_order(self) -> None:
        samples = [0.5 * math.sin(i / 7) for i in range(2 * 960)]
        for fmt, order in ((">", "big"), ("<", "little")):
            buf = bytearray(matrix(fmt, samples, 960, 1))
            chunk, length = lp.next_chunk(buf)
            self.assertEqual(length, len(buf))
            self.assertEqual((chunk["order"], chunk["planes"], chunk["type"]), (order, 2, 2))
            self.assertEqual(chunk["dims"], [960, 1])
            self.assertEqual(chunk["time"], 5.0)
            got = lp.stats(chunk["data"])
            self.assertAlmostEqual(got[order]["peak"], max(abs(s) for s in samples), places=5)
            other = "little" if order == "big" else "big"
            self.assertNotAlmostEqual(got[other]["rms"], got[order]["rms"], places=3)

    def test_an_incomplete_chunk_waits_and_junk_raises(self) -> None:
        whole = matrix(">", [0.0] * 4, 2, 1)
        self.assertEqual(lp.next_chunk(bytearray(whole[:-1])), (None, 0))
        self.assertEqual(lp.next_chunk(bytearray(whole[:100])), (None, 0))
        with self.assertRaises(ValueError):
            lp.next_chunk(bytearray(b"ABCDEFGH"))
        broken = bytearray(whole)
        broken[12:16] = struct.pack(">i", 7)
        with self.assertRaises(ValueError):
            lp.next_chunk(broken)
        chunk, length = lp.next_chunk(bytearray(message("<", b"sx\0")))
        self.assertEqual((chunk["order"], chunk["hex"], length), ("little", "737800", 11))

    def test_the_announce_is_read_without_names(self) -> None:
        got = lp.parse_announce(announce(9))
        self.assertEqual((got["device"], got["sample_rate"], got["version"]), (9, 48000, "v1.50.0"))
        self.assertEqual(got["track_hash"], lp.short_hash("Vox 1"))
        self.assertNotIn("Vox 1", str(got))
        self.assertIsNone(lp.parse_announce(b"/fohmixer/lufs\0\0"))

    def test_the_select_and_the_latency_answer(self) -> None:
        data = lp.select_datagram(9, 1)
        self.assertEqual(data[-8:], struct.pack(">ii", 9, 1))
        self.assertTrue(data.startswith(b"/fohmixer/listen/select\0,ii\0"))
        self.assertEqual(lp.latency("little", 1.0, 2.0, 3.0)[:4], b"PLMJ")
        self.assertEqual(len(lp.latency("big", 1.0, 2.0, 3.0)), 32)


class RunTest(unittest.TestCase):
    def test_a_fake_device_is_selected_streams_and_stops(self) -> None:
        port = free_port()
        select_port = free_port()
        args = argparse.Namespace(
            port=port,
            select_port=select_port,
            idle=0.5,
            seconds=1.0,
            device=None,
            no_latency=False,
            out=None,
        )
        result = {}
        thread = threading.Thread(target=lambda: result.update(lp.run(args)))
        thread.start()
        selects = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        selects.bind(("127.0.0.1", select_port))
        selects.settimeout(5.0)
        time.sleep(0.1)
        sender = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sender.sendto(announce(9), ("127.0.0.1", port))
        data = selects.recv(64)
        self.assertEqual(struct.unpack(">i", data[-8:-4])[0], 9)
        tcp = socket.create_connection(("127.0.0.1", port))
        tcp.sendall(message(">", b"sfohmixer_listen\0"))
        samples = [0.25] * (2 * 960)
        for _ in range(60):
            tcp.sendall(matrix(">", samples, 960, 1))
        answers = b""
        tcp.settimeout(5.0)
        while len(answers) < 32:
            answers += tcp.recv(4096)
        self.assertEqual(answers[:4], b"JMLP")
        thread.join(10.0)
        tcp.close()
        for sock in (selects, sender):
            sock.close()
        self.assertEqual(result["selected"], 9)
        self.assertGreaterEqual(result["selects_sent"], 2)
        conn = result["connections"][0]
        self.assertEqual(conn["matrices"], 60)
        self.assertEqual(conn["chunk_ids"], {"JMTX/big": 60})
        self.assertAlmostEqual(conn["sample_stats"]["big"]["peak"], 0.25)
        self.assertEqual(len(conn["messages"]), 1)


def free_port() -> int:
    for _ in range(64):
        udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        udp.bind(("127.0.0.1", 0))
        port = udp.getsockname()[1]
        tcp = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            tcp.bind(("127.0.0.1", port))
            return port
        except OSError:
            continue
        finally:
            tcp.close()
            udp.close()
    raise RuntimeError("no free port")


if __name__ == "__main__":
    unittest.main()
