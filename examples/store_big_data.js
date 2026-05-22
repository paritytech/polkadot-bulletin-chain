/**
 * Store a big file on Bulletin Chain using the TypeScript SDK.
 *
 * Demonstrates the high-level `uploadFile()` API:
 *   1. Authorize a user account (//Alice as the Authorizer origin).
 *   2. `client.uploadFile(bytes).send()` — the SDK chunks, builds the
 *      manifest, submits everything through the shared upload pipeline,
 *      and returns the single root CID.
 *   3. Optionally verify via IPFS (root CID download + per-chunk reassembly).
 *
 * Usage:
 *   node store_big_data.js [ws_url] [seed] [ipfs_gateway_url] [image_size]
 *   image_size ∈ { small | big32 | big64 | big96 }
 *
 * Flags:
 *   --signer-disc=XX     Append discriminator to user seed for parallel CI runs.
 *   --skip-authorize     Skip account authorization (account is pre-auth'd).
 *   --skip-ipfs-verify   Skip IPFS download verification.
 */

import assert from 'assert';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws';

import { bulletin } from './.papi/descriptors/dist/index.js';
import {
    BulletinClient,
    UploadStatus,
} from '../sdk/typescript/dist/index.mjs';

import { fetchCid } from './api.js';
import {
    setupKeyringAndSigners,
    newSigner,
    fileToDisk,
    filesAreEqual,
    generateTextImage,
    DEFAULT_IPFS_GATEWAY_URL,
} from './common.js';
import {
    logHeader,
    logConnection,
    logStep,
    logSuccess,
    logError,
    logTestResult,
} from './logger.js';

// -------------------- CLI args --------------------
const args = process.argv.slice(2).filter(arg => !arg.startsWith('--'));
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Alice';
const IPFS_GATEWAY_URL = args[2] || DEFAULT_IPFS_GATEWAY_URL;
const IMAGE_SIZE = args[3] || 'big64';

const signerDiscriminator = process.argv.find(arg => arg.startsWith('--signer-disc='))?.split('=')[1] ?? null;
const SKIP_AUTHORIZE = process.argv.includes('--skip-authorize');
const SKIP_IPFS_VERIFY = process.argv.includes('--skip-ipfs-verify');

