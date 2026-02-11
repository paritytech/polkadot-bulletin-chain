import { ChopsticksProvider, setup } from "@acala-network/chopsticks-core";

const endpoint = process.argv[2] || "wss://westend-bulletin-rpc.polkadot.io";
console.log(`Setting up Chopsticks with Bulletin chain (endpoint: ${endpoint})...`);

try {
  const chain = await setup({ endpoint });
  await chain.api.isReady;
  console.log("Chain setup complete.");

  const innerProvider = new ChopsticksProvider(chain);
  await innerProvider.isReady;
  console.log("Provider ready. Attempting to build a new block...");

  const blockNumBefore = chain.head.number;
  const result = await innerProvider.send("dev_newBlock", [], false);
  console.log("dev_newBlock result:", { result });

  if (!result) {
    throw new Error("dev_newBlock returned empty result");
  }

  const blockNumAfter = chain.head.number;
  if (blockNumAfter <= blockNumBefore) {
    throw new Error(`Block number did not increase: before=${blockNumBefore}, after=${blockNumAfter}`);
  }
  console.log(`Block number increased: ${blockNumBefore} -> ${blockNumAfter}`);

  await chain.close();
  console.log("Success! Chopsticks works with Bulletin chain.");
  process.exit(0);
} catch (err) {
  console.error("Failed:", err.message || err);
  process.exit(1);
}
