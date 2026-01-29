import assert from "assert";
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { authorizeAccount, authorizePreimage, fetchCid, store, TX_MODE_IN_BLOCK, TX_MODE_FINALIZED_BLOCK } from './api.js';
import { setupKeyringAndSigners, getContentHash } from './common.js';
import { logHeader, logConnection, logSection, logSuccess, logError, logInfo, logTestResult } from './logger.js';
import { cidFromBytes } from "./cid_dag_metadata.js";
import { bulletin } from './.papi/descriptors/dist/index.mjs';

// Command line arguments: [ws_url] [seed] [ipfs_api_url]
const args = process.argv.slice(2);
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Alice';
const HTTP_IPFS_API = args[2] || 'http://127.0.0.1:8080';

/**
 * Run a preimage authorization + store test.
 *
 * @param {string} testName - Name of the test for logging
 * @param {object} bulletinAPI - PAPI typed API
 * @param {object} sudoSigner - Sudo signer for authorization
 * @param {object|null} signer - Signer for store (null for unsigned)
 * @param {string|null} signerAddress - Address of the signer (required if signer is not null)
 * @param {number|null} cidCodec - CID codec (null for default)
 * @param {number|null} mhCode - Multihash code (null for default)
 * @param {object|null} client - Client for unsigned transactions
 */
async function runPreimageStoreTest(testName, bulletinAPI, sudoSigner, signer, signerAddress, cidCodec, mhCode, client) {
    logSection(testName);

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
        BigInt(dataToStore.length),
        TX_MODE_FINALIZED_BLOCK
    );

    // If signer is provided, also authorize the account (to increment inc_providers/inc_sufficients for `CheckNonce`).
    if (signer != null && signerAddress != null) {
        logInfo(`Also authorizing account ${signerAddress} to verify preimage auth is preferred`);
        await authorizeAccount(
            bulletinAPI,
            sudoSigner,
            signerAddress,
            10,        // dummy transactions
            BigInt(10000),  // dummy bytes
            TX_MODE_FINALIZED_BLOCK
        );
    }

    // Store data
    const { cid } = await store(bulletinAPI, signer, dataToStore, cidCodec, mhCode, TX_MODE_IN_BLOCK, client);
    logSuccess(`Data stored successfully with CID: ${cid.toString()}`);

    // Read back from IPFS
    const downloadedContent = await fetchCid(HTTP_IPFS_API, cid);
    logSuccess(`Downloaded content: ${downloadedContent.toString()}`);

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

    logSuccess('Verified content!');
}

async function main() {
    await cryptoWaitReady();

    logHeader('AUTHORIZE PREIMAGE AND STORE TEST');
    logConnection(NODE_WS, SEED, HTTP_IPFS_API);

    let client, resultCode;
    try {
        // Init WS PAPI client and typed api.
        client = createClient(getWsProvider(NODE_WS));
        const bulletinAPI = client.getTypedApi(bulletin);

        // Signers.
        const { sudoSigner, whoSigner, whoAddress } = setupKeyringAndSigners(SEED, '//Preimagesigner');

        // Test 1: Unsigned store with preimage auth (default CID config)
        await runPreimageStoreTest(
            "Test 1: Unsigned store with preimage auth",
            bulletinAPI,
            sudoSigner,
            null,       // unsigned
            null,       // no signer address
            null,       // default codec
            null,       // default hash
            client
        );

        // Test 2: Signed store with preimage auth and custom CID config (raw + SHA2-256)
        // Also authorizes account to verify preimage auth is preferred
        await runPreimageStoreTest(
            "Test 2: Signed store with preimage auth and custom CID",
            bulletinAPI,
            sudoSigner,
            whoSigner,      // signed
            whoAddress,     // signer address for account auth
            0x55,           // raw
            0x12,           // sha2-256
            client
        );

        logTestResult(true, 'Authorize Preimage and Store Test');
        resultCode = 0;
    } catch (error) {
        logError(`Error: ${error.message}`);
        console.error(error);
        resultCode = 1;
    } finally {
        if (client) client.destroy();
        process.exit(resultCode);
    }
}

await main();
