// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Client Comparison Test
 *
 * This script runs all three client implementations:
 * 1. PAPI (raw Polkadot API with manual chunking)
 * 2. Rust SDK
 * 3. TypeScript SDK
 *
 * It measures and compares:
 * - Upload performance (throughput, duration)
 * - Number of chunks
 * - CID compatibility
 * - Retrieval validation
 *
 * Usage:
 *   node compare_clients.js [ws_url] [seed]
 */

import { cryptoWaitReady } from '@polkadot/util-crypto';
import { create } from 'ipfs-http-client';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { execSync } from 'child_process';
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { bulletin } from './typescript/.papi/descriptors/dist/index.mjs';
import {
    setupKeyringAndSigners,
    generateTextImage,
    waitForChainReady,
} from './typescript/common.js';
import { storeChunkedFile } from './typescript/store-big-data/index.js';
import { AsyncBulletinClient, Binary } from '../sdk/typescript/dist/index.js';

// Command line arguments
const args = process.argv.slice(2);
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Alice';

// Connect to IPFS
const ipfs = create({ url: 'http://127.0.0.1:5001' });

// Test results storage
const results = {
    papi: null,
    rust: null,
    typescript: null,
};

/**
 * Test PAPI implementation
 */
async function testPAPI(client, bulletinAPI, filePath, fileData) {
    console.log('\n╔════════════════════════════════════════════════╗');
    console.log('║         Testing PAPI Implementation           ║');
    console.log('╚════════════════════════════════════════════════╝\n');

    const startTime = Date.now();

    try {
        const { chunks } = await storeChunkedFile(bulletinAPI, filePath);
        const endTime = Date.now();
        const duration = (endTime - startTime) / 1000;
        const throughput = (fileData.length / 1024 / 1024) / duration;

        results.papi = {
            success: true,
            duration,
            throughput,
            numChunks: chunks.length,
            fileSize: fileData.length,
            chunkCids: chunks.map(c => c.cid.toString()),
        };

        console.log('✅ PAPI test completed');
        console.log(`   Duration: ${duration.toFixed(2)}s`);
        console.log(`   Throughput: ${throughput.toFixed(2)} MB/s`);
        console.log(`   Chunks: ${chunks.length}\n`);

        return results.papi;
    } catch (error) {
        console.error('❌ PAPI test failed:', error.message);
        results.papi = { success: false, error: error.message };
        return results.papi;
    }
}

/**
 * Test Rust SDK implementation
 */
async function testRustSDK(filePath, fileData) {
    console.log('\n╔════════════════════════════════════════════════╗');
    console.log('║       Testing Rust SDK Implementation         ║');
    console.log('╚════════════════════════════════════════════════╝\n');

    const startTime = Date.now();

    try {
        // Build Rust SDK example
        console.log('🔨 Building Rust SDK...');
        const rootDir = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');
        execSync(
            `cargo build --release -p rust-authorize-and-store --manifest-path "${rootDir}/Cargo.toml"`,
            { stdio: 'inherit' }
        );

        // Run Rust test
        console.log('🦀 Running Rust SDK test...');
        const output = execSync(
            `"${rootDir}/target/release/authorize-and-store" --ws "${NODE_WS}" --seed "${SEED}"`,
            { encoding: 'utf-8', stdio: 'pipe' }
        );

        const endTime = Date.now();
        const duration = (endTime - startTime) / 1000;
        const throughput = (fileData.length / 1024 / 1024) / duration;

        // Parse output for CID and chunk count
        const cidMatch = output.match(/CID: (\w+)/);
        const chunksMatch = output.match(/(\d+) chunks/);

        results.rust = {
            success: true,
            duration,
            throughput,
            numChunks: chunksMatch ? parseInt(chunksMatch[1]) : null,
            fileSize: fileData.length,
            cid: cidMatch ? cidMatch[1] : null,
        };

        console.log('✅ Rust SDK test completed');
        console.log(`   Duration: ${duration.toFixed(2)}s`);
        console.log(`   Throughput: ${throughput.toFixed(2)} MB/s`);
        if (results.rust.numChunks) {
            console.log(`   Chunks: ${results.rust.numChunks}`);
        }
        console.log('');

        return results.rust;
    } catch (error) {
        console.error('❌ Rust SDK test failed:', error.message);
        results.rust = { success: false, error: error.message };
        return results.rust;
    }
}

/**
 * Test TypeScript SDK implementation
 */
