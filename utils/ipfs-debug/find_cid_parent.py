#!/usr/bin/env python3
"""
Find which dag-pb bundle contains a given (leaf) CID.

Given a gateway and one or more root CIDs, walks each root's dag-pb tree and
reports whether the target CID appears as a descendant. Use it to map a missing
raw leaf (e.g. a file inside an app bundle) back to the bundle/app that
references it.

Each node is read as its raw block (`?format=raw`) and the dag-pb protobuf is
parsed locally to get child links (gateways won't convert dag-pb to dag-json
server-side). Non-dag-pb codecs (raw leaves) are terminal.

Usage:
  find_cid_parent.py --gateway https://ipfs.io --target <leafCID> <rootCID>...
"""
import argparse
import base64
import subprocess
import sys

DAG_PB = 0x70


def _b32decode(s):
    s = s.upper()
    s += "=" * ((8 - len(s) % 8) % 8)
    return base64.b32decode(s)


def _b32encode(b):
    return base64.b32encode(b).decode().lower().rstrip("=")


def _b58decode(s):
    alpha = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    n = 0
    for c in s:
        n = n * 58 + alpha.index(c)
    full = n.to_bytes((n.bit_length() + 7) // 8, "big")
    pad = len(s) - len(s.lstrip("1"))
    return b"\x00" * pad + full


def _uvarint(b, i):
    x = s = 0
    while True:
        c = b[i]
        i += 1
        x |= (c & 0x7F) << s
        if not c & 0x80:
            return x, i
        s += 7


def _putvarint(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        out.append(b | (0x80 if n else 0))
        if not n:
            return bytes(out)


def cid_str_to_bin(cidstr):
    if cidstr.startswith("Qm") or cidstr.startswith("1"):  # CIDv0: bytes are the multihash
        mh = _b58decode(cidstr)
        return b"\x01" + _putvarint(DAG_PB) + mh  # normalize to v1 dag-pb
    return _b32decode(cidstr[1:])  # strip 'b' multibase, base32


def cid_parts_from_bin(b):
    """(codec, multihash_bytes), normalized across versions."""
    if b[0] == 0x12 and b[1] == 0x20:  # bare CIDv0 sha256 multihash
        return DAG_PB, b
    if b[0] != 0x01:
        raise ValueError("unsupported CID binary")
    codec, i = _uvarint(b, 1)
    return codec, b[i:]


def cid_parts(cidstr):
    return cid_parts_from_bin(cid_str_to_bin(cidstr))


def bin_to_v1_str(b):
    """Binary CID -> base32 CIDv1 string (for the gateway path)."""
    if b[0] == 0x12 and b[1] == 0x20:  # CIDv0 multihash -> wrap as v1 dag-pb
        b = b"\x01" + _putvarint(DAG_PB) + b
    return "b" + _b32encode(b)


def same_cid(a_str, b_str):
    try:
        return cid_parts(a_str) == cid_parts(b_str)
    except Exception:
        return a_str == b_str


def fetch_raw(gateway, cidstr, timeout):
    # Shell out to curl: the system Python's TLS is too old for some gateways.
    url = f"{gateway.rstrip('/')}/ipfs/{cidstr}?format=raw"
    p = subprocess.run(
        ["curl", "-fsS", "--max-time", str(int(timeout)), "-A", "cid-walker/1.0",
         "-H", "Accept: application/vnd.ipld.raw", url],
        capture_output=True, timeout=timeout + 5,
    )
    if p.returncode != 0:
        raise RuntimeError(f"curl rc={p.returncode}: {p.stderr.decode('utf-8','replace').strip()[:120]}")
    return p.stdout


def parse_dagpb_links(block):
    """Return [(name, binary_cid)] from a dag-pb block. PBNode.Links = field 2."""
    links, i, n = [], 0, len(block)
    while i < n:
        tag, i = _uvarint(block, i)
        field, wire = tag >> 3, tag & 7
        if wire == 2:
            ln, i = _uvarint(block, i)
            chunk = block[i : i + ln]
            i += ln
            if field == 2:  # a PBLink message
                links.append(_parse_pblink(chunk))
        elif wire == 0:
            _, i = _uvarint(block, i)
        else:
            break
    return [lk for lk in links if lk]


def _parse_pblink(chunk):
    h = name = None
    i, n = 0, len(chunk)
    while i < n:
        tag, i = _uvarint(chunk, i)
        field, wire = tag >> 3, tag & 7
        if wire == 2:
            ln, i = _uvarint(chunk, i)
            val = chunk[i : i + ln]
            i += ln
            if field == 1:
                h = val  # Hash = binary CID
            elif field == 2:
                name = val.decode("utf-8", "replace")
        elif wire == 0:
            _, i = _uvarint(chunk, i)  # Tsize
        else:
            break
    return (name or "", h) if h else None


def walk(gateway, root, target, timeout, max_nodes):
    seen, stack, visited = set(), [(root, "")], 0
    while stack:
        cid, path = stack.pop()
        if cid in seen:
            continue
        seen.add(cid)
        visited += 1
        if same_cid(cid, target):
            return path or "<root>", visited
        if visited >= max_nodes:
            print(f"  ! stopped at {max_nodes} nodes", file=sys.stderr)
            break
        try:
            codec, _ = cid_parts(cid)
        except Exception:
            continue
        if codec != DAG_PB:
            continue
        try:
            block = fetch_raw(gateway, cid, timeout)
            for name, bincid in parse_dagpb_links(block):
                child = bin_to_v1_str(bincid)
                label = name or child[:12] + "…"
                stack.append((child, f"{path}/{label}"))
        except Exception as e:
            print(f"  ! {cid[:16]}…: {e}", file=sys.stderr)
    return None, visited


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--gateway", required=True)
    ap.add_argument("--target", required=True)
    ap.add_argument("--timeout", type=float, default=30.0)
    ap.add_argument("--max-nodes", type=int, default=5000)
    ap.add_argument("roots", nargs="+")
    a = ap.parse_args()

    print(f"target:  {a.target}\ngateway: {a.gateway}\n")
    hit = False
    for root in a.roots:
        where, n = walk(a.gateway, root, a.target, a.timeout, a.max_nodes)
        if where:
            print(f"FOUND in root {root}\n  path: {where}  (scanned {n} nodes)")
            hit = True
        else:
            print(f"not in root {root}  (scanned {n} nodes)")
    sys.exit(0 if hit else 1)


if __name__ == "__main__":
    main()
