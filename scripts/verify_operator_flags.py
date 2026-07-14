#!/usr/bin/env python3
# Copyright (C) Parity Technologies (UK) Ltd.
# SPDX-License-Identifier: Apache-2.0
"""Verify the node-operator doc's Bulletin/HOP flags against the polkadot-sdk source.

Flags are extracted from the clap definitions in polkadot-sdk at the exact commit
this repo pins (`.github/env` POLKADOT_NODE_VERSION), not from a built binary. This
checks both directions:

  * every `--enable-hop` / `--hop-*` / `--ipfs-*` flag named in the doc exists in
    the source (catches renamed/removed flags), and
  * every such flag in the source appears in the doc (catches missing flags),

and compares the numeric value the doc states for each `--hop-*` flag against the
default constant in the source.

Usage:
  scripts/verify_operator_flags.py <doc.md>

Source resolution (in order):
  * $SDK_SRC (a polkadot-sdk checkout) -> `git show <commit>:<path>`
  * ./.polkadot-binaries/_src/polkadot-sdk (populated by `just binaries-polkadot`)
  * https://raw.githubusercontent.com/paritytech/polkadot-sdk/<commit>/<path>
"""
import os
import re
import subprocess
import sys
import urllib.request

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
HOP_CLI = "substrate/client/hop/src/cli.rs"
HOP_TYPES = "substrate/client/hop/src/types.rs"
NET_PARAMS = "substrate/client/cli/src/params/network_params.rs"


def pinned_commit():
    for line in open(os.path.join(REPO, ".github/env")):
        if line.startswith("POLKADOT_NODE_VERSION="):
            return line.strip().split("=", 1)[1]
    sys.exit("POLKADOT_NODE_VERSION not found in .github/env")


def source_of(path, commit):
    """Return the contents of `path` in polkadot-sdk at `commit`."""
    for src in (os.environ.get("SDK_SRC"),
                os.path.join(REPO, ".polkadot-binaries/_src/polkadot-sdk")):
        if src and os.path.isdir(os.path.join(src, ".git")):
            try:
                return subprocess.check_output(
                    ["git", "-C", src, "show", f"{commit}:{path}"],
                    stderr=subprocess.DEVNULL, text=True)
            except subprocess.CalledProcessError:
                pass  # commit not fetched locally; fall through to network
    url = f"https://raw.githubusercontent.com/paritytech/polkadot-sdk/{commit}/{path}"
    return urllib.request.urlopen(url).read().decode()


def resolve_defaults(types_src):
    """Parse `pub const DEFAULT_* = <expr>;` and evaluate, resolving cross-refs."""
    raw = dict(re.findall(r"pub const (DEFAULT_[A-Z_]+):[^=]+=\s*([^;]+);", types_src))
    out = {}

    def ev(expr, seen=()):
        expr = expr.strip()
        for name in re.findall(r"DEFAULT_[A-Z_]+", expr):
            if name in seen:
                raise ValueError(f"cyclic default {name}")
            expr = expr.replace(name, str(ev(raw[name], seen + (name,))))
        if not re.fullmatch(r"[0-9_*/+\-() ]+", expr):
            raise ValueError(f"unsafe expr: {expr}")
        val = eval(expr)  # noqa: S307 - input constrained to arithmetic above
        return int(val) if val == int(val) else val

    for name, expr in raw.items():
        try:
            out[name] = ev(expr)
        except Exception:
            pass
    return out


def extract_arg_flags(src, defaults):
    """Map long-flag name -> default value (or None) from clap `#[arg(...)] pub field`."""
    flags = {}
    for m in re.finditer(r"#\[arg\((?P<attr>.*?)\)\]\s*pub\s+(?P<field>\w+)\s*:", src, re.S):
        attr, field = m.group("attr"), m.group("field")
        lm = re.search(r'long\s*=\s*"([^"]+)"', attr)
        name = lm.group(1) if lm else (field.replace("_", "-") if "long" in attr else None)
        if not name:
            continue
        dm = re.search(r"default_value_t\s*=\s*([A-Za-z0-9_]+)", attr)
        val = None
        if dm:
            tok = dm.group(1)
            val = defaults.get(tok, tok)
        flags["--" + name] = val
    return flags


def doc_flags_with_values(doc):
    """Return {flag: stated-int-value-or-None} for --enable-hop/--hop-*/--ipfs-* in the doc."""
    out = {}
    for m in re.finditer(r"(--(?:enable-hop|hop-[a-z-]+|ipfs-[a-z-]+))`?\s*:?\s*([0-9]+)?", doc):
        flag, val = m.group(1), m.group(2)
        if flag not in out or (out[flag] is None and val):
            out[flag] = int(val) if val else None
    return out


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    doc_path = sys.argv[1]
    doc = open(doc_path).read()
    commit = pinned_commit()
    print(f"### polkadot-sdk @ {commit[:12]} (from .github/env)")

    defaults = resolve_defaults(source_of(HOP_TYPES, commit))
    src_flags = {}
    src_flags.update(extract_arg_flags(source_of(HOP_CLI, commit), defaults))
    # only the ipfs_* flags from network params are Bulletin-relevant here
    for k, v in extract_arg_flags(source_of(NET_PARAMS, commit), defaults).items():
        if k.startswith("--ipfs"):
            src_flags[k] = v

    doc_flags = doc_flags_with_values(doc)
    print(f"### source defines {len(src_flags)} Bulletin/HOP flags; doc references {len(doc_flags)}\n")

    # Renamed/removed/missing flags fail the check; a doc value that differs from
    # the source default is a deliberate operator override, reported but not fatal.
    rc = 0
    print("FLAG                           SOURCE-DEFAULT   IN-DOC   DOC-VALUE   STATUS")
    for flag in sorted(src_flags):
        default = src_flags[flag]
        in_doc = flag in doc_flags
        docval = doc_flags.get(flag)
        status = "ok"
        if not in_doc:
            status = "MISSING FROM DOC"
            rc = 1
        elif docval is not None and default is not None and docval != default:
            status = f"override (source default={default})"
        print(f"{flag:<30} {str(default):<16} {'yes' if in_doc else 'no':<8} "
              f"{str(docval) if docval is not None else '-':<11} {status}")

    dead = sorted(f for f in doc_flags if f not in src_flags)
    print()
    if dead:
        rc = 1
        print("DEAD FLAGS IN DOC (not in source — will fail at startup):")
        for f in dead:
            print(f"  {f}")
    else:
        print("No dead flags in doc.")

    print("\nRESULT:", "PASS" if rc == 0 else "FAIL")
    sys.exit(rc)


if __name__ == "__main__":
    main()
