// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

import assert from 'assert';
import { getContentHash } from './common.js';

export async function fetchCid(httpIpfsApi, cid) {
    const contentUrl = `${httpIpfsApi}/ipfs/${cid.toString()}`;
    console.log('⬇️ Downloading the full content (no chunking) by cid from url: ', contentUrl);
    const res = await fetch(contentUrl);
    if (!res.ok) throw new Error(`HTTP error ${res.status}`);
    return Buffer.from(await res.arrayBuffer())
}

/**
 * Fetch the single raw block for `cid` from the IPFS gateway via `?format=raw`.
 *
 * Unlike `fetchCid`, this never traverses dag-pb links: for a dag-pb CID the
 * gateway returns the root node bytes, not the assembled file, so the result
 * hashes back to the CID for any codec.
 */
export async function fetchCidRaw(httpIpfsApi, cid) {
    const contentUrl = `${httpIpfsApi}/ipfs/${cid.toString()}?format=raw`;
    console.log('⬇️ Downloading raw block by cid from url: ', contentUrl);
    const res = await fetch(contentUrl);
    if (!res.ok) throw new Error(`HTTP error ${res.status}`);
    return Buffer.from(await res.arrayBuffer())
}

/**
 * Fetch the stored block directly from the node via the `bitswap_v1_get` RPC.
 */
export async function fetchCidFromNode(client, cid) {
    console.log('⬇️ Downloading content by cid from node via bitswap_v1_get: ', cid.toString());
    // `client` is a raw PAPI client (`_request`) or the SDK BulletinClient (`request`).
    const request = (client._request ?? client.request).bind(client);
    const hex = await request('bitswap_v1_get', [cid.toString()]);
    return Buffer.from(hex.replace(/^0x/, ''), 'hex');
}

/**
 * Hash `data` with the algorithm the CID encodes and assert it matches the CID's
 * digest. This is the actual integrity check: a CID always hashes its own block
 * (raw bytes, or the dag-pb root node), never a composed file. `source` labels
 * the origin of the data in the failure message.
 */
export function verifyCid(data, cid, source = 'data') {
    const expected = Buffer.from(cid.multihash.digest);
    const actual = Buffer.from(getContentHash(data, cid.multihash.code));
    assert(
        actual.equals(expected),
        `❌ ${source} does not hash to CID ${cid.toString()}: ` +
        `got ${actual.toString('hex')}, expected ${expected.toString('hex')}`,
    );
}

/** A retrieval path: a `name` plus an async `(cid) => Buffer` fetcher of the raw block. */
export const gatewaySource = (httpIpfsApi) =>
    ({ name: `ipfs-gateway ${httpIpfsApi}`, fetch: (cid) => fetchCidRaw(httpIpfsApi, cid) });

export const nodeRpcSource = (client, label = 'bitswap-rpc') =>
    ({ name: label, fetch: (cid) => fetchCidFromNode(client, cid) });

/** Smoldot serves the same `bitswap_v1_get`, fetching the block from its peers over p2p bitswap. */
export const smoldotRpcSource = (client) => nodeRpcSource(client, 'smoldot-bitswap-rpc');

/**
 * Fetch the block `cid` addresses from every source and assert each hashes to the
 * CID. With a single source this is a plain hash-verified fetch; with several it
 * is also a cross-check — verifying every block against the CID proves they are
 * byte-identical (equal digests under a collision-resistant hash), so no separate
 * cross-source compare is needed. Returns the block, the raw node for dag-pb CIDs,
 * not the assembled file (use `fetchCid` for that).
 */
export async function fetchAndVerifyBlock(cid, ...sources) {
    assert(sources.length >= 1, `verification needs >=1 source, got ${sources.length}`);
    const blocks = await Promise.all(
        sources.map(async (s) => ({ name: s.name, data: await s.fetch(cid) })),
    );
    for (const { name, data } of blocks) verifyCid(data, cid, name);
    return blocks[0].data;
}
