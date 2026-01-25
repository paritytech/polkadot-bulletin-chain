import fs from 'fs';
import assert from 'assert';
import { cidFromBytes } from "./cid_dag_metadata.js";
import { Binary, Enum } from '@polkadot-api/substrate-bindings';
import { CHUNK_SIZE, toHex, toHashingEnum } from './common.js';

// Convert data to Binary for PAPI (handles string, Uint8Array, and array-like types)
function toBinary(data) {
    let bytes;
    if (typeof data === 'string') {
        const buf = Buffer.from(data);
        bytes = new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
    } else if (data instanceof Uint8Array) {
        bytes = data;
    } else {
        bytes = new Uint8Array(data);
    }
    return new Binary(bytes);
}

export async function authorizeAccount(
    typedApi,
    sudoSigner,
    whos,
    transactions,
    bytes,
    txMode = TX_MODE_IN_BLOCK
) {
    const accounts = Array.isArray(whos) ? whos : [whos];

    console.log(
        `⬆️ Authorizing accounts: ${accounts.join(', ')} ` +
        `for transactions: ${transactions} and bytes: ${bytes}...`
    );

    // Collect accounts that need authorization
    const accountsToAuthorize = [];
    for (const who of accounts) {
        const auth = await typedApi.query.TransactionStorage.Authorizations.getValue(Enum("Account", who));
        console.log(`ℹ Account: ${who} Authorization info: `, auth);
        if (auth != null) {
            const authValue = auth.extent;
            const accountTransactions = authValue.transactions;
            const accountBytes = authValue.bytes;

            if (accountTransactions > transactions && accountBytes > bytes) {
                console.log('✅ Account authorization is sufficient.');
                continue;
            }
        } else {
            console.log('ℹ️ No existing authorization found — requesting new one...');
        }
        accountsToAuthorize.push(who);
    }

    if (accountsToAuthorize.length === 0) {
        console.log('✅ All accounts already have sufficient authorization.');
        return;
    }

    // Build batch of authorize_account calls
    const authorizeCalls = accountsToAuthorize.map(who =>
        typedApi.tx.TransactionStorage.authorize_account({
            who,
            transactions,
            bytes
        }).decodedCall
    );

    // Wrap in Sudo(Utility::batchAll(...))
    const batchTx = typedApi.tx.Utility.batch_all({
        calls: authorizeCalls
    });
    const sudoTx = typedApi.tx.Sudo.sudo({
        call: batchTx.decodedCall
    });

    await waitForTransaction(sudoTx, sudoSigner, "BatchAuthorize", txMode);
}

export async function authorizePreimage(
    typedApi,
    sudoSigner,
    contentHashes,
    maxSize = CHUNK_SIZE,
    txMode = TX_MODE_IN_BLOCK,
    batchSize = UTILITY_BATCH_SIZE,
) {
    const contentHashesArray = Array.isArray(contentHashes) ? contentHashes : [contentHashes];
    const totalBatches = Math.ceil(contentHashesArray.length / batchSize);

    for (let i = 0; i < contentHashesArray.length; i += batchSize) {
        const batchNumber = Math.floor(i / batchSize) + 1;
        const batch = contentHashesArray.slice(i, i + batchSize);
        console.log(`\n🔄 Processing batch ${batchNumber} of ${totalBatches}`);
        console.log(`⬆️ Authorizing preimage with content hash: ${batch.map(toHex).join(', ')}`);

        const authorizeCalls = batch.map(contentHash =>
            typedApi.tx.TransactionStorage.authorize_preimage({
                content_hash: toBinary(contentHash),
                max_size: BigInt(maxSize)
            }).decodedCall
        );

        // Wrap in Sudo(Utility::batchAll(...))
        const batchTx = typedApi.tx.Utility.batch_all({
            calls: authorizeCalls
        });
        const sudoTx = typedApi.tx.Sudo.sudo({
            call: batchTx.decodedCall
        });

        await waitForTransaction(sudoTx, sudoSigner, `BatchAuthorize Preimages ${batchNumber}`, txMode);
    }
}

export async function store(typedApi, signer, data, cidCodec = null, mhCode = null, txMode = TX_MODE_IN_BLOCK, client = null) {
    console.log('⬆️ Storing data with length=', data.length);

    // Add custom `TransactionExtension` for codec, if specified.
    const txOpts = {};
    let expectedCid;
    if (cidCodec != null && mhCode != null) {
        txOpts.customSignedExtensions = {
            ProvideCidConfig: {
                value: {
                    codec: BigInt(cidCodec),
                    hashing: toHashingEnum(mhCode),
                }
            }
        };
        expectedCid = await cidFromBytes(data, cidCodec, mhCode);
    } else {
        expectedCid = await cidFromBytes(data);
    }

    const tx = typedApi.tx.TransactionStorage.store({ data: toBinary(data) });
    await waitForTransaction(tx, signer, "Store", txMode, DEFAULT_TX_TIMEOUT_MS, client, txOpts);
    return expectedCid;
}

