// open_hrmp_channel_westend.js
//
// Encodes and optionally submits an XCM message to open HRMP channel with a system parachain.
//
// Usage:
//   node open_hrmp_channel_westend.js [parachain_ws] [relay_ws] [target_para_id] [--submit] [--seed <seed>]
//
// Examples:
//   # Encode only (dry run)
//   node open_hrmp_channel_westend.js ws://localhost:10000 ws://localhost:9942 1000
//
//   # Submit with default sudo (//Alice)
//   node open_hrmp_channel_westend.js ws://localhost:10000 ws://localhost:9942 1000 --submit
//
//   # Submit with custom seed
//   node open_hrmp_channel_westend.js wss://westend-bulletin-rpc.polkadot.io wss://westend-rpc.polkadot.io 1000 --submit --seed "//Alice"

import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { Binary } from '@polkadot-api/substrate-bindings';
import { Keyring } from '@polkadot/keyring';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { getPolkadotSigner } from '@polkadot-api/signer';
import {
    bulletin,
    XcmVersionedXcm,
    XcmVersionedLocation,
    XcmV5Instruction,
    XcmV5Junctions,
    XcmV5Junction,
    XcmV5AssetFilter,
    XcmV5WildAsset,
    westend,
} from './.papi/descriptors/dist/index.mjs';

// Parse command line arguments
const args = process.argv.slice(2);
const PARACHAIN_WS = args[0] || 'ws://localhost:10000';
const RELAY_WS = args[1] || 'wss://westend-rpc.polkadot.io';
const TARGET_PARA_ID = parseInt(args[2] || '1000'); // Default: Asset Hub

// Check for --submit flag
const SUBMIT = args.includes('--submit');

// Get seed from --seed argument or default to //Alice
const seedIndex = args.indexOf('--seed');
const SEED = seedIndex !== -1 && args[seedIndex + 1] ? args[seedIndex + 1] : '//Alice';

// Execution fee in relay chain native token (1 WND = 10^12 planck)
const EXECUTION_FEE = 1_000_000_000_000n;

function toHex(bytes) {
    // Handle PAPI Binary type or regular Uint8Array/Buffer
    const data = bytes.asBytes ? bytes.asBytes() : bytes;
    return '0x' + Buffer.from(data).toString('hex');
}

function createSigner(seed) {
    const keyring = new Keyring({ type: 'sr25519' });
    const account = keyring.addFromUri(seed);
    return {
        signer: getPolkadotSigner(
            account.publicKey,
            'Sr25519',
            (input) => account.sign(input)
        ),
        address: account.address
    };
}

