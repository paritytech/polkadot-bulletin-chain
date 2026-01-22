import { cryptoWaitReady } from '@polkadot/util-crypto';
import { create } from 'ipfs-http-client';
import fs from 'fs'
import os from "os";
import path from "path";
import assert from "assert";
import { authorizeAccount, store, TX_MODE_FINALIZED_BLOCK } from "./api.js";
import {cidFromBytes} from "./cid_dag_metadata.js";
import {
    setupKeyringAndSigners,
    CHUNK_SIZE,
    newSigner,
    fileToDisk,
    filesAreEqual,
    generateTextImage
} from "./common.js";
import { createClient } from 'polkadot-api';
import { getWsProvider } from "polkadot-api/ws-provider";
import { bulletin } from './.papi/descriptors/dist/index.mjs';

// Command line arguments: [ws_url] [seed]
// Note: --signer-disc=XX flag is also supported for parallel runs
const args = process.argv.slice(2).filter(arg => !arg.startsWith('--'));
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Alice';

// -------------------- queue --------------------
const queue = [];
function pushToQueue(data) {
    queue.push(data);
}

const resultQueue = [];
function pushToResultQueue(data) {
    resultQueue.push(data);
}
function waitForQueueLength(targetLength, timeoutMs = 60000) {
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
        }, 50); // check every 50ms
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

    let cid = await store(typedApi, signer.signer, chunk.bytes);
    pushToResultQueue(cid);
    console.log(`Worker ${workerId} tx included in the block with CID: ${cid}`);
}

// -------------------- helpers --------------------
function sleep(ms) {
    return new Promise(r => setTimeout(r, ms));
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

    // ---- 2️⃣ Store chunks in Bulletin ----
    for (let i = 0; i < chunks.length; i++) {
        pushToQueue(chunks[i]);
    }
    return { chunks, dataSize: fileData.length };
}

// Connect to a local IPFS gateway (e.g. Kubo)
const ipfs = create({
    url: 'http://127.0.0.1:5001', // Local IPFS API
});

async function readFromIpfs(cid) {
    // Fetch the block (downloads via Bitswap if not local)
    console.log('Trying to get cid: ', cid);
    const chunks = [];
    for await (const chunk of ipfs.cat(cid)) {
        chunks.push(chunk);
    }
    const fullData = Buffer.concat(chunks);
    console.log('Received block: ', fullData);
    return fullData
}

// Optional signer discriminator, when we want to run the script in parallel and don't take care of nonces.
// E.g.: node store_big_data.js --signer-disc=BB
const signerDiscriminator = process.argv.find(arg => arg.startsWith("--signer-disc="))?.split("=")[1] ?? null;

async function main() {
    await cryptoWaitReady()

    console.log(`Connecting to: ${NODE_WS}`);
    console.log(`Using seed: ${SEED}`);

    let client, resultCode;
    try {
        const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "bulletinimggen-"));
        const filePath = path.join(tmpDir, "image.jpeg");
        const downloadedFilePath = path.join(tmpDir, "downloaded.jpeg");
        generateTextImage(filePath, "Hello, Bulletin big - " + new Date().toString(), "big");

        // Init WS PAPI client and typed api.
        client = createClient(getWsProvider(NODE_WS));
        const bulletinAPI = client.getTypedApi(bulletin);
        const { sudoSigner, _ } = setupKeyringAndSigners(SEED, '//Bigdatasigner');

        // Let's do parallelism with multiple accounts
        const signers = Array.from({ length: 12 }, (_, i) => {
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
            await waitForQueueLength(chunks.length, 180_000);
            console.log(`All chunks ${chunks.length} are stored!`);
        } catch (err) {
            console.error(err.message);
            throw new Error('❌ Storing chunks failed! Error:' + err.message);
        }

        // Check all chunks are there.
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

        console.log(`\n\n\n✅✅✅ Test passed! ✅✅✅`);
        resultCode = 0;
    } catch (error) {
        console.error("❌ Error:", error);
        resultCode = 1;
    } finally {
        if (client) client.destroy();
        process.exit(resultCode);
    }
}

await main();
