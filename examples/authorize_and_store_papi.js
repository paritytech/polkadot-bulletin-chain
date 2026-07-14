// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

/**
 * Authorize an account and store a single payload on Bulletin Chain.
 *
 * Single source of truth for the smoke test — supports both ws and
 * smoldot light-client transports via the shared `buildProviders` helper.
 *
 * Usage:
 *   node authorize_and_store_papi.js [ws_url] [seed] [ipfs_api_url]
 *
 * Flags:
 *   --smoldot=<relay-spec>:<para-spec>
 *                                 Use smoldot light client instead of ws.
 *                                 Paths to relay + parachain chain spec
 *                                 JSON files, colon-separated.
 *   --smoldot-sync-wait=N         Seconds to wait for smoldot sync (default 30).
 *   --signer-disc=XX              Append discriminator to user seed for parallel CI runs.
 */

import assert from 'assert';
import { cryptoWaitReady } from '@polkadot/util-crypto';

import { bulletin } from './.papi/descriptors/dist/index.js';
import { blobFromBytes, BulletinClient, WaitFor } from '../sdk/typescript/dist/index.mjs';

import { fetchCid } from './api.js';
import { cidFromBytes } from './cid_dag_metadata.js';
import {
    setupKeyringAndSigners,
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
    logSuccess,
    logError,
    logTestResult,
} from './logger.js';

// -------------------- CLI args --------------------
const args = process.argv.slice(2).filter(arg => !arg.startsWith('--'));
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Eve';
const HTTP_IPFS_API = args[2] || DEFAULT_IPFS_GATEWAY_URL;
const PROVIDER_CFG = parseProviderArgs(process.argv);
const signerDiscriminator =
    process.argv.find(arg => arg.startsWith('--signer-disc='))?.split('=')[1] ?? null;

async function main() {
    await cryptoWaitReady();

    if (PROVIDER_CFG.mode === 'smoldot') {
        logHeader('AUTHORIZE AND STORE TEST (Smoldot Light Client)');
        logConfig({
            Mode: 'Smoldot Light Client',
            'Relay Spec': PROVIDER_CFG.relaySpecPath,
            'Para Spec': PROVIDER_CFG.paraSpecPath,
            'IPFS API': HTTP_IPFS_API,
        });
    } else {
        logHeader('AUTHORIZE AND STORE TEST (WebSocket)');
        logConnection(NODE_WS, SEED, HTTP_IPFS_API);
    }

    let client, providersHandle, resultCode;
    try {
        providersHandle = await buildProviders({ ...PROVIDER_CFG, wsUrl: NODE_WS });

        // Same signer set regardless of transport mode — one invocation =
        // one mode, and the CI matrix runs the script per mode. Callers
        // who need to coexist with another run on the same chain pass
        // --signer-disc=XX to produce a fresh user account.
        const userSeed = signerDiscriminator
            ? `//Papisigner${signerDiscriminator}`
            : '//Papisigner';
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

        // Data to store.
        const dataToStore = `Hello, Bulletin (${PROVIDER_CFG.mode}) - ${new Date().toString()}`;
        const dataBytes = new TextEncoder().encode(dataToStore);
        const expectedCid = await cidFromBytes(dataBytes);

        // Authorize the user account.
        await client
            .authorizeAccount(whoAddress, 100, BigInt(100 * 1024 * 1024)) // 100 MiB
            .withWaitFor(WaitFor.Finalized)
            .send();
        logSuccess(`Account ${whoAddress} authorized`);

        // Store data via the SDK pipeline: estimate, then submit. The root
        // CID (retrieval id) is the last unit of the upload.
        const src = blobFromBytes(dataBytes);
        const { cids } = await client.submit(await client.estimateUpload(src), src).send();
        const cid = cids[cids.length - 1];
        logSuccess(`Data stored successfully with CID: ${cid}`);

        assert.deepStrictEqual(
            cid.toString(),
            expectedCid.toString(),
            '❌ expectedCid does not match cid!',
        );

        // IPFS verification — optional, skipped if the gateway isn't reachable.
        try {
            const downloadedContent = await fetchCid(HTTP_IPFS_API, cid.toString());
            assert.deepStrictEqual(
                dataToStore,
                downloadedContent.toString(),
                '❌ dataToStore does not match downloadedContent!',
            );
            logSuccess('Verified content via IPFS!');
        } catch (err) {
            console.log(`⚠️  IPFS verification skipped (${HTTP_IPFS_API} unreachable): ${err.message}`);
        }

        logTestResult(true, `Authorize and Store Test (${PROVIDER_CFG.mode})`);
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
