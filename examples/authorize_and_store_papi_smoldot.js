import assert from "assert";
import * as smoldot from 'smoldot';
import { readFileSync } from 'fs';
import { createClient } from 'polkadot-api';
import { getSmProvider } from 'polkadot-api/sm-provider';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { authorizeAccount, fetchCid, store } from './api.js';
import { setupKeyringAndSigners, waitForChainReady } from './common.js';
import { cidFromBytes } from "./cid_dag_metadata.js";
import { bulletin } from './.papi/descriptors/dist/index.mjs';

// Constants
const SYNC_WAIT_SEC = 15;
const SMOLDOT_LOG_LEVEL = 3; // 0=off, 1=error, 2=warn, 3=info, 4=debug, 5=trace
const HTTP_IPFS_API = 'http://127.0.0.1:8080'   // Local IPFS HTTP gateway

function readChainSpec(chainspecPath) {
    const chainSpecContent = readFileSync(chainspecPath, 'utf8');
    const chainSpecObj = JSON.parse(chainSpecContent);
    chainSpecObj.protocolId = null;
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

async function createSmoldotClient(chainSpecPath, parachainSpecPath = null) {
    const sd = initSmoldot();
    
    const chainSpec = readChainSpec(chainSpecPath);
    const mainChain = await sd.addChain({ chainSpec });
    console.log(`✅ Added main chain: ${chainSpecPath}`);
    
    if (parachainSpecPath) {
        const parachainSpec = readChainSpec(parachainSpecPath);
        const parachain = await sd.addChain({
            chainSpec: parachainSpec,
            potentialRelayChains: [mainChain]
        });
        console.log(`✅ Added parachain: ${parachainSpecPath}`);
        const client = createClient(getSmProvider(parachain));
        return { client, sd };
    }
    
    const client = createClient(getSmProvider(mainChain));
    return { client, sd };
}

async function main() {
    await cryptoWaitReady();
    
    // Get chainspec path from command line argument (required - main chain: relay for para, or solo)
    const chainSpecPath = process.argv[2];
    if (!chainSpecPath) {
        console.error('❌ Error: Chain spec path is required as first argument');
        console.error('Usage: node authorize_and_store_papi_smoldot.js <chain-spec-path> [parachain-spec-path]');
        console.error('  For parachains: <relay-chain-spec-path> <parachain-spec-path>');
        console.error('  For solochains: <solo-chain-spec-path>');
        process.exit(1);
    }
    
    // Optional parachain chainspec path (only needed for parachains)
    const parachainSpecPath = process.argv[3] || null;
    
    let sd, client, resultCode;
    try {
        // Init Smoldot PAPI client and typed api.
        ({ client, sd } = await createSmoldotClient(chainSpecPath, parachainSpecPath));
        console.log(`⏭️ Waiting ${SYNC_WAIT_SEC} seconds for smoldot to sync...`);
        // TODO: check better way, when smoldot is synced, maybe some RPC/runtime api that checks best vs finalized block?        
        await new Promise(resolve => setTimeout(resolve, SYNC_WAIT_SEC * 1000));
        
        console.log('🔍 Checking if chain is ready...');
        const bulletinAPI = client.getTypedApi(bulletin);
        await waitForChainReady(bulletinAPI);

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
