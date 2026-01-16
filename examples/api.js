import { cidFromBytes } from "./cid_dag_metadata.js";
import { Binary, Enum } from '@polkadot-api/substrate-bindings';

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

    // TODO: rewrite with batch
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

        const authorizeTx = typedApi.tx.TransactionStorage.authorize_account({
            who,
            transactions,
            bytes
        });
        const sudoTx = typedApi.tx.Sudo.sudo({
            call: authorizeTx.decodedCall
        });

        await waitForTransaction(sudoTx, sudoSigner, "Authorize", txMode);
    }
}

export async function store(typedApi, signer, data, txMode = TX_MODE_IN_BLOCK) {
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
    await waitForTransaction(tx, signer, "Store", txMode);
    return cid;
}

export const TX_MODE_IN_BLOCK = "in-block";
export const TX_MODE_FINALIZED_BLOCK = "finalized-block";
export const TX_MODE_IN_POOL = "in-tx-pool";

const DEFAULT_TX_TIMEOUT_MS = 120_000; // 120 seconds - includes time for in-block inclusion + stability delay

const TX_MODE_CONFIG = {
    [TX_MODE_IN_BLOCK]: {
        match: (ev) => ev.type === "txBestBlocksState" && ev.found,
        log: (txName, ev) => `📦 ${txName} included in block: ${ev.block.hash}`,
        stabilityDelayMs: 6000, // Wait 6s for block to stabilize (helps with light client reorgs)
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

function waitForTransaction(tx, signer, txName, txMode = TX_MODE_IN_BLOCK, timeoutMs = DEFAULT_TX_TIMEOUT_MS) {
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

        sub = tx.signSubmitAndWatch(signer).subscribe({
            next: (ev) => {
                console.log(`✅ ${txName} event:`, ev.type);
                if (!resolved && config.match(ev)) {
                    console.log(config.log(txName, ev));
                    
                    // Mark as resolved immediately to prevent error handler from rejecting
                    resolved = true;
                    
                    // Unsubscribe to stop receiving events
                    if (sub) sub.unsubscribe();
                    
                    // If config specifies a stability delay, wait before resolving
                    if (config.stabilityDelayMs) {
                        console.log(`⏳ Waiting ${config.stabilityDelayMs}ms for block stability...`);
                        setTimeout(() => {
                            clearTimeout(timeoutId);
                            resolve(ev);
                        }, config.stabilityDelayMs);
                    } else {
                        clearTimeout(timeoutId);
                        resolve(ev);
                    }
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
