import { ApiPromise, WsProvider } from '@polkadot/api';
import { Keyring } from '@polkadot/keyring';
import { cryptoWaitReady, blake2AsU8a } from '@polkadot/util-crypto';
import * as dagPB from '@ipld/dag-pb';
import { CID } from 'multiformats/cid';
import * as sha256 from 'multiformats/hashes/sha2';
import * as multihash from 'multiformats/hashes/digest';
import { create } from 'ipfs-http-client';
import { TextDecoder } from 'util';
import { UnixFS } from 'ipfs-unixfs'

async function authorizeAccount(api, pair, who, transactions, bytes) {
    const tx = api.tx.transactionStorage.authorizeAccount(who, transactions, bytes);
    const sudo_tx = api.tx.sudo.sudo(tx);
    const result = await sudo_tx.signAndSend(pair);
    console.log('Transaction authorizeAccount result:', result.toHuman());
}

async function storeProof(api, pair, proof) {
    console.log('\n\n\=====Storing proof:\n', proof);
    let { nonce } = await api.query.system.account(pair.address);
    const tx = api.tx.system.remark(proof);
    const sudo_tx = api.tx.sudo.sudo(tx);
    const result = await sudo_tx.signAndSend(pair, { nonce });
    console.log('Transaction storeProof result: ', result);
    console.log("=====\n\n\n");
}

async function store(api, pair, data, nonce) {
    console.log('Storing data:', data);

    // 1️⃣ Hash the data using blake2b-256
    const hash = blake2AsU8a(data)
    // 2️⃣ Wrap the hash as a multihash
    const mh = multihash.create(0xb220, hash); // 0xb220 = blake2b-256
    // 3️⃣ Generate CID (CIDv1, raw codec)
    const cid = CID.createV1(0x55, mh); // 0x55 = raw codec

    // submit transaction
    console.log('Sending store transaction: ', nonce);
    const tx = api.tx.transactionStorage.store(data);
    const result = await tx.signAndSend(pair, { nonce });
    console.log('Transaction store result:', result.toHuman());
    return cid
}

// Connect to a local IPFS node or Infura/IPFS gateway
const ipfs = create({
    url: 'http://127.0.0.1:5001', // Local IPFS API
});

async function read_from_ipfs(cid) {
    // Fetch the block (downloads via Bitswap if not local)
    console.log('\n\n\n=================\nTrying to get cid: ', cid);
    const block = await ipfs.block.get(cid);
    console.log('Received block: ', block);
    return block
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

    console.log('\n============================\n1. Doing authorization...');
    await authorizeAccount(api, sudo_pair, who, transactions, bytes);
    await new Promise(resolve => setTimeout(resolve, 7000));
    let { nonce } = await api.query.system.account(who_pair.address);
    console.log('Authorized!');

    // 2. Store partial chunks (for example, split a big picture or video into chunks)
    console.log('\n============================\n2. Storing data...');
    const contents = [
        "Hello, Bulletin remote1 - " + new Date().toString(),
        "Hello, Bulletin remote2 - " + new Date().toString(),
        "Hello, Bulletin remote3 - " + new Date().toString()
    ];
    const chunks = [];
    for (let i = 0; i < contents.length; i++) {
        const cid = await store(api, who_pair, contents[i], nonce.addn(i));
        console.log(`Stored data with CID${i + 1}:`, cid);
        chunks.push({ cid, len: contents[i].length });
    }
    await new Promise(resolve => setTimeout(resolve, 5000));

    // 3. Read partial chunks
    console.log('\n============================\n3. Reading partial contents...');
    for (let i = 0; i < chunks.length; i++) {
        const {cid, len} = chunks[i];
        const content = await read_from_ipfs(cid);
        console.log(`[*] CID${i + 1} ${cid} as bytes (${len} / ${content.length}), content:\n${new TextDecoder().decode(content)}`);
    }

    // 4. Store DAG file
    console.log('\n============================\n4. DAG handling...');
    // Construct UnixFS file DAG
    const blocksizes = chunks.map(c => c.len)
    const totalSize = blocksizes.reduce((a, b) => a + b, 0)

    const fileData = new UnixFS({
        type: 'file',
        fileSize: totalSize,
        blocksizes
    })
    const dagNode = dagPB.prepare({
        Data: fileData.marshal(),
        Links: chunks.map(({ cid, len }) => ({
            Name: '',
            Tsize: len,
                Hash: cid
        }))
    });
    // Dag Encode + hash
    const dagNodeAsBytes = dagPB.encode(dagNode);
    const dagHash = await sha256.sha256.digest(dagNodeAsBytes);
    const expectedRootCid = CID.createV1(dagPB.code, dagHash);
    console.log(`[DAG] ExpectedRootCid: ${expectedRootCid}`);

    // Store DAG-PB file with rootCID as and on-chain proof, can be custom pallet/state
    console.log('\n============================\n5. Storing DAG-PB proof...');
    await storeProof(api, who_pair, expectedRootCid);

    // Store DAG-PB node in IPFS
    // TODO: replace with store would work?
    const dagCid = await ipfs.block.put(dagNodeAsBytes, {
        format: 'dag-pb',
        mhtype: 'sha2-256'
    })
    console.log("[DAG] RootCID:", dagCid.toString())

    // 4. Retrieve DAG-PB node and content from IPFS
    console.log('\n============================\n6.Reading chunks by DAG file:');
    const dagResult = await ipfs.dag.get(dagCid);
    console.log('Retrieved DAG-PB node:', JSON.stringify(dagResult.value, null, 2));

    // Read each linked chunk from IPFS
    for (const link of dagResult.value.Links) {
        const bytes = [];
        for await (const chunk of ipfs.cat(link.Hash)) {
            bytes.push(chunk);
        }
        const content = Buffer.concat(bytes);
        console.log(` [*] Chunk (${link.Hash}) length ${link.Tsize}:, content:\n`, new TextDecoder().decode(content));
    }

    // Reading the content by rootCID
    console.log('\n============================\n6.Reading the content by rootCID:');
    const stored_chunks = [];
    for await (const chunk of ipfs.cat(dagCid)) {
        stored_chunks.push(chunk);
    }
    const content = Buffer.concat(stored_chunks);
    console.log('[*] Content:\n', new TextDecoder().decode(content));
    // TODO: verify if http IPFS gateway returns the content correctly for rootCID

    await api.disconnect();
}

main().catch(console.error);
