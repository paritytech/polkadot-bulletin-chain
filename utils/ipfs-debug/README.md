# IPFS gateway debug tools

Small tools for tracing a missing IPFS CID: deciding whether it is on the
Bulletin chain at all, and (for web content) which dotNS app references it.
Built while investigating a 504 storm on the Paseo bulletin gateway, where
Polkadot Desktop polled one missing CID once a minute for hours.

## `bulletin-has-cid.mjs` — is this CID stored on Bulletin?

The authoritative check. `pallet-transaction-storage::store` content-addresses
with blake2b + raw codec, so a `bafk2bzace…` CID maps directly to an on-chain
`content_hash`. This queries `TransactionByContentHash` for that digest.

```bash
# extract the 32-byte blake2b digest from the CID first (see find_cid_parent.cid_parts),
# then:
node bulletin-has-cid.mjs 0x<content-hash>
# -> STORED: yes — block #…, index …    (within retention)
# -> STORED: no  — expired or never stored
```

Needs a papi project with the `bulletin` descriptor generated
(`@polkadot-api/descriptors`) and `polkadot-api` installed. Env: `BULLETIN_RPC`
(default `wss://paseo-bulletin-next-rpc.polkadot.io`).

## `find_cid_parent.py` — which bundle contains a leaf?

Given a gateway and dag-pb root CIDs, walks each root's tree and reports whether
a target CID is a descendant — without fetching the (possibly missing) target,
since it matches on the parent's link list.

```bash
python3 find_cid_parent.py --gateway https://paseo-bulletin-next-ipfs.polkadot.io \
  --target <cid> <rootCID1> <rootCID2> ...
```

Fetches raw blocks (`?format=raw`) and parses dag-pb locally. Matches CIDs by
(codec, multihash) so v0/v1 and base differences don't cause misses. Needs
`curl` (the system Python's TLS is too old for some gateways).

## `find-app-for-cid.sh` — which dotNS app references a CID?

Resolves candidate `.dot` names to bundle roots via the `dotns` CLI, then walks
each bundle with `find_cid_parent.py`.

```bash
./find-app-for-cid.sh <cid> bulletin-benchmarks dotns my-app ...
```

Env: `DOTNS` (how to invoke the CLI), `KEY_URI` (any address-mapped account;
reads are dry-runs), `GATEWAY`. dotNS has no on-chain "list all names", so you
supply candidate names (playground/demo apps, an indexer, or known owners via
`dotns lookup`).

## Hash types matter

dotNS web-app bundles use sha256 leaves (`bafkrei…`). Bulletin-native content
(preimages, `dotns bulletin upload` blobs) uses blake2b (`bafk2bzace…`). A
blake2b leaf will not be found inside a sha256 web bundle, so for those start
with `bulletin-has-cid.mjs` (chain truth) rather than walking dotNS bundles.
