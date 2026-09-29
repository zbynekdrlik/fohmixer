#!/usr/bin/env python3
"""LAN DNS check (#26): the public name resolves only locally on the LAN.

On the LAN the public name must lead straight to the PC, never to
Cloudflare. A LAN DNS entry that is an A record alone answers A locally,
but lets AAAA and HTTPS (type 65) go upstream, and Cloudflare's HTTPS
record carries hints (ipv4hint / ipv6hint, ECH, h3): iOS follows them, so
system traffic and Home Screen apps went through Cloudflare Access while
Safari went straight (the owner's first iPad session). The fix is a CNAME
to a local-only name; this check proves it after every deploy.

It asks the LAN resolver for A, AAAA and HTTPS of the name (a stdlib DNS
client over UDP) and fails, one line per problem, when
- the A answer has no address (the LAN does not resolve the name: a missing
  entry, a CNAME to a name without an A, or a mistyped --name);
- an A or AAAA answer is a public address (anything the hub would not take
  for the local network: loopback, RFC 1918, CGNAT, link-local, IPv6
  unique-local and link-local are local, as in `access::is_private_ip`);
- the HTTPS answer has an HTTPS record at all: a CNAME to a local-only name
  has none, and the upstream one carries ipv4hint / ipv6hint (the public
  addresses), ech (a key the PC cannot use) and h3.
A CNAME to a local name with a local A passes; so do an empty AAAA and an
empty HTTPS answer.

    python3 scripts/check_lan_dns.py --name <public name> --resolver <LAN resolver>

The name and the resolver are site values: pass them at run time, never
commit them (`.claude/rules/public-repo-hygiene.md`). Exit 0: clean (one
summary line); 1: problems (one line each on stderr); 2: bad arguments.
"""
from __future__ import annotations

import argparse
import ipaddress
import secrets
import socket
import struct
import sys
from dataclasses import dataclass

TYPE_A = 1
TYPE_CNAME = 5
TYPE_AAAA = 28
TYPE_OPT = 41
TYPE_HTTPS = 65
CLASS_IN = 1
TYPE_NAMES = {TYPE_A: "A", TYPE_AAAA: "AAAA", TYPE_HTTPS: "HTTPS"}
QUERIED = (TYPE_A, TYPE_AAAA, TYPE_HTTPS)

RCODE_NOERROR = 0
RCODE_NXDOMAIN = 3
RCODE_NAMES = {1: "FORMERR", 2: "SERVFAIL", 4: "NOTIMP", 5: "REFUSED"}

# SvcParamKeys (RFC 9460 and the ECH draft) by name; the hints point a
# client at addresses.
SVC_IPV4HINT = 4
SVC_IPV6HINT = 6
SVC_NAMES = {0: "mandatory", 1: "alpn", 2: "no-default-alpn", 3: "port", 5: "ech"}

# The EDNS0 UDP payload offered (the DNS flag day's recommendation).
EDNS_PAYLOAD = 1232

# The networks the hub takes for the local network (access::is_private_ip).
LOCAL_NETWORKS = tuple(
    ipaddress.ip_network(n)
    for n in (
        "127.0.0.0/8",
        "10.0.0.0/8",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "100.64.0.0/10",
        "169.254.0.0/16",
        "::1/128",
        "fc00::/7",
        "fe80::/10",
    )
)


def is_local(address: ipaddress.IPv4Address | ipaddress.IPv6Address) -> bool:
    """Whether the hub takes `address` for the local network."""
    mapped = getattr(address, "ipv4_mapped", None)
    if mapped is not None:
        return is_local(mapped)
    return any(address in network for network in LOCAL_NETWORKS)


def encode_name(name: str) -> bytes:
    """A domain name in DNS wire form."""
    out = bytearray()
    for label in name.rstrip(".").split("."):
        raw = label.encode("idna")
        if not 0 < len(raw) < 64:
            raise ValueError(f"bad label {label!r} in {name!r}")
        out += bytes([len(raw)]) + raw
    return bytes(out + b"\x00")


def build_query(name: str, qtype: int, qid: int) -> bytes:
    """One recursive question (RD) with an EDNS0 OPT record."""
    header = struct.pack("!HHHHHH", qid, 0x0100, 1, 0, 0, 1)
    question = encode_name(name) + struct.pack("!HH", qtype, CLASS_IN)
    opt = b"\x00" + struct.pack("!HHIH", TYPE_OPT, EDNS_PAYLOAD, 0, 0)
    return header + question + opt


