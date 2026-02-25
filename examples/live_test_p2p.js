import assert from "assert";
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { store, fetchCidViaP2P, TX_MODE_FINALIZED_BLOCK } from './api.js';
import { newSigner } from './common.js';
import { logHeader, logSuccess, logError, logTestResult, logConfig } from './logger.js';
import { cidFromBytes } from "./cid_dag_metadata.js";
import { bulletin } from './.papi/descriptors/dist/index.mjs';

// Command line arguments: <ws_url> <seed> <peer_multiaddr1> [peer_multiaddr2] ...
const args = process.argv.slice(2);
if (args.length < 3) {
    console.error('Usage: node live_test_p2p.js <ws_url> <seed> <peer_multiaddr1> [peer_multiaddr2] ...');
    process.exit(1);
}

const NODE_WS = args[0];
const SEED = args[1];
const PEER_MULTIADDRS = args.slice(2);

async function main() {
    await cryptoWaitReady();

    logHeader('LIVE TEST: Store & P2P Verify');
    logConfig({
        'RPC Endpoint': NODE_WS,
        'Account/Seed': SEED,
        'P2P Peers': PEER_MULTIADDRS.length,
    });

    let client, resultCode;
    try {
        client = createClient(getWsProvider(NODE_WS));
        const bulletinAPI = client.getTypedApi(bulletin);

        const { signer } = newSigner(SEED);

        // Store small data
        const dataToStore = "Bulletin live test - " + new Date().toISOString();
        const expectedCid = await cidFromBytes(dataToStore);
        console.log(`\nStoring: "${dataToStore}"`);

        const { cid } = await store(bulletinAPI, signer, dataToStore, null, null, TX_MODE_FINALIZED_BLOCK);
        logSuccess(`Stored with CID: ${cid}`);
        assert.deepStrictEqual(cid, expectedCid, 'CID mismatch');

        // Verify via P2P
        console.log(`\nVerifying CID via P2P...`);
        const downloaded = await fetchCidViaP2P(PEER_MULTIADDRS, cid);
        logSuccess(`Downloaded ${downloaded.length} bytes via P2P`);

        assert.deepStrictEqual(
            dataToStore,
            downloaded.toString(),
            'Downloaded content does not match stored data!'
        );
        logSuccess('Content verified via P2P!');

        logTestResult(true, 'Live P2P Test');
        resultCode = 0;
    } catch (error) {
        logError(`Error: ${error.message}`);
        console.error(error);
        logTestResult(false, 'Live P2P Test');
        resultCode = 1;
    } finally {
        if (client) client.destroy();
        process.exit(resultCode);
    }
}

await main();
