/**
 * ============================================================================
 * KNOWN ISSUE: Test temporarily disabled in CI
 * ============================================================================
 *
 * This test validates unsigned store() transactions with preimage authorization,
 * a feature explicitly supported by the pallet (see check_unsigned() in
 * pallets/transaction-storage/src/lib.rs:1051-1095).
 *
 * Current Status: FAILING
 *   Error: InvalidTxError: { "type": "Invalid", "value": { "type": "Payment" } }
 *   Stage: Transaction broadcasts successfully but fails during pool validation
 *
 * Root Cause: Runtime Transaction Extension Pipeline Issue
 *   The runtime's ValidateSigned extension rejects unsigned transactions with
 *   "Invalid Payment" BEFORE the pallet's ValidateUnsigned can approve them.
 *
 *   Expected: Runtime → ValidateUnsigned → Pallet approves → Success
 *   Actual:   ValidateSigned rejects early → Never reaches pallet → Failure
 *
 * This is NOT a client SDK or test bug. The pallet correctly supports this use
 * case, but the runtime's transaction extension ordering prevents it from working.
 *
 * Solution: Fix ValidateSigned in runtime to skip unsigned transactions or adjust
 *           extension pipeline ordering. See examples/justfile:511-563 for detailed
 *           investigation and potential solutions.
 *
 * Test Status: Disabled in CI at examples/justfile:546 until runtime is fixed.
 * ============================================================================
 */

import assert from "assert";
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { authorizeAccount, authorizePreimage, fetchCid, store, TX_MODE_IN_BLOCK } from '../api.js';
import { setupKeyringAndSigners, getContentHash } from '../common.js';
import { cidFromBytes } from "../cid_dag_metadata.js";
import { bulletin } from '../.papi/descriptors/dist/index.mjs';

// Command line arguments: [ws_url] [seed] [http_ipfs_api]
const args = process.argv.slice(2);
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Alice';
const HTTP_IPFS_API = args[2] || 'http://127.0.0.1:8283';

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

    // If signer is provided, also authorize the account (to increment inc_providers/inc_sufficients for `CheckNonce`).
    if (signer != null && signerAddress != null) {
        console.log(`ℹ️ Also authorizing account ${signerAddress} to verify preimage auth is preferred`);
        await authorizeAccount(
            bulletinAPI,
            sudoSigner,
            signerAddress,
            10,        // dummy transactions
            BigInt(10000)  // dummy bytes
        );
    }

    // Store data
    const { cid } = await store(bulletinAPI, signer, dataToStore, cidCodec, mhCode, TX_MODE_IN_BLOCK, client);
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

    console.log(`Connecting to: ${NODE_WS}`);
    console.log(`Using seed: ${SEED}`);

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
