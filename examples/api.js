import { cidFromBytes } from "./cid_dag_metadata.js";
import { Binary } from '@polkadot-api/substrate-bindings';

export async function authorizeAccount(typedApi, sudoSigner, who, transactions, bytes) {
    console.log('Authorizing account...');

    const authorizeTx = typedApi.tx.TransactionStorage.authorize_account({
        who,
        transactions,
        bytes
    });

    const sudoTx = typedApi.tx.Sudo.sudo({
        call: authorizeTx.decodedCall
    });

    // Wait for finalization to ensure nonce is updated for subsequent transactions
    await waitForTransaction(sudoTx, sudoSigner, "Authorize", TX_MODE_FINALIZED_BLOCK);
}

export async function store(typedApi, signer, data) {
    console.log('⬆️ Storing data with length=', data.length);
    const cid = await cidFromBytes(data);

    // Convert data to Uint8Array then wrap in Binary for PAPI typed API
    const dataBytes = typeof data === 'string' ?
        new Uint8Array(Buffer.from(data)) :
        new Uint8Array(data);

    const binaryData = Binary.fromBytes(dataBytes);
    const tx = typedApi.tx.TransactionStorage.store({ data: binaryData });

    // Wait for finalization to ensure data is committed before fetching from IPFS
    await waitForTransaction(tx, signer, "Store", TX_MODE_FINALIZED_BLOCK);
    return cid;
}

export const TX_MODE_IN_BLOCK = "in-block";
export const TX_MODE_FINALIZED_BLOCK = "finalized-block";
export const TX_MODE_IN_POOL = "in-tx-pool";

const DEFAULT_TX_TIMEOUT_MS = 180_000; // 180 seconds (3 minutes) - increased for smoldot/light client sync delays

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
