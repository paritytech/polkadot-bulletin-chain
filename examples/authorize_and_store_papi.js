// npm install polkadot-api @polkadot-api/pjs-signer @polkadot/keyring @polkadot/util-crypto multiformats ipfs-http-client
// npx papi add -w ws://localhost:10000 bulletin
// ipfs daemon &
// ipfs swarm connect /ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm
// ipfs swarm connect /ip4/127.0.0.1/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKugpsqfpggl4nzazpieyemw6xme
// ipfs swarm peers - should be there
// ipfs bitswap stat
// ipfs block get /ipfs/bafk2bzacebcnty2x5l3jr2sk5rvn7engdfkugpsqfpggl4nzazpieyemw6xme

import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';
import { getPolkadotSignerFromPjs } from '@polkadot-api/pjs-signer';
import { Keyring } from '@polkadot/keyring';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { create } from 'ipfs-http-client';
import { waitForNewBlock, cidFromBytes } from './common.js';
import { bulletin } from '@polkadot-api/descriptors';

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
    
    const result = await sudoTx.signAndSubmit(sudoPair);
    console.log('Transaction authorizeAccount submitted:', result);
    return result;
}

async function store(typedApi, pair, data) {
    console.log('Storing data:', data);
    const cid = cidFromBytes(data);
    
    const tx = typedApi.tx.TransactionStorage.store({
        data: Array.from(typeof data === 'string' ? Buffer.from(data) : data)
    });
    
    const result = await tx.signAndSubmit(pair);
    console.log('Transaction store submitted:', result);
    
    return cid;
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

async function main() {
    await cryptoWaitReady();

    // Create PAPI client with WebSocket provider
    const wsProvider = getWsProvider('ws://localhost:10000');
    const client = createClient(wsProvider);
    
    // Get typed API - requires generated descriptors
    const typedApi = client.getTypedApi(bulletin);

    // Create keyring and accounts
    const keyring = new Keyring({ type: 'sr25519' });
    const sudoAccount = keyring.addFromUri('//Alice');
    const whoAccount = keyring.addFromUri('//Alice');

    // Create PAPI-compatible signers using pjs-signer
    const sudoSigner = getPolkadotSignerFromPjs(sudoAccount);
    const whoSigner = getPolkadotSignerFromPjs(whoAccount);

    // Data
    const who = whoAccount.address;
    const transactions = 32; // u32 - regular number
    const bytes = 64n * 1024n * 1024n; // u64 - BigInt for large numbers

    console.log('Doing authorization...');
    await authorizeAccount(typedApi, sudoSigner, who, transactions, bytes);
    await waitForNewBlock();
    console.log('Authorized!');

    console.log('Storing data ...');
    const dataToStore = "Hello, Bulletin with PAPI - " + new Date().toString();
    let cid = await store(typedApi, whoSigner, dataToStore);
    console.log('Stored data with CID: ', cid);
    await waitForNewBlock();

    console.log('Reading content... cid: ', cid);
    let content = await read_from_ipfs(cid);
    console.log('Content as bytes:', content);
    console.log('Content as string:', content.toString());

    client.destroy();
}

main().catch(console.error);

