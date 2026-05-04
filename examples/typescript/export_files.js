/**
 * Export file-level (DAG) aggregation from Paseo Bulletin Chain.
 *
 * For each stored file, outputs: original file size, chunk count, CID, etc.
 * Handles both standalone files (single tx) and chunked files (manifest + chunks).
 *
 * Usage:
 *   node typescript/export_files.js [ws_url]
 *
 * Outputs:
 *   - files.csv          (one row per logical file)
 *   - transactions.csv   (raw per-transaction data)
 */

import { createClient, Binary } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws';
import { bulletin } from './.papi/descriptors/dist/index.js';
import { writeFileSync } from 'node:fs';
import * as dagPB from '@ipld/dag-pb';
import { UnixFS } from 'ipfs-unixfs';
import { CID } from 'multiformats/cid';
import * as mfDigest from 'multiformats/hashes/digest';

const NODE_WS = process.argv[2] || 'wss://paseo-bulletin-rpc.polkadot.io';
const CONCURRENCY = 5; // keep low for public RPCs

const HASH_CODES = { Blake2b256: 0xb220, Sha2_256: 0x12, Keccak256: 0x1b };

function toHex(bytes) {
    return '0x' + [...bytes].map(b => b.toString(16).padStart(2, '0')).join('');
}

function hexToBytes(hex) {
    const clean = hex.startsWith('0x') ? hex.slice(2) : hex;
    const bytes = new Uint8Array(clean.length / 2);
    for (let i = 0; i < bytes.length; i++) {
        bytes[i] = parseInt(clean.substr(i * 2, 2), 16);
    }
    return bytes;
}

function buildCid(contentHashHex, hashingType, cidCodec) {
    const hashBytes = hexToBytes(contentHashHex);
    const code = HASH_CODES[hashingType];
    if (code === undefined) return null;
    return CID.createV1(cidCodec, mfDigest.create(code, hashBytes));
}

