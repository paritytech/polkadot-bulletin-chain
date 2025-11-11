import { ApiPromise, WsProvider } from '@polkadot/api';
import { Keyring } from '@polkadot/keyring';
import { cryptoWaitReady, blake2AsU8a } from '@polkadot/util-crypto';
import * as dagPB from '@ipld/dag-pb';
import { CID } from 'multiformats/cid';
import * as sha256 from 'multiformats/hashes/sha2';
import * as multihash from 'multiformats/hashes/digest';
import { create } from 'ipfs-http-client';
import { TextDecoder } from 'util';
import assert from 'assert';
import { UnixFS } from 'ipfs-unixfs'

async function authorizeAccount(api, pair, who, transactions, bytes) {
    const tx = api.tx.transactionStorage.authorizeAccount(who, transactions, bytes);
    const sudo_tx = api.tx.sudo.sudo(tx);
    const result = await sudo_tx.signAndSend(pair);
    console.log('Transaction authorizeAccount result:', result.toHuman());
}

async function storeProof(api, pair, rootCID, dagFileBytes) {
    console.log(`[Proof] Storing proof for rootCID: ${rootCID}`);
    let { nonce } = await api.query.system.account(pair.address);
    // store proof aka DAG file as raw.
    let proof_cid = await store(api, pair, dagFileBytes, nonce);

    // This can be a serious pallet, this is just a demonstration.
    const proof = `ProofCid: ${proof_cid.toString()} -> rootCID: ${rootCID.toString()}`;
    const tx = api.tx.system.remark(proof);
    const sudo_tx = api.tx.sudo.sudo(tx);
    const result = await sudo_tx.signAndSend(pair, { nonce: nonce.addn(1) });
    if (result.isError) {
        console.error('Transaction failed', result.dispatchError?.toHuman());
        result
    } else {
        console.log(`\n[Proof] !!! Proof stored: ${proof}\n\n`);
    }
    return { proof_cid }
}

async function store(api, pair, data, nonce) {
    console.log(`Storing with nonce: ${nonce}, requested data: `, data);

    // 1️⃣ Hash the data using blake2b-256
    const hash = blake2AsU8a(data)
    // 2️⃣ Wrap the hash as a multihash
    const mh = multihash.create(0xb220, hash); // 0xb220 = blake2b-256
    // 3️⃣ Generate CID (CIDv1, raw codec)
    const cid = CID.createV1(0x55, mh); // 0x55 = raw codec

    // submit transaction
    const tx = api.tx.transactionStorage.store(data);
    const result = await tx.signAndSend(pair, { nonce });
    console.log('Transaction store result:', result.toHuman());
    return cid
}

// Connect to a local IPFS gateway (e.g. Kubo)
const ipfs = create({
    url: 'http://127.0.0.1:5001', // Local IPFS API
});
// For HTTP downloading
const ipfs_over_http = 'http://127.0.0.1:8080/ipfs/';

async function read_from_ipfs(cid) {
    // Fetch the block (downloads via Bitswap if not local)
    console.log('\n\n\n=================\nTrying to get cid: ', cid);
    const block = await ipfs.block.get(cid);
    console.log('Received block: ', block);
    return block
}

async function constructIpfsDAG_PB(chunks) {
    // Construct UnixFS file DAG
    const blockSizes = chunks.map(c => BigInt(c.len))
    const dagFileData = new UnixFS({
        type: 'file',
        blockSizes: blockSizes
    });
    console.log(`[DAG] BlockSizes: ${blockSizes}, dagFileData: `, dagFileData);
    // Important part
    const dagNode = dagPB.prepare({
        Data: dagFileData.marshal(),
        Links: chunks.map(({cid, len}) => ({
            Name: '',       // can leave empty
            Tsize: len,
            Hash: cid
        }))
    });
    console.log(`[DAG] dagNode: `, dagNode);
    // Dag Encode + hash
    const dagNodeAsBytes = dagPB.encode(dagNode);
    const expectedRootCid = await calculateRootCID(dagNodeAsBytes);
    return {dagNodeAsBytes, expectedRootCid};
}

