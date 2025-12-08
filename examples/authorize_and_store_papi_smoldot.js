import * as smoldot from 'smoldot';
import { ApiPromise, WsProvider } from "@polkadot/api";
import { createClient } from 'polkadot-api';
import { Keyring } from "@polkadot/keyring";
import { getSmProvider } from 'polkadot-api/sm-provider';
import { getPolkadotSigner } from '@polkadot-api/signer';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { cidFromBytes } from './common.js';
import { bulletin } from './.papi/descriptors/dist/index.mjs';
import { Binary } from '@polkadot-api/substrate-bindings';

// Generate PAPI descriptors using local node:
// npx papi add -w ws://localhost:10000 bulletin
// npx papi

// Constants
const BOB_NODE_WS = 'ws://localhost:12346';
const ALICE_ADDRESS = "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY";
const SYNC_WAIT_MS = 15000;
const SMOLDOT_LOG_LEVEL = 1; // 0=off, 1=error, 2=warn, 3=info, 4=debug, 5=trace

// Authorization parameters
const AUTH_TRANSACTIONS = 32;
const AUTH_BYTES = 64n * 1024n * 1024n; // 64 MB

/**
 * Fetches and modifies the chain spec from a node for local testing
 * Note: In production, the chain spec with bootNodes should be bundled with the dApp
 */
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

/**
 * Initializes Smoldot client with logging
 */
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

/**
 * Creates signers for accounts
 */
function createSigner(account) {
    return getPolkadotSigner(
        account.publicKey,
        'Sr25519',
        (input) => account.sign(input)
    );
}

/**
 * Waits for a transaction to complete using observables
 */
function waitForTransaction(tx, signer, eventPrefix = "tx") {
    return new Promise((resolve, reject) => {
        tx.signSubmitAndWatch(signer).subscribe({
            next: (ev) => {
                console.log(`✅ ${eventPrefix} event:`, ev.type);
                if (ev.type === "txBestBlocksState" && ev.found) {
                    console.log(`✅ ${eventPrefix} included in block:`, ev.block.hash);
                }
            },
            error: (err) => {
                console.error(`❌ ${eventPrefix} error:`, err);
                reject(err);
            },
            complete: () => {
                console.log(`✅ ${eventPrefix} complete!`);
                resolve();
            }
        });
    });
}

/**
 * Authorizes an account for transaction storage
 */
async function authorizeAccount(bulletinAPI, signer, who, transactions, bytes) {
    console.log('Authorizing account...');
    const authorizeTx = bulletinAPI.tx.TransactionStorage.authorize_account({
        who,
        transactions,
        bytes
    });
    const sudoTx = bulletinAPI.tx.Sudo.sudo({
        call: authorizeTx.decodedCall
    });
    
    await waitForTransaction(sudoTx, signer, "Authorize");
}

/**
 * Stores data to the bulletin chain
 */
async function storeData(bulletinAPI, signer, data) {
    console.log('Storing data...');
    const dataBytes = new Uint8Array(Buffer.from(data));
    const binaryData = Binary.fromBytes(dataBytes);
    
    const storeTx = bulletinAPI.tx.TransactionStorage.store({ data: binaryData });
    await waitForTransaction(storeTx, signer, "Store");
    
    const expectedCid = cidFromBytes(data);
    console.log("✅ Expected CID:", expectedCid);
    return expectedCid;
}

async function main() {
    await cryptoWaitReady();
    
    let sd, client;
    
    try {
        // Setup keyring and accounts
        const keyring = new Keyring({ type: 'sr25519' });
        const sudoAccount = keyring.addFromUri('//Alice');
        const whoAccount = keyring.addFromUri('//Alice');
        const sudoSigner = createSigner(sudoAccount);
        const whoSigner = createSigner(whoAccount);

        // Prepare data for storage
        const dataToStore = "Hello, Bulletin with PAPI + Smoldot - " + new Date().toString();

        // Fetch chain spec and initialize Smoldot
        const chainSpec = await fetchChainSpec(BOB_NODE_WS);
        sd = initSmoldot();
        
        const chain = await sd.addChain({ chainSpec });
        client = createClient(getSmProvider(chain));
        const bulletinAPI = client.getTypedApi(bulletin);

        // Wait for smoldot to sync
        console.log(`⏭️ Waiting ${SYNC_WAIT_MS / 1000} seconds for smoldot to sync...`);
        await new Promise(resolve => setTimeout(resolve, SYNC_WAIT_MS));

        // Execute authorization and storage sequentially
        await authorizeAccount(
            bulletinAPI,
            sudoSigner,
            ALICE_ADDRESS,
            AUTH_TRANSACTIONS,
            AUTH_BYTES
        );
        
        await storeData(bulletinAPI, whoSigner, dataToStore);
        
        console.log("✅ Data stored successfully.");
        
    } catch (error) {
        console.error("❌ Error:", error);
        process.exit(1);
    } finally {
        // Cleanup
        if (client) client.destroy();
        if (sd) sd.terminate();
        process.exit(0);
    }
}

await main();
