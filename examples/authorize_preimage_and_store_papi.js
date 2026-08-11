// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

/**
 * Authorize a preimage and store data on Bulletin Chain via the SDK.
 *
 * Covers two scenarios:
 *   1. Unsigned store with preimage auth (default CID config)
 *   2. Signed store with preimage auth + custom CID (raw codec + SHA2-256).
 *      The account is also `authorize_account`-authorized so we can confirm
 *      the preimage path is preferred when both are available.
 *
 * Usage:
 *   node authorize_preimage_and_store_papi.js [ws_url] [seed] [ipfs_api_url]
 *
 * Flags:
 *   --smoldot=<relay-spec>:<para-spec>   Use smoldot light client.
 *   --smoldot-sync-wait=N                Seconds to wait for smoldot sync (default 30).
 *   --signer-disc=XX                     Append discriminator to user seed.
 */

import assert from 'assert';
import { cryptoWaitReady } from '@polkadot/util-crypto';

import { bulletin } from './.papi/descriptors/dist/index.js';
import { blobFromItems, BulletinClient, WaitFor } from '../sdk/typescript/dist/index.mjs';

import { fetchAndVerifyBlock, gatewaySource, nodeRpcSource } from './api.js';
import { cidFromBytes } from './cid_dag_metadata.js';
import {
    setupKeyringAndSigners,
    getContentHash,
    waitForChainReady,
    waitForBlockProduction,
    parseProviderArgs,
    buildProviders,
    DEFAULT_IPFS_GATEWAY_URL,
} from './common.js';
import {
    logHeader,
    logConnection,
    logConfig,
    logSection,
    logSuccess,
    logError,
    logInfo,
    logTestResult,
} from './logger.js';

const args = process.argv.slice(2).filter(arg => !arg.startsWith('--'));
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Eve';
const HTTP_IPFS_API = args[2] || DEFAULT_IPFS_GATEWAY_URL;
const PROVIDER_CFG = parseProviderArgs(process.argv);
const signerDiscriminator =
    process.argv.find(arg => arg.startsWith('--signer-disc='))?.split('=')[1] ?? null;

/**
 * Run one preimage-auth + store iteration through the SDK.
 *
 * @param {object} args
 * @param {string} args.label                    test name (logging only)
 * @param {BulletinClient} args.client           SDK client (uploadSigner already set)
 * @param {string} args.userAddress              address whose account-level auth
 *                                               we ALSO grant for the signed test
 * @param {boolean} args.signed                  true → signed upload via uploadSigner
 *                                               false → unsigned (.asUnsigned())
 * @param {number|null} args.cidCodec            CID codec override or null for default
 * @param {number|null} args.mhCode              multihash code override or null
 */
async function runPreimageStoreTest({ label, client, userAddress, signed, cidCodec, mhCode }) {
    logSection(label);

    const dataToStore = `Hello, Bulletin - ${label} - ${new Date().toString()}`;
    const dataBytes = new TextEncoder().encode(dataToStore);

    // Authorization always uses blake2_256 hash (pallet internal behavior).
    const contentHash = getContentHash(dataToStore);
    const expectedCid = await cidFromBytes(dataToStore, cidCodec ?? undefined, mhCode ?? undefined);

    await client
        .authorizePreimage(contentHash, BigInt(dataToStore.length))
        .withWaitFor(WaitFor.Finalized)
        .send();
    logSuccess('Preimage authorized');

    if (signed) {
        logInfo(`Also authorizing account ${userAddress} to verify preimage auth is preferred`);
        await client
            .authorizeAccount(userAddress, 10, BigInt(10_000))
            .withWaitFor(WaitFor.Finalized)
            .send();
    }

    const item = { data: dataBytes };
    if (cidCodec != null) item.codec = cidCodec;
    if (mhCode != null) item.hashAlgo = mhCode;
    const items = [item];
    let builder = client
        .submit(await client.estimateUpload(items), blobFromItems(items))
        .withWaitFor(WaitFor.Finalized);
    if (!signed) builder = builder.asUnsigned();
    const { cids } = await builder.send();
    const cid = cids[0];
    logSuccess(`Data stored successfully with CID: ${cid.toString()}`);

    // Read back from the IPFS gateway and the node RPC, verifying both match.
    const downloadedContent = await fetchAndVerifyBlock(
        cid,
        gatewaySource(HTTP_IPFS_API),
        nodeRpcSource(client),
    );
    logSuccess(`Downloaded content: ${downloadedContent.toString()}`);

    assert.deepStrictEqual(
        cid.toString(),
        expectedCid.toString(),
        '❌ Expected CID does not match actual CID!',
    );
    assert.deepStrictEqual(
        dataToStore,
        downloadedContent.toString(),
        '❌ Stored data does not match downloaded content!',
    );
    logSuccess('Verified content!');
}

async function main() {
    await cryptoWaitReady();

    logHeader('AUTHORIZE PREIMAGE AND STORE TEST');
    if (PROVIDER_CFG.mode === 'smoldot') {
        logConfig({
            Mode: 'Smoldot Light Client',
            'Relay Spec': PROVIDER_CFG.relaySpecPath,
            'Para Spec': PROVIDER_CFG.paraSpecPath,
            'IPFS API': HTTP_IPFS_API,
        });
    } else {
        logConnection(NODE_WS, SEED, HTTP_IPFS_API);
    }

    let client, providersHandle, resultCode;
    try {
        providersHandle = await buildProviders({ ...PROVIDER_CFG, wsUrl: NODE_WS });

        const userSeed = signerDiscriminator
            ? `//Preimagesigner${signerDiscriminator}`
            : '//Preimagesigner';
        const { authorizationSigner, whoSigner, whoAddress } = setupKeyringAndSigners(SEED, userSeed);

        client = new BulletinClient({
            descriptor: bulletin,
            providers: providersHandle.providers,
            uploadSigner: whoSigner,
            authorizerSigner: authorizationSigner,
        });

        console.log('🔍 Checking if chain is ready...');
        await waitForChainReady(client.api);
        await waitForBlockProduction(client.api);

        // Test 1: Unsigned store with preimage auth (default CID).
        await runPreimageStoreTest({
            label: 'Test 1: Unsigned store with preimage auth',
            client,
            userAddress: whoAddress,
            signed: false,
            cidCodec: null,
            mhCode: null,
        });

        // Test 2: Signed store with preimage auth and custom CID (raw + SHA2-256).
        await runPreimageStoreTest({
            label: 'Test 2: Signed store with preimage auth and custom CID',
            client,
            userAddress: whoAddress,
            signed: true,
            cidCodec: 0x55, // raw
            mhCode: 0x12,   // sha2-256
        });

        logTestResult(true, 'Authorize Preimage and Store Test');
        resultCode = 0;
    } catch (error) {
        logError(`Error: ${error.message}`);
        console.error(error);
        resultCode = 1;
    } finally {
        if (client) await client.destroy();
        if (providersHandle) await providersHandle.cleanup();
        process.exit(resultCode);
    }
}

await main();
