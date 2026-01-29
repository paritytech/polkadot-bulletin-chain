import { cryptoWaitReady } from '@polkadot/util-crypto';
import { create } from 'ipfs-http-client';
import fs from 'fs'
import os from "os";
import path from "path";
import assert from "assert";
import { authorizeAccount, store, fetchCid, TX_MODE_FINALIZED_BLOCK } from "./api.js";
import { buildUnixFSDagPB, cidFromBytes } from "./cid_dag_metadata.js";
import {
    setupKeyringAndSigners,
    CHUNK_SIZE,
    newSigner,
    fileToDisk,
    filesAreEqual,
    generateTextImage,
} from "./common.js";
import {
    logHeader,
    logConnection,
    logStep,
    logSuccess,
    logError,
    logTestResult,
} from "./logger.js";
import { createClient } from 'polkadot-api';
import { getWsProvider } from "polkadot-api/ws-provider";
import { bulletin } from './.papi/descriptors/dist/index.mjs';

// Command line arguments: [ws_url] [seed] [ipfs_api_url]
// Note: --signer-disc=XX flag is also supported for parallel runs
const args = process.argv.slice(2).filter(arg => !arg.startsWith('--'));
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Alice';
const HTTP_IPFS_API = args[2] || 'http://127.0.0.1:5001';
const NUM_SIGNERS = 16;

// -------------------- queue --------------------
const queue = [];
function pushToQueue(data) {
    queue.push(data);
}

const resultQueue = [];
function pushToResultQueue(data) {
    resultQueue.push(data);
}

// -------------------- statistics --------------------
const stats = {
    startTime: null,
    endTime: null,
    blockNumbers: [],  // Track all block numbers where txs were included
};

function waitForQueueLength(targetLength, timeoutMs = 300000) {
    return new Promise((resolve, reject) => {
        const start = Date.now();

        const interval = setInterval(() => {
            if (resultQueue.length >= targetLength) {
                clearInterval(interval);
                resolve(resultQueue.slice(0, targetLength));
            } else if (Date.now() - start > timeoutMs) {
                clearInterval(interval);
                reject(new Error(`Timeout waiting for ${targetLength} entries in queue`));
            }
        }, 500); // check every 500ms
    });
}

// -------------------- worker --------------------
async function startWorker(typedApi, workerId, signer) {
    console.log(`Worker ${workerId} started`);

    while (true) {
        const job = queue.shift();

        if (!job) {
            await sleep(500);
            continue;
        }

        try {
            await processJob(typedApi, workerId, signer, job);
        } catch (err) {
            console.error(`Worker ${workerId} failed job`, err);
        }
    }
}

// -------------------- job processing --------------------
async function processJob(typedApi, workerId, signer, chunk) {
    console.log(
        `Worker ${workerId} submitting tx for chunk ${chunk.cid} of size ${chunk.len} bytes`
    );

    // Use longer timeout (120s) for parallel workers to avoid timeouts under heavy load
    let { cid, blockHash, blockNumber } = await store(typedApi, signer.signer, chunk.bytes);
    pushToResultQueue({ cid, blockNumber });
    if (blockNumber !== undefined) {
        stats.blockNumbers.push(blockNumber);
    }
    console.log(`Worker ${workerId} tx included in block #${blockNumber} with CID: ${cid}`);
}

// -------------------- helpers --------------------
function sleep(ms) {
    return new Promise(r => setTimeout(r, ms));
}

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

