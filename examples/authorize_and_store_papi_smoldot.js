import * as smoldot from 'smoldot';
import { ApiPromise, WsProvider } from "@polkadot/api";
import { createClient } from 'polkadot-api';
import { Keyring } from "@polkadot/keyring";
import { getSmProvider } from 'polkadot-api/sm-provider';
import { getPolkadotSigner } from '@polkadot-api/signer';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { create } from 'ipfs-http-client';
import { cidFromBytes } from './common.js';
import { bulletin } from './.papi/descriptors/dist/index.mjs';
import { authorizeAccount, store } from './api.js';
import assert from "assert";

// Generate PAPI descriptors using local node:
// npx papi add -w ws://localhost:10000 bulletin
// npx papi
async function main() {
    await cryptoWaitReady();
    
    // Bob's address - to get the chainspec
    console.log('Fetching chainspec from Bob node...');
    const bobWs = new WsProvider('ws://localhost:12346');
    const bobApi = await ApiPromise.create({ provider: bobWs });
    await bobApi.isReady;

    // Create keyring and accounts
    const keyring = new Keyring({ type: 'sr25519' });
    const sudoAccount = keyring.addFromUri('//Alice');
    const whoAccount = keyring.addFromUri('//Alice');
    const sudoSigner = getPolkadotSigner(
        sudoAccount.publicKey,
        'Sr25519',
        (input) => sudoAccount.sign(input)
    );
    const whoSigner = getPolkadotSigner(
        whoAccount.publicKey,
        'Sr25519',
        (input) => whoAccount.sign(input)
    );

    // Data
    const who = whoAccount.publicKey;
    const transactions = 32;
    const bytes = 64 * 1024 * 1024; // 64 MB

    // Prepare data for storage
    const dataToStore = "Hello, Bulletin with PAPI + Smoldot - " + new Date().toString();
    const expectedCid = cidFromBytes(dataToStore);

    // Note: In real usage, this step is not required — the chain spec with bootNodes should be included as part of the dApp.
    //       For local testing, we use this to fetch the actual chain spec from the local node.
    // Get chain spec from Bob node and remove protocolId to allow smoldot to sync with local chain.
    const chainSpec = (await bobApi.rpc.syncstate.genSyncSpec(true)).toString();
    const chainSpecObj = JSON.parse(chainSpec);
    chainSpecObj.protocolId = null;
    const modifiedChainSpec = JSON.stringify(chainSpecObj);

    // Initialize Smoldot client
    const sd = smoldot.start({
        maxLogLevel: 3, // 0=off, 1=error, 2=warn, 3=info, 4=debug, 5=trace
        logCallback: (level, target, message) => {
            const levelNames = ['ERROR', 'WARN', 'INFO', 'DEBUG', 'TRACE'];
            const levelName = levelNames[level - 1] || 'UNKNOWN';
            console.log(`[smoldot:${levelName}] ${target}: ${message}`);
        }
    });
    const chain = await sd.addChain({ chainSpec: modifiedChainSpec });
    const client = createClient(getSmProvider(chain));
    const typedApi = client.getTypedApi(bulletin);

    // authorize
    await authorizeAccount(typedApi, sudoSigner, who, transactions, bytes);
    // store
    let cid = await store(typedApi, whoSigner, dataToStore);
    assert.strictEqual(
        cid.toString(),
        expectedCid.toString(),
        '❌ CID does not match expected root CID'
    );
}

await main();