async function main() {
    await cryptoWaitReady();

    console.log('═══════════════════════════════════════════════════════════');
    console.log(' HRMP CHANNEL OPEN - CALL ENCODER');
    console.log('═══════════════════════════════════════════════════════════');
    console.log(`Target System Parachain: ${TARGET_PARA_ID}`);
    console.log(`Parachain Node: ${PARACHAIN_WS}`);
    console.log(`Relay Node: ${RELAY_WS}`);
    console.log(`Mode: ${SUBMIT ? 'SUBMIT' : 'ENCODE ONLY (dry run)'}`);
    console.log('');

    let parachainClient, relayClient;
    try {
        // Connect to both chains
        console.log('Connecting to chains...');
        parachainClient = createClient(getWsProvider(PARACHAIN_WS));
        relayClient = createClient(getWsProvider(RELAY_WS));

        const parachainApi = parachainClient.getTypedApi(bulletin);
        const relayApi = relayClient.getTypedApi(westend);

        // Wait for connections
        await new Promise(resolve => setTimeout(resolve, 3000));

        // Get our parachain ID
        const paraId = await parachainApi.query.ParachainInfo.ParachainId.getValue();
        console.log(`Source Parachain ID: ${paraId}`);
        console.log('');

        // ─────────────────────────────────────────────────────────────
        // Step 1: Encode the relay chain call using relay chain's PAPI
        // hrmp.establish_channel_with_system(target_system_chain)
        // ─────────────────────────────────────────────────────────────

        const relayCallTx = relayApi.tx.Hrmp.establish_channel_with_system({
            target_system_chain: TARGET_PARA_ID
        });

        const relayCallEncoded = await relayCallTx.getEncodedData();

        console.log('─── Relay Chain Call (to be executed via Transact) ───');
        console.log(`Call: Hrmp.establish_channel_with_system(${TARGET_PARA_ID})`);
        console.log(`Encoded: ${toHex(relayCallEncoded)}`);
        console.log('');

        // ─────────────────────────────────────────────────────────────
        // Step 2: Build the XCM message using PAPI enum constructors
        // ─────────────────────────────────────────────────────────────

        // Location for native relay token (Here = relay chain's native asset)
        const relayTokenLocation = {
            parents: 0,
            interior: XcmV5Junctions.Here()
        };

        // Asset: native relay token
        const relayTokenAsset = {
            id: relayTokenLocation,
            fun: { type: "Fungible", value: EXECUTION_FEE }
        };

        // Get raw bytes from the encoded call
        const relayCallBytes = relayCallEncoded.asBytes
            ? relayCallEncoded.asBytes()
            : relayCallEncoded;

        // XCM V5 instructions
        const xcmInstructions = [
            // 1. WithdrawAsset - from parachain sovereign account on relay
            XcmV5Instruction.WithdrawAsset([relayTokenAsset]),

            // 2. BuyExecution
            XcmV5Instruction.BuyExecution({
                fees: relayTokenAsset,
                weight_limit: { type: "Unlimited" }
            }),

            // 3. Transact - execute relay call
            XcmV5Instruction.Transact({
                origin_kind: { type: "Native" },
                call: Binary.fromBytes(new Uint8Array(relayCallBytes)),
            }),

            // 4. RefundSurplus
            XcmV5Instruction.RefundSurplus(),

            // 5. DepositAsset - return leftovers to parachain sovereign
            XcmV5Instruction.DepositAsset({
                assets: XcmV5AssetFilter.Wild(XcmV5WildAsset.All()),
                beneficiary: {
                    parents: 0,
                    interior: XcmV5Junctions.X1(XcmV5Junction.Parachain(paraId))
                }
            })
        ];

        // Wrap in VersionedXcm::V5
        const xcmMessage = XcmVersionedXcm.V5(xcmInstructions);

        // Destination: relay chain (parent)
        const destination = XcmVersionedLocation.V5({
            parents: 1,
            interior: XcmV5Junctions.Here()
        });

        // ─────────────────────────────────────────────────────────────
        // Step 3: Create and encode PolkadotXcm.send call
        // ─────────────────────────────────────────────────────────────

        const sendXcmTx = parachainApi.tx.PolkadotXcm.send({
            dest: destination,
            message: xcmMessage
        });

        const sendXcmEncoded = await sendXcmTx.getEncodedData();

        console.log('─── PolkadotXcm.send Call ───');
        console.log(`Encoded: ${toHex(sendXcmEncoded)}`);
        console.log('');

        // ─────────────────────────────────────────────────────────────
        // Step 4: Wrap in Sudo and encode
        // ─────────────────────────────────────────────────────────────

        const sudoTx = parachainApi.tx.Sudo.sudo({
            call: sendXcmTx.decodedCall
        });

        const sudoEncoded = await sudoTx.getEncodedData();

        console.log('─── Sudo(PolkadotXcm.send) Call ───');
        console.log(`Encoded: ${toHex(sudoEncoded)}`);
        console.log('');

        // ─────────────────────────────────────────────────────────────
        // Summary
        // ─────────────────────────────────────────────────────────────

        console.log('═══════════════════════════════════════════════════════════');
        console.log(' SUMMARY');
        console.log('═══════════════════════════════════════════════════════════');
        console.log(`Channel: ${paraId} <-> ${TARGET_PARA_ID} (bidirectional)`);
        console.log(`Execution fee: ${EXECUTION_FEE} planck (${Number(EXECUTION_FEE) / 1e12} tokens)`);
        console.log('');

        // ─────────────────────────────────────────────────────────────
        // Step 5: Submit if --submit flag is set
        // ─────────────────────────────────────────────────────────────

        if (SUBMIT) {
            console.log('═══════════════════════════════════════════════════════════');
            console.log(' SUBMITTING TRANSACTION');
            console.log('═══════════════════════════════════════════════════════════');

            const { signer, address } = createSigner(SEED);
            console.log(`Signing with: ${address}`);
            console.log('');

            const result = await new Promise((resolve, reject) => {
                let sub;
                const timeoutId = setTimeout(() => {
                    if (sub) sub.unsubscribe();
                    reject(new Error('Transaction timed out after 120 seconds'));
                }, 120_000);

                console.log('📤 Submitting sudo(PolkadotXcm.send)...');

                sub = sudoTx.signSubmitAndWatch(signer).subscribe({
                    next: (ev) => {
                        console.log(`   Event: ${ev.type}`);
                        if (ev.type === "txBestBlocksState" && ev.found) {
                            console.log(`   ✅ Included in block: ${ev.block.hash}`);
                        }
                        if (ev.type === "finalized") {
                            clearTimeout(timeoutId);
                            sub.unsubscribe();
                            resolve(ev);
                        }
                    },
                    error: (err) => {
                        clearTimeout(timeoutId);
                        if (sub) sub.unsubscribe();
                        reject(err);
                    }
                });
            });

            console.log('');
            console.log('═══════════════════════════════════════════════════════════');
            console.log(' ✅ TRANSACTION FINALIZED');
            console.log('═══════════════════════════════════════════════════════════');
            console.log(`Block hash: ${result.block.hash}`);
            console.log(`Block number: ${result.block.number}`);
            console.log('');
            console.log('Next steps:');
            console.log('  1. Check relay chain for HrmpSystemChannelOpened events');
            console.log('  2. Verify channels exist in hrmp.hrmpChannels storage');
            console.log(`  3. Expected channels: ${paraId} <-> ${TARGET_PARA_ID}`);
            console.log('');
        } else {
            console.log('To submit via Polkadot.js Apps:');
            console.log('  1. Go to Developer > Extrinsics > Decode');
            console.log('  2. Paste the Sudo-wrapped call');
            console.log('  3. Submit with sudo account');
            console.log('');
            console.log('Or run with --submit flag:');
            console.log(`  node open_hrmp_channel_westend.js ${PARACHAIN_WS} ${RELAY_WS} ${TARGET_PARA_ID} --submit`);
            console.log('');
        }

    } catch (error) {
        console.error('❌ Error:', error.message);
        console.error(error);
        process.exit(1);
    } finally {
        if (parachainClient) parachainClient.destroy();
        if (relayClient) relayClient.destroy();
        process.exit(0);
    }
}

await main();