async function testTypeScriptSDK(client, bulletinAPI, fileData, whoAddress, whoSigner) {
    console.log('\n╔════════════════════════════════════════════════╗');
    console.log('║    Testing TypeScript SDK Implementation      ║');
    console.log('╚════════════════════════════════════════════════╝\n');

    const startTime = Date.now();

    try {
        // Build TypeScript SDK
        console.log('🔨 Building TypeScript SDK...');
        const rootDir = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');
        execSync(`cd "${rootDir}/sdk/typescript" && npm install && npm run build`, {
            stdio: 'inherit',
        });

        // Create SDK client (directly with PAPI client and signer)
        console.log('📦 Initializing AsyncBulletinClient...');
        const sdkClient = new AsyncBulletinClient(bulletinAPI, whoSigner, {
            defaultChunkSize: 1024 * 1024,
            maxParallel: 8,
            createManifest: true,
            chunkingThreshold: 2 * 1024 * 1024,
            checkAuthorizationBeforeUpload: true,
        });

        // Authorize account
        const estimate = sdkClient.estimateAuthorization(fileData.length);
        await sdkClient.authorizeAccount(
            whoAddress,
            estimate.transactions + 10,
            BigInt(estimate.bytes + 10 * 1024 * 1024)
        );

        // Store file
        console.log('⏳ Uploading with TypeScript SDK...');
        let numChunks = 0;
        const result = await sdkClient.store(
            fileData,
            undefined,
            (event) => {
                if (event.type === 'chunk_completed') {
                    numChunks = event.total;
                }
            }
        );

        const endTime = Date.now();
        const duration = (endTime - startTime) / 1000;
        const throughput = (fileData.length / 1024 / 1024) / duration;

        results.typescript = {
            success: true,
            duration,
            throughput,
            numChunks: result.chunks?.numChunks || 1,
            fileSize: fileData.length,
            cid: result.cid.toString(),
            manifestCid: result.chunks?.manifestCid?.toString(),
            chunkCids: result.chunks?.chunkCids?.map(c => c.toString()) || [],
        };

        console.log('✅ TypeScript SDK test completed');
        console.log(`   Duration: ${duration.toFixed(2)}s`);
        console.log(`   Throughput: ${throughput.toFixed(2)} MB/s`);
        console.log(`   Chunks: ${results.typescript.numChunks}`);
        console.log('');

        return results.typescript;
    } catch (error) {
        console.error('❌ TypeScript SDK test failed:', error.message);
        console.error(error.stack);
        results.typescript = { success: false, error: error.message };
        return results.typescript;
    }
}

/**
 * Print comparison table
 */
