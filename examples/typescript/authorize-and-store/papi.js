import assert from "assert";
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { authorizeAccount, fetchCid, store, TX_MODE_FINALIZED_BLOCK } from '../api.js';
import { setupKeyringAndSigners } from '../common.js';
import { cidFromBytes } from "../cid_dag_metadata.js";
import { bulletin } from '../.papi/descriptors/dist/index.mjs';

// Command line arguments: [ws_url] [seed] [ipfs_api_url]
const args = process.argv.slice(2);
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Alice';
const HTTP_IPFS_API = args[2] || 'http://127.0.0.1:8283';

async function main() {
    await cryptoWaitReady();

    console.log('\n' + '═'.repeat(60));
    console.log('  AUTHORIZE AND STORE TEST (WebSocket)');
    console.log('═'.repeat(60));
    console.log('\n📋 Configuration:');
    console.log(`   RPC Endpoint: ${NODE_WS}`);
    console.log(`   Account/Seed: ${SEED}`);
    console.log(`   IPFS API: ${HTTP_IPFS_API}`);

    let client, resultCode;
    try {
        // Init WS PAPI client and typed api.
        client = createClient(getWsProvider(NODE_WS));
        const bulletinAPI = client.getTypedApi(bulletin);

        // Signers.
        const { sudoSigner, whoSigner, whoAddress } = setupKeyringAndSigners(SEED, '//Papisigner');

        // Data to store.
        const dataToStore = "Hello, Bulletin with PAPI - " + new Date().toString();
        let expectedCid = await cidFromBytes(dataToStore);

        // Authorize an account.
        await authorizeAccount(
            bulletinAPI,
            sudoSigner,
            whoAddress,
            100,
            BigInt(100 * 1024 * 1024), // 100 MiB
            TX_MODE_FINALIZED_BLOCK,
        );

        // Store data.
        const { cid } = await store(bulletinAPI, whoSigner, dataToStore);
        console.log(`Data stored successfully with CID: ${cid}`);

        // Read back from IPFS
        let downloadedContent = await fetchCid(HTTP_IPFS_API, cid);
        console.log(`Downloaded content: ${downloadedContent.toString()}`);
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
        console.log('Verified content!');

        console.log('\n✅ Authorize and Store Test PASSED');
        resultCode = 0;
    } catch (error) {
        console.error(`Error: ${error.message}`);
        console.error(error);
        resultCode = 1;
    } finally {
        if (client) client.destroy();
        process.exit(resultCode);
    }
}

await main();
