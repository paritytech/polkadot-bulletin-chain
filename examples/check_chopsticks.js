import { ChopsticksProvider, setStorage, setup } from "@acala-network/chopsticks-core";

const endpoint = process.argv[2] || "wss://westend-bulletin-rpc.polkadot.io";
// Authoring past `RetentionPeriod` requires the collator-side InherentDataProvider to attach a
// `TransactionStorageProof`. Chopsticks doesn't carry the off-chain chunks, so the only way
// it can build N+RetentionPeriod blocks is if the runtime doesn't expect a proof at all.
// `RetentionPeriod=u32::MAX` makes `target_number.saturating_sub(MAX) == 0` for every block,
// short-circuiting `proof_ok` via the `target_number.is_zero()` branch.
const newBlocksToProduce = parseInt(process.env.CHOPSTICKS_NEW_BLOCKS || "8", 10);

console.log(`Setting up Chopsticks with Bulletin chain (endpoint: ${endpoint})...`);

try {
  const chain = await setup({ endpoint, mockSignatureHost: true });
  await chain.api.isReady;
  console.log("Chain setup complete.");

  await setStorage(chain, {
    TransactionStorage: {
      RetentionPeriod: 4294967295,
    },
  });
  console.log("Storage override applied: TransactionStorage.RetentionPeriod = u32::MAX");

  const innerProvider = new ChopsticksProvider(chain);
  await innerProvider.isReady;

  const startHead = chain.head.number;
  console.log(`Fork head: ${startHead}. Attempting to build ${newBlocksToProduce} blocks...`);

  for (let i = 0; i < newBlocksToProduce; i++) {
    const result = await innerProvider.send("dev_newBlock", [], false);
    if (!result) {
      throw new Error(`dev_newBlock returned empty result at iteration ${i}`);
    }
  }

  const endHead = chain.head.number;
  const produced = endHead - startHead;
  if (produced !== newBlocksToProduce) {
    throw new Error(`Produced ${produced} blocks, expected ${newBlocksToProduce} (head: ${startHead} -> ${endHead})`);
  }
  console.log(`Produced ${produced} blocks: ${startHead} -> ${endHead}`);

  await chain.close();
  console.log("Success! Chopsticks works with Bulletin chain.");
  process.exit(0);
} catch (err) {
  console.error("Failed:", err.message || err);
  process.exit(1);
}
