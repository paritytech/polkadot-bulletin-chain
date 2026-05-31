/**
 * Authorize and store data on Bulletin Chain using the TypeScript SDK.
 *
 * TypeScript equivalent of the Rust authorize-and-store example.
 * Demonstrates:
 * 1. Authorizing an account to store data (sudo via BulletinClient)
 * 2. Storing data on chain via BulletinClient.uploadFile().send()
 * 3. Verifying the returned CID
 *
 * Usage:
 *   node typescript/authorize_and_store.js [ws_url] [seed]
 */

import { cryptoWaitReady } from '@polkadot/util-crypto';
import { Keyring } from '@polkadot/keyring';
import { getPolkadotSigner } from '@polkadot-api/signer';
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws';
import { bulletin } from '../.papi/descriptors/dist/index.js';
import { BulletinClient, WaitFor } from '../../sdk/typescript/dist/index.mjs';

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

    let client, resultCode;
    try {
        // Create signers: an Authorizer (e.g. //Alice) and a regular user account.
        const authorizer = createSignerFromSeed(SEED);
        const user = createSignerFromSeed('//SDKSigner');
        console.log(`User account: ${user.address}`);

        // SDK owns the PAPI client lifecycle. `uploadSigner` is the
        // user; `authorizerSigner` is REQUIRED to call authorize/refresh.
        client = new BulletinClient({
            descriptor: bulletin,
            providers: () => [getWsProvider(NODE_WS)],
            uploadSigner: user.signer,
            authorizerSigner: authorizer.signer,
        });

        // Step 1: Authorize the account to store data
        console.log('\nStep 1: Authorizing account...');
        await client.authorizeAccount(
            user.address,
            100,
            BigInt(100 * 1024 * 1024), // 100 MiB
        ).withWaitFor(WaitFor.Finalized).send();
        console.log('Account authorized successfully!');

        // Step 2: Store data using the SDK
        const dataToStore = `Hello from Bulletin SDK at ${new Date().toISOString()}`;
        const dataBytes = new TextEncoder().encode(dataToStore);
        console.log(`\nStep 2: Storing data: "${dataToStore}"`);
        console.log(`  Size: ${dataBytes.length} bytes`);

        const { cid } = await client.uploadFile(dataBytes).send();

        console.log('Data stored successfully!');
        console.log(`  CID: ${cid.toString()}`);

        console.log('\nTest passed!');
        resultCode = 0;
    } catch (error) {
        console.error('Error:', error);
        resultCode = 1;
    } finally {
        if (client) await client.destroy();
        process.exit(resultCode);
    }
}

await main();
