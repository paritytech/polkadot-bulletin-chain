import { ApiPromise, WsProvider, Keyring } from '@polkadot/api';
import { cryptoWaitReady, blake2AsU8a } from '@polkadot/util-crypto';
import { CID } from 'multiformats/cid';
import { create } from 'multiformats/hashes/digest';

const [,, endpoint, seed, data] = process.argv;

await cryptoWaitReady();
const api = await ApiPromise.create({ provider: new WsProvider(endpoint), noInitWarn: true });
const pair = new Keyring({ type: 'sr25519' }).addFromUri(seed);

// Authorize
console.error('Authorizing...');
await api.tx.sudo.sudo(api.tx.transactionStorage.authorizeAccount(pair.address, 2, 65536)).signAndSend(pair);
await new Promise(r => setTimeout(r, 7000));

// Store
console.error('Storing...');
const cid = CID.createV1(0x55, create(0xb220, blake2AsU8a(data)));
await api.tx.transactionStorage.store(data).signAndSend(pair);
await new Promise(r => setTimeout(r, 7000));

console.log(`OUTPUT_CID=${cid.toString()}`);
await api.disconnect();
