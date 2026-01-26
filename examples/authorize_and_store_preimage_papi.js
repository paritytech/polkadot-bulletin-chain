import assert from "assert";
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { cryptoWaitReady, blake2AsU8a } from '@polkadot/util-crypto';
import { Binary } from '@polkadot-api/substrate-bindings';
import { fetchCid, TX_MODE_FINALIZED_BLOCK } from './api.js';
import { setupKeyringAndSigners } from './common.js';
import { cidFromBytes } from "./cid_dag_metadata.js";
import { bulletin } from './.papi/descriptors/dist/index.mjs';

// Command line arguments: [ws_url] [seed]
const args = process.argv.slice(2);
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Alice';
const HTTP_IPFS_API = 'http://127.0.0.1:8080';   // Local IPFS HTTP gateway

const TX_MODE_CONFIG = {
    [TX_MODE_FINALIZED_BLOCK]: {
        match: (ev) => ev.type === "finalized",
        log: (txName, ev) => `📦 ${txName} included in finalized block: ${ev.block.hash}`,
    },
};

const DEFAULT_TX_TIMEOUT_MS = 120_000; // 120 seconds or 20 blocks

async function waitForTransaction(tx, signer, txName, txMode = TX_MODE_FINALIZED_BLOCK, timeoutMs = DEFAULT_TX_TIMEOUT_MS) {
    const config = TX_MODE_CONFIG[txMode];
    if (!config) {
        throw new Error(`Unhandled txMode: ${txMode}`);
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

        sub = tx.signSubmitAndWatch(signer).subscribe({
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
        });
    });
}

async function waitForRawTransaction(client, tx, txName, txMode = TX_MODE_FINALIZED_BLOCK, timeoutMs = DEFAULT_TX_TIMEOUT_MS) {
    const config = TX_MODE_CONFIG[txMode];
    if (!config) {
        throw new Error(`Unhandled txMode: ${txMode}`);
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

        sub = client.submitAndWatch(tx).subscribe({
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
        });
    });
}

async function authorizePreimage(typedApi, sudoSigner, contentHash, maxSize, txMode) {
    console.log(
        `⬆️ Authorizing preimage with content hash: ${contentHash} and max size: ${maxSize}...`
    );
    const authorizeCall = typedApi.tx.TransactionStorage.authorize_preimage({
        content_hash: contentHash,
        max_size: maxSize,
    }).decodedCall;

    const batchTx = typedApi.tx.Utility.batch_all({
        calls: [authorizeCall],
    });
    const sudoTx = typedApi.tx.Sudo.sudo({
        call: batchTx.decodedCall,
    });

    await waitForTransaction(sudoTx, sudoSigner, "BatchAuthorize Preimages", txMode);
}

async function storeUnsigned(typedApi, client, data, txMode) {
    console.log('⬆️ Submitting unsigned Store');
    const cid = await cidFromBytes(data);

    const binaryData = new Binary(data);
    const tx = typedApi.tx.TransactionStorage.store({ data: binaryData });
    const bareTx = await tx.getBareTx();
    await waitForRawTransaction(client, bareTx, "Store", txMode);
    return cid;
}

async function main() {
    await cryptoWaitReady();

    console.log(`Connecting to: ${NODE_WS}`);
    console.log(`Using seed: ${SEED}`);

    let client, resultCode;
    try {
        // Init WS PAPI client and typed api.
        client = createClient(getWsProvider(NODE_WS));
        const bulletinAPI = client.getTypedApi(bulletin);

        // Signers.
        const { sudoSigner } = setupKeyringAndSigners(SEED, '//PapiPreimageSigner');

        // Data to store.
        const dataToStore = "Hello, Bulletin with preimage auth - " + new Date().toString();
        const dataBytes = new Uint8Array(Buffer.from(dataToStore));
        const expectedCid = await cidFromBytes(dataBytes);
        const contentHash = new Binary(blake2AsU8a(dataBytes));

        // Authorize preimage.
        await authorizePreimage(
            bulletinAPI,
            sudoSigner,
            contentHash,
            BigInt(dataBytes.length),
            TX_MODE_FINALIZED_BLOCK,
        );

        // Store data as unsigned.
        const cid = await storeUnsigned(bulletinAPI, client, dataBytes, TX_MODE_FINALIZED_BLOCK);
        console.log("✅ Data stored successfully with CID:", cid);

        // Read back from IPFS
        const downloadedContent = await fetchCid(HTTP_IPFS_API, cid);
        console.log("✅ Downloaded content:", downloadedContent.toString());
        assert.deepStrictEqual(
            cid,
            expectedCid,
            '❌ expectedCid does not match cid!'
        );
        assert.deepStrictEqual(
            dataToStore,
            downloadedContent.toString(),
            '❌ dataToStore does not match downloadedContent!'
        );
        console.log(`✅ Verified content!`);

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


