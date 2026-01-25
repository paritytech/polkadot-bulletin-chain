import assert from "assert";
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { authorizePreimage, fetchCid, store, TX_MODE_IN_BLOCK } from './api.js';
import { setupKeyringAndSigners, getContentHash } from './common.js';
import { cidFromBytes } from "./cid_dag_metadata.js";
import { bulletin } from './.papi/descriptors/dist/index.mjs';

const NODE_WS = 'ws://localhost:10000';
const HTTP_IPFS_API = 'http://127.0.0.1:8080'   // Local IPFS HTTP gateway

async function main() {
    await cryptoWaitReady();

    let client, resultCode;
    try {
        // Init WS PAPI client and typed api.
        client = createClient(getWsProvider(NODE_WS));
        const bulletinAPI = client.getTypedApi(bulletin);

        // Signers.
        const { sudoSigner, whoSigner, _ } = setupKeyringAndSigners('//Alice', '//Preimagesigner');

        // =====================================================================
        // Test 1: authorizePreimage + unsigned store (default CID config)
        // =====================================================================
        console.log(`\n========== Test 1: Unsigned store with preimage auth ==========\n`);

        // Data to store.
        const dataToStore = "Hello, Bulletin with PAPI - " + new Date().toString();
        let expectedCid = await cidFromBytes(dataToStore);
        let contentHash = getContentHash(dataToStore);

        // Authorize a preimage.
        await authorizePreimage(
            bulletinAPI,
            sudoSigner,
            contentHash,
            BigInt(dataToStore.length)
        );

        // Store data.
        const cid = await store(bulletinAPI, null, dataToStore, null, null, TX_MODE_IN_BLOCK, client);
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
        console.log(`✅ Verified content!`);

        // =====================================================================
        // Test 2: authorizePreimage + signed store with custom CID config
        // =====================================================================
        console.log(`\n\n========== Test 2: Signed store with preimage auth and custom CID ==========\n`);

        const dataToStore2 = "Hello, Bulletin with signed preimage auth and custom CID - " + new Date().toString();
        // Authorization always uses blake2_256 hash (pallet internal behavior)
        let contentHash2 = getContentHash(dataToStore2); // blake2_256 by default

        // Custom CID config: DAG-PB codec (0x70) with SHA2-256 hash (0x12)
        const cidCodec = 0x70;  // dag-pb
        const mhCode = 0x12;    // sha2-256
        let expectedCid2 = await cidFromBytes(dataToStore2, cidCodec, mhCode);

        // Authorize the preimage (uses blake2_256 hash internally)
        await authorizePreimage(
            bulletinAPI,
            sudoSigner,
            contentHash2,
            BigInt(dataToStore2.length)
        );

        // Store data with signer and custom CID config
        // Since preimage is authorized, it should use preimage auth instead of account auth
        const cid2 = await store(bulletinAPI, whoSigner, dataToStore2, cidCodec, mhCode, TX_MODE_IN_BLOCK, client);
        console.log("✅ Data stored successfully with custom CID:", cid2.toString());

        // Read back from IPFS
        let downloadedContent2 = await fetchCid(HTTP_IPFS_API, cid2);
        console.log("✅ Downloaded content:", downloadedContent2.toString());
        assert.deepStrictEqual(
            cid2.toString(),
            expectedCid2.toString(),
            '❌ expectedCid2 does not match cid2!'
        );
        assert.deepStrictEqual(
            dataToStore2,
            downloadedContent2.toString(),
            '❌ dataToStore2 does not match downloadedContent2!'
        );
        console.log(`✅ Verified content with custom CID config (DAG-PB + SHA2-256)!`);

        console.log(`\n\n\n✅✅✅ All tests passed! ✅✅✅`);
        resultCode = 0;
    } catch (error) {
        console.error("❌ Error:", error);
        resultCode = 1;
    } finally {
        if (client) client.destroy();
        process.exit(resultCode);
    }
}

await main();
