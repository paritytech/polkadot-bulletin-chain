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
    console.log('Storing data...');
    const cid = cidFromBytes(data);

    const dataBytes = typeof data === 'string' ?
        new Uint8Array(Buffer.from(data)) :
        new Uint8Array(data);

    const binaryData = Binary.fromBytes(dataBytes);
    const tx = typedApi.tx.TransactionStorage.store({ data: binaryData });

    await waitForTransaction(tx, signer, "Store");
    
    console.log("✅ Expected CID:", cid);
    return cid;
}

function waitForTransaction(tx, signer, txName) {
    return new Promise((resolve, reject) => {
        const sub = tx.signSubmitAndWatch(signer).subscribe({
            next: (ev) => {
                console.log(`✅ ${txName} event:`, ev.type);
                if (ev.type === "txBestBlocksState" && ev.found) {
                    console.log(`📦 ${txName} included in block:`, ev.block.hash);
                    sub.unsubscribe();
                    resolve(ev);
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
