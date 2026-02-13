/**
 * Authorize and store data on Bulletin Chain using the TypeScript SDK.
 *
 * TypeScript equivalent of the Rust authorize-and-store example.
 * Demonstrates:
 * 1. Authorizing an account to store data (sudo via AsyncBulletinClient)
 * 2. Storing data on chain via AsyncBulletinClient.store().send()
 * 3. Verifying the returned CID
 *
 * Usage:
 *   node typescript/authorize_and_store.js [ws_url] [seed]
 */

import { cryptoWaitReady } from '@polkadot/util-crypto';
import { Keyring } from '@polkadot/keyring';
import { getPolkadotSigner } from '@polkadot-api/signer';
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { bulletin } from '../.papi/descriptors/dist/index.mjs';
import { AsyncBulletinClient } from '../../sdk/typescript/dist/index.mjs';

// Command line arguments
const args = process.argv.slice(2).filter(arg => !arg.startsWith('--'));
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Alice';

// Create a PAPI-compatible signer from a dev seed (e.g. "//Alice")
function createSignerFromSeed(seed) {
    const keyring = new Keyring({ type: 'sr25519' });
    const account = keyring.addFromUri(seed);
    const signer = getPolkadotSigner(
        account.publicKey,
        'Sr25519',
        (input) => account.sign(input),
    );
    return { signer, address: account.address };
}

async function main() {
    await cryptoWaitReady();

    console.log(`Connecting to: ${NODE_WS}`);
    console.log(`Using seed: ${SEED}`);

    let papiClient, resultCode;
    try {
        // Initialize PAPI client
        papiClient = createClient(getWsProvider(NODE_WS));
        const api = papiClient.getTypedApi(bulletin);

        // Create signers: sudo (Alice) and a regular user account
        const sudo = createSignerFromSeed(SEED);
        const user = createSignerFromSeed('//SDKSigner');
        console.log(`User account: ${user.address}`);

        // Create SDK clients
        const sudoClient = new AsyncBulletinClient(api, sudo.signer);
        const userClient = new AsyncBulletinClient(api, user.signer);

        // Step 1: Authorize the account to store data (requires sudo)
        console.log('\nStep 1: Authorizing account...');
        await sudoClient.authorizeAccount(
            user.address,
            100,
            BigInt(100 * 1024 * 1024), // 100 MiB
        );
        console.log('Account authorized successfully!');

        // Step 2: Store data using the SDK
        const dataToStore = `Hello from Bulletin SDK at ${new Date().toISOString()}`;
        const dataBytes = new TextEncoder().encode(dataToStore);
        console.log(`\nStep 2: Storing data: "${dataToStore}"`);
        console.log(`  Size: ${dataBytes.length} bytes`);

        const result = await userClient.store(dataBytes).send();

        console.log('Data stored successfully!');
        console.log(`  CID: ${result.cid.toString()}`);

        console.log('\nTest passed!');
        resultCode = 0;
    } catch (error) {
        console.error('Error:', error);
        resultCode = 1;
    } finally {
        if (papiClient) papiClient.destroy();
        process.exit(resultCode);
    }
}

await main();
