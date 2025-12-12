import assert from "assert";
import * as smoldot from 'smoldot';
import { ApiPromise, WsProvider } from "@polkadot/api";
import { createClient } from 'polkadot-api';
import { getSmProvider } from 'polkadot-api/sm-provider';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { authorizeAccount, fetchCid, store } from './api.js';
import { setupKeyringAndSigners, cidFromBytes } from './common.js';
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
    
    let sd, client, resultCode;
    try {
        // Init Smoldot PAPI client and typed api.
        ({ client, sd } = await createSmoldotClient());
        console.log(`⏭️ Waiting ${SYNC_WAIT_SEC} seconds for smoldot to sync...`);
        // TODO: check better way, when smoldot is synced, maybe some RPC/runtime api that checks best vs finalized block?        
        await new Promise(resolve => setTimeout(resolve, SYNC_WAIT_SEC * 1000));
        const bulletinAPI = client.getTypedApi(bulletin);

        // Signers.
        const { sudoSigner, whoSigner, whoAddress } = setupKeyringAndSigners('//Alice', '//Alice');

        // Data to store.
        const dataToStore = "Hello, Bulletin with PAPI + Smoldot - " + new Date().toString();
        let expectedCid = await cidFromBytes(dataToStore);

        // Authorize an account.
        await authorizeAccount(
            bulletinAPI,
            sudoSigner,
            whoAddress,
            1,
            BigInt(dataToStore.length)
        );

        // Store data.
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
        console.log(`✅ Verified content!`);

        console.log(`\n\n\n✅✅✅ Test passed! ✅✅✅`);
        resultCode = 0;
    } catch (error) {
        console.error("❌ Error:", error);
        resultCode = 1;
    } finally {
        if (client) client.destroy();
        if (sd) sd.terminate();
        process.exit(resultCode);
    }
}

await main();
