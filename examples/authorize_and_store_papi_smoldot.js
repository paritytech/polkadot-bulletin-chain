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
const SYNC_WAIT_SEC = 30;
const OVERALL_TIMEOUT_SEC = 180; // 3 minutes total timeout for the test
const SMOLDOT_LOG_LEVEL = 3; // 0=off, 1=error, 2=warn, 3=info, 4=debug, 5=trace
const HTTP_IPFS_API = 'http://127.0.0.1:8080'   // Local IPFS HTTP gateway

const TCP_BOOTNODE_REGEX = /^(\/ip[46]\/[^/]+)\/tcp\/(\d+)\/p2p\/(.+)$/;
const WS_BOOTNODE_REGEX = /\/tcp\/\d+\/ws\/p2p\//;

/**
 * Converts a TCP bootnode to WebSocket format for smoldot compatibility.
 * Uses convention: WebSocket port = TCP p2p_port + 1
 *
 * Example: /ip4/127.0.0.1/tcp/30333/p2p/PEER_ID -> /ip4/127.0.0.1/tcp/30334/ws/p2p/PEER_ID
 */
function convertBootNodeToWebSocket(addr) {
    // Already a WebSocket address
    if (WS_BOOTNODE_REGEX.test(addr)) {
        console.log(`  ✅ Already WebSocket: ${addr.substring(0, 50)}...`);
        return addr;
    }

    const match = addr.match(TCP_BOOTNODE_REGEX);
    if (match) {
        const [, hostPart, portStr, peerId] = match;
        const wsPort = parseInt(portStr, 10) + 1;
        console.log(`  📡 Converted: tcp/${portStr} -> tcp/${wsPort}/ws`);
        return `${hostPart}/tcp/${wsPort}/ws/p2p/${peerId}`;
    }

    return null;
}

function readChainSpec(chainspecPath) {
    const chainSpecObj = JSON.parse(readFileSync(chainspecPath, 'utf8'));
    chainSpecObj.protocolId = null;

    const bootNodes = chainSpecObj.bootNodes || [];
    if (bootNodes.length === 0) {
        console.log(`⚠️ No bootnodes found in chain spec: ${chainspecPath}`);
        return JSON.stringify(chainSpecObj);
    }

    console.log(`🔄 Converting ${bootNodes.length} bootnode(s) to WebSocket for smoldot...`);
    const wsBootNodes = bootNodes.map(convertBootNodeToWebSocket).filter(Boolean);

    if (wsBootNodes.length > 0) {
        chainSpecObj.bootNodes = wsBootNodes;
        console.log(`✅ Using ${wsBootNodes.length} WebSocket bootnode(s)`);
    } else {
        console.log(`⚠️ No bootnodes could be converted to WebSocket`);
    }

    return JSON.stringify(chainSpecObj);
}

function initSmoldot() {
    return smoldot.start({
        maxLogLevel: SMOLDOT_LOG_LEVEL,
        logCallback: (level, target, message) => {
            const levelName = ['ERROR', 'WARN', 'INFO', 'DEBUG', 'TRACE'][level - 1] || 'UNKNOWN';
            console.log(`[smoldot:${levelName}] ${target}: ${message}`);
        }
    });
}

async function createSmoldotClient(chainSpecPath, parachainSpecPath = null) {
    const sd = initSmoldot();

    const mainChain = await sd.addChain({ chainSpec: readChainSpec(chainSpecPath) });
    console.log(`✅ Added main chain: ${chainSpecPath}`);

    let targetChain = mainChain;
    if (parachainSpecPath) {
        targetChain = await sd.addChain({
            chainSpec: readChainSpec(parachainSpecPath),
            potentialRelayChains: [mainChain]
        });
        console.log(`✅ Added parachain: ${parachainSpecPath}`);
    }

    return { client: createClient(getSmProvider(targetChain)), sd };
}

async function runTestWithTimeout() {
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
        const isReady = await waitForChainReady(bulletinAPI);
        
        if (!isReady) {
            throw new Error('Chain failed to become ready after maximum retries');
        }

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

async function main() {
    const timeoutPromise = new Promise((_, reject) => {
        setTimeout(() => {
            reject(new Error(`❌ Test timeout: exceeded ${OVERALL_TIMEOUT_SEC} seconds`));
        }, OVERALL_TIMEOUT_SEC * 1000);
    });

    try {
        await Promise.race([runTestWithTimeout(), timeoutPromise]);
    } catch (error) {
        console.error(error.message || error);
        process.exit(1);
    }
}

await main();
