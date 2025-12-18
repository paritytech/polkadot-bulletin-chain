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

    await waitForTransaction(sudoTx, sudoSigner, "Authorize");
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

    await waitForTransaction(tx, signer, "Store");
    return cid;
}

export const TX_MODE_IN_BLOCK = "in-block";
export const TX_MODE_FINALIZED_BLOCK = "finalized-block";
export const TX_MODE_IN_POOL = "in-tx-pool";

function waitForTransaction(tx, signer, txName, txMode = TX_MODE_IN_BLOCK) {
    return new Promise((resolve, reject) => {
        const sub = tx.signSubmitAndWatch(signer).subscribe({
            next: (ev) => {
                console.log(`✅ ${txName} event:`, ev.type);
                switch (txMode) {
                    case TX_MODE_IN_BLOCK:
                        if (ev.type === "txBestBlocksState" && ev.found) {
                            console.log(`📦 ${txName} included in block:`, ev.block.hash);
                            sub.unsubscribe();
                            resolve(ev);
                        }
                        break;
                    case TX_MODE_IN_POOL:
                        if (ev.type === "broadcasted") {
                            console.log(`📦 ${txName} broadcasted with txHash:`, ev.txHash);
                            sub.unsubscribe();
                            resolve(ev);
                        }
                        break;
                    case TX_MODE_FINALIZED_BLOCK:
                        if (ev.type === "finalized") {
                            console.log(`📦 ${txName} included in finalized block:`, ev.block.hash);
                            sub.unsubscribe();
                            resolve(ev);
                        }
                        break;

                    default:
                        throw new Error("Unhandled txMode: " + txMode)
                }
            },
            error: (err) => {
                console.error(`❌ ${txName} error:`, err);
                sub.unsubscribe();
                reject(err);
            },
            complete: () => {
                console.log(`✅ ${txName} complete!`);
            }
        });
    });
}

export async function fetchCid(httpIpfsApi, cid, maxRetries = 10, initialDelay = 2000) {
    const contentUrl = `${httpIpfsApi}/ipfs/${cid.toString()}`;
    console.log('⬇️ Downloading the full content (no chunking) by cid from url: ', contentUrl);
    
    let lastError;
    for (let attempt = 0; attempt < maxRetries; attempt++) {
        try {
            const res = await fetch(contentUrl);
            if (res.ok) {
                console.log(`✅ Content fetched successfully on attempt ${attempt + 1}`);
                return Buffer.from(await res.arrayBuffer());
            }
            
            // If we get a 404 or 504, retry (content might not be available yet)
            if (res.status === 404 || res.status === 504 || res.status === 502) {
                lastError = new Error(`HTTP error ${res.status}`);
                const delay = initialDelay * Math.pow(1.5, attempt);
                console.log(`⏳ Attempt ${attempt + 1}/${maxRetries} failed with status ${res.status}, retrying in ${delay}ms...`);
                await new Promise(resolve => setTimeout(resolve, delay));
                continue;
            }
            
            // For other errors, throw immediately
            throw new Error(`HTTP error ${res.status}`);
        } catch (error) {
            // Network errors, timeouts, etc.
            lastError = error;
            if (attempt < maxRetries - 1) {
                const delay = initialDelay * Math.pow(1.5, attempt);
                console.log(`⏳ Attempt ${attempt + 1}/${maxRetries} failed with error: ${error.message}, retrying in ${delay}ms...`);
                await new Promise(resolve => setTimeout(resolve, delay));
            }
        }
    }
    
    throw new Error(`Failed to fetch CID after ${maxRetries} attempts. Last error: ${lastError?.message}`);
}
