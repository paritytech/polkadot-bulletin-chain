import assert from "assert";
import * as smoldot from 'smoldot';
import { readFileSync } from 'fs';
import { createClient } from 'polkadot-api';
import { getSmProvider } from 'polkadot-api/sm-provider';
import { cryptoWaitReady, blake2AsU8a } from '@polkadot/util-crypto';
import { Binary } from '@polkadot-api/substrate-bindings';
import { fetchCid, TX_MODE_FINALIZED_BLOCK } from './api.js';
import { setupKeyringAndSigners, waitForChainReady } from './common.js';
import { cidFromBytes } from "./cid_dag_metadata.js";
import { bulletin } from './.papi/descriptors/dist/index.mjs';

// Constants
const SYNC_WAIT_SEC = 30;
const SMOLDOT_LOG_LEVEL = 3; // 0=off, 1=error, 2=warn, 3=info, 4=debug, 5=trace
const HTTP_IPFS_API = 'http://127.0.0.1:8080';   // Local IPFS HTTP gateway

const TCP_BOOTNODE_REGEX = /^(\/ip[46]\/[^/]+)\/tcp\/(\d+)\/p2p\/(.+)$/;
const WS_BOOTNODE_REGEX = /\/tcp\/\d+\/ws\/p2p\//;

function convertBootNodeToWebSocket(addr) {
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

const TX_MODE_CONFIG = {
    [TX_MODE_FINALIZED_BLOCK]: {
        match: (ev) => ev.type === "finalized",
        log: (txName, ev) => `📦 ${txName} included in finalized block: ${ev.block.hash}`,
    },
};

const DEFAULT_TX_TIMEOUT_MS = 120_000;

async function waitForTransaction(tx, signer, txName, txMode = TX_MODE_FINALIZED_BLOCK, timeoutMs = DEFAULT_TX_TIMEOUT_MS) {
    const config = TX_MODE_CONFIG[txMode];
    if (!config) {
        throw new Error(`Unhandled txMode: ${txMode}`);
    }

    return new Promise((resolve, reject) => {
        let sub;
        let resolved = false;

        const cleanup = () => {
            resolved = true;
            clearTimeout(timeoutId);
            if (sub) sub.unsubscribe();
        };

        const timeoutId = setTimeout(() => {
            if (!resolved) {
                cleanup();
                reject(new Error(`${txName} transaction timed out after ${timeoutMs}ms waiting for ${txMode}`));
            }
        }, timeoutMs);

        sub = tx.signSubmitAndWatch(signer).subscribe({
            next: (ev) => {
                console.log(`✅ ${txName} event:`, ev.type);
                if (!resolved && config.match(ev)) {
                    console.log(config.log(txName, ev));
                    cleanup();
                    resolve(ev);
                }
            },
            error: (err) => {
                console.error(`❌ ${txName} error:`, err);
                if (!resolved) {
                    cleanup();
                    reject(err);
                }
            },
        });
    });
}

async function waitForRawTransaction(client, tx, txName, txMode = TX_MODE_FINALIZED_BLOCK, timeoutMs = DEFAULT_TX_TIMEOUT_MS) {
    const config = TX_MODE_CONFIG[txMode];
    if (!config) {
        throw new Error(`Unhandled txMode: ${txMode}`);
    }

    return new Promise((resolve, reject) => {
        let sub;
        let resolved = false;

        const cleanup = () => {
            resolved = true;
            clearTimeout(timeoutId);
            if (sub) sub.unsubscribe();
        };

        const timeoutId = setTimeout(() => {
            if (!resolved) {
                cleanup();
                reject(new Error(`${txName} transaction timed out after ${timeoutMs}ms waiting for ${txMode}`));
            }
        }, timeoutMs);

        sub = client.submitAndWatch(tx).subscribe({
            next: (ev) => {
                console.log(`✅ ${txName} event:`, ev.type);
                if (!resolved && config.match(ev)) {
                    console.log(config.log(txName, ev));
                    cleanup();
                    resolve(ev);
                }
            },
            error: (err) => {
                console.error(`❌ ${txName} error:`, err);
                if (!resolved) {
                    cleanup();
                    reject(err);
                }
            },
        });
    });
}

async function authorizePreimage(typedApi, sudoSigner, contentHash, maxSize, txMode) {
    console.log(
        `⬆️ Authorizing preimage with content hash: ${contentHash} and max size: ${maxSize}...`
    );
    const authorizeCall = typedApi.tx.TransactionStorage.authorize_preimage({
        content_hash: contentHash,
        max_size: maxSize,
    }).decodedCall;

    const batchTx = typedApi.tx.Utility.batch_all({
        calls: [authorizeCall],
    });
    const sudoTx = typedApi.tx.Sudo.sudo({
        call: batchTx.decodedCall,
    });

    await waitForTransaction(sudoTx, sudoSigner, "BatchAuthorize Preimages", txMode);
}

async function storeUnsigned(typedApi, client, data, txMode) {
    console.log('⬆️ Submitting unsigned Store');
    const cid = await cidFromBytes(data);

    const binaryData = new Binary(data);
    const tx = typedApi.tx.TransactionStorage.store({ data: binaryData });
    const bareTx = await tx.getBareTx();
    await waitForRawTransaction(client, bareTx, "Store", txMode);
    return cid;
}

async function main() {
    await cryptoWaitReady();

    const chainSpecPath = process.argv[2];
    if (!chainSpecPath) {
        console.error('❌ Error: Chain spec path is required as first argument');
        console.error('Usage: node authorize_and_store_preimage_papi_smoldot.js <chain-spec-path> [parachain-spec-path]');
        console.error('  For parachains: <relay-chain-spec-path> <parachain-spec-path>');
        console.error('  For solochains: <solo-chain-spec-path>');
        process.exit(1);
    }

    const parachainSpecPath = process.argv[3] || null;

    let sd, client, resultCode;
    try {
        ({ client, sd } = await createSmoldotClient(chainSpecPath, parachainSpecPath));
        console.log(`⏭️ Waiting ${SYNC_WAIT_SEC} seconds for smoldot to sync...`);
        await new Promise(resolve => setTimeout(resolve, SYNC_WAIT_SEC * 1000));

        console.log('🔍 Checking if chain is ready...');
        const bulletinAPI = client.getTypedApi(bulletin);
        await waitForChainReady(bulletinAPI);

        const { sudoSigner } = setupKeyringAndSigners('//Alice', '//PapiPreimageSmolSigner');

        const dataToStore = "Hello, Bulletin with preimage auth + Smoldot - " + new Date().toString();
        const dataBytes = new Uint8Array(Buffer.from(dataToStore));
        const expectedCid = await cidFromBytes(dataBytes);
        const contentHash = new Binary(blake2AsU8a(dataBytes));

        await authorizePreimage(
            bulletinAPI,
            sudoSigner,
            contentHash,
            BigInt(dataBytes.length),
            TX_MODE_FINALIZED_BLOCK,
        );

        const cid = await storeUnsigned(bulletinAPI, client, dataBytes, TX_MODE_FINALIZED_BLOCK);
        console.log("✅ Data stored successfully with CID:", cid);

        const downloadedContent = await fetchCid(HTTP_IPFS_API, cid);
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