const UTILITY_BATCH_SIZE = 20;
export const TX_MODE_IN_BLOCK = "in-block";
export const TX_MODE_FINALIZED_BLOCK = "finalized-block";
export const TX_MODE_IN_POOL = "in-tx-pool";

const DEFAULT_TX_TIMEOUT_MS = 180_000; // 180 seconds or 30 blocks

const TX_MODE_CONFIG = {
    [TX_MODE_IN_BLOCK]: {
        match: (ev) => ev.type === "txBestBlocksState" && ev.found,
        log: (txName, ev) => `📦 ${txName} included in block: ${ev.block.hash}`,
    },
    [TX_MODE_IN_POOL]: {
        match: (ev) => ev.type === "broadcasted",
        log: (txName, ev) => `📦 ${txName} broadcasted with txHash: ${ev.txHash}`,
    },
    [TX_MODE_FINALIZED_BLOCK]: {
        match: (ev) => ev.type === "finalized",
        log: (txName, ev) => `📦 ${txName} included in finalized block: ${ev.block.hash}`,
    },
};

async function waitForTransaction(tx, signer = null, txName, txMode = TX_MODE_IN_BLOCK, timeoutMs = DEFAULT_TX_TIMEOUT_MS, client = null, txOpts = {}) {
    const config = TX_MODE_CONFIG[txMode];
    if (!config) {
        throw new Error(`Unhandled txMode: ${txMode}`);
    }

    // Get the observable - either signed or unsigned
    let observable;
    if (signer === null) {
        console.log(`⬆️ Submitting unsigned ${txName}`);
        // TODO: https://github.com/polkadot-api/polkadot-api/issues/760
        // const bareTx = await tx.getBareTx(txOpts);
        if (Object.keys(txOpts).length > 0) {
            throw new Error(`txOpts not supported for unsigned transactions (getBareTx doesn't accept options). See: https://github.com/polkadot-api/polkadot-api/issues/760`);
        }
        const bareTx = await tx.getBareTx();
        observable = client.submitAndWatch(bareTx);
    } else {
        observable = tx.signSubmitAndWatch(signer, txOpts);
    }

    return new Promise((resolve, reject) => {
        let sub;
        let resolved = false;

        const cleanup = () => {
            resolved = true;
            clearTimeout(timeoutId);
            if (sub) sub.unsubscribe();
        };

        const timeoutId = setTimeout(() => {
            if (!resolved) {
                cleanup();
                reject(new Error(`${txName} transaction timed out after ${timeoutMs}ms waiting for ${txMode}`));
            }
        }, timeoutMs);

        sub = observable.subscribe({
            next: (ev) => {
                console.log(`✅ ${txName} event:`, ev.type);
                if (!resolved && config.match(ev)) {
                    console.log(config.log(txName, ev));
                    cleanup();
                    resolve(ev);
                }
            },
            error: (err) => {
                console.error(`❌ ${txName} error:`, err);
                if (!resolved) {
                    cleanup();
                    reject(err);
                }
            },
            complete: () => {
                console.log(`✅ ${txName} complete!`);
            }
        });
    });
}

export async function fetchCid(httpIpfsApi, cid) {
    const contentUrl = `${httpIpfsApi}/ipfs/${cid.toString()}`;
    console.log('⬇️ Downloading the full content (no chunking) by cid from url: ', contentUrl);
    const res = await fetch(contentUrl);
    if (!res.ok) throw new Error(`HTTP error ${res.status}`);
    return Buffer.from(await res.arrayBuffer())
}

/**
 * Read the file, chunk it, store in Bulletin and return CIDs.
 * @param {object} typedApi - PAPI typed API
 * @param {object} signer - Signer for transactions
 * @param {string} filePath - Path to file to chunk and store
 * @param {number} chunkSize - Size of each chunk in bytes
 * @returns {{ chunks: Array<{ cid, bytes, len }> }}
 */
export async function storeChunkedFile(typedApi, signer, filePath, chunkSize) {
    const fileData = fs.readFileSync(filePath);
    console.log(`📁 Read ${filePath}, size ${fileData.length} bytes`);

    const chunks = [];
    for (let i = 0; i < fileData.length; i += chunkSize) {
        const chunk = fileData.subarray(i, i + chunkSize);
        const cid = await cidFromBytes(chunk);
        chunks.push({ cid, bytes: chunk, len: chunk.length });
    }
    console.log(`✂️ Split into ${chunks.length} chunks`);

    // Store chunks in Bulletin
    for (let i = 0; i < chunks.length; i++) {
        const { cid: expectedCid, bytes } = chunks[i];
        console.log(`📤 Storing chunk #${i + 1} CID: ${expectedCid}`);
        let cid = await store(typedApi, signer, bytes);
        assert.deepStrictEqual(expectedCid, cid);
        console.log(`✅ Stored chunk #${i + 1} and CID equals!`);
    }
    return { chunks };
}
