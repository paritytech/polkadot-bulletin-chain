import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { create } from 'ipfs-http-client';
import { authorizeAccount, store } from './api.js';
import { setupKeyringAndSigners, AUTH_TRANSACTIONS, AUTH_BYTES, ALICE_ADDRESS } from './common.js';
import { bulletin } from './.papi/descriptors/dist/index.mjs';

// Generate PAPI descriptors using local node:
// npx papi add -w ws://localhost:10000 bulletin
// npx papi

const NODE_WS = 'ws://localhost:10000';

// Connect to a local IPFS gateway (e.g. Kubo)
const ipfs = create({
    url: 'http://127.0.0.1:5001', // Local IPFS API
});

async function readFromIPFS(cid) {
    console.log('Reading from IPFS, CID:', cid);
    try {
        const block = await ipfs.block.get(cid, { timeout: 10000 });
        console.log('Received block:', block);
        if (block.length !== 0) {
            return block;
        }
    } catch (error) {
        console.log('Block not found directly, trying cat...', error.message);
    }

    console.log('Trying to chunk CID:', cid);
    const chunks = [];
    for await (const chunk of ipfs.cat(cid)) {
        chunks.push(chunk);
    }

    return Buffer.concat(chunks);
}

async function main() {
    await cryptoWaitReady();
    
    let client;
    
    try {
        client = createClient(getWsProvider(NODE_WS));
        
        const bulletinAPI = client.getTypedApi(bulletin);

        const { sudoSigner, whoSigner } = setupKeyringAndSigners();

        const dataToStore = "Hello, Bulletin with PAPI - " + new Date().toString();

        await authorizeAccount(
            bulletinAPI,
            sudoSigner,
            ALICE_ADDRESS,
            AUTH_TRANSACTIONS,
            AUTH_BYTES
        );
        
        const cid = await store(bulletinAPI, whoSigner, dataToStore);
        console.log("✅ Data stored successfully with CID:", cid);

        // // Read back from IPFS
        // const content = await readFromIPFS(cid);
        // console.log('Content as bytes:', content);
        // console.log('Content as string:', content.toString());
        
    } catch (error) {
        console.error("❌ Error:", error);
        process.exit(1);
    } finally {
        // Cleanup
        if (client) client.destroy();
        process.exit(0);
    }
}

await main();

