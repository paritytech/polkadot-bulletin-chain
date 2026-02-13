import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { Enum } from '@polkadot-api/substrate-bindings';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { newSigner } from './common.js';
import { westend } from './.papi/descriptors/dist/index.mjs';

// Command line arguments: [relay_ws_url] [para_id] [num_cores] [seed]
const args = process.argv.slice(2);
const RELAY_WS = args[0] || 'ws://localhost:9942';
const PARA_ID = parseInt(args[1] || '2487');
const NUM_CORES = parseInt(args[2] || '3');
const SEED = args[3] || '//Alice';

async function main() {
    await cryptoWaitReady();

    console.log(`Connecting to relay chain at ${RELAY_WS}...`);
    const client = createClient(getWsProvider(RELAY_WS));
    const api = client.getTypedApi(westend);

    const { signer } = newSigner(SEED);

    // Build assignCore calls for each core
    const calls = [];
    for (let core = 0; core < NUM_CORES; core++) {
        const call = api.tx.Coretime.assign_core({
            core,
            begin: 0,
            assignment: [
                [Enum("Task", PARA_ID), 57600]
            ],
            end_hint: undefined,
        }).decodedCall;
        calls.push(call);
        console.log(`  Prepared assignCore(core=${core}, paraId=${PARA_ID}, parts=57600)`);
    }

    // Batch all assign calls and wrap in sudo
    const batch = api.tx.Utility.batch_all({ calls });
    const sudoTx = api.tx.Sudo.sudo({ call: batch.decodedCall });

    console.log(`Submitting sudo batchAll to assign ${NUM_CORES} cores to para ${PARA_ID}...`);

    await new Promise((resolve, reject) => {
        const sub = sudoTx.signSubmitAndWatch(signer).subscribe({
            next: (ev) => {
                console.log(`  Event: ${ev.type}`);
                if (ev.type === "txBestBlocksState" && ev.found) {
                    console.log(`Included in block: ${ev.block.hash}`);
                    console.log(`Successfully assigned ${NUM_CORES} cores to para ${PARA_ID}`);
                    sub.unsubscribe();
                    resolve();
                }
            },
            error: (err) => {
                sub.unsubscribe();
                reject(err);
            },
        });
    });

    client.destroy();
}

main().catch((err) => {
    console.error('Error:', err.message || err);
    process.exit(1);
});
