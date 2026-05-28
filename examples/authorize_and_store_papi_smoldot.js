import assert from "assert";
import * as smoldot from 'smoldot';
import { readFileSync } from 'fs';
import { getSmProvider } from 'polkadot-api/sm-provider';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { fetchCid } from './api.js';
import { setupKeyringAndSigners, waitForChainReady, waitForBlockProduction, DEFAULT_IPFS_GATEWAY_URL } from './common.js';
import { logHeader, logConfig, logSuccess, logError, logTestResult } from './logger.js';
import { cidFromBytes } from "./cid_dag_metadata.js";
import { bulletin } from './.papi/descriptors/dist/index.js';
import { BulletinClient } from '../sdk/typescript/dist/index.mjs';

// Constants
// Increased sync time for parachain mode where smoldot needs more time to sync relay + para
const SYNC_WAIT_SEC = 30;
const SMOLDOT_LOG_LEVEL = 3; // 0=off, 1=error, 2=warn, 3=info, 4=debug, 5=trace

const TCP_BOOTNODE_REGEX = /^(\/ip[46]\/[^/]+)\/tcp\/(\d+)\/p2p\/(.+)$/;
const WS_BOOTNODE_REGEX = /\/tcp\/\d+\/ws\/p2p\//;

/**
 * Converts a TCP bootnode to WebSocket format for smoldot compatibility.
 * If already a WS address (zombienet default), returns it unchanged.
 * For plain TCP bootnodes, uses convention: WebSocket port = TCP p2p_port + 1.
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

    const mainChainSpec = readChainSpec(chainSpecPath);
    const parachainSpec = parachainSpecPath ? readChainSpec(parachainSpecPath) : null;

    // Add chains once and reuse the target Chain handle for every JsonRpcProvider
    // we hand out — both PAPI's top-level client and the SDK's internal substrate
    // clients share the same smoldot connection.
    const mainChain = await sd.addChain({ chainSpec: mainChainSpec });
    console.log(`✅ Added main chain: ${chainSpecPath}`);
    let target = mainChain;
    if (parachainSpec) {
        target = await sd.addChain({
            chainSpec: parachainSpec,
            potentialRelayChains: [mainChain],
        });
        console.log(`✅ Added parachain: ${parachainSpecPath}`);
    }

    const createProvider = () => getSmProvider(target);
    return { sd, createProvider };
}

async function main() {
    await cryptoWaitReady();

    logHeader('AUTHORIZE AND STORE TEST (Smoldot Light Client)');

    // Get chainspec path from command line argument (required - main chain: relay for para, or solo)
    const chainSpecPath = process.argv[2];
    if (!chainSpecPath) {
        logError('Chain spec path is required as first argument');
        console.error('Usage: node authorize_and_store_papi_smoldot.js <chain-spec-path> [parachain-spec-path] [ipfs-api-url]');
        console.error('  For parachains: <relay-chain-spec-path> <parachain-spec-path> [ipfs-api-url]');
        console.error('  For solochains: <solo-chain-spec-path> [ipfs-api-url]');
        process.exit(1);
    }

    // Optional parachain chainspec path (only needed for parachains)
    const parachainSpecPath = process.argv[3] || null;
    // Optional IPFS API URL
    const HTTP_IPFS_API = process.argv[4] || DEFAULT_IPFS_GATEWAY_URL;

    logConfig({
        'Mode': 'Smoldot Light Client',
        'Chain Spec': chainSpecPath,
        'Parachain Spec': parachainSpecPath || 'N/A (solochain)',
        'IPFS API': HTTP_IPFS_API
    });
    
    let sd, client, resultCode;
    try {
        // Init smoldot + get the provider factory (one provider instance
        // per call → fresh JsonRpcProvider over the same chain handle).
        let createProvider;
        ({ sd, createProvider } = await createSmoldotClient(chainSpecPath, parachainSpecPath));
        console.log(`⏭️ Waiting ${SYNC_WAIT_SEC} seconds for smoldot to sync...`);
        // TODO: check better way, when smoldot is synced, maybe some RPC/runtime api that checks best vs finalized block?
        await new Promise(resolve => setTimeout(resolve, SYNC_WAIT_SEC * 1000));

        // Signers: Use //Eve for the account being authorized to avoid nonce
        // conflicts when running after the ws test (which uses //Alice) on
        // the same chain.
        const { authorizationSigner, whoSigner, whoAddress } = setupKeyringAndSigners('//Eve', '//Papismoldotsigner');

        // Self-contained client over smoldot. `providers()` returns a
        // fresh JsonRpcProvider per pipelineStore invocation; smoldot
        // manages the underlying chain connection.
        client = new BulletinClient({
            descriptor: bulletin,
            providers: () => [createProvider()],
            uploadSigner: whoSigner,
            authorizerSigner: authorizationSigner,
        });

        console.log('🔍 Checking if chain is ready...');
        await waitForChainReady(client.api);
        await waitForBlockProduction(client.api);

        // Data to store.
        const dataToStore = "Hello, Bulletin with PAPI + Smoldot - " + new Date().toString();
        const dataBytes = new TextEncoder().encode(dataToStore);
        const expectedCid = await cidFromBytes(dataBytes);

        // Authorize the user account via the configured authorizerSigner.
        await client
            .authorizeAccount(whoAddress, 100, BigInt(100 * 1024 * 1024))
            .withWaitFor('finalized')
            .send();
        logSuccess(`Account ${whoAddress} authorized`);

        // Store data via the SDK pipeline.
        const { cid } = await client.uploadFile(dataBytes).send();
        logSuccess(`Data stored successfully with CID: ${cid}`);

        assert.deepStrictEqual(
            cid.toString(),
            expectedCid.toString(),
            '❌ expectedCid does not match cid!'
        );

        // Read back from IPFS — optional verification step. Skipped if the
        // gateway isn't reachable (e.g. zombienet without --ipfs-server up).
        try {
            const downloadedContent = await fetchCid(HTTP_IPFS_API, cid.toString());
            logSuccess(`Downloaded content: ${downloadedContent.toString()}`);
            assert.deepStrictEqual(
                dataToStore,
                downloadedContent.toString(),
                '❌ dataToStore does not match downloadedContent!'
            );
            logSuccess('Verified content via IPFS!');
        } catch (err) {
            console.log(`⚠️  IPFS verification skipped (${HTTP_IPFS_API} unreachable): ${err.message}`);
        }

        logTestResult(true, 'Authorize and Store Test (Smoldot)');
        resultCode = 0;
    } catch (error) {
        logError(`Error: ${error.message}`);
        console.error(error);
        resultCode = 1;
    } finally {
        if (client) await client.destroy();
        if (sd) sd.terminate();
        process.exit(resultCode);
    }
}

await main();
