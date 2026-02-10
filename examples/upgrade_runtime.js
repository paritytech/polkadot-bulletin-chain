#!/usr/bin/env node
/**
 * Runtime Upgrade Script for Bulletin Chain Networks
 *
 * Usage:
 *   node upgrade_runtime.js <seed> <wasm_path> [options]
 *   node upgrade_runtime.js --verify-only [--network <name>]
 *
 * Options:
 *   --network <name>   Network: westend, paseo, pop, polkadot (default: westend)
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

// --- Network configs ---

const NETWORKS = {
    westend: {
        rpc: 'wss://westend-bulletin-rpc.polkadot.io',
        method: 'sudo',
    },
    paseo: {
        rpc: 'wss://paseo-bulletin-rpc.polkadot.io',
        method: 'sudo',
    },
    pop: {
        rpc: 'wss://pop-bulletin-rpc.polkadot.io',
        method: 'authorize',
    },
    polkadot: {
        rpc: 'wss://polkadot-bulletin-rpc.polkadot.io',
        method: 'authorize',
    }
};

// --- Arg parsing ---

function parseArgs() {
    const args = process.argv.slice(2);
    const opts = {
        seed: null,
        wasmPath: null,
        network: 'westend',
        rpc: null,
        method: null,
        verifyOnly: false,
        dryRun: false,
    };

    let i = 0;
    while (i < args.length) {
        const arg = args[i];
        if (arg === '--network' && args[i + 1])    { opts.network = args[++i]; }
        else if (arg === '--rpc' && args[i + 1])    { opts.rpc = args[++i]; }
        else if (arg === '--method' && args[i + 1]) { opts.method = args[++i]; }
        else if (arg === '--verify-only')            { opts.verifyOnly = true; }
        else if (arg === '--dry-run')                { opts.dryRun = true; }
        else if (arg.startsWith('--'))               { console.error(`Unknown option: ${arg}`); process.exit(1); }
        else if (!opts.seed)                         { opts.seed = arg; }
        else if (!opts.wasmPath)                     { opts.wasmPath = arg; }
        i++;
    }

    return opts;
}

function resolveNetwork(opts) {
    const net = NETWORKS[opts.network];
    if (!net && !opts.rpc) {
        console.error(`Unknown network: ${opts.network}. Available: ${Object.keys(NETWORKS).join(', ')}`);
        process.exit(1);
    }
    return {
        rpc: opts.rpc || net.rpc,
        method: opts.method || net?.method || 'sudo',
    };
}

// --- Chain queries ---

async function getChainInfo(client) {
    const unsafeApi = client.getUnsafeApi();
    const runtimeVersion = await unsafeApi.constants.System.Version();

    let lastUpgrade = null;
    try { lastUpgrade = await unsafeApi.query.System.LastRuntimeUpgrade(); } catch (_) {}

    return { runtimeVersion, lastUpgrade };
}

function printChainInfo({ runtimeVersion, lastUpgrade }) {
    console.log('\nRuntime Version:');
    console.log(`  spec_name:           ${runtimeVersion.spec_name}`);
    console.log(`  spec_version:        ${runtimeVersion.spec_version}`);
    console.log(`  impl_version:        ${runtimeVersion.impl_version}`);
    console.log(`  authoring_version:   ${runtimeVersion.authoring_version}`);
    console.log(`  transaction_version: ${runtimeVersion.transaction_version}`);

    if (lastUpgrade) {
        console.log('\nLast Runtime Upgrade:');
        console.log(`  spec_version: ${lastUpgrade.spec_version}`);
        console.log(`  spec_name:    ${lastUpgrade.spec_name}`);
    }
}

// --- Upgrade methods ---

// Binary.fromBytes wraps raw bytes; polkadot-api handles SCALE encoding
// (including compact length prefix) internally when serializing the transaction.

async function upgradeWithSetCode(client, signer, wasmCode) {
    console.log('\nUsing sudo.sudo(system.setCode)...');
    const unsafeApi = client.getUnsafeApi();

    const setCodeCall = unsafeApi.tx.System.set_code({
        code: Binary.fromBytes(wasmCode),
    }).decodedCall;

    const tx = unsafeApi.tx.Sudo.sudo({ call: setCodeCall });
    const result = await tx.signAndSubmit(signer);
    console.log(`Success! Block: ${result.block.hash}`);
}

async function upgradeWithAuthorize(client, signer, wasmCode, codeHash) {
    console.log('\nUsing authorize_upgrade + apply_authorized_upgrade...');
    const unsafeApi = client.getUnsafeApi();
    const hashHex = `0x${Buffer.from(codeHash).toString('hex')}`;

    // Step 1: Authorize (needs sudo or governance origin)
    console.log(`\nStep 1: Authorizing upgrade (hash: ${hashHex})...`);

    const authorizeCall = unsafeApi.tx.System.authorize_upgrade({
        code_hash: Binary.fromBytes(codeHash),
    }).decodedCall;

    let authorizeTx;
    try {
        authorizeTx = unsafeApi.tx.Sudo.sudo({ call: authorizeCall });
        console.log('  via sudo.sudo(system.authorize_upgrade)');
    } catch (_) {
        authorizeTx = unsafeApi.tx.System.authorize_upgrade({
            code_hash: Binary.fromBytes(codeHash),
        });
        console.log('  via system.authorize_upgrade (requires governance origin)');
    }

    const result1 = await authorizeTx.signAndSubmit(signer);
    console.log(`  Authorized! Block: ${result1.block.hash}`);

    // Step 2: Apply as unsigned extrinsic (no signer/fees needed for the large WASM payload).
    // apply_authorized_upgrade supports ValidateUnsigned in the runtime.
    console.log('\nStep 2: Applying authorized upgrade (unsigned)...');
    const applyTx = unsafeApi.tx.System.apply_authorized_upgrade({
        code: Binary.fromBytes(wasmCode),
    });
    const bareExtrinsic = await applyTx.getBareTx();
    const result2 = await client.submit(bareExtrinsic);
    console.log(`  Applied! Block: ${result2.block.hash}`);
}

// --- Dry run ---

function dryRunSetCode(wasmCode) {
    console.log('\n=== DRY RUN ===');
    console.log('Would submit: sudo.sudo(system.setCode)');
    console.log(`  WASM size: ${wasmCode.length} bytes`);
}

function dryRunAuthorize(wasmCode, codeHash) {
    console.log('\n=== DRY RUN ===');
    console.log(`Would submit: authorize_upgrade (hash: 0x${Buffer.from(codeHash).toString('hex')})`);
    console.log(`Would submit: apply_authorized_upgrade (unsigned, no fees)`);
    console.log(`  WASM size: ${wasmCode.length} bytes`);
}

// --- Verify ---

async function verifyUpgrade(client, expectedVersion) {
    console.log('\nVerifying upgrade...');
    await new Promise(resolve => setTimeout(resolve, 6000));

    const { runtimeVersion } = await getChainInfo(client);
    console.log(`Current spec_version: ${runtimeVersion.spec_version}`);

    if (runtimeVersion.spec_version >= expectedVersion) {
        console.log(`Upgrade successful! Runtime is at version ${runtimeVersion.spec_version}`);
    } else {
        console.log(`Warning: Expected version ${expectedVersion}, got ${runtimeVersion.spec_version}`);
    }
}

// --- Main ---

async function main() {
    const opts = parseArgs();
    const { rpc, method } = resolveNetwork(opts);

    // -- Verify-only mode --
    if (opts.verifyOnly) {
        console.log(`Connecting to ${rpc}...`);
        const client = createClient(withPolkadotSdkCompat(getWsProvider(rpc)));
        try {
            printChainInfo(await getChainInfo(client));
        } finally {
            client.destroy();
        }
        process.exit(0);
    }

    // -- Validate inputs --
    if (!opts.seed || !opts.wasmPath) {
        console.error('Missing required arguments: <seed> <wasm_path>');
        console.error('Run with --help or see script header for usage.');
        process.exit(1);
    }
    if (!fs.existsSync(opts.wasmPath)) {
        console.error(`WASM file not found: ${opts.wasmPath}`);
        process.exit(1);
    }

    // -- Prepare --
    await cryptoWaitReady();
    const { signer, address } = newSigner(opts.seed);
    const wasmCode = fs.readFileSync(opts.wasmPath);
    const codeHash = blake2AsU8a(wasmCode, 256);

    console.log(`Signer:   ${address}`);
    console.log(`WASM:     ${opts.wasmPath} (${(wasmCode.length / 1024 / 1024).toFixed(2)} MB)`);
    console.log(`Hash:     0x${Buffer.from(codeHash).toString('hex')}`);
    console.log(`Network:  ${opts.network} (${method})`);

    // -- Dry run --
    if (opts.dryRun) {
        if (method === 'setCode') dryRunSetCode(wasmCode);
        else                      dryRunAuthorize(wasmCode, codeHash);
        process.exit(0);
    }

    // -- Connect & upgrade --
    console.log(`\nConnecting to ${rpc}...`);
    const client = createClient(withPolkadotSdkCompat(getWsProvider(rpc)));

    try {
        const { runtimeVersion: current } = await getChainInfo(client);
        console.log(`Current runtime: ${current.spec_name} v${current.spec_version}`);

        if (method === 'setCode') {
            await upgradeWithSetCode(client, signer, wasmCode);
        } else {
            await upgradeWithAuthorize(client, signer, wasmCode, codeHash);
        }

        await verifyUpgrade(client, current.spec_version + 1);
    } catch (error) {
        console.error('\nError:', error.message);
        if (error.cause) console.error('Cause:', error.cause);
        process.exit(1);
    } finally {
        client.destroy();
    }
}

main();
