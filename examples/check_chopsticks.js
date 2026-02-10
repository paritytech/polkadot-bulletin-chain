import { ChopsticksProvider, setup } from "@acala-network/chopsticks-core";

console.log("Setting up Chopsticks with Bulletin chain...");

try {
  const chain = await setup({ endpoint: "wss://rpc.interweb-it.com/bulletin" });
  await chain.api.isReady;
  console.log("Chain setup complete.");

  const innerProvider = new ChopsticksProvider(chain);
  await innerProvider.isReady;
  console.log("Provider ready. Attempting to build a new block...");

  const result = await innerProvider.send("dev_newBlock", [], false);
  console.log("dev_newBlock result:", { result });

  await chain.close();
  console.log("Success! Chopsticks works with Bulletin chain.");
  process.exit(0);
} catch (err) {
  console.error("Failed:", err.message || err);
  process.exit(1);
}
