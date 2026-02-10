#!/usr/bin/env node
/**
 * Runtime Upgrade Script for Bulletin Chain Networks
 *
 * Supports multiple networks and upgrade methods:
 * - sudo: Uses sudo.sudo(system.setCode) or sudo.sudo(system.authorize_upgrade)
 * - authorize: Uses system.authorize_upgrade (for production chains without sudo)
 *
 * Usage:
 *   node upgrade_runtime.js <seed> <wasm_path> [options]
 *
 * Options:
 *   --network <name>   Network: testnet, westend, paseo, pop, polkadot (default: westend)
 *   --rpc <url>        Custom RPC endpoint (overrides network default)
 *   --method <type>    Upgrade method: setCode, authorize (default: based on network)
 *   --verify-only      Only verify current runtime version, don't upgrade
 *   --dry-run          Show what would be done without submitting
 */

import { cryptoWaitReady, blake2AsU8a } from '@polkadot/util-crypto';
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';
import { withPolkadotSdkCompat } from 'polkadot-api/polkadot-sdk-compat';
import { newSigner } from './common.js';
import fs from 'fs';
import { Binary } from 'polkadot-api';

// Network configurations
const NETWORKS = {
    westend: {
        rpc: 'wss://westend-bulletin-rpc.polkadot.io',
        method: 'sudo',
        runtime: 'bulletin-westend'
    },
    paseo: {
        rpc: 'wss://paseo-bulletin-rpc.polkadot.io', // Update when available
        method: 'sudo',
        runtime: 'bulletin-westend'
    },
    pop: {
        rpc: 'wss://pop-bulletin-rpc.polkadot.io', // Update when available
        method: 'authorize',
        runtime: 'bulletin-westend'
    },
    polkadot: {
        rpc: 'wss://polkadot-bulletin-rpc.polkadot.io', // Update when available
        method: 'authorize',
        runtime: 'bulletin-polkadot'
    }
};

function parseArgs() {
    const args = process.argv.slice(2);
    const options = {
        seed: null,
        wasmPath: null,
        network: 'westend',
        rpc: null,
        method: null,
        verifyOnly: false,
        dryRun: false
    };

    let i = 0;
    while (i < args.length) {
        const arg = args[i];
        if (arg === '--network' && args[i + 1]) {
            options.network = args[++i];
        } else if (arg === '--rpc' && args[i + 1]) {
            options.rpc = args[++i];
        } else if (arg === '--method' && args[i + 1]) {
            options.method = args[++i];
        } else if (arg === '--verify-only') {
            options.verifyOnly = true;
        } else if (arg === '--dry-run') {
            options.dryRun = true;
        } else if (arg.startsWith('--')) {
            console.error(`Unknown option: ${arg}`);
            process.exit(1);
        } else if (!options.seed) {
            options.seed = arg;
        } else if (!options.wasmPath) {
            options.wasmPath = arg;
        }
        i++;
    }

    return options;
}

function printUsage() {
    console.log(`
Runtime Upgrade Script for Bulletin Chain Networks

Usage:
  node upgrade_runtime.js <seed> <wasm_path> [options]
  node upgrade_runtime.js --verify-only [--network <name>] [--rpc <url>]

Arguments:
  seed        Sudo/signer account seed phrase (e.g., "//Alice" or mnemonic)
  wasm_path   Path to the runtime WASM file

Options:
  --network <name>   Network: testnet, westend, paseo, pop, polkadot (default: westend)
  --rpc <url>        Custom RPC endpoint (overrides network default)
  --method <type>    Upgrade method: setCode, authorize (default: based on network)
  --verify-only      Only verify current runtime version
  --dry-run          Show what would be done without submitting

Networks:
  testnet   - Local solochain (ws://localhost:9944)
  westend   - Westend Bulletin (wss://westend-bulletin-rpc.polkadot.io)
  paseo     - Paseo Bulletin
  pop       - PoP Bulletin (uses authorize method)
  polkadot  - Polkadot Bulletin (uses authorize method)

Examples:
  # Upgrade westend with sudo
  node upgrade_runtime.js "//Alice" ./runtime.wasm --network westend

  # Upgrade local testnet
  node upgrade_runtime.js "//Alice" ./runtime.wasm --network testnet

  # Verify current version only
  node upgrade_runtime.js --verify-only --network westend

  # Dry run to see what would happen
  node upgrade_runtime.js "//Alice" ./runtime.wasm --network westend --dry-run

  # Custom RPC endpoint
  node upgrade_runtime.js "//Alice" ./runtime.wasm --rpc wss://custom-rpc.example.com
`);
}

