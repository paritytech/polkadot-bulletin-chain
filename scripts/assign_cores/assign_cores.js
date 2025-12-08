/**
 * Assign cores to a parachain.
 * Usage: node assign_cores.js <relay_endpoint> <para_id> <cores...>
 */
const { ApiPromise, WsProvider, Keyring } = require("@polkadot/api");

const [,, endpoint, paraIdStr, ...coreStrs] = process.argv;
const paraId = parseInt(paraIdStr);
const cores = coreStrs.map(c => parseInt(c));

if (!endpoint || isNaN(paraId) || cores.length === 0) {
    console.log("Usage: node assign_cores.js <relay_endpoint> <para_id> <cores...>");
    process.exit(1);
}

console.log("Assigning cores to parachain...");

const api = await ApiPromise.create({ provider: new WsProvider(endpoint), noInitWarn: true });
const alice = new Keyring({ type: "sr25519" }).addFromUri("//Alice");

const calls = cores.map(core => api.tx.coretime.assignCore(core, 0, [[{ Task: paraId }, 57600]], null));
const tx = api.tx.sudo.sudo(api.tx.utility.batch(calls));

await new Promise((resolve, reject) => {
    tx.signAndSend(alice, ({ status, dispatchError }) => {
        if (status.isFinalized) {
            if (dispatchError) reject(new Error(dispatchError.toString()));
            else resolve();
        }
    });
});

console.log("✅ Cores assigned successfully!");
await api.disconnect();
