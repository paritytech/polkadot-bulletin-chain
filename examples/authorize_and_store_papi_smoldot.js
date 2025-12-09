import assert from "assert";
import * as smoldot from 'smoldot';
import { ApiPromise, WsProvider } from "@polkadot/api";
import { createClient } from 'polkadot-api';
import { getSmProvider } from 'polkadot-api/sm-provider';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { authorizeAccount, fetchCid, store } from './api.js';
import {
    setupKeyringAndSigners,
    AUTH_TRANSACTIONS,
    AUTH_BYTES,
    ALICE_ADDRESS,
    cidFromBytes
} from './common.js';
import { bulletin } from './.papi/descriptors/dist/index.mjs';

// Constants
const BOB_NODE_WS = 'ws://localhost:12346';
const SYNC_WAIT_SEC = 15;
const SMOLDOT_LOG_LEVEL = 3; // 0=off, 1=error, 2=warn, 3=info, 4=debug, 5=trace
const HTTP_IPFS_API = 'http://127.0.0.1:8080'   // Local IPFS HTTP gateway

async function fetchChainSpec(nodeWs) {
    console.log('Fetching chainspec from node...');
    const provider = new WsProvider(nodeWs);
    const api = await ApiPromise.create({ provider });
    await api.isReady;

    const chainSpec = (await api.rpc.syncstate.genSyncSpec(true)).toString();
    const chainSpecObj = JSON.parse(chainSpec);
    chainSpecObj.protocolId = null; // Allow smoldot to sync with local chain
    
    await api.disconnect();
    return JSON.stringify(chainSpecObj);
}

function initSmoldot() {
    const sd = smoldot.start({
        maxLogLevel: SMOLDOT_LOG_LEVEL,
        logCallback: (level, target, message) => {
            const levelNames = ['ERROR', 'WARN', 'INFO', 'DEBUG', 'TRACE'];
            const levelName = levelNames[level - 1] || 'UNKNOWN';
            console.log(`[smoldot:${levelName}] ${target}: ${message}`);
        }
    });
    return sd;
}

async function createSmoldotClient() {
    const chainSpec = await fetchChainSpec(BOB_NODE_WS);
    const sd = initSmoldot();
    const chain = await sd.addChain({ chainSpec });
    const client = createClient(getSmProvider(chain));
    
    return { client, sd };
}

async function main() {
    await cryptoWaitReady();
    
    let sd, client;
    
    try {
        ({ client, sd } = await createSmoldotClient());
        console.log(`⏭️ Waiting ${SYNC_WAIT_SEC} seconds for smoldot to sync...`);
        await new Promise(resolve => setTimeout(resolve, SYNC_WAIT_SEC * 1000));
        
        const bulletinAPI = client.getTypedApi(bulletin);

        const { sudoSigner, whoSigner } = setupKeyringAndSigners();

        const dataToStore = "Hello, Bulletin with PAPI + Smoldot - " + new Date().toString();
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
        if (client) client.destroy();
        if (sd) sd.terminate();
        process.exit(0);
    }
}

await main();