async function getChainInfo(api) {
    // Use unsafe API for dynamic access
    const unsafeApi = api.getUnsafeApi();

    // Query runtime version
    const runtimeVersion = await unsafeApi.constants.System.Version();

    // Query last runtime upgrade
    let lastUpgrade = null;
    try {
        lastUpgrade = await unsafeApi.query.System.LastRuntimeUpgrade();
    } catch (e) {
        // May not exist on all chains
    }

    return { runtimeVersion, lastUpgrade };
}

async function verifyUpgrade(api, expectedVersion) {
    console.log('\nVerifying upgrade...');

    // Wait a bit for the upgrade to take effect
    await new Promise(resolve => setTimeout(resolve, 6000));

    const { runtimeVersion, lastUpgrade } = await getChainInfo(api);

    console.log(`Current spec_version: ${runtimeVersion.spec_version}`);

    if (lastUpgrade) {
        console.log(`Last upgrade: spec ${lastUpgrade.spec_version}, name: ${lastUpgrade.spec_name}`);
    }

    if (expectedVersion && runtimeVersion.spec_version >= expectedVersion) {
        console.log(`\nUpgrade successful! Runtime is at version ${runtimeVersion.spec_version}`);
        return true;
    } else if (expectedVersion) {
        console.log(`\nWarning: Expected version ${expectedVersion}, but got ${runtimeVersion.spec_version}`);
        return false;
    }

    return true;
}

async function upgradeWithSetCode(api, signer, wasmCode, dryRun) {
    console.log('\nUsing sudo.sudo(system.setCode) method...');

    const unsafeApi = api.getUnsafeApi();

    // Build the setCode call
    // Note: Binary.fromBytes wraps raw bytes; polkadot-api handles SCALE encoding (including
    // compact length prefix) internally when serializing the transaction.
    const setCodeCall = unsafeApi.tx.System.set_code({ code: Binary.fromBytes(wasmCode) }).decodedCall;
    const sudoTx = unsafeApi.tx.Sudo.sudo({ call: setCodeCall });

    if (dryRun) {
        console.log('DRY RUN: Would submit sudo.sudo(system.setCode)');
        console.log(`  WASM size: ${wasmCode.length} bytes`);
        return true;
    }

    console.log('Submitting transaction...');
    const result = await sudoTx.signAndSubmit(signer);
    console.log(`Success! Block hash: ${result.block.hash}`);
    return true;
}

async function upgradeWithAuthorize(api, signer, wasmCode, codeHash, dryRun) {
    console.log('\nUsing authorize_upgrade + apply_authorized_upgrade method...');

    const unsafeApi = api.getUnsafeApi();

    // Step 1: Authorize the upgrade
    console.log('\nStep 1: Authorizing upgrade...');
    console.log(`  Code hash: 0x${Buffer.from(codeHash).toString('hex')}`);

    const authorizeCall = unsafeApi.tx.System.authorize_upgrade({
        code_hash: Binary.fromBytes(codeHash)
    }).decodedCall;

    // Check if we have sudo - if so, wrap in sudo call
    let authorizeTx;
    try {
        // Try with sudo first
        authorizeTx = unsafeApi.tx.Sudo.sudo({ call: authorizeCall });
        console.log('  Using sudo.sudo(system.authorize_upgrade)');
    } catch (e) {
        // No sudo pallet, use direct call (requires governance)
        authorizeTx = unsafeApi.tx.System.authorize_upgrade({
            code_hash: Binary.fromBytes(codeHash)
        });
        console.log('  Using direct system.authorize_upgrade (requires governance origin)');
    }

    if (dryRun) {
        console.log('DRY RUN: Would submit authorize_upgrade');
        console.log('DRY RUN: Would then submit apply_authorized_upgrade (unsigned, no fees)');
        return true;
    }

    const result1 = await authorizeTx.signAndSubmit(signer);
    console.log(`  Authorized! Block: ${result1.block.hash}`);

    // Step 2: Apply the authorized upgrade as an unsigned extrinsic.
    // apply_authorized_upgrade supports ValidateUnsigned in the runtime, so no signer/fees needed.
    // This avoids requiring the submitter to have funds for the large WASM payload.
    console.log('\nStep 2: Applying authorized upgrade (unsigned)...');
    const applyTx = unsafeApi.tx.System.apply_authorized_upgrade({
        code: Binary.fromBytes(wasmCode)
    });

    const bareExtrinsic = await applyTx.getBareTx();
    const result2 = await api.submit(bareExtrinsic);
    console.log(`  Applied! Block: ${result2.block.hash}`);

    return true;
}

