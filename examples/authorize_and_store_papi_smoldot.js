import assert from "assert";
import * as smoldot from 'smoldot';
import { readFileSync } from 'fs';
import { createClient } from 'polkadot-api';
import { getSmProvider } from 'polkadot-api/sm-provider';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { authorizeAccount, fetchCid, store } from './api.js';
import { setupKeyringAndSigners } from './common.js';
import { cidFromBytes } from "./cid_dag_metadata.js";
import { bulletin } from './.papi/descriptors/dist/index.mjs';

// Constants
const SYNC_WAIT_SEC = 30; // Increased for parachain sync (relay chain + parachain)
const SMOLDOT_LOG_LEVEL = 3; // 0=off, 1=error, 2=warn, 3=info, 4=debug, 5=trace
const HTTP_IPFS_API = 'http://127.0.0.1:8080'   // Local IPFS HTTP gateway
const CHAIN_READY_MAX_RETRIES = 10; // Maximum retries for chain readiness check
const CHAIN_READY_RETRY_DELAY_MS = 2000; // Delay between retries (2 seconds)

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

async function createSmoldotClient(chainspecPath, relayChainSpecPath = null) {
    const sd = initSmoldot();
    let relayChain = null;
    
    if (relayChainSpecPath) {
        const relayChainSpec = readChainSpec(relayChainSpecPath);
        relayChain = await sd.addChain({ chainSpec: relayChainSpec });
        console.log(`✅ Added relay chain: ${relayChainSpecPath}`);
    }
    
    // Add the main chain (parachain or solochain)
    const chainSpec = readChainSpec(chainspecPath);
    const chainOptions = { chainSpec };
    
    if (relayChain) {
        chainOptions.potentialRelayChains = [relayChain];
    }
    
    const chain = await sd.addChain(chainOptions);
    const client = createClient(getSmProvider(chain));
    
    return { client, sd };
}

/**
 * Wait for the chain to be ready by checking if we can query chain state.
 * Retries with fixed delay until the chain is ready or max retries reached.
 * Checks both runtime version and block number to ensure chain is fully synced.
 */
async function waitForChainReady(bulletinAPI, maxRetries = CHAIN_READY_MAX_RETRIES, retryDelay = CHAIN_READY_RETRY_DELAY_MS) {
    for (let attempt = 1; attempt <= maxRetries; attempt++) {
        try {
            // Try multiple queries to ensure chain is fully synced
            const [runtimeVersion, blockNumber] = await Promise.all([
                bulletinAPI.query.System.LastRuntimeUpgrade(),
                bulletinAPI.query.System.Number()
            ]);
            
            const blockNum = blockNumber ?? 0;
            console.log(`✅ Chain is ready! Block #${blockNum}, Runtime: ${runtimeVersion ? 'available' : 'checking...'}`);
            return true;
        } catch (error) {
            if (attempt < maxRetries) {
                console.log(`⏳ Chain not ready yet (attempt ${attempt}/${maxRetries}), retrying in ${retryDelay/1000}s... Error: ${error.message}`);
                await new Promise(resolve => setTimeout(resolve, retryDelay));
            } else {
                console.log(`⚠️ Chain readiness check failed after ${maxRetries} attempts. Proceeding anyway... Error: ${error.message}`);
                return false;
            }
        }
    }
    return false;
}

async function main() {
    await cryptoWaitReady();
    
    // Get chainspec path from command line argument
    const chainspecPath = process.argv[2];
    if (!chainspecPath) {
        console.error('❌ Error: Chainspec path is required as first argument');
        console.error('Usage: node authorize_and_store_papi_smoldot.js <chainspec-path> [relay-chain-chainspec-path]');
        process.exit(1);
    }
    
    // Optional relay chain chainspec path (required for parachains)
    const relayChainSpecPath = process.argv[3] || null;
    
    let sd, client, resultCode;
    try {
        // Init Smoldot PAPI client and typed api.
        ({ client, sd } = await createSmoldotClient(chainspecPath, relayChainSpecPath));
        console.log(`⏭️ Waiting ${SYNC_WAIT_SEC} seconds for smoldot to sync...`);
        // TODO: check better way, when smoldot is synced, maybe some RPC/runtime api that checks best vs finalized block?        
        await new Promise(resolve => setTimeout(resolve, SYNC_WAIT_SEC * 1000));
        
        console.log('🔍 Checking if chain is ready...');
        const bulletinAPI = client.getTypedApi(bulletin);
        
        // Wait for chain to be ready with retries
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
