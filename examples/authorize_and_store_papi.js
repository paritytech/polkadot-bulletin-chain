import { createClient } from 'polkadot-api';
import { Binary } from '@polkadot-api/substrate-bindings';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { getPolkadotSigner } from '@polkadot-api/signer';
import { Keyring } from '@polkadot/keyring';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { create } from 'ipfs-http-client';
import { cidFromBytes, to_hashing_enum } from './common.js';
import { bulletin } from './.papi/descriptors/dist/index.mjs';
import { withPolkadotSdkCompat } from "polkadot-api/polkadot-sdk-compat"
import assert from "assert";

async function authorizeAccount(typedApi, sudoPair, who, transactions, bytes) {
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

async function store(typedApi, pair, data, cidCodec, mhCode) {
    console.log(`Storing (with cidCodec: ${cidCodec}) data: `, data);

    // Convert data to Uint8Array then wrap in Binary for PAPI typed API
    const dataBytes = typeof data === 'string' ? 
        new Uint8Array(Buffer.from(data)) : 
        new Uint8Array(data);
    
    // Wrap in Binary object for typed API - pass as an object with 'data' property
    const binaryData = Binary.fromBytes(dataBytes);
    const tx = typedApi.tx.TransactionStorage.store({ data: binaryData });

    // Add custom `TransactionExtension` for codec, if specified.
    const txOpts = {};
    let expectedCid;
    if (cidCodec != null) {
        txOpts.customSignedExtensions = {
            ProvideCidConfig: {
                value: {
                    codec: BigInt(cidCodec),
                    hashing: to_hashing_enum(mhCode),
                }
            }
        };
        expectedCid = cidFromBytes(data, cidCodec, mhCode);
    } else {
        expectedCid = cidFromBytes(data);
    }

    // Wait for a new block.
    return new Promise((resolve, reject) => {
        const sub = tx
            .signSubmitAndWatch(pair, txOpts)
            .subscribe({
                next: (ev) => {
                    if (ev.type === "txBestBlocksState" && ev.found) {
                        console.log("📦 Included in block:", ev.block.hash);
                        sub.unsubscribe();
                        resolve(expectedCid);
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

// Connect to a local IPFS gateway (e.g. Kubo)
const ipfs = create({
    url: 'http://127.0.0.1:5001', // Local IPFS API
});

async function read_from_ipfs(cid) {
    // Fetch the block (downloads via Bitswap if not local)
    console.log('Trying to get cid: ', cid);
    try {
        const block = await ipfs.block.get(cid, {timeout: 10000});
        console.log('Received block: ', block);
        if (block.length !== 0) {
            return block;
        }
    } catch (error) {
        console.log('Block not found directly, trying cat...', error.message);
    }

    // Fetch the content from IPFS
    console.log('Trying to chunk cid: ', cid);
    const chunks = [];
    for await (const chunk of ipfs.cat(cid)) {
        chunks.push(chunk);
    }

    const content = Buffer.concat(chunks);
    return content;
}

// Global client reference for cleanup
let client;

async function main() {
    await cryptoWaitReady();

    // Create PAPI client with WebSocket provider
    client = createClient(withPolkadotSdkCompat(getWsProvider('ws://localhost:10000')));
    
    // Get typed API with generated descriptors
    const typedApi = client.getTypedApi(bulletin);

    // Create keyring and accounts
    const keyring = new Keyring({ type: 'sr25519' });
    const sudoAccount = keyring.addFromUri('//Alice');
    const whoAccount = keyring.addFromUri('//Alice');

    // Create PAPI-compatible signers using @polkadot-api/signer
    // getPolkadotSigner expects (publicKey: Uint8Array, signingType, sign function)
    const sudoSigner = getPolkadotSigner(
        sudoAccount.publicKey,
        'Sr25519',
        (input) => sudoAccount.sign(input)
    );
    const whoSigner = getPolkadotSigner(
        whoAccount.publicKey,
        'Sr25519',
        (input) => whoAccount.sign(input)
    );

    // Data
    const who = whoAccount.address;
    const transactions = 32; // u32 - regular number
    const bytes = 64n * 1024n * 1024n; // u64 - BigInt for large numbers

    console.log('Doing authorization...');
    await authorizeAccount(typedApi, sudoSigner, who, transactions, bytes);
    console.log('Authorized!');

    console.log('\n\nStoring data ...');
    const dataToStore = "Hello, Bulletin with PAPI - " + new Date().toString();
    // Raw CID without any codec - defaults to 0x55 and Blake2b256.
    let cidRawBlake2b256 = await store(typedApi, whoSigner, dataToStore);
    console.log('Stored data with cidRawBlake2b256: ', cidRawBlake2b256);

    // Raw CID without any codec - defaults to 0x55 but using Sha2_256.
    let cidRawSha2_256 = await store(typedApi, whoSigner, dataToStore, 0x55, 0x12);
    console.log('Stored data with CID: ', cidRawSha2_256);

    console.log('Reading content... cid_raw: ', cidRawBlake2b256);
    let content1 = await read_from_ipfs(cidRawBlake2b256);
    let content2 = await read_from_ipfs(cidRawSha2_256);
    assert.deepStrictEqual(
        content1.buffer,
        content2.buffer,
        '❌ content1 does not match content2!'
    );
    assert.notDeepStrictEqual(
        cidRawBlake2b256,
        cidRawSha2_256,
        '❌ cidRaw can not match cidWithCodec!'
    );
    console.log(`✅ Verified contents and CIDs!`);
}

main().catch(console.error).finally(() => {
    if (client) client.destroy();
});