// -------------------- helpers --------------------
function formatBytes(bytes) {
    if (bytes >= 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(2) + ' MiB';
    if (bytes >= 1024) return (bytes / 1024).toFixed(2) + ' KiB';
    return bytes + ' B';
}

function formatDuration(ms) {
    if (ms >= 60000) return (ms / 60000).toFixed(2) + ' min';
    if (ms >= 1000) return (ms / 1000).toFixed(2) + ' s';
    return ms + ' ms';
}

function printPerBlock(perBlock) {
    const blocks = Object.keys(perBlock).map(Number).sort((a, b) => a - b);
    if (!blocks.length) return;
    console.log('\n📦 Items finalized per block');
    console.log('──────────────────────────────────────────────────────');
    console.log('│ Block       │ Items │ Bar                          │');
    console.log('├─────────────┼───────┼──────────────────────────────┤');
    for (let blk = blocks[0]; blk <= blocks[blocks.length - 1]; blk++) {
        const count = perBlock[blk] || 0;
        const bar = count > 0 ? '█'.repeat(Math.min(count, 30)) : '';
        console.log(`│ #${String(blk).padEnd(10)} │ ${String(count).padStart(5)} │ ${bar.padEnd(28)} │`);
    }
    console.log('──────────────────────────────────────────────────────');
}

async function main() {
    await cryptoWaitReady();

    logHeader('STORE BIG DATA TEST (SDK)');
    logConnection(NODE_WS, SEED, IPFS_GATEWAY_URL);

    let papiClient, resultCode;
    try {
        // 1) Generate the input file
        const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'bulletinimggen-'));
        const filePath = path.join(tmpDir, 'image.jpeg');
        const downloadedByManifestPath = path.join(tmpDir, 'downloadedByManifest.jpeg');
        const downloadedByChunksPath = path.join(tmpDir, 'downloadedByChunks.jpeg');
        generateTextImage(filePath, `Hello, Bulletin ${IMAGE_SIZE} - ${new Date().toString()}`, IMAGE_SIZE);
        const fileBytes = fs.readFileSync(filePath);
        console.log(`📁 Generated ${filePath}, size ${formatBytes(fileBytes.length)}`);

        // 2) PAPI + SDK clients
        papiClient = createClient(getWsProvider(NODE_WS));
        const api = papiClient.getTypedApi(bulletin);

        // SEED is the Authorizer-origin account (e.g. //Alice on local
        // zombienet and Paseo Bulletin). It calls `authorize_account` directly,
        // no Sudo wrapping needed.
        const { authorizationSigner: authorizerSigner } = setupKeyringAndSigners(SEED, '//Bigdatasigner');
        const userSeed = signerDiscriminator
            ? `//SDKBigDataSigner${signerDiscriminator}`
            : '//SDKBigDataSigner';
        const user = newSigner(userSeed);
        console.log(`User account: ${user.address}`);
        const authorizerClient = new BulletinClient(api, authorizerSigner);
        // wsUrls opts the SDK into the pipelined submission engine — the
        // same code path used for both single-item and chunked uploads.
        const userClient = new BulletinClient(api, user.signer, undefined, {
            wsUrls: [NODE_WS],
        });

        // 3) Authorize the user account
        if (!SKIP_AUTHORIZE) {
            logStep('1️⃣', 'Authorizing user account...');
            await authorizerClient
                .authorizeAccount(user.address, 100, BigInt(100 * 1024 * 1024)) // 100 MiB
                .withWaitFor('finalized')
                .send();
            logSuccess('Account authorized');
        }

        // 4) Upload the file. uploadFile() handles chunking + manifest
        //    internally and resolves with the single root CID. Per-item
        //    progress comes through ItemStarted / ItemInBlock / ItemFinalized
        //    events — each event carries the item's CID so callers can
        //    correlate with their own bookkeeping.
        logStep('2️⃣', `Uploading ${formatBytes(fileBytes.length)} via SDK...`);
        const perBlock = {};
        const chunkCids = [];
        let lastItemTotal = 0;
        const start = Date.now();

        const { cid: rootCid } = await userClient
            .uploadFile(new Uint8Array(fileBytes))
            .ensureAuthorized()  // fail fast if the account has no/expired auth
            .withCallback((ev) => {
                lastItemTotal = ev.total;
                const cidShort = ev.cid.toString().slice(0, 16) + '…';
                const tag = `item ${String(ev.index + 1).padStart(3)}/${ev.total}`;
                if (ev.type === UploadStatus.ItemStarted) {
                    console.log(`  ${tag}  STARTED    ${cidShort}`);
                } else if (ev.type === UploadStatus.ItemInBlock) {
                    console.log(`  ${tag}  IN_BLOCK   ${cidShort}  @#${ev.blockNumber}`);
                } else if (ev.type === UploadStatus.ItemFinalized) {
                    console.log(`  ${tag}  FINALIZED  ${cidShort}  @#${ev.blockNumber}`);
                    perBlock[ev.blockNumber] = (perBlock[ev.blockNumber] ?? 0) + 1;
                    // The last item is the manifest (when chunking happened); everything before is a chunk.
                    if (ev.index < ev.total - 1) chunkCids.push(ev.cid);
                } else if (ev.type === UploadStatus.ItemFailed) {
                    console.log(`  ${tag}  FAILED     ${cidShort}  ${ev.error?.message}`);
                }
            })
            .send();

        const elapsed = Date.now() - start;
        const numChunks = lastItemTotal > 1 ? lastItemTotal - 1 : 1;

        logSuccess(`Uploaded! Root CID: ${rootCid}`);
        console.log(`  items       : ${lastItemTotal} (${numChunks} chunk${numChunks === 1 ? '' : 's'}${lastItemTotal > 1 ? ' + 1 manifest' : ''})`);
        console.log(`  elapsed     : ${formatDuration(elapsed)}`);
        console.log(`  throughput  : ${formatBytes(fileBytes.length / (elapsed / 1000))}/s`);

        printPerBlock(perBlock);

        // 5) Verify via IPFS
        if (!SKIP_IPFS_VERIFY) {
            logStep('3️⃣', 'Downloading root CID from IPFS...');
            const downloadedManifest = await fetchCid(IPFS_GATEWAY_URL, rootCid.toString());
            await fileToDisk(downloadedByManifestPath, downloadedManifest);
            filesAreEqual(filePath, downloadedByManifestPath);
            assert.strictEqual(
                fileBytes.length,
                downloadedManifest.length,
                '❌ Failed to download all the data via root CID!',
            );
            logSuccess(`Reconstructed via root CID: ${downloadedManifest.length} bytes`);

            if (chunkCids.length) {
                logStep('4️⃣', 'Downloading each chunk from IPFS...');
                const downloadedChunks = [];
                for (const cid of chunkCids) {
                    downloadedChunks.push(await fetchCid(IPFS_GATEWAY_URL, cid.toString()));
                }
                const fullBuffer = Buffer.concat(downloadedChunks);
                await fileToDisk(downloadedByChunksPath, fullBuffer);
                filesAreEqual(filePath, downloadedByChunksPath);
                assert.strictEqual(
                    fileBytes.length,
                    fullBuffer.length,
                    '❌ Failed to download all the data via chunks!',
                );
                logSuccess(`Reconstructed from ${chunkCids.length} chunks: ${fullBuffer.length} bytes`);
            }
        }

        logTestResult(true, SKIP_IPFS_VERIFY ? 'Store Big Data SDK Test (Storage Only)' : 'Store Big Data SDK Test');
        resultCode = 0;
    } catch (error) {
        logError(`Error: ${error.message}`);
        console.error(error);
        resultCode = 1;
    } finally {
        if (papiClient) papiClient.destroy();
        process.exit(resultCode);
    }
}

await main();