async function main() {
    const options = parseArgs();

    // Handle verify-only mode
    if (options.verifyOnly) {
        const networkConfig = NETWORKS[options.network];
        if (!networkConfig && !options.rpc) {
            console.error(`Unknown network: ${options.network}`);
            process.exit(1);
        }

        const rpcUrl = options.rpc || networkConfig.rpc;
        console.log(`Connecting to ${rpcUrl}...`);

        const client = createClient(withPolkadotSdkCompat(getWsProvider(rpcUrl)));

        try {
            const { runtimeVersion, lastUpgrade } = await getChainInfo(client);

            console.log('\nRuntime Version:');
            console.log(`  spec_name: ${runtimeVersion.spec_name}`);
            console.log(`  spec_version: ${runtimeVersion.spec_version}`);
            console.log(`  impl_version: ${runtimeVersion.impl_version}`);
            console.log(`  authoring_version: ${runtimeVersion.authoring_version}`);
            console.log(`  transaction_version: ${runtimeVersion.transaction_version}`);

            if (lastUpgrade) {
                console.log('\nLast Runtime Upgrade:');
                console.log(`  spec_version: ${lastUpgrade.spec_version}`);
                console.log(`  spec_name: ${lastUpgrade.spec_name}`);
            }
        } finally {
            client.destroy();
        }

        process.exit(0);
    }

    // Validate required arguments for upgrade
    if (!options.seed || !options.wasmPath) {
        printUsage();
        process.exit(1);
    }

    // Validate network
    const networkConfig = NETWORKS[options.network];
    if (!networkConfig && !options.rpc) {
        console.error(`Unknown network: ${options.network}. Use --rpc to specify custom endpoint.`);
        process.exit(1);
    }

    const rpcUrl = options.rpc || networkConfig.rpc;
    const method = options.method || networkConfig.method;

    // Validate WASM file
    if (!fs.existsSync(options.wasmPath)) {
        console.error(`WASM file not found: ${options.wasmPath}`);
        process.exit(1);
    }

    // Initialize crypto
    await cryptoWaitReady();

    // Create signer
    const { signer, address } = newSigner(options.seed);
    console.log(`Signer address: ${address}`);

    // Read WASM file
    const wasmCode = fs.readFileSync(options.wasmPath);
    console.log(`WASM file: ${options.wasmPath}`);
    console.log(`WASM size: ${wasmCode.length} bytes (${(wasmCode.length / 1024 / 1024).toFixed(2)} MB)`);

    // Calculate code hash
    const codeHash = blake2AsU8a(wasmCode, 256);
    console.log(`Blake2-256 hash: 0x${Buffer.from(codeHash).toString('hex')}`);

    // Connect to chain
    console.log(`\nConnecting to ${rpcUrl}...`);
    const client = createClient(withPolkadotSdkCompat(getWsProvider(rpcUrl)));

    try {
        // Get current chain info
        const { runtimeVersion: currentVersion } = await getChainInfo(client);
        console.log(`Current runtime: ${currentVersion.spec_name} v${currentVersion.spec_version}`);

        if (options.dryRun) {
            console.log('\n=== DRY RUN MODE ===');
        }

        // Execute upgrade
        let success = false;
        if (method === 'setCode') {
            success = await upgradeWithSetCode(client, signer, wasmCode, options.dryRun);
        } else {
            success = await upgradeWithAuthorize(client, signer, wasmCode, codeHash, options.dryRun);
        }

        // Verify upgrade (skip in dry run)
        if (success && !options.dryRun) {
            await verifyUpgrade(client, currentVersion.spec_version + 1);
        }

    } catch (error) {
        console.error('\nError:', error.message);
        if (error.cause) {
            console.error('Cause:', error.cause);
        }
        process.exit(1);
    } finally {
        client.destroy();
    }

    process.exit(0);
}

main();
