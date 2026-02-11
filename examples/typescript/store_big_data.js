/**
 * Store large files using the Bulletin TypeScript SDK
 *
 * This example demonstrates storing large files with automatic chunking,
 * parallel uploads, and DAG-PB manifest generation using the SDK client.
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
import pkg from '../../sdk/typescript/dist/index.js';
const { AsyncBulletinClient, PAPITransactionSubmitter } = pkg;
import {
    setupKeyringAndSigners,
    CHUNK_SIZE,
    DEFAULT_IPFS_GATEWAY_URL,
    fileToDisk,
    filesAreEqual,
    generateTextImage,
} from '../common.js';
import { authorizeAccount, fetchCid, TX_MODE_FINALIZED_BLOCK } from '../api.js';

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
        console.log('📝 Authorizing account...');
        await authorizeAccount(
            bulletinAPI,
            sudoSigner,
            whoSigner.address,
            100,
            BigInt(100 * 1024 * 1024), // 100 MiB
            TX_MODE_FINALIZED_BLOCK,
        );

        // Create SDK client with transaction submitter
        const submitter = new PAPITransactionSubmitter(bulletinAPI, whoSigner.signer);
        const sdkClient = new AsyncBulletinClient(submitter);

        // Read file
        const fileData = fs.readFileSync(filePath);
        console.log(`📁 File size: ${fileData.length} bytes`);

        // Store with automatic chunking and parallel uploads
        console.log('📤 Storing file with SDK (automatic chunking + parallel uploads)...');
        const result = await sdkClient
            .store(fileData)
            .withChunking({
                chunkSize: CHUNK_SIZE,
                maxParallel: 8,
                createManifest: true,
            })
            .withProgress((event) => {
                if (event.type === 'chunk_completed') {
                    console.log(`  ✓ Chunk ${event.chunkIndex + 1} stored (${event.bytesUploaded}/${event.totalBytes} bytes)`);
                }
            })
            .submit();

        console.log(`✅ Storage complete!`);
        console.log(`   Chunks stored: ${result.chunks?.length || 0}`);
        console.log(`   Manifest CID: ${result.manifestCid?.toString()}`);

        // Download and verify via DAG-PB manifest
        console.log('\n📥 Downloading via DAG-PB manifest...');
        const downloadedContent = await fetchCid(HTTP_IPFS_API, result.manifestCid);
        console.log(`✅ Downloaded: ${downloadedContent.length} bytes`);
        await fileToDisk(downloadedFileByDagPath, downloadedContent);
        filesAreEqual(filePath, downloadedFileByDagPath);
        assert.strictEqual(
            fileData.length,
            downloadedContent.length,
            '❌ Downloaded size mismatch!'
        );

        // Check all chunks are there (optional, can be slow/fail if IPFS doesn't cache chunks).
        if (!skipIpfsVerify) {
            console.log('\n📥 Downloading by individual chunks...');
            const downloadedChunks = [];
            for (const chunk of result.chunks) {
                const block = await ipfs.block.get(chunk.cid, { timeout: 15000 });
                downloadedChunks.push(block);
            }
            const fullBuffer = Buffer.concat(downloadedChunks);
            console.log(`✅ Reconstructed: ${fullBuffer.length} bytes`);
            await fileToDisk(downloadedByChunksPath, fullBuffer);
            filesAreEqual(filePath, downloadedByChunksPath);
            assert.strictEqual(
                fileData.length,
                fullBuffer.length,
                '❌ Reconstructed size mismatch!'
            );
        } else {
            console.log(`ℹ️  Skipping individual chunk download verification (--skip-ipfs-verify)`);
        }

        console.log('\n\n\n✅✅✅ Test passed! ✅✅✅');
        resultCode = 0;
    } catch (error) {
        console.error('❌ Error:', error);
        resultCode = 1;
    } finally {
        if (client) client.destroy();
        process.exit(resultCode);
    }
}

await main();