def read_name(data: bytes, offset: int) -> tuple[str, int]:
    """The (possibly compressed) name at `offset`, and the offset after it."""
    labels: list[str] = []
    end = None
    # Every step reads at least one byte or follows a pointer backwards, so
    # the message's length bounds a valid name; more steps mean a loop.
    for _ in range(len(data)):
        if offset >= len(data):
            raise ValueError("a name runs past the end of the message")
        length = data[offset]
        if length == 0:
            return ".".join(labels), (offset + 1 if end is None else end)
        if length & 0xC0 == 0xC0:
            if offset + 1 >= len(data):
                raise ValueError("a name pointer runs past the end of the message")
            pointer = ((length & 0x3F) << 8) | data[offset + 1]
            if pointer >= offset:
                raise ValueError("a name pointer that does not point back")
            if end is None:
                end = offset + 2
            offset = pointer
            continue
        if length & 0xC0:
            raise ValueError(f"an unknown label type {length:#x}")
        label = data[offset + 1 : offset + 1 + length]
        if len(label) != length:
            raise ValueError("a label runs past the end of the message")
        labels.append(label.decode("ascii", errors="replace"))
        offset += 1 + length
    raise ValueError("a name that never ends")


@dataclass(frozen=True)
class Record:
    """An answer record: its owner, type, and where its data is."""

    owner: str
    rtype: int
    start: int
    length: int


@dataclass(frozen=True)
class Answer:
    rcode: int
    truncated: bool
    records: tuple[Record, ...]


def parse_response(data: bytes, qid: int) -> Answer:
    """The answer section of the response to query `qid`."""
    if len(data) < 12:
        raise ValueError("a response shorter than its header")
    rid, flags, qdcount, ancount, _, _ = struct.unpack("!HHHHHH", data[:12])
    if rid != qid:
        raise ValueError(f"a response to another query (id {rid}, asked {qid})")
    if not flags & 0x8000:
        raise ValueError("not a response (QR is 0)")
    offset = 12
    for _ in range(qdcount):
        _, offset = read_name(data, offset)
        offset += 4
    records = []
    for _ in range(ancount):
        owner, offset = read_name(data, offset)
        if offset + 10 > len(data):
            raise ValueError("a record header runs past the end of the message")
        rtype, _, _, length = struct.unpack("!HHIH", data[offset : offset + 10])
        start = offset + 10
        if start + length > len(data):
            raise ValueError("a record's data runs past the end of the message")
        records.append(Record(owner, rtype, start, length))
        offset = start + length
    return Answer(flags & 0x000F, bool(flags & 0x0200), tuple(records))


def address_of(data: bytes, record: Record) -> ipaddress.IPv4Address | ipaddress.IPv6Address:
    """The address of an A or AAAA record."""
    raw = data[record.start : record.start + record.length]
    if record.rtype == TYPE_A and len(raw) == 4:
        return ipaddress.IPv4Address(raw)
    if record.rtype == TYPE_AAAA and len(raw) == 16:
        return ipaddress.IPv6Address(raw)
    raise ValueError(f"an address record of {len(raw)} bytes")


def svc_params(data: bytes, record: Record) -> list[str]:
    """An HTTPS record's parameters in words: `ipv4hint 192.0.2.1` per
    hinted address, else the key's name (`alpn`, `ech`, `key9`)."""
    end = record.start + record.length
    if record.length < 3:
        raise ValueError("an HTTPS record shorter than its priority and target")
    _, offset = read_name(data, record.start + 2)
    params = []
    while offset < end:
        if offset + 4 > end:
            raise ValueError("an HTTPS parameter header runs past the record")
        key, length = struct.unpack("!HH", data[offset : offset + 4])
        value = data[offset + 4 : offset + 4 + length]
        if offset + 4 + length > end:
            raise ValueError("an HTTPS parameter runs past the record")
        if key == SVC_IPV4HINT:
            params += [f"ipv4hint {ipaddress.IPv4Address(value[i : i + 4])}" for i in range(0, length, 4)]
        elif key == SVC_IPV6HINT:
            params += [f"ipv6hint {ipaddress.IPv6Address(value[i : i + 16])}" for i in range(0, length, 16)]
        else:
            params.append(SVC_NAMES.get(key, f"key{key}"))
        offset += 4 + length
    return params


