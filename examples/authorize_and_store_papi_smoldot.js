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
const SMOLDOT_LOG_LEVEL = 4; // 0=off, 1=error, 2=warn, 3=info, 4=debug, 5=trace
const HTTP_IPFS_API = 'http://127.0.0.1:8080'   // Local IPFS HTTP gateway

/**
 * Converts TCP bootnodes to WebSocket bootnodes for smoldot compatibility.
 * Uses convention: WebSocket port = TCP p2p_port + 1
 *
 * Example: /ip4/127.0.0.1/tcp/30333/p2p/PEER_ID
 *       -> /ip4/127.0.0.1/tcp/30334/ws/p2p/PEER_ID
 */
function convertBootNodesToWebSocket(bootNodes) {
    if (!bootNodes || bootNodes.length === 0) {
        return [];
    }

    const wsBootNodes = [];
    for (const addr of bootNodes) {
        // Parse multiaddr: /ip4/HOST/tcp/PORT/p2p/PEER_ID or /ip4/HOST/tcp/PORT/ws/p2p/PEER_ID
        const tcpMatch = addr.match(/^(\/ip[46]\/[^/]+)\/tcp\/(\d+)\/p2p\/(.+)$/);
        if (tcpMatch) {
            const [, hostPart, portStr, peerId] = tcpMatch;
            const tcpPort = parseInt(portStr, 10);
            const wsPort = tcpPort + 1; // Convention: WS port = TCP port + 1
            const wsAddr = `${hostPart}/tcp/${wsPort}/ws/p2p/${peerId}`;
            wsBootNodes.push(wsAddr);
            console.log(`  📡 Converted: tcp/${tcpPort} -> tcp/${wsPort}/ws`);
        }
        // Check if already a WebSocket address
        const wsMatch = addr.match(/\/tcp\/\d+\/ws\/p2p\//);
        if (wsMatch) {
            wsBootNodes.push(addr);
            console.log(`  ✅ Already WebSocket: ${addr.substring(0, 50)}...`);
        }
    }
    return wsBootNodes;
}

function readChainSpec(chainspecPath) {
    const chainSpecContent = readFileSync(chainspecPath, 'utf8');
    const chainSpecObj = JSON.parse(chainSpecContent);
    chainSpecObj.protocolId = null;

    // Convert TCP bootnodes to WebSocket for smoldot
    if (chainSpecObj.bootNodes && chainSpecObj.bootNodes.length > 0) {
        console.log(`🔄 Converting ${chainSpecObj.bootNodes.length} bootnode(s) to WebSocket for smoldot...`);
        const wsBootNodes = convertBootNodesToWebSocket(chainSpecObj.bootNodes);
        if (wsBootNodes.length > 0) {
            chainSpecObj.bootNodes = wsBootNodes;
            console.log(`✅ Using ${wsBootNodes.length} WebSocket bootnode(s)`);
        } else {
            console.log(`⚠️ No bootnodes could be converted to WebSocket`);
        }
    } else {
        console.log(`⚠️ No bootnodes found in chain spec: ${chainspecPath}`);
    }

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
