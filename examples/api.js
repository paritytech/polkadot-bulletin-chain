import {cidFromBytes} from "./common.js";
import { Binary } from '@polkadot-api/substrate-bindings';

export async function authorizeAccount(typedApi, sudoPair, who, transactions, bytes) {
    console.log('Creating authorizeAccount transaction...');

    const authorizeTx = typedApi.tx.TransactionStorage.authorize_account({
        who,
        transactions,
        bytes
    });

    const sudoTx = typedApi.tx.Sudo.sudo({
        call: authorizeTx.decodedCall
    });

    // Wait for a new block.
    return new Promise((resolve, reject) => {
        const sub = sudoTx
            .signSubmitAndWatch(sudoPair)
            .subscribe({
                next: (ev) => {
                    if (ev.type === "txBestBlocksState" && ev.found) {
                        console.log("📦 Included in block:", ev.block.hash);
                        sub.unsubscribe();
                        resolve(ev);
                    }
                },
                error: (err) => {
                    console.log("Error:", err);
                    sub.unsubscribe();
                    reject(err);
                },
                complete: () => {
                    console.log("Subscription complete");
                }
            });
    })
}

export async function store(typedApi, pair, data) {
    console.log('Storing data:', data);
    const cid = cidFromBytes(data);

    // Convert data to Uint8Array then wrap in Binary for PAPI typed API
    const dataBytes = typeof data === 'string' ?
        new Uint8Array(Buffer.from(data)) :
        new Uint8Array(data);

    // Wrap in Binary object for typed API - pass as an object with 'data' property
    const binaryData = Binary.fromBytes(dataBytes);
    const tx = typedApi.tx.TransactionStorage.store({ data: binaryData });

    // Wait for a new block.
    return new Promise((resolve, reject) => {
        const sub = tx
            .signSubmitAndWatch(pair)
            .subscribe({
                next: (ev) => {
                    if (ev.type === "txBestBlocksState" && ev.found) {
                        console.log("📦 Included in block:", ev.block.hash);
                        sub.unsubscribe();
                        resolve(cid);
                    }
                },
                error: (err) => {
                    console.log("Error:", err);
                    sub.unsubscribe();
                    reject(err);
                },
                complete: () => {
                    console.log("Subscription complete");
                }
            });
    })
}