async function calculateRootCID(dagNodeAsBytes) {
    const dagHash = await sha256.sha256.digest(dagNodeAsBytes);
    const expectedRootCid = CID.createV1(dagPB.code, dagHash);
    return expectedRootCid;
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

    // 1. Authorize an account
    console.log('\n============================\n||1. Doing authorization...');
    await authorizeAccount(api, sudo_pair, who, transactions, bytes);
    await new Promise(resolve => setTimeout(resolve, 7000));
    console.log('Authorized!');

    // 2. Store partial chunks (for example, split a big picture or video into chunks)
    console.log('\n============================\n||2. Storing chunked data...');
    let { nonce } = await api.query.system.account(who_pair.address);
    const contents = [
        "Hello, Bulletin remote1 - " + new Date().toString(),
        "Hello, Bulletin remote22 - " + new Date().toString(),
        "Hello, Bulletin remote333 - " + new Date().toString()
    ];
    const chunks = [];
    for (let i = 0; i < contents.length; i++) {
        const cid = await store(api, who_pair, contents[i], nonce.addn(i));
        console.log(`Stored data with CID${i + 1}:`, cid);
        // Collect CIDs and lengths for DAG construction.
        chunks.push({ cid, len: contents[i].length });
    }
    await new Promise(resolve => setTimeout(resolve, 5000));

    // 3. Store DAG file
    console.log('\n============================\n||3. DAG-PB/UnixFs handling...');
    const {dagNodeAsBytes, expectedRootCid} = await constructIpfsDAG_PB(chunks);
    console.log(`[DAG] Calculated/ExpectedRootCid: ${expectedRootCid}`);

    // 4. Store DAG-PB file (dagNode or dagNodeAsBytes) or just expectedRootCid as an on-chain proof.
    console.log('\n============================\n||4. Storing DAG-PB file/proof on-chain...');
    const { proof_cid } = await storeProof(api, who_pair, expectedRootCid, dagNodeAsBytes.buffer);
    await new Promise(resolve => setTimeout(resolve, 7000));
    // TODO: (just check for completion - how to reconstruct/verify).
    // let dagContent = await read_from_ipfs(proof_cid);
    // console.log(`dagContent: ${dagContent}`);
    // let storedRootCid = await calculateRootCID(dagContent)
    // assert.strictEqual(
    //     storedRootCid.toString(),
    //     expectedRootCid.toString(),
    //     '❌ DAG CID does not match expected root CID'
    // );

    // Store DAG-PB node in the IPFS (There are two options, how to do that),
    // so the IPFS HTTP gateways can read the whole just by rootCID.
    console.log('\n============================\n||5. !!! IPFS - putting the DAG-PB file, so HTTP gateways can download it as chunked!');
    // (Option 1)
    const rootCid = await ipfs.block.put(dagNodeAsBytes, {
        format: 'dag-pb',
        mhtype: 'sha2-256'
    });
    // (Option 2)
    // const rootCid = await ipfs.dag.put(dagNode, {
    //     storeCodec: 'dag-pb',
    //     hashAlg: 'sha2-256',
    //     pin: true
    // });
    console.log("[DAG] IPFS stored rootCID:", rootCid.toString())
    assert.strictEqual(
        rootCid.toString(),
        expectedRootCid.toString(),
        '❌ DAG CID does not match expected root CID'
    );

    // 6. Retrieve DAG-PB node and content from IPFS by chunks
    console.log('\n============================\n||6.Reading chunks by DAG file:');
    const dagResult = await ipfs.dag.get(rootCid);
    console.log('Retrieved DAG-PB node:', JSON.stringify(dagResult.value));

    // Reading the content by rootCID
    console.log('\n============================\n||7.Reading the content (`ipfs cat`) by rootCID: ', rootCid.toString());
    const stored_chunks = [];
    for await (const chunk of ipfs.cat(rootCid)) {
        stored_chunks.push(chunk);
    }
    const content = Buffer.concat(stored_chunks);
    console.log('Content:', new TextDecoder().decode(content));

    // Download the content from IPFS gateway
    const url = ipfs_over_http + rootCid.toString();
    console.log('\n============================\n||8. Downloading the full content (no chunking) by rootCID from url: ', url);
    const res = await fetch(url);
    if (!res.ok) throw new Error(`HTTP error ${res.status}`);

    const buffer = Buffer.from(await res.arrayBuffer());
    console.log('File size:', buffer.length);
    console.log('Content:', buffer.toString());

    await api.disconnect();
}

main().catch(console.error);
