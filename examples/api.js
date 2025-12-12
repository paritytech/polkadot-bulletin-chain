import { cidFromBytes } from "./common.js";
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

export async function fetchCid(httpIpfsApi, cid) {
    const contentUrl = `${httpIpfsApi}/ipfs/${cid.toString()}`;
    console.log('⬇️ Downloading the full content (no chunking) by cid from url: ', contentUrl);
    const res = await fetch(contentUrl);
    if (!res.ok) throw new Error(`HTTP error ${res.status}`);
    return Buffer.from(await res.arrayBuffer())
}
