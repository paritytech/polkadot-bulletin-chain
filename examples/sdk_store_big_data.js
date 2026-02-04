// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * SDK Store Big Data Test
 *
 * This test demonstrates using the TypeScript SDK (AsyncBulletinClient) to:
 * - Store a large file (64MB) with automatic chunking
 * - Track upload performance metrics
 * - Validate data retrieval via IPFS
 * - Compare with PAPI and Rust SDK implementations
 */

import { cryptoWaitReady } from '@polkadot/util-crypto';
import { create } from 'ipfs-http-client';
import fs from 'fs';
import os from 'os';
import path from 'path';
import assert from 'assert';
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { bulletin } from './typescript/.papi/descriptors/dist/index.mjs';
import {
    setupKeyringAndSigners,
    HTTP_IPFS_API,
    generateTextImage,
    fileToDisk,
    filesAreEqual,
    waitForChainReady,
} from './typescript/common.js';
import { AsyncBulletinClient, Binary } from '../sdk/typescript/dist/index.js';
import { PerformanceMetrics } from './metrics.js';

// Command line arguments
const args = process.argv.slice(2).filter(arg => !arg.startsWith('--'));
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Alice';

// Performance metrics
const metrics = new PerformanceMetrics();

// Connect to local IPFS gateway
const ipfs = create({
    url: 'http://127.0.0.1:5001',
});

