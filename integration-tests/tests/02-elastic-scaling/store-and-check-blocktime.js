import { ApiPromise, WsProvider, Keyring } from '@polkadot/api';
import { cryptoWaitReady } from '@polkadot/util-crypto';

const [,, endpoint, seed] = process.argv;
const NUM_TRANSACTIONS = 10;
const MAX_AVG_BLOCK_TIME = 2500;

await cryptoWaitReady();
const api = await ApiPromise.create({ provider: new WsProvider(endpoint), noInitWarn: true });
const pair = new Keyring({ type: 'sr25519' }).addFromUri(seed);

// Authorize account
console.log('Authorizing account...');
await api.tx.sudo.sudo(api.tx.transactionStorage.authorizeAccount(pair.address, NUM_TRANSACTIONS + 1, 65536 * NUM_TRANSACTIONS)).signAndSend(pair);
await new Promise(r => setTimeout(r, 7000));

// Start block time measurement
const blockTimes = [];
let lastBlockTime = null;

const unsub = await api.rpc.chain.subscribeNewHeads((header) => {
    const now = Date.now();
    if (lastBlockTime) {
        const delta = now - lastBlockTime;
        blockTimes.push(delta);
        console.log(`Block #${header.number}: ${delta}ms`);
    } else {
        console.log(`Block #${header.number}: (first)`);
    }
    lastBlockTime = now;
});

// Submit transactions spaced 2 seconds apart to land in different blocks
console.log(`Submitting ${NUM_TRANSACTIONS} store transactions (2s apart)...`);
for (let i = 0; i < NUM_TRANSACTIONS; i++) {
    const data = `0x${Buffer.from(`test-data-${i}`).toString('hex')}`;
    await api.tx.transactionStorage.store(data).signAndSend(pair);
    console.log(`Tx ${i} submitted`);
    if (i < NUM_TRANSACTIONS - 1) {
        await new Promise(r => setTimeout(r, 2000));
    }
}

// Wait for last tx to be included
await new Promise(r => setTimeout(r, 5000));

unsub();

if (blockTimes.length < 3) {
    console.error(`❌ Not enough blocks (only ${blockTimes.length} intervals)`);
    await api.disconnect();
    process.exit(1);
}

const avg = blockTimes.reduce((a, b) => a + b, 0) / blockTimes.length;
console.log(`Average block time: ${avg.toFixed(0)}ms over ${blockTimes.length} intervals`);

if (avg > MAX_AVG_BLOCK_TIME) {
    console.error(`❌ Block time too slow: ${avg.toFixed(0)}ms > ${MAX_AVG_BLOCK_TIME}ms`);
    await api.disconnect();
    process.exit(1);
}

console.log(`✅ Elastic scaling working - average block time: ${avg.toFixed(0)}ms`);
await api.disconnect();
