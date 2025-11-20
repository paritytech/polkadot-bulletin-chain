// npm install @polkadot/api @polkadot/keyring @polkadot/util-crypto @polkadot/util multiformats ipfs-http-client
// ipfs daemon &
// ipfs swarm connect /ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm
// ipfs swarm connect /ip4/127.0.0.1/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby
// ipfs swarm peers - should be there
// ipfs bitswap stat
// ipfs block get /ipfs/bafk2bzacebcnty2x5l3jr2sk5rvn7engdfkugpsqfpggl4nzazpieyemw6xme

import { ApiPromise, WsProvider } from '@polkadot/api';
import { Keyring } from '@polkadot/keyring';
import { cryptoWaitReady, blake2AsU8a } from '@polkadot/util-crypto';
import { CID } from 'multiformats/cid';
import * as multihash from 'multiformats/hashes/digest';
import { create } from 'ipfs-http-client';
import { waitForNewBlock, cidFromBytes } from './common.js';

async function authorizeAccount(api, pair, who, transactions, bytes) {
    const tx = api.tx.transactionStorage.authorizeAccount(who, transactions, bytes);
    const sudo_tx = api.tx.sudo.sudo(tx);
    const result = await sudo_tx.signAndSend(pair);
    console.log('Transaction authorizeAccount result:', result.toHuman());
}

async function store(api, pair, data) {
    console.log('Storing data:', data);
    const cid = cidFromBytes(data);

    const tx = api.tx.transactionStorage.store(data);
    const result = await tx.signAndSend(pair);
    console.log('Transaction store result:', result.toHuman());
    
    return cid;
}
// Connect to a local IPFS gateway (e.g. Kubo)
const ipfs = create({
    url: 'http://127.0.0.1:5001', // Local IPFS API
});

async function read_from_ipfs(cid) {
    // Fetch the block (downloads via Bitswap if not local)
    console.log('Trying to get cid: ', cid);
    const block = await ipfs.block.get(cid, {timeout: 15000});
    console.log('Received block: ', block);
    if (block.length !== 0) {
        return block
    }

    // Fetch the content from IPFS
    console.log('Trying to chunk cid: ', cid);
    const chunks = [];
    for await (const chunk of ipfs.cat(cid)) {
        chunks.push(chunk);
    }

    const content = Buffer.concat(chunks);
    return content
}

async function main() {
    await cryptoWaitReady();

    const ws = new WsProvider('ws://localhost:10000');
    const api = await ApiPromise.create({ provider: ws });
    await api.isReady;

    const keyring = new Keyring({ type: 'sr25519' });
    const sudo_pair = keyring.addFromUri('//Alice');
    const who_pair = keyring.addFromUri('//Alice');

    // data
    const who = who_pair.address; // ✅ base58 string
    const transactions = 32;
    const bytes = 64 * 1024 * 1024; // 64 MB

    console.log('Doing authorization...');
    await authorizeAccount(api, sudo_pair, who, transactions, bytes);
    await waitForNewBlock();
    console.log('Authorized!');

    console.log('Storing data ...');
    let cid = await store(api, who_pair, "Hello, Bulletin remote3 - " + new Date().toString());
    console.log('Stored data with CID: ', cid);
    await waitForNewBlock();

    console.log('Reading content... cid: ', cid);
    let content = await read_from_ipfs(cid);
    console.log('Content as bytes:', content);
    console.log('Content as string:', content.toString());

    await api.disconnect();
}

main().catch(console.error);
