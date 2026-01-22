import { cidFromBytes } from "./cid_dag_metadata.js";
import { Binary, Enum } from '@polkadot-api/substrate-bindings';
import { CHUNK_SIZE } from './common.js';
import util from 'util';

const UTILITY_BATCH_SIZE = 10;

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
        console.log(
            `⬆️ Authorizing preimage with content hash: `, util.inspect(batch, { depth: null, colors: true })
        );

        const authorizeCalls = batch.map(contentHash => {
            typedApi.tx.TransactionStorage.authorize_preimage({
                contentHash,
                maxSize
            }).decodedCall
        });

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

export async function store(typedApi, signer, data, txMode = TX_MODE_IN_BLOCK, client) {
    console.log('⬆️ Storing data with length=', data.length);
    const cid = await cidFromBytes(data);

    // Convert data to Uint8Array then wrap in Binary for PAPI typed API
    const bytes =
        typeof data === 'string'
            ? new Uint8Array(Buffer.from(data))
            : data instanceof Uint8Array
                ? data
                : new Uint8Array(data);
    const binaryData = new Binary(bytes);

    const tx = typedApi.tx.TransactionStorage.store({ data: binaryData });
    await waitForTransaction(tx, signer, "Store", txMode, DEFAULT_TX_TIMEOUT_MS, client);
    return cid;
}

export const TX_MODE_IN_BLOCK = "in-block";
export const TX_MODE_FINALIZED_BLOCK = "finalized-block";
export const TX_MODE_IN_POOL = "in-tx-pool";

const DEFAULT_TX_TIMEOUT_MS = 120_000; // 120 seconds or 20 blocks

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

function waitForTransaction(tx, signer = null, txName, txMode = TX_MODE_IN_BLOCK, timeoutMs = DEFAULT_TX_TIMEOUT_MS, client = null) {
    const config = TX_MODE_CONFIG[txMode];
    if (!config) {
        return Promise.reject(new Error(`Unhandled txMode: ${txMode}`));
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

        let observer;
        if (signer === null) {
            console.log(`⬆️ Submitting ${txName} with client: `, util.inspect(client, { depth: null, colors: true }));
            observer = client.submitAndWatch(tx);
        } else {
            observer = tx.signSubmitAndWatch(signer);
        }
        sub = observer.subscribe({
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
