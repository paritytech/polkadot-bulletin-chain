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