function printStatistics(dataSize) {
    const numTxs = stats.blockNumbers.length;
    const elapsed = stats.endTime - stats.startTime;

    // Calculate startBlock and endBlock from actual transaction blocks
    const startBlock = Math.min(...stats.blockNumbers);
    const endBlock = Math.max(...stats.blockNumbers);
    const blocksElapsed = endBlock - startBlock;

    // Count transactions per block
    const txsPerBlock = {};
    for (const blockNum of stats.blockNumbers) {
        txsPerBlock[blockNum] = (txsPerBlock[blockNum] || 0) + 1;
    }
    const numBlocksWithTxs = Object.keys(txsPerBlock).length;
    const avgTxsPerBlock = numBlocksWithTxs > 0 ? (numTxs / numBlocksWithTxs).toFixed(2) : 'N/A';

    console.log('\n');
    console.log('═══════════════════════════════════════════════════════════════════════════════');
    console.log('                            📊 STORAGE STATISTICS                              ');
    console.log('═══════════════════════════════════════════════════════════════════════════════');
    console.log(`| File size           | ${formatBytes(dataSize).padEnd(20)} |`);
    console.log(`| Chunk/TX size       | ${formatBytes(CHUNK_SIZE).padEnd(20)} |`);
    console.log(`| Number of chunks    | ${numTxs.toString().padEnd(20)} |`);
    console.log(`| Avg txs per block   | ${avgTxsPerBlock.toString().padEnd(20)} |`);
    console.log(`| Time elapsed        | ${formatDuration(elapsed).padEnd(20)} |`);
    console.log(`| Blocks elapsed      | ${`${blocksElapsed} (#${startBlock} → #${endBlock})`.padEnd(20)} |`);
    console.log(`| Throughput          | ${formatBytes(dataSize / (elapsed / 1000)).padEnd(20)} /s |`);
    console.log('═══════════════════════════════════════════════════════════════════════════════');
    console.log('                         📦 TRANSACTIONS PER BLOCK                             ');
    console.log('═══════════════════════════════════════════════════════════════════════════════');
    for (let blockNum = startBlock; blockNum <= endBlock; blockNum++) {
        const count = txsPerBlock[blockNum] || 0;
        const size = count > 0 ? formatBytes(count * CHUNK_SIZE) : '-';
        const bar = count > 0 ? '█'.repeat(count) : '';
        console.log(`| Block #${blockNum.toString().padEnd(10)} | ${count.toString().padStart(3)} txs | ${size.padEnd(12)} | ${bar}`);
    }
    console.log('═══════════════════════════════════════════════════════════════════════════════');
    console.log('\n');
}

/**
 * Read the file, chunk it and put to the queue for storing in Bulletin and return CIDs.
 * Returns { chunks }
 */
export async function storeChunkedFile(api, filePath) {
    // ---- 1️⃣ Read and split a file ----
    const fileData = fs.readFileSync(filePath)
    console.log(`📁 Read ${filePath}, size ${fileData.length} bytes`)

    const chunks = []
    for (let i = 0; i < fileData.length; i += CHUNK_SIZE) {
        const chunk = fileData.subarray(i, i + CHUNK_SIZE)
        const cid = await cidFromBytes(chunk)
        chunks.push({ cid, bytes: chunk, len: chunk.length })
    }
    console.log(`✂️ Split into ${chunks.length} chunks`)

    // Start timing for statistics
    stats.startTime = Date.now();

    // ---- 2️⃣ Store chunks in Bulletin ----
    for (let i = 0; i < chunks.length; i++) {
        pushToQueue(chunks[i]);
    }
    return { chunks, dataSize: fileData.length };
}

// Connect to a local IPFS gateway (e.g. Kubo)
const ipfs = create({
    url: HTTP_IPFS_API,
});

// Optional signer discriminator, when we want to run the script in parallel and don't take care of nonces.
// E.g.: node store_big_data.js --signer-disc=BB
const signerDiscriminator = process.argv.find(arg => arg.startsWith("--signer-disc="))?.split("=")[1] ?? null;

async function main() {
    await cryptoWaitReady()

    logHeader('STORE BIG DATA TEST');
    logConnection(NODE_WS, SEED, HTTP_IPFS_API);

    let client, resultCode;
    try {
        const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "bulletinimggen-"));
        const filePath = path.join(tmpDir, "image.jpeg");
        const downloadedFilePath = path.join(tmpDir, "downloaded.jpeg");
        const downloadedFileByDagPath = path.join(tmpDir, "downloadedByDag.jpeg");
        generateTextImage(filePath, "Hello, Bulletin big - " + new Date().toString(), "big32");

        // Init WS PAPI client and typed api.
        client = createClient(getWsProvider(NODE_WS));
        const bulletinAPI = client.getTypedApi(bulletin);
        const { sudoSigner, _ } = setupKeyringAndSigners(SEED, '//Bigdatasigner');

        // Let's do parallelism with multiple accounts
        const signers = Array.from({ length: NUM_SIGNERS }, (_, i) => {
            if (!signerDiscriminator) {
                return newSigner(`//Signer${i + 1}`)
            } else {
                console.log(`Using signerDiscriminator: "//Signer${signerDiscriminator}${i + 1}"`);
                return newSigner(`//Signer${signerDiscriminator}${i + 1}`)
            }
        });

        // Authorize accounts.
        await authorizeAccount(
            bulletinAPI,
            sudoSigner,
            signers.map(a => a.address),
            100,
            BigInt(100 * 1024 * 1024), // 100 MiB
            TX_MODE_FINALIZED_BLOCK,
        );

        // Start 8 workers
        signers.forEach((signer, i) => {
            startWorker(bulletinAPI, i, signer);
        });

        // push data to queue
        // Read the file, chunk it, store in Bulletin and return CIDs.
        let { chunks, dataSize } = await storeChunkedFile(bulletinAPI, filePath);

        // wait for all chunks are stored
        try {
            console.log(`Waiting for all chunks ${chunks.length} to be stored!`);
            await waitForQueueLength(chunks.length);
            stats.endTime = Date.now();
            console.log(`All chunks ${chunks.length} are stored!`);
        } catch (err) {
            stats.endTime = Date.now();
            console.error(err.message);
            throw new Error('❌ Storing chunks failed! Error:' + err.message);
        }

        console.log(`Storing DAG...`);
        let { rootCid, dagBytes } = await buildUnixFSDagPB(chunks, 0xb220);
        let { cid } = await store(
            bulletinAPI,
            signers[0].signer,
            dagBytes,
            undefined,
            undefined,
            TX_MODE_FINALIZED_BLOCK
        );
        console.log(`Downloading...${cid} / ${rootCid}`);
        let downloadedContent = await fetchCid(HTTP_IPFS_API, rootCid);
        console.log(`✅ Reconstructed file size: ${downloadedContent.length} bytes`);
        await fileToDisk(downloadedFileByDagPath, downloadedContent);
        filesAreEqual(filePath, downloadedFileByDagPath);
        assert.strictEqual(
            dataSize,
            downloadedContent.length,
            '❌ Failed to download all the data!'
        );

        // Check all chunks are there.
        console.log(`Downloading by chunks...`);
        let downloadedChunks = [];
        for (const chunk of chunks) {
            // Download the chunk from IPFS.
            let block = await ipfs.block.get(chunk.cid, {timeout: 15000});
            downloadedChunks.push(block);
        }
        let fullBuffer = Buffer.concat(downloadedChunks);
        console.log(`✅ Reconstructed file size: ${fullBuffer.length} bytes`);
        await fileToDisk(downloadedFilePath, fullBuffer);
        filesAreEqual(filePath, downloadedFilePath);
        assert.strictEqual(
            dataSize,
            fullBuffer.length,
            '❌ Failed to download all the data!'
        );

        // Print storage statistics
        printStatistics(dataSize);

        logTestResult(true, 'Store Big Data Test');
        resultCode = 0;
    } catch (error) {
        logError(`Error: ${error.message}`);
        console.error(error);
        resultCode = 1;
    } finally {
        if (client) client.destroy();
        process.exit(resultCode);
    }
}

await main();
