import { createClient, Binary, FixedSizeBinary } from "polkadot-api";
import { getWsProvider } from "polkadot-api/ws-provider/node";
import { bulletin } from "@polkadot-api/descriptors";

const RPC = process.env.BULLETIN_RPC ?? "wss://paseo-bulletin-next-rpc.polkadot.io";
const contentHash = process.argv[2]; // 0x… 32-byte blake2b digest
if (!contentHash) {
  console.error("usage: node bulletin-has-cid.mjs <0x-content-hash>");
  process.exit(2);
}

const client = createClient(getWsProvider(RPC));
const api = client.getTypedApi(bulletin);

// content_hash is a fixed 32-byte type.
const arg = FixedSizeBinary.fromHex(contentHash);

const stored = await api.query.TransactionStorage.TransactionByContentHash.getValue(arg);
const retention = await api.query.TransactionStorage.RetentionPeriod.getValue().catch(() => null);
const head = await client.getFinalizedBlock();

console.log(`bulletin: ${RPC}`);
console.log(`content_hash: ${contentHash}`);
console.log(`finalized head: #${head.number}`);
if (retention != null) console.log(`retention period: ${retention} blocks`);
if (stored) {
  const [block, index] = Array.isArray(stored) ? stored : [stored.block ?? stored[0], stored.index ?? stored[1]];
  console.log(`STORED: yes — block #${block}, index ${index}`);
} else {
  console.log(`STORED: no — not currently in TransactionByContentHash (expired or never stored)`);
}
client.destroy();
