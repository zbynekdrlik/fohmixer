"""Tests for scripts/check_lan_dns.py (#26), on hand-built DNS messages.

Names and addresses are examples only: documentation addresses
(192.0.2.0/24, 203.0.113.0/24, 2001:db8::/32) stand for public ones,
`foh.example.org` for the public name, `mixer-pc.home.arpa` for the local
one.
"""
from __future__ import annotations

import contextlib
import io
import ipaddress
import socket
import struct
import sys
import threading
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_lan_dns as dns  # noqa: E402

NAME = "foh.example.org"
LOCAL_NAME = "mixer-pc.home.arpa"
# The question's name sits right after the 12-byte header.
AT_QNAME = b"\xc0\x0c"


def rr(owner: bytes, rtype: int, rdata: bytes) -> bytes:
    """An answer record (class IN, TTL 300)."""
    return owner + struct.pack("!HHIH", rtype, dns.CLASS_IN, 300, len(rdata)) + rdata


def a(address: str) -> bytes:
    return ipaddress.IPv4Address(address).packed


def aaaa(address: str) -> bytes:
    return ipaddress.IPv6Address(address).packed


def svc_param(key: int, value: bytes) -> bytes:
    return struct.pack("!HH", key, len(value)) + value


def https(*params: bytes, priority: int = 1, target: str = "") -> bytes:
    """An HTTPS record's data: priority, target (`""` is the root) and params."""
    wire_target = dns.encode_name(target) if target else b"\x00"
    return struct.pack("!H", priority) + wire_target + b"".join(params)


ALPN = svc_param(1, b"\x02h3\x02h2")
ECH = svc_param(5, b"\x00\x04abcd")


def response(qid: int, qtype: int, answers: list[bytes], rcode: int = 0, truncated: bool = False,
             qname: str = NAME) -> bytes:
    flags = 0x8180 | rcode | (0x0200 if truncated else 0)
    header = struct.pack("!HHHHHH", qid, flags, 1, len(answers), 0, 0)
    question = dns.encode_name(qname) + struct.pack("!HH", qtype, dns.CLASS_IN)
    return header + question + b"".join(answers)


def local_cname_chain(qid: int, qtype: int) -> bytes:
    """The fixed LAN answer: a CNAME to a local name, which has a local A only."""
    cname = rr(AT_QNAME, dns.TYPE_CNAME, dns.encode_name(LOCAL_NAME))
    target_at = 12 + len(dns.encode_name(NAME)) + 4 + 12  # the CNAME's data
    pointer = struct.pack("!H", 0xC000 | target_at)
    answers = [cname]
    if qtype == dns.TYPE_A:
        answers.append(rr(pointer, dns.TYPE_A, a("10.0.0.5")))
    return response(qid, qtype, answers)


def problems_of(data: bytes, qtype: int) -> list[str]:
    qid = struct.unpack("!H", data[:2])[0]
    return dns.problems(qtype, data, dns.parse_response(data, qid))


class LocalAddresses(unittest.TestCase):
    def test_the_hubs_local_networks_are_local(self) -> None:
        for address in ["127.0.0.1", "10.0.0.5", "172.16.0.1", "172.31.255.255", "192.168.1.20",
                        "100.64.0.1", "100.127.255.255", "169.254.1.1", "::1", "fc00::1", "fdff::1",
                        "fe80::1", "febf::1", "::ffff:10.0.0.1", "::ffff:127.0.0.1"]:
            self.assertTrue(dns.is_local(ipaddress.ip_address(address)), address)

    def test_everything_else_is_public(self) -> None:
        for address in ["192.0.2.10", "203.0.113.7", "8.8.8.8", "100.63.255.255", "100.128.0.0",
                        "172.32.0.1", "0.0.0.0", "2001:db8::1", "fbff::1", "fec0::1",
                        "::ffff:192.0.2.10"]:
            self.assertFalse(dns.is_local(ipaddress.ip_address(address)), address)


class Query(unittest.TestCase):
    def test_a_query_is_one_recursive_question_with_edns(self) -> None:
        query = dns.build_query(NAME, dns.TYPE_HTTPS, 0xBEEF)
        self.assertEqual(struct.unpack("!HHHHHH", query[:12]), (0xBEEF, 0x0100, 1, 0, 0, 1))
        wire = b"\x03foh\x07example\x03org\x00"
        self.assertEqual(query[12 : 12 + len(wire)], wire)
        rest = query[12 + len(wire) :]
        self.assertEqual(struct.unpack("!HH", rest[:4]), (65, 1))
        self.assertEqual(rest[4:], b"\x00" + struct.pack("!HHIH", 41, 1232, 0, 0))

    def test_a_bad_name_is_refused(self) -> None:
        for name in ["", "foh..example.org", "x" * 64 + ".org"]:
            with self.assertRaises(ValueError, msg=name):
                dns.encode_name(name)
        self.assertEqual(dns.encode_name("foh.example.org."), dns.encode_name(NAME))


