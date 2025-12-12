import { ChopsticksProvider, setup } from "@acala-network/chopsticks-core";

const chain = await setup({ endpoint: "wss://rpc.interweb-it.com/bulletin" });
await chain.api.isReady;

const innerProvider = new ChopsticksProvider(chain);
await innerProvider.isReady;

const result = await innerProvider.send("dev_newBlock", [], false);
console.log({ result });