function escapeCsv(v) {
    const s = String(v);
    return s.includes(',') || s.includes('"') || s.includes('\n')
        ? '"' + s.replace(/"/g, '""') + '"' : s;
}

function writeCsv(filename, headers, rows) {
    const lines = [headers.join(',')];
    for (const row of rows) lines.push(row.map(escapeCsv).join(','));
    writeFileSync(filename, lines.join('\n') + '\n');
    console.log(`  Wrote ${rows.length} rows to ${filename}`);
}

/**
 * Scan raw extrinsic bytes for a SCALE-encoded Vec<u8> whose content
 * decodes as a valid DAG-PB file node with Links.
 *
 * Instead of parsing the extrinsic envelope (version, signature, extensions),
 * we scan for every possible compact-length prefix and try DAG-PB decode.
 * The manifest is typically 100-10000 bytes, so we filter by size.
 */
function findDagPbPayload(extBytes) {
    for (let i = 0; i < extBytes.length - 10; i++) {
        const mode = extBytes[i] & 0x03;
        let dataLen, headerLen;

        if (mode === 0) {
            dataLen = extBytes[i] >> 2;
            headerLen = 1;
        } else if (mode === 1) {
            if (i + 1 >= extBytes.length) continue;
            dataLen = ((extBytes[i + 1] << 8) | extBytes[i]) >> 2;
            headerLen = 2;
        } else if (mode === 2) {
            if (i + 3 >= extBytes.length) continue;
            dataLen = ((extBytes[i + 3] << 24) | (extBytes[i + 2] << 16) |
                       (extBytes[i + 1] << 8) | extBytes[i]) >>> 2;
            headerLen = 4;
        } else {
            continue;
        }

        // Manifest size is typically 50-10000 bytes
        if (dataLen < 20 || dataLen > 50000) continue;
        const start = i + headerLen;
        if (start + dataLen > extBytes.length) continue;

        const candidate = extBytes.slice(start, start + dataLen);
        try {
            const node = dagPB.decode(candidate);
            if (node.Links && node.Links.length > 0 && node.Data) {
                const unixfs = UnixFS.unmarshal(node.Data);
                if (unixfs.type === 'file') {
                    return { node, unixfs, data: candidate };
                }
            }
        } catch {
            // Not valid DAG-PB at this offset
        }
    }
    return null;
}

async function parallelMap(items, fn, concurrency) {
    const results = new Array(items.length);
    let idx = 0;
    async function worker() {
        while (idx < items.length) {
            const i = idx++;
            results[i] = await fn(items[i], i);
        }
    }
    await Promise.all(Array.from({ length: Math.min(concurrency, items.length) }, () => worker()));
    return results;
}

async function main() {
    console.log(`Connecting to: ${NODE_WS}`);
    const papiClient = createClient(getWsProvider(NODE_WS));
    const api = papiClient.getTypedApi(bulletin);

    try {
        // =========================================================
        // Phase 1: Fetch all transaction metadata
        // =========================================================
        console.log('\nPhase 1: Fetching all transaction metadata...');
        const txEntries = await api.query.TransactionStorage.Transactions.getEntries();

        const allTxs = [];
        const manifests = [];
        const rawEntries = [];

        for (const { keyArgs, value: infos } of txEntries) {
            const blockNumber = keyArgs[0];
            for (let i = 0; i < infos.length; i++) {
                const info = infos[i];
                const entry = {
                    blockNumber,
                    indexInBlock: i,
                    contentHash: toHex(info.content_hash),
                    hashingType: info.hashing.type,
                    cidCodec: Number(info.cid_codec),
                    size: info.size,
                    blockChunks: info.block_chunks,
                };
                allTxs.push(entry);
                if (entry.cidCodec === 0x70) manifests.push(entry);
                else rawEntries.push(entry);
            }
        }

        console.log(`  Total: ${allTxs.length}, Manifests: ${manifests.length}, Raw: ${rawEntries.length}`);

        // Write transactions.csv with CIDs
        writeCsv('transactions.csv', [
            'block_number', 'index_in_block', 'content_hash',
            'hashing_algorithm', 'cid_codec', 'size_bytes',
            'block_chunks', 'cid',
        ], allTxs.map(tx => {
            const cid = buildCid(tx.contentHash, tx.hashingType, tx.cidCodec);
            return [
                tx.blockNumber, tx.indexInBlock, tx.contentHash,
                tx.hashingType, tx.cidCodec, tx.size,
                tx.blockChunks, cid ? cid.toString() : '',
            ];
        }));

        // =========================================================
        // Phase 2: Fetch block bodies for manifests and decode DAG-PB
        // =========================================================
        console.log(`\nPhase 2: Decoding ${manifests.length} manifests from block bodies...`);

        // Group manifests by block to minimize RPC calls
        const manifestsByBlock = {};
        for (const m of manifests) {
            if (!manifestsByBlock[m.blockNumber]) manifestsByBlock[m.blockNumber] = [];
            manifestsByBlock[m.blockNumber].push(m);
        }
        const blockNumbers = Object.keys(manifestsByBlock).map(Number);
        console.log(`  Unique blocks to fetch: ${blockNumbers.length}`);

        let decoded = 0, failed = 0, fetched = 0;
        const manifestData = new Map(); // manifest contentHash -> { chunkCids, totalFileSize }

        // Process blocks sequentially in batches with retry to handle RPC drops
        const BATCH_SIZE = 50;
        for (let batchStart = 0; batchStart < blockNumbers.length; batchStart += BATCH_SIZE) {
            const batch = blockNumbers.slice(batchStart, batchStart + BATCH_SIZE);

            await parallelMap(batch, async (blockNum) => {
                for (let attempt = 0; attempt < 3; attempt++) {
                    try {
                        const blockHash = await papiClient._request('chain_getBlockHash', [blockNum]);
                        const block = await papiClient._request('chain_getBlock', [blockHash]);
                        const extrinsics = block.block.extrinsics;

                        fetched++;

                        for (const manifest of manifestsByBlock[blockNum]) {
                            let found = false;
                            for (const extHex of extrinsics) {
                                const extBytes = hexToBytes(extHex);
                                const result = findDagPbPayload(extBytes);
                                if (result) {
                                    const chunkCids = result.node.Links.map(l => l.Hash);
                                    const totalFileSize = Number(result.unixfs.fileSize());
                                    manifestData.set(manifest.contentHash, {
                                        chunkCids, totalFileSize, chunkCount: chunkCids.length,
                                    });
                                    decoded++;
                                    found = true;
                                    break;
                                }
                            }
                            if (!found) failed++;
                        }
                        return; // success
                    } catch (err) {
                        if (attempt === 2) {
                            for (const m of manifestsByBlock[blockNum]) failed++;
                        } else {
                            await new Promise(r => setTimeout(r, 1000 * (attempt + 1)));
                        }
                    }
                }
            }, CONCURRENCY);

            process.stdout.write(`  Progress: ${Math.min(batchStart + BATCH_SIZE, blockNumbers.length)}/${blockNumbers.length} blocks, ${decoded} decoded\r`);
        }

        console.log(`\n  Decoded: ${decoded}, Failed: ${failed}`);

        // =========================================================
        // Phase 3: Build files.csv
        // =========================================================
        console.log('\nPhase 3: Building files.csv...');

        // Track which content hashes are chunks
        const chunkContentHashes = new Set();
        for (const [, data] of manifestData) {
            for (const cid of data.chunkCids) {
                const hashHex = '0x' + [...cid.multihash.digest]
                    .map(b => b.toString(16).padStart(2, '0')).join('');
                chunkContentHashes.add(hashHex);
            }
        }

        const fileRows = [];

        // Chunked files
        for (const m of manifests) {
            const data = manifestData.get(m.contentHash);
            const manifestCid = buildCid(m.contentHash, m.hashingType, m.cidCodec);

            if (data) {
                fileRows.push([
                    'chunked',
                    manifestCid ? manifestCid.toString() : '',
                    data.totalFileSize,
                    data.chunkCount,
                    m.size,
                    data.totalFileSize + m.size,
                    m.blockNumber,
                    m.contentHash,
                ]);
            } else {
                fileRows.push([
                    'chunked (undecoded)',
                    manifestCid ? manifestCid.toString() : '',
                    '?', '?', m.size, '?',
                    m.blockNumber,
                    m.contentHash,
                ]);
            }
        }

        // Standalone files (raw, not referenced by any manifest)
        for (const tx of rawEntries) {
            if (!chunkContentHashes.has(tx.contentHash)) {
                const cid = buildCid(tx.contentHash, tx.hashingType, tx.cidCodec);
                fileRows.push([
                    'standalone',
                    cid ? cid.toString() : '',
                    tx.size, 1, 0, tx.size,
                    tx.blockNumber,
                    tx.contentHash,
                ]);
            }
        }

        writeCsv('files.csv', [
            'file_type', 'cid', 'file_size_bytes', 'chunk_count',
            'manifest_size_bytes', 'total_on_chain_bytes',
            'block_number', 'content_hash',
        ], fileRows);

        // =========================================================
        // Summary
        // =========================================================
        const standalone = fileRows.filter(r => r[0] === 'standalone');
        const chunked = fileRows.filter(r => r[0] === 'chunked');
        const undecoded = fileRows.filter(r => r[0] === 'chunked (undecoded)');

        console.log(`\n=== File-level Summary ===`);
        console.log(`Standalone files: ${standalone.length}`);
        console.log(`Chunked files (decoded): ${chunked.length}`);
        console.log(`Chunked files (undecoded): ${undecoded.length}`);
        console.log(`Chunks matched to manifests: ${chunkContentHashes.size}`);

        if (chunked.length > 0) {
            const sizes = chunked.map(r => Number(r[2]));
            const chunks = chunked.map(r => Number(r[3]));
            console.log(`\nChunked files:`);
            console.log(`  Total data: ${(sizes.reduce((a, b) => a + b, 0) / 1024 / 1024).toFixed(1)} MiB`);
            console.log(`  Largest: ${(Math.max(...sizes) / 1024 / 1024).toFixed(1)} MiB`);
            console.log(`  Avg chunks/file: ${(chunks.reduce((a, b) => a + b, 0) / chunks.length).toFixed(1)}`);
        }
        if (standalone.length > 0) {
            const sizes = standalone.map(r => Number(r[2]));
            console.log(`\nStandalone files:`);
            console.log(`  Total data: ${(sizes.reduce((a, b) => a + b, 0) / 1024 / 1024).toFixed(1)} MiB`);
            console.log(`  Avg size: ${(sizes.reduce((a, b) => a + b, 0) / sizes.length / 1024).toFixed(1)} KiB`);
        }
    } finally {
        papiClient.destroy();
    }
}

await main();
