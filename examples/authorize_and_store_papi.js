import assert from "assert";
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import {authorizeAccount, fetchCid, store} from './api.js';
import {
    setupKeyringAndSigners,
    AUTH_TRANSACTIONS,
    AUTH_BYTES,
    ALICE_ADDRESS,
    cidFromBytes
} from './common.js';
import { bulletin } from './.papi/descriptors/dist/index.mjs';

const NODE_WS = 'ws://localhost:10000';
const HTTP_IPFS_API = 'http://127.0.0.1:8080'   // Local IPFS HTTP gateway

async function main() {
    await cryptoWaitReady();
    
    let client;
    
    try {
        client = createClient(getWsProvider(NODE_WS));
        
        const bulletinAPI = client.getTypedApi(bulletin);

        const { sudoSigner, whoSigner } = setupKeyringAndSigners();

        const dataToStore = "Hello, Bulletin with PAPI - " + new Date().toString();
        let expectedCid = await cidFromBytes(dataToStore);

        await authorizeAccount(
            bulletinAPI,
            sudoSigner,
            ALICE_ADDRESS,
            AUTH_TRANSACTIONS,
            AUTH_BYTES
        );
        
        const cid = await store(bulletinAPI, whoSigner, dataToStore);
        console.log("✅ Data stored successfully with CID:", cid);

        // Read back from IPFS
        let downloadedContent = await fetchCid(HTTP_IPFS_API, cid);
        console.log("✅ Downloaded content:", downloadedContent.toString());
        assert.deepStrictEqual(
            cid,
            expectedCid,
            '❌ expectedCid does not match cid!'
        );
        assert.deepStrictEqual(
            dataToStore,
            downloadedContent.toString(),
            '❌ dataToStore does not match downloadedContent!'
        );
        console.log(`✅ Verified content - test passed!`);
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

