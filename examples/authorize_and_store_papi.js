// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

import assert from "assert";
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { authorizeAccount, fetchAndVerifyBlock, gatewaySource, nodeRpcSource, store, verifyStoredOnNode, TX_MODE_FINALIZED_BLOCK } from './api.js';
import { setupKeyringAndSigners, waitForBlockProduction, DEFAULT_IPFS_GATEWAY_URL } from './common.js';
import { logHeader, logConnection, logSuccess, logError, logTestResult } from './logger.js';
import { cidFromBytes } from "./cid_dag_metadata.js";
import { bulletin } from './.papi/descriptors/dist/index.js';

// Command line arguments: [ws_url] [seed] [ipfs_api_url]
const args = process.argv.slice(2);
// Comma-separated list of node WS URLs: the first one is used to submit, the
// stored transaction is then verified on every node in the list (collator-1
// rocksdb and collator-2 paritydb in the mixed-backend test network).
const NODE_WS_URLS = (args[0] || 'ws://localhost:10000').split(',');
const NODE_WS = NODE_WS_URLS[0];
const SEED = args[1] || '//Eve';
const HTTP_IPFS_API = args[2] || DEFAULT_IPFS_GATEWAY_URL;

async function main() {
    await cryptoWaitReady();

    logHeader('AUTHORIZE AND STORE TEST (WebSocket)');
    logConnection(NODE_WS, SEED, HTTP_IPFS_API);

    let client, resultCode;
    try {
        // Init WS PAPI client and typed api.
        client = createClient(getWsProvider(NODE_WS));
        const bulletinAPI = client.getTypedApi(bulletin);
        await waitForBlockProduction(bulletinAPI);

        // Signers.
        const { authorizationSigner, whoSigner, whoAddress } = setupKeyringAndSigners(SEED, '//Papisigner');

        // Data to store.
        const dataToStore = "Hello, Bulletin with PAPI - " + new Date().toString();
        let expectedCid = await cidFromBytes(dataToStore);

        // Authorize an account.
        await authorizeAccount(
            bulletinAPI,
            authorizationSigner,
            whoAddress,
            100,
            BigInt(100 * 1024 * 1024), // 100 MiB
            TX_MODE_FINALIZED_BLOCK,
        );

        // Store data.
        const { cid, blockNumber } = await store(bulletinAPI, whoSigner, dataToStore);
        logSuccess(`Data stored successfully with CID: ${cid}`);

        // Read back from IPFS and the node RPC, verifying both match.
        let downloadedContent = await fetchAndVerifyBlock(cid, gatewaySource(HTTP_IPFS_API), nodeRpcSource(client));
        logSuccess(`Downloaded content: ${downloadedContent.toString()}`);
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
        logSuccess('Verified content!');

        for (const wsUrl of NODE_WS_URLS) {
            const nodeClient = createClient(getWsProvider(wsUrl));
            try {
                await verifyStoredOnNode(
                    nodeClient,
                    nodeClient.getTypedApi(bulletin),
                    blockNumber,
                    cid,
                );
            } finally {
                nodeClient.destroy();
            }
            logSuccess(`Verified stored transaction on ${wsUrl}!`);
        }

        logTestResult(true, 'Authorize and Store Test');
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