class Names(unittest.TestCase):
    def test_a_compressed_name_is_followed(self) -> None:
        data = local_cname_chain(1, dns.TYPE_A)
        answer = dns.parse_response(data, 1)
        self.assertEqual([r.owner for r in answer.records], [NAME, LOCAL_NAME])
        self.assertEqual(dns.read_name(data, answer.records[0].start), (LOCAL_NAME, answer.records[0].start
                                                                         + len(dns.encode_name(LOCAL_NAME))))

    def test_a_broken_name_is_an_error_not_a_hang(self) -> None:
        header = b"\x00" * 12
        for data, offset in [
            (header + b"\xc0\x0c", 12),  # a pointer to itself
            (header + b"\x01a\xc0\x0e", 12),  # a pointer forwards
            (header + b"\x05ab", 12),  # a label past the end
            (header + b"\x01a", 12),  # no end
            (header + b"\x80", 12),  # an unknown label type
            (header + b"\xc0", 12),  # half a pointer
        ]:
            with self.assertRaises(ValueError, msg=data):
                dns.read_name(data, offset)


class Responses(unittest.TestCase):
    def test_another_querys_response_or_a_query_is_refused(self) -> None:
        data = local_cname_chain(7, dns.TYPE_A)
        with self.assertRaises(ValueError):
            dns.parse_response(data, 8)
        not_a_response = struct.pack("!HHHHHH", 7, 0x0100, 0, 0, 0, 0)
        with self.assertRaises(ValueError):
            dns.parse_response(not_a_response, 7)
        with self.assertRaises(ValueError):
            dns.parse_response(b"\x00\x07", 7)
        cut = local_cname_chain(7, dns.TYPE_A)[:-2]
        with self.assertRaises(ValueError):
            dns.parse_response(cut, 7)

    def test_a_cname_to_a_local_name_passes(self) -> None:
        for qtype in dns.QUERIED:
            data = local_cname_chain(3, qtype)
            self.assertEqual(problems_of(data, qtype), [], qtype)
        data = local_cname_chain(3, dns.TYPE_A)
        answer = dns.parse_response(data, 3)
        self.assertEqual(dns.describe(data, answer), f"CNAME {LOCAL_NAME}, A 10.0.0.5")

    def test_local_addresses_and_empty_aaaa_and_https_answers_pass(self) -> None:
        self.assertEqual(problems_of(response(1, dns.TYPE_A, [rr(AT_QNAME, 1, a("192.168.1.20"))]), 1), [])
        self.assertEqual(problems_of(response(1, dns.TYPE_AAAA, [rr(AT_QNAME, 28, aaaa("fd00::5"))]), 28), [])
        empty = response(1, dns.TYPE_AAAA, [])
        self.assertEqual(problems_of(empty, dns.TYPE_AAAA), [])
        self.assertEqual(dns.describe(empty, dns.parse_response(empty, 1)), "-")
        nxdomain = response(1, dns.TYPE_HTTPS, [], rcode=3)
        self.assertEqual(problems_of(nxdomain, dns.TYPE_HTTPS), [])

    def test_a_name_the_lan_does_not_resolve_fails(self) -> None:
        # No entry (a mistyped --name upstream: NXDOMAIN), no address, or a
        # CNAME to a name without an A.
        unresolved = ["A: the LAN does not resolve the name to an address"]
        for data in [response(1, dns.TYPE_A, [], rcode=3), response(1, dns.TYPE_A, []),
                     response(1, dns.TYPE_A, [rr(AT_QNAME, 5, dns.encode_name(LOCAL_NAME))])]:
            self.assertEqual(problems_of(data, dns.TYPE_A), unresolved)
        # An A answer with only an AAAA in it has no address either.
        data = response(1, dns.TYPE_A, [rr(AT_QNAME, 28, aaaa("fd00::5"))])
        self.assertEqual(problems_of(data, dns.TYPE_A), unresolved)

    def test_a_public_address_fails(self) -> None:
        data = response(1, dns.TYPE_A, [rr(AT_QNAME, 1, a("192.0.2.10")), rr(AT_QNAME, 1, a("10.0.0.5"))])
        self.assertEqual(problems_of(data, dns.TYPE_A), [f"A: {NAME} A 192.0.2.10 is a public address"])
        data = response(1, dns.TYPE_AAAA, [rr(AT_QNAME, 28, aaaa("2001:db8::1"))])
        self.assertEqual(problems_of(data, dns.TYPE_AAAA), [f"AAAA: {NAME} AAAA 2001:db8::1 is a public address"])

    def test_a_malformed_address_is_an_error(self) -> None:
        data = response(1, dns.TYPE_A, [rr(AT_QNAME, 1, b"\x0a\x00\x00")])
        with self.assertRaises(ValueError):
            problems_of(data, dns.TYPE_A)

    def test_an_https_record_fails_naming_its_hints(self) -> None:
        record = https(ALPN, svc_param(4, a("192.0.2.1") + a("192.0.2.2")), ECH,
                       svc_param(6, aaaa("2001:db8::1")))
        data = response(1, dns.TYPE_HTTPS, [rr(AT_QNAME, 65, record)])
        self.assertEqual(problems_of(data, dns.TYPE_HTTPS), [
            f"HTTPS: {NAME} has an HTTPS record (alpn, ipv4hint 192.0.2.1, ipv4hint 192.0.2.2, ech, "
            "ipv6hint 2001:db8::1); a CNAME to a local-only name has none",
        ])
        self.assertEqual(dns.describe(data, dns.parse_response(data, 1)), "HTTPS")

    def test_any_https_record_fails(self) -> None:
        # A CNAME to a local-only name has no HTTPS record: one without hints
        # (only ech or h3, an alias) still came from somewhere it should not.
        for record, params in [
            (https(svc_param(4, a("10.0.0.5"))), "ipv4hint 10.0.0.5"),
            (https(ALPN, ECH), "alpn, ech"),
            (https(svc_param(0, b"\x00\x01"), svc_param(3, b"\x01\xbb"), svc_param(9, b"x")), "mandatory, port, key9"),
            (https(svc_param(2, b"")), "no-default-alpn"),
            (https(priority=0, target=LOCAL_NAME), "no parameters"),
            (https(), "no parameters"),
        ]:
            data = response(1, dns.TYPE_HTTPS, [rr(AT_QNAME, 65, record)])
            self.assertEqual(problems_of(data, dns.TYPE_HTTPS), [
                f"HTTPS: {NAME} has an HTTPS record ({params}); a CNAME to a local-only name has none",
            ], params)

    def test_a_malformed_https_record_is_an_error(self) -> None:
        for record in [b"\x00", https(b"\x00\x04\x00"), https(svc_param(4, b"\x0a\x00")),
                       https() + b"\x00\x04\x00\x08\x0a\x00\x00\x05"]:
            data = response(1, dns.TYPE_HTTPS, [rr(AT_QNAME, 65, record)])
            with self.assertRaises(ValueError, msg=record):
                problems_of(data, dns.TYPE_HTTPS)

    def test_a_resolver_failure_or_a_truncated_answer_fails(self) -> None:
        self.assertEqual(problems_of(response(1, dns.TYPE_A, [], rcode=2), dns.TYPE_A),
                         ["A: the resolver answered SERVFAIL"])
        self.assertEqual(problems_of(response(1, dns.TYPE_AAAA, [], rcode=5), dns.TYPE_AAAA),
                         ["AAAA: the resolver answered REFUSED"])
        self.assertEqual(problems_of(response(1, dns.TYPE_A, [], rcode=9), dns.TYPE_A),
                         ["A: the resolver answered 9"])
        truncated = response(1, dns.TYPE_HTTPS, [], truncated=True)
        self.assertEqual(problems_of(truncated, dns.TYPE_HTTPS),
                         ["HTTPS: the answer was truncated (TC): not checked"])