async function main() {
    await cryptoWaitReady();

    console.log('\n╔════════════════════════════════════════════════╗');
    console.log('║    TypeScript SDK - Store Big Data Test       ║');
    console.log('╚════════════════════════════════════════════════╝\n');
    console.log(`📡 Connecting to: ${NODE_WS}`);
    console.log(`🔑 Using seed: ${SEED}\n`);

    let client, resultCode;
    try {
        // Create temp files
        const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'bulletin-sdk-test-'));
        const filePath = path.join(tmpDir, 'test-image.jpeg');
        const downloadedFilePath = path.join(tmpDir, 'downloaded-via-dag.jpeg');
        const downloadedChunksPath = path.join(tmpDir, 'downloaded-via-chunks.jpeg');

        // Generate test image (~64MB)
        console.log('🎨 Generating test image (~64MB)...');
        generateTextImage(filePath, 'SDK Test - ' + new Date().toISOString(), 'big');

        const fileData = fs.readFileSync(filePath);
        metrics.setFileSize(fileData.length);
        console.log(`📁 Test file size: ${(fileData.length / 1024 / 1024).toFixed(2)} MB\n`);

        // Initialize PAPI client
        console.log('🔧 Initializing Polkadot API client...');
        client = createClient(getWsProvider(NODE_WS));
        const bulletinAPI = client.getTypedApi(bulletin);

        // Wait for chain to be ready
        await waitForChainReady(bulletinAPI);

        const { sudoSigner, whoSigner, whoAddress } = setupKeyringAndSigners(SEED, '//SDKTester');

        // Create SDK client (directly with PAPI client and signer)
        console.log('🚀 Initializing AsyncBulletinClient...');
        const sdkClient = new AsyncBulletinClient(bulletinAPI, whoSigner, {
            defaultChunkSize: 1024 * 1024, // 1 MiB chunks
            maxParallel: 8,
            createManifest: true,
            chunkingThreshold: 2 * 1024 * 1024, // 2 MiB threshold
            checkAuthorizationBeforeUpload: true,
        });

        // Estimate authorization needed
        const estimate = sdkClient.estimateAuthorization(fileData.length);
        console.log('📊 Authorization estimate:');
        console.log(`   Transactions: ${estimate.transactions}`);
        console.log(`   Bytes: ${estimate.bytes} (${(estimate.bytes / 1024 / 1024).toFixed(2)} MB)\n`);

        // Authorize account
        console.log('🔐 Authorizing account...');
        await sdkClient.authorizeAccount(
            whoAddress,
            estimate.transactions + 10, // Add buffer
            BigInt(estimate.bytes + 10 * 1024 * 1024) // Add 10MB buffer
        );
        console.log('✅ Authorization complete\n');

        // Store file with SDK
        console.log('⏳ Uploading file with SDK (automatic chunking)...\n');
        let chunksCompleted = 0;

        metrics.startUpload();

        const result = await sdkClient.store(
            fileData,
            undefined, // use default options
            (event) => {
                switch (event.type) {
                    case 'chunk_started':
                        if (chunksCompleted === 0) {
                            metrics.setNumChunks(event.total);
                            console.log(`   📦 Chunking into ${event.total} chunks...`);
                        }
                        break;

                    case 'chunk_completed':
                        chunksCompleted++;
                        const progress = (chunksCompleted / event.total) * 100;
                        process.stdout.write(`\r   ✓ Uploaded: ${chunksCompleted}/${event.total} chunks (${progress.toFixed(1)}%)`);
                        break;

                    case 'chunk_failed':
                        console.error(`\n   ✗ Chunk ${event.index + 1} failed:`, event.error.message);
                        break;

                    case 'manifest_created':
                        console.log('\n   📋 DAG-PB manifest created');
                        break;

                    case 'completed':
                        console.log('\n   ✅ Upload complete!');
                        break;
                }
            }
        );

        metrics.endUpload();

        console.log('\n╔════════════════════════════════════════════════╗');
        console.log('║            Upload Performance Metrics          ║');
        console.log('╚════════════════════════════════════════════════╝');
        console.log(`📝 Final CID:      ${result.cid.toString()}`);
        if (result.chunks) {
            console.log(`🔗 Manifest CID:   ${result.chunks.manifestCid?.toString() || 'N/A'}`);
            console.log(`📑 Chunk CIDs:     ${result.chunks.chunkCids.length} CIDs`);
        }
        metrics.print();

        // Validate retrieval via IPFS
        console.log('🔍 Validating data retrieval...\n');

        // Test 1: Download via manifest CID (DAG-PB)
        if (result.chunks?.manifestCid) {
            console.log('   📥 Downloading via manifest CID...');
            const retrievalStart = Date.now();

            const manifestCid = result.chunks.manifestCid;
            const downloadedChunks = [];

            // Use IPFS to get the full file via manifest
            for await (const chunk of ipfs.cat(manifestCid, { timeout: 30000 })) {
                downloadedChunks.push(chunk);
            }

            const downloadedContent = Buffer.concat(downloadedChunks);
            const retrievalDuration = Date.now() - retrievalStart;
            metrics.setRetrievalDuration(retrievalDuration);

            console.log(`   ✓ Downloaded ${downloadedContent.length} bytes in ${(retrievalDuration / 1000).toFixed(2)}s`);

            await fileToDisk(downloadedFilePath, downloadedContent);
            filesAreEqual(filePath, downloadedFilePath);

            assert.strictEqual(
                fileData.length,
                downloadedContent.length,
                '❌ Downloaded file size mismatch!'
            );
            console.log('   ✅ Content matches original file (via manifest)\n');
        }

        // Test 2: Download individual chunks and reassemble
        if (result.chunks?.chunkCids) {
            console.log('   📥 Downloading individual chunks...');
            const chunkDownloadStart = Date.now();

            const downloadedChunks = [];
            for (const chunkCid of result.chunks.chunkCids) {
                const block = await ipfs.block.get(chunkCid, { timeout: 15000 });
                downloadedChunks.push(block);
            }

            const fullBuffer = Buffer.concat(downloadedChunks);
            const chunkDownloadDuration = (Date.now() - chunkDownloadStart) / 1000;

            console.log(`   ✓ Downloaded ${result.chunks.chunkCids.length} chunks in ${chunkDownloadDuration.toFixed(2)}s`);

            await fileToDisk(downloadedChunksPath, fullBuffer);
            filesAreEqual(filePath, downloadedChunksPath);

            assert.strictEqual(
                fileData.length,
                fullBuffer.length,
                '❌ Reassembled file size mismatch!'
            );
            console.log('   ✅ Content matches original file (via chunks)\n');
        }

        console.log('\n✅✅✅ TypeScript SDK Test PASSED! ✅✅✅\n');
        resultCode = 0;
    } catch (error) {
        console.error('\n❌ Test FAILED:', error);
        console.error(error.stack);
        resultCode = 1;
    } finally {
        if (client) client.destroy();
        process.exit(resultCode);
    }
}

await main();
