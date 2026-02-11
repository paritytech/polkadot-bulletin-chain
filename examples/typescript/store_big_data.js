/**
 * Store large files using the Bulletin TypeScript SDK
 *
 * This example demonstrates the SDK's chunking, CID calculation, and DAG-PB
 * manifest generation capabilities. Transaction submission uses PAPI directly
 * since the SDK's store().send() is not yet fully implemented.
 *
 * SDK features used:
 * - FixedSizeChunker: splits data into chunks
 * - calculateCid: computes CIDs for chunks
 * - UnixFsDagBuilder: creates IPFS-compatible DAG-PB manifest
 */

import { cryptoWaitReady } from '@polkadot/util-crypto';
import { create } from 'ipfs-http-client';
import fs from 'fs';
import os from 'os';
import path from 'path';
import assert from 'assert';
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { bulletin } from '../.papi/descriptors/dist/index.mjs';
import { FixedSizeChunker, UnixFsDagBuilder, calculateCid } from '../../sdk/typescript/dist/index.mjs';
import {
    setupKeyringAndSigners,
    CHUNK_SIZE,
    DEFAULT_IPFS_GATEWAY_URL,
    fileToDisk,
    filesAreEqual,
    generateTextImage,
} from '../common.js';
import { authorizeAccount, store, fetchCid, TX_MODE_FINALIZED_BLOCK } from '../api.js';

// Command line arguments
const args = process.argv.slice(2).filter(arg => !arg.startsWith('--'));
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Alice';
const HTTP_IPFS_API = args[2] || DEFAULT_IPFS_GATEWAY_URL;

// Connect to local IPFS gateway
const ipfs = create({ url: 'http://127.0.0.1:5001' });

// Optional flags
const skipIpfsVerify = process.argv.includes("--skip-ipfs-verify");

async function main() {
    await cryptoWaitReady();

    console.log(`Connecting to: ${NODE_WS}`);
    console.log(`Using seed: ${SEED}`);

    let client, resultCode;
    try {
        const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'bulletinimggen-'));
        const filePath = path.join(tmpDir, 'image.jpeg');
        const downloadedFileByDagPath = path.join(tmpDir, 'downloadedByDag.jpeg');
        const downloadedByChunksPath = path.join(tmpDir, 'downloadedByChunks.jpeg');

        generateTextImage(filePath, 'Hello, Bulletin SDK - ' + new Date().toString(), 'big64');

        // Initialize PAPI client
        client = createClient(getWsProvider(NODE_WS));
        const bulletinAPI = client.getTypedApi(bulletin);
        const { sudoSigner, whoSigner } = setupKeyringAndSigners(SEED, '//SDKSigner');

        // Authorize account for storage
        console.log('Authorizing account...');
        await authorizeAccount(
            bulletinAPI,
            sudoSigner,
            whoSigner.address,
            100,
            BigInt(100 * 1024 * 1024), // 100 MiB
            TX_MODE_FINALIZED_BLOCK,
        );

        // Read file
        const fileData = fs.readFileSync(filePath);
        console.log(`File size: ${fileData.length} bytes`);

        // Step 1: Use SDK chunker to split data
        console.log('Chunking file with SDK FixedSizeChunker...');
        const chunker = new FixedSizeChunker({ chunkSize: CHUNK_SIZE });
        const chunks = chunker.chunk(fileData);
        console.log(`  Chunks: ${chunks.length} (${CHUNK_SIZE} bytes each)`);

        // Step 2: Calculate CIDs for each chunk using SDK
        console.log('Calculating CIDs with SDK...');
        for (const chunk of chunks) {
            chunk.cid = await calculateCid(chunk.data);
        }

        // Step 3: Submit each chunk via PAPI
        console.log('Storing chunks on chain via PAPI...');
        for (let i = 0; i < chunks.length; i++) {
            const chunk = chunks[i];
            await store(bulletinAPI, whoSigner.signer, chunk.data, null, null, TX_MODE_FINALIZED_BLOCK);
            console.log(`  Chunk ${i + 1}/${chunks.length} stored (CID: ${chunk.cid.toString()})`);
        }

        // Step 4: Build DAG-PB manifest using SDK
        console.log('Building DAG-PB manifest with SDK UnixFsDagBuilder...');
        const dagBuilder = new UnixFsDagBuilder();
        const manifest = await dagBuilder.build(chunks);
        console.log(`  Manifest CID: ${manifest.rootCid.toString()}`);
        console.log(`  Manifest size: ${manifest.dagBytes.length} bytes`);

        // Step 5: Submit manifest via PAPI
        console.log('Storing manifest on chain...');
        await store(bulletinAPI, whoSigner.signer, manifest.dagBytes, null, null, TX_MODE_FINALIZED_BLOCK);

        console.log('Storage complete!');
        console.log(`   Chunks stored: ${chunks.length}`);
        console.log(`   Manifest CID: ${manifest.rootCid.toString()}`);

        // Download and verify via DAG-PB manifest
        console.log('\nDownloading via DAG-PB manifest...');
        const downloadedContent = await fetchCid(HTTP_IPFS_API, manifest.rootCid);
        console.log(`Downloaded: ${downloadedContent.length} bytes`);
        await fileToDisk(downloadedFileByDagPath, downloadedContent);
        filesAreEqual(filePath, downloadedFileByDagPath);
        assert.strictEqual(
            fileData.length,
            downloadedContent.length,
            'Downloaded size mismatch!'
        );

        // Check all chunks are there (optional, can be slow/fail if IPFS doesn't cache chunks).
        if (!skipIpfsVerify) {
            console.log('\nDownloading by individual chunks...');
            const downloadedChunks = [];
            for (const chunk of chunks) {
                const block = await ipfs.block.get(chunk.cid, { timeout: 15000 });
                downloadedChunks.push(block);
            }
            const fullBuffer = Buffer.concat(downloadedChunks);
            console.log(`Reconstructed: ${fullBuffer.length} bytes`);
            await fileToDisk(downloadedByChunksPath, fullBuffer);
            filesAreEqual(filePath, downloadedByChunksPath);
            assert.strictEqual(
                fileData.length,
                fullBuffer.length,
                'Reconstructed size mismatch!'
            );
        } else {
            console.log('Skipping individual chunk download verification (--skip-ipfs-verify)');
        }

        console.log('\n\nTest passed!');
        resultCode = 0;
    } catch (error) {
        console.error('Error:', error);
        resultCode = 1;
    } finally {
        if (client) client.destroy();
        process.exit(resultCode);
    }
}

await main();