class FakeResolver:
    """A resolver on 127.0.0.1 that answers each question with `answer(qid, qtype)`:
    a datagram, a list of datagrams sent in order, or None (silence)."""

    def __init__(self, answer) -> None:
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(("127.0.0.1", 0))
        self.sock.settimeout(0.1)
        self.port = self.sock.getsockname()[1]
        self.answer = answer
        self.questions: list[int] = []
        self.done = threading.Event()
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()

    def serve(self) -> None:
        while not self.done.is_set():
            try:
                query, peer = self.sock.recvfrom(4096)
            except TimeoutError:
                continue
            qid = struct.unpack("!H", query[:2])[0]
            _, offset = dns.read_name(query, 12)
            qtype = struct.unpack("!H", query[offset : offset + 2])[0]
            self.questions.append(qtype)
            reply = self.answer(qid, qtype)
            for datagram in [reply] if isinstance(reply, bytes) else reply or []:
                self.sock.sendto(datagram, peer)

    def close(self) -> None:
        self.done.set()
        self.thread.join()
        self.sock.close()


def run(port: int, *extra: str) -> tuple[int, str, str]:
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = dns.main(["--name", NAME, "--resolver", "127.0.0.1", "--port", str(port), *extra])
    return code, out.getvalue(), err.getvalue()