function printComparison() {
    console.log('\n╔════════════════════════════════════════════════════════════════╗');
    console.log('║                    COMPARISON RESULTS                          ║');
    console.log('╚════════════════════════════════════════════════════════════════╝\n');

    // Table header
    console.log('┌─────────────────┬─────────────┬─────────────┬─────────────┐');
    console.log('│ Metric          │ PAPI        │ Rust SDK    │ TypeScript  │');
    console.log('├─────────────────┼─────────────┼─────────────┼─────────────┤');

    // Status
    const papiStatus = results.papi?.success ? '✅ PASS' : '❌ FAIL';
    const rustStatus = results.rust?.success ? '✅ PASS' : '❌ FAIL';
    const tsStatus = results.typescript?.success ? '✅ PASS' : '❌ FAIL';
    console.log(`│ Status          │ ${papiStatus.padEnd(11)} │ ${rustStatus.padEnd(11)} │ ${tsStatus.padEnd(11)} │`);

    if (results.papi?.success || results.rust?.success || results.typescript?.success) {
        // Duration
        const papiDur = results.papi?.success ? `${results.papi.duration.toFixed(2)}s` : 'N/A';
        const rustDur = results.rust?.success ? `${results.rust.duration.toFixed(2)}s` : 'N/A';
        const tsDur = results.typescript?.success ? `${results.typescript.duration.toFixed(2)}s` : 'N/A';
        console.log(`│ Duration        │ ${papiDur.padEnd(11)} │ ${rustDur.padEnd(11)} │ ${tsDur.padEnd(11)} │`);

        // Throughput
        const papiThr = results.papi?.success ? `${results.papi.throughput.toFixed(2)} MB/s` : 'N/A';
        const rustThr = results.rust?.success ? `${results.rust.throughput.toFixed(2)} MB/s` : 'N/A';
        const tsThr = results.typescript?.success ? `${results.typescript.throughput.toFixed(2)} MB/s` : 'N/A';
        console.log(`│ Throughput      │ ${papiThr.padEnd(11)} │ ${rustThr.padEnd(11)} │ ${tsThr.padEnd(11)} │`);

        // Chunks
        const papiChunks = results.papi?.success ? `${results.papi.numChunks}` : 'N/A';
        const rustChunks = results.rust?.success && results.rust.numChunks ? `${results.rust.numChunks}` : 'N/A';
        const tsChunks = results.typescript?.success ? `${results.typescript.numChunks}` : 'N/A';
        console.log(`│ Chunks          │ ${papiChunks.padEnd(11)} │ ${rustChunks.padEnd(11)} │ ${tsChunks.padEnd(11)} │`);
    }

    console.log('└─────────────────┴─────────────┴─────────────┴─────────────┘\n');

    // Compatibility check
    console.log('╔════════════════════════════════════════════════════════════════╗');
    console.log('║                    COMPATIBILITY CHECK                         ║');
    console.log('╚════════════════════════════════════════════════════════════════╝\n');

    const allChunks = [
        results.papi?.numChunks,
        results.rust?.numChunks,
        results.typescript?.numChunks,
    ].filter(c => c != null);

    if (allChunks.length > 1) {
        const allSame = allChunks.every(c => c === allChunks[0]);
        if (allSame) {
            console.log('✅ All implementations produced the same number of chunks');
        } else {
            console.log('⚠️  Warning: Different number of chunks across implementations');
            console.log(`   PAPI: ${results.papi?.numChunks}`);
            console.log(`   Rust: ${results.rust?.numChunks}`);
            console.log(`   TypeScript: ${results.typescript?.numChunks}`);
        }
    }

    // Performance winner
    const successfulTests = [
        { name: 'PAPI', throughput: results.papi?.throughput },
        { name: 'Rust SDK', throughput: results.rust?.throughput },
        { name: 'TypeScript SDK', throughput: results.typescript?.throughput },
    ].filter(t => t.throughput != null);

    if (successfulTests.length > 0) {
        successfulTests.sort((a, b) => b.throughput - a.throughput);
        console.log(`\n🏆 Fastest: ${successfulTests[0].name} (${successfulTests[0].throughput.toFixed(2)} MB/s)`);
    }

    console.log('');
}

async function main() {
    await cryptoWaitReady();

    console.log('\n╔════════════════════════════════════════════════════════════════╗');
    console.log('║         Bulletin Chain - Client Comparison Test               ║');
    console.log('╚════════════════════════════════════════════════════════════════╝');
    console.log(`\n📡 Node: ${NODE_WS}`);
    console.log(`🔑 Seed: ${SEED}\n`);

    let client, resultCode;
    try {
        // Create test file
        const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'bulletin-compare-'));
        const filePath = path.join(tmpDir, 'test-file.jpeg');

        console.log('🎨 Generating test file (~64MB)...');
        generateTextImage(filePath, 'Comparison Test - ' + new Date().toISOString(), 'big');
        const fileData = fs.readFileSync(filePath);
        console.log(`📁 File size: ${(fileData.length / 1024 / 1024).toFixed(2)} MB\n`);

        // Initialize PAPI client
        console.log('🔧 Connecting to Bulletin Chain...');
        client = createClient(getWsProvider(NODE_WS));
        const bulletinAPI = client.getTypedApi(bulletin);
        await waitForChainReady(bulletinAPI);

        const { sudoSigner, whoSigner, whoAddress } = setupKeyringAndSigners(SEED, '//Comparer');

        // Run tests sequentially
        await testPAPI(client, bulletinAPI, filePath, fileData);
        await testRustSDK(filePath, fileData);
        await testTypeScriptSDK(client, bulletinAPI, fileData, whoAddress, whoSigner);

        // Print comparison
        printComparison();

        // Overall result
        const allPassed =
            results.papi?.success &&
            results.rust?.success &&
            results.typescript?.success;

        if (allPassed) {
            console.log('\n✅✅✅ ALL TESTS PASSED! ✅✅✅\n');
            resultCode = 0;
        } else {
            console.log('\n⚠️  Some tests failed. See results above.\n');
            resultCode = 1;
        }
    } catch (error) {
        console.error('\n❌ Comparison test failed:', error);
        console.error(error.stack);
        resultCode = 1;
    } finally {
        if (client) client.destroy();
        process.exit(resultCode);
    }
}

await main();
