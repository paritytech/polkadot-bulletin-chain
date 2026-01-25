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

/**
 * Run a preimage authorization + store test.
 *
 * @param {string} testName - Name of the test for logging
 * @param {object} bulletinAPI - PAPI typed API
 * @param {object} sudoSigner - Sudo signer for authorization
 * @param {object|null} signer - Signer for store (null for unsigned)
 * @param {number|null} cidCodec - CID codec (null for default)
 * @param {number|null} mhCode - Multihash code (null for default)
 * @param {object|null} client - Client for unsigned transactions
 */
async function runPreimageStoreTest(testName, bulletinAPI, sudoSigner, signer, cidCodec, mhCode, client) {
    console.log(`\n========== ${testName} ==========\n`);

    // Data to store
    const dataToStore = `Hello, Bulletin - ${testName} - ${new Date().toString()}`;

    // Compute expected CID (use undefined to get defaults, since null overrides them)
    const expectedCid = await cidFromBytes(dataToStore, cidCodec ?? undefined, mhCode ?? undefined);

    // Authorization always uses blake2_256 hash (pallet internal behavior)
    const contentHash = getContentHash(dataToStore);

    // Authorize the preimage
    await authorizePreimage(
        bulletinAPI,
        sudoSigner,
        contentHash,
        BigInt(dataToStore.length)
    );

    // Store data
    const cid = await store(bulletinAPI, signer, dataToStore, cidCodec, mhCode, TX_MODE_IN_BLOCK, client);
    console.log("✅ Data stored successfully with CID:", cid.toString());

    // Read back from IPFS
    const downloadedContent = await fetchCid(HTTP_IPFS_API, cid);
    console.log("✅ Downloaded content:", downloadedContent.toString());

    // Verify CID matches
    assert.deepStrictEqual(
        cid.toString(),
        expectedCid.toString(),
        '❌ Expected CID does not match actual CID!'
    );

    // Verify content matches
    assert.deepStrictEqual(
        dataToStore,
        downloadedContent.toString(),
        '❌ Stored data does not match downloaded content!'
    );

    console.log(`✅ Verified content!`);
}

async function main() {
    await cryptoWaitReady();

    let client, resultCode;
    try {
        // Init WS PAPI client and typed api.
        client = createClient(getWsProvider(NODE_WS));
        const bulletinAPI = client.getTypedApi(bulletin);

        // Signers.
        const { sudoSigner, whoSigner, _ } = setupKeyringAndSigners('//Alice', '//Preimagesigner');

        // Test 1: Unsigned store with preimage auth (default CID config)
        await runPreimageStoreTest(
            "Test 1: Unsigned store with preimage auth",
            bulletinAPI,
            sudoSigner,
            null,   // unsigned
            null,   // default codec
            null,   // default hash
            client
        );

        // Test 2: Signed store with preimage auth and custom CID config (DAG-PB + SHA2-256)
        await runPreimageStoreTest(
            "Test 2: Signed store with preimage auth and custom CID",
            bulletinAPI,
            sudoSigner,
            whoSigner,  // signed
            0x70,       // dag-pb
            0x12,       // sha2-256
            client
        );

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