class OverUdp(unittest.TestCase):
    def test_a_clean_lan_answer_passes(self) -> None:
        resolver = FakeResolver(local_cname_chain)
        try:
            code, out, err = run(resolver.port)
        finally:
            resolver.close()
        self.assertEqual((code, err), (0, ""))
        self.assertEqual(resolver.questions, [dns.TYPE_A, dns.TYPE_AAAA, dns.TYPE_HTTPS])
        self.assertEqual(out, f"check_lan_dns: OK {NAME} via 127.0.0.1: A: CNAME {LOCAL_NAME}, "
                              f"A 10.0.0.5; AAAA: CNAME {LOCAL_NAME}; HTTPS: CNAME {LOCAL_NAME}\n")

    def test_the_upstream_hints_fail_one_line_each(self) -> None:
        # The old router entry: A local, AAAA and HTTPS from upstream.
        def upstream(qid: int, qtype: int) -> bytes:
            if qtype == dns.TYPE_A:
                return response(qid, qtype, [rr(AT_QNAME, 1, a("10.0.0.5"))])
            if qtype == dns.TYPE_AAAA:
                return response(qid, qtype, [rr(AT_QNAME, 28, aaaa("2001:db8::7"))])
            return response(qid, qtype, [rr(AT_QNAME, 65, https(ALPN, svc_param(4, a("203.0.113.7"))))])

        resolver = FakeResolver(upstream)
        try:
            code, out, err = run(resolver.port)
        finally:
            resolver.close()
        self.assertEqual((code, out), (1, ""))
        prefix = f"check_lan_dns: FAIL {NAME} via 127.0.0.1: "
        self.assertEqual(err.splitlines(), [
            f"{prefix}AAAA: {NAME} AAAA 2001:db8::7 is a public address",
            f"{prefix}HTTPS: {NAME} has an HTTPS record (alpn, ipv4hint 203.0.113.7); "
            "a CNAME to a local-only name has none",
        ])

    def test_a_silent_resolver_fails(self) -> None:
        resolver = FakeResolver(lambda qid, qtype: None)
        try:
            code, out, err = run(resolver.port, "--timeout", "0.2")
        finally:
            resolver.close()
        self.assertEqual((code, out), (1, ""))
        lines = err.splitlines()
        self.assertEqual(len(lines), 3)
        for line, kind in zip(lines, ["A", "AAAA", "HTTPS"], strict=True):
            self.assertIn(f"{kind}: no usable answer from 127.0.0.1: ", line)

    def test_a_stray_datagram_is_skipped(self) -> None:
        # Before each answer, a response to another query (another id) that
        # names a public address: it must not count.
        def stray_then_answer(qid: int, qtype: int) -> list[bytes]:
            stray = response(qid ^ 0xFFFF, qtype, [rr(AT_QNAME, 1, a("192.0.2.10"))])
            local = [rr(AT_QNAME, 1, a("10.0.0.5"))] if qtype == dns.TYPE_A else []
            return [stray, response(qid, qtype, local)]

        resolver = FakeResolver(stray_then_answer)
        try:
            code, out, err = run(resolver.port)
        finally:
            resolver.close()
        self.assertEqual((code, err), (0, ""))
        self.assertEqual(out, f"check_lan_dns: OK {NAME} via 127.0.0.1: A: A 10.0.0.5; AAAA: -; HTTPS: -\n")

    def test_bad_arguments_exit_2(self) -> None:
        for args in [["--name", NAME, "--resolver", "not-an-address"],
                     ["--name", "foh..example.org", "--resolver", "192.0.2.53"],
                     ["--name", NAME]]:
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as exit_:
                dns.main(args)
            self.assertEqual(exit_.exception.code, 2, args)


if __name__ == "__main__":
    unittest.main()