def problems(qtype: int, data: bytes, answer: Answer) -> list[str]:
    """What is wrong with the answer to a `qtype` question (nothing: [])."""
    kind = TYPE_NAMES[qtype]
    if answer.truncated:
        return [f"{kind}: the answer was truncated (TC): not checked"]
    if answer.rcode not in (RCODE_NOERROR, RCODE_NXDOMAIN):
        return [f"{kind}: the resolver answered {RCODE_NAMES.get(answer.rcode, answer.rcode)}"]
    found = []
    for record in answer.records:
        if record.rtype in (TYPE_A, TYPE_AAAA):
            address = address_of(data, record)
            if not is_local(address):
                name = TYPE_NAMES[record.rtype]
                found.append(f"{kind}: {record.owner} {name} {address} is a public address")
        elif record.rtype == TYPE_HTTPS:
            params = ", ".join(svc_params(data, record)) or "no parameters"
            found.append(f"{kind}: {record.owner} has an HTTPS record ({params}); "
                         "a CNAME to a local-only name has none")
    if qtype == TYPE_A and not any(record.rtype == TYPE_A for record in answer.records):
        found.append(f"{kind}: the LAN does not resolve the name to an address")
    return found


def describe(data: bytes, answer: Answer) -> str:
    """The answer in one short phrase (`CNAME mixer-pc.home.arpa, A 10.0.0.5`, or `-`)."""
    parts = []
    for record in answer.records:
        if record.rtype == TYPE_CNAME:
            parts.append(f"CNAME {read_name(data, record.start)[0]}")
        elif record.rtype in (TYPE_A, TYPE_AAAA):
            parts.append(f"{TYPE_NAMES[record.rtype]} {address_of(data, record)}")
        elif record.rtype == TYPE_HTTPS:
            parts.append("HTTPS")
    return ", ".join(parts) or "-"


def ask(resolver: str, port: int, name: str, qtype: int, timeout: float) -> tuple[bytes, Answer]:
    """Asks `resolver` one question over UDP: the response and its answer."""
    qid = secrets.randbits(16)
    family = socket.AF_INET6 if ":" in resolver else socket.AF_INET
    with socket.socket(family, socket.SOCK_DGRAM) as sock:
        sock.settimeout(timeout)
        sock.connect((resolver, port))
        sock.send(build_query(name, qtype, qid))
        # A stray datagram of another query is skipped (its id differs).
        for _ in range(4):
            data = sock.recv(65535)
            if data[:2] == struct.pack("!H", qid):
                return data, parse_response(data, qid)
    raise ValueError("no response to this query among the datagrams received")


def check(resolver: str, port: int, name: str, timeout: float) -> tuple[list[str], list[str]]:
    """Asks every type: (problems, one summary phrase per type)."""
    found: list[str] = []
    summary: list[str] = []
    for qtype in QUERIED:
        kind = TYPE_NAMES[qtype]
        try:
            data, answer = ask(resolver, port, name, qtype, timeout)
            found += problems(qtype, data, answer)
            summary.append(f"{kind}: {describe(data, answer)}")
        except (OSError, ValueError) as error:
            found.append(f"{kind}: no usable answer from {resolver}: {error}")
    return found, summary


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--name", required=True, help="the public name")
    parser.add_argument("--resolver", required=True, help="the LAN resolver's address")
    parser.add_argument("--port", type=int, default=53)
    parser.add_argument("--timeout", type=float, default=3.0, help="seconds per question")
    args = parser.parse_args(argv)
    try:
        ipaddress.ip_address(args.resolver)
        encode_name(args.name)
    except ValueError as error:
        parser.error(str(error))
    found, summary = check(args.resolver, args.port, args.name, args.timeout)
    if found:
        for line in found:
            print(f"check_lan_dns: FAIL {args.name} via {args.resolver}: {line}", file=sys.stderr)
        return 1
    print(f"check_lan_dns: OK {args.name} via {args.resolver}: " + "; ".join(summary))
    return 0


if __name__ == "__main__":
    sys.exit(main())
