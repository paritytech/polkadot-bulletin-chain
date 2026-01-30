import { cryptoWaitReady } from '@polkadot/util-crypto';
import fs from 'fs'
import os from "os";
import path from "path";
import assert from "assert";
import {store, fetchCid, TX_MODE_IN_BLOCK, TX_MODE_IN_POOL} from "./api.js";
import { buildUnixFSDagPB, cidFromBytes } from "./cid_dag_metadata.js";
import {
    newSigner,
    CHUNK_SIZE,
    fileToDisk,
    filesAreEqual,
    generateTextImage,
    DEFAULT_IPFS_GATEWAY_URL,
} from "./common.js";
import {
    logHeader,
    logConnection,
    logError,
    logTestResult,
} from "./logger.js";
import { createClient } from 'polkadot-api';
import { getWsProvider } from "polkadot-api/ws-provider";
import { bulletin } from './.papi/descriptors/dist/index.mjs';

// Command line arguments: [ws_url] [seed] [ipfs_gateway_url] [image_size]
// Note: --signer-disc=XX flag is also supported for parallel runs
// Note: --num-signers=N flag controls number of parallel workers (default: 1)
const args = process.argv.slice(2).filter(arg => !arg.startsWith('--'));
const NODE_WS = args[0] || 'wss://westend-bulletin-rpc.polkadot.io';
const SEED = args[1];
const IPFS_GATEWAY_URL = args[2] || DEFAULT_IPFS_GATEWAY_URL;
// Image size preset: small, big32, big64, big96 (default to small for live tests)
const IMAGE_SIZE = args[3] || 'small';

// Optional flags
const signerDiscriminator = process.argv.find(arg => arg.startsWith("--signer-disc="))?.split("=")[1] ?? null;
const numSignersArg = process.argv.find(arg => arg.startsWith("--num-signers="))?.split("=")[1];
const NUM_SIGNERS = numSignersArg ? parseInt(numSignersArg, 10) : 1;
const SKIP_IPFS_VERIFY = process.argv.includes("--skip-ipfs-verify");
const FAST_MODE = process.argv.includes("--fast");  // Use TX_MODE_IN_POOL for faster uploads

if (!SEED) {
    console.error('Error: Seed phrase is required for live network testing.');
    console.error('Usage: node store_big_data_live.js <ws_url> <seed> [ipfs_gateway_url] [image_size] [options]');
    console.error('');
    console.error('Options:');
    console.error('  --signer-disc=XX    Signer discriminator for parallel runs');
    console.error('  --num-signers=N     Number of parallel signers (default: 1)');
    console.error('  --skip-ipfs-verify  Skip IPFS download verification (storage-only test)');
    console.error('  --fast              Use fast mode (broadcast only, don\'t wait for block inclusion)');
    console.error('');
    console.error('Example:');
    console.error('  node store_big_data_live.js wss://westend-bulletin-rpc.polkadot.io "any" http://127.0.0.1:8283 small --skip-ipfs-verify');
    console.error('  node store_big_data_live.js wss://westend-bulletin-rpc.polkadot.io "any" http://127.0.0.1:8283 big32 --skip-ipfs-verify --fast --num-signers=8');
    process.exit(1);
}

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
    blockNumbers: [],
    blockHashes: {},
};

function waitForQueueLength(targetLength, timeoutMs = 600000) {
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
        }, 500);
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

    const txMode = FAST_MODE ? TX_MODE_IN_POOL : TX_MODE_IN_BLOCK;
    let { cid, blockHash, blockNumber } = await store(typedApi, signer.signer, chunk.bytes, null, null, txMode);
    pushToResultQueue({ cid, blockNumber });
    if (blockNumber !== undefined) {
        stats.blockNumbers.push(blockNumber);
        if (blockHash && !stats.blockHashes[blockNumber]) {
            stats.blockHashes[blockNumber] = blockHash;
        }
    }
    if (FAST_MODE) {
        console.log(`Worker ${workerId} tx broadcasted with CID: ${cid}`);
    } else {
        console.log(`Worker ${workerId} tx included in block #${blockNumber} with CID: ${cid}`);
    }
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

async function printStatistics(dataSize, typedApi) {
    const numTxs = stats.blockNumbers.length;
    const elapsed = stats.endTime - stats.startTime;

    const startBlock = Math.min(...stats.blockNumbers);
    const endBlock = Math.max(...stats.blockNumbers);
    const blocksElapsed = endBlock - startBlock;

    const txsPerBlock = {};
    for (const blockNum of stats.blockNumbers) {
        txsPerBlock[blockNum] = (txsPerBlock[blockNum] || 0) + 1;
    }
    const numBlocksWithTxs = Object.keys(txsPerBlock).length;
    const totalBlocksInRange = blocksElapsed + 1;
    const avgTxsPerBlock = totalBlocksInRange > 0 ? (numTxs / totalBlocksInRange).toFixed(2) : 'N/A';

    const blockTimestamps = {};
    for (let blockNum = startBlock; blockNum <= endBlock; blockNum++) {
        try {
            let blockHash = stats.blockHashes[blockNum];
            if (!blockHash) {
                const queriedHash = await typedApi.query.System.BlockHash.getValue(blockNum);
                const hashStr = typeof queriedHash === 'string'
                    ? queriedHash
                    : (queriedHash?.asHex?.() || queriedHash?.toHex?.() || queriedHash?.toString?.() || '');
                if (hashStr && !hashStr.match(/^(0x)?0+$/)) {
                    blockHash = queriedHash;
                }
            }
            if (blockHash) {
                const timestamp = await typedApi.query.Timestamp.Now.getValue({ at: blockHash });
                blockTimestamps[blockNum] = timestamp;
            }
        } catch (e) {
            console.error(`Failed to fetch timestamp for block #${blockNum}:`, e.message);
        }
    }

    console.log('\n');
    console.log('========================================================================================================');
    console.log('                                       STORAGE STATISTICS (LIVE)                                        ');
    console.log('========================================================================================================');
    console.log(`| File size           | ${formatBytes(dataSize).padEnd(25)} |`);
    console.log(`| Chunk/TX size       | ${formatBytes(CHUNK_SIZE).padEnd(25)} |`);
    console.log(`| Number of chunks    | ${numTxs.toString().padEnd(25)} |`);
    console.log(`| Num signers         | ${NUM_SIGNERS.toString().padEnd(25)} |`);
    console.log(`| Avg txs per block   | ${`${avgTxsPerBlock} (${numTxs}/${totalBlocksInRange})`.padEnd(25)} |`);
    console.log(`| Time elapsed        | ${formatDuration(elapsed).padEnd(25)} |`);
    console.log(`| Blocks elapsed      | ${`${blocksElapsed} (#${startBlock} -> #${endBlock})`.padEnd(25)} |`);
    console.log(`| Throughput          | ${formatBytes(dataSize / (elapsed / 1000)).padEnd(22)} /s |`);
    console.log('========================================================================================================');
    console.log('                                      TRANSACTIONS PER BLOCK                                            ');
    console.log('========================================================================================================');
    console.log('| Block       | Time                    | TXs | Size         | Bar                  |');
    console.log('|-------------|-------------------------|-----|--------------|----------------------|');
    for (let blockNum = startBlock; blockNum <= endBlock; blockNum++) {
        const count = txsPerBlock[blockNum] || 0;
        const size = count > 0 ? formatBytes(count * CHUNK_SIZE) : '-';
        const bar = count > 0 ? '#'.repeat(Math.min(count, 20)) : '';
        const timestamp = blockTimestamps[blockNum];
        const timeStr = timestamp ? new Date(Number(timestamp)).toISOString().replace('T', ' ').replace('Z', '') : '-';
        console.log(`| #${blockNum.toString().padEnd(10)} | ${timeStr.padEnd(23)} | ${count.toString().padStart(3)} | ${size.padEnd(12)} | ${bar.padEnd(20)} |`);
    }
    console.log('========================================================================================================');
    console.log('\n');
}

/**
 * Read the file, chunk it and put to the queue for storing in Bulletin and return CIDs.
 * Returns { chunks }
 */
export async function storeChunkedFile(api, filePath) {
    const fileData = fs.readFileSync(filePath)
    console.log(`Read ${filePath}, size ${fileData.length} bytes`)

    const chunks = []
    for (let i = 0; i < fileData.length; i += CHUNK_SIZE) {
        const chunk = fileData.subarray(i, i + CHUNK_SIZE)
        const cid = await cidFromBytes(chunk)
        chunks.push({ cid, bytes: chunk, len: chunk.length })
    }
    console.log(`Split into ${chunks.length} chunks`)

    stats.startTime = Date.now();

    for (let i = 0; i < chunks.length; i++) {
        pushToQueue(chunks[i]);
    }
    return { chunks, dataSize: fileData.length };
}

async function main() {
    await cryptoWaitReady()

    logHeader('STORE BIG DATA TEST (LIVE NETWORK)');
    logConnection(NODE_WS, SEED.substring(0, 10) + '...', IPFS_GATEWAY_URL);
    console.log(`Image size: ${IMAGE_SIZE}, Num signers: ${NUM_SIGNERS}`);

    let client, resultCode;
    try {
        const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "bulletinimggen-live-"));
        const filePath = path.join(tmpDir, "image.jpeg");
        const downloadedFileByDagPath = path.join(tmpDir, "downloadedByDag.jpeg");
        generateTextImage(filePath, `Hello, Bulletin LIVE ${IMAGE_SIZE} - ` + new Date().toString(), IMAGE_SIZE);

        // Init WS PAPI client and typed api.
        client = createClient(getWsProvider(NODE_WS));
        const bulletinAPI = client.getTypedApi(bulletin);

        // For live networks, we use pre-authorized accounts.
        // Use the same dev seeds as the original test (//Signer1, //Signer2, etc.)
        // These are standard Substrate dev accounts that were authorized previously.
        const signers = Array.from({ length: NUM_SIGNERS }, (_, i) => {
            if (signerDiscriminator) {
                console.log(`Using signerDiscriminator: "//Signer${signerDiscriminator}${i + 1}"`);
                return newSigner(`//Signer${signerDiscriminator}${i + 1}`);
            } else {
                // Use standard dev seeds to match the original test
                return newSigner(`//Signer${i + 1}`);
            }
        });

        console.log(`Using ${signers.length} signer(s) with addresses:`);
        signers.forEach((s, i) => console.log(`  Signer ${i}: ${s.address}`));

        // NOTE: No authorization call for live networks - accounts must be pre-authorized!
        console.log('Skipping authorization (live network - accounts must be pre-authorized)');

        // Start workers
        signers.forEach((signer, i) => {
            startWorker(bulletinAPI, i, signer);
        });

        // push data to queue
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
            throw new Error('Storing chunks failed! Error:' + err.message);
        }

        console.log(`Storing DAG...`);
        let { rootCid, dagBytes } = await buildUnixFSDagPB(chunks, 0xb220);
        const dagTxMode = FAST_MODE ? TX_MODE_IN_POOL : TX_MODE_IN_BLOCK;
        let { cid } = await store(
            bulletinAPI,
            signers[0].signer,
            dagBytes,
            0x70,   // dag-pb codec
            0xb220, // blake2b-256
            dagTxMode
        );
        console.log(`DAG stored with CID: ${cid}`);
        assert.deepStrictEqual(cid, rootCid, 'CID mismatch between stored and computed DAG root');

        // Print storage statistics
        await printStatistics(dataSize, bulletinAPI);

        if (SKIP_IPFS_VERIFY) {
            console.log('\n--skip-ipfs-verify flag set, skipping IPFS download verification');
            console.log(`Root CID for manual verification: ${rootCid}`);
            console.log(`IPFS Gateway URL: ${IPFS_GATEWAY_URL}/ipfs/${rootCid}`);
            logTestResult(true, 'Store Big Data Test (LIVE) - Storage Only');
        } else {
            console.log(`Downloading from IPFS...`);
            let downloadedContent = await fetchCid(IPFS_GATEWAY_URL, rootCid);
            console.log(`Reconstructed file size: ${downloadedContent.length} bytes`);
            await fileToDisk(downloadedFileByDagPath, downloadedContent);
            filesAreEqual(filePath, downloadedFileByDagPath);
            assert.strictEqual(
                dataSize,
                downloadedContent.length,
                'Failed to download all the data!'
            );
            logTestResult(true, 'Store Big Data Test (LIVE)');
        }
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
