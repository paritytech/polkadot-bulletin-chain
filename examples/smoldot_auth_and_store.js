import * as smoldot from 'smoldot';
import fs from 'fs';
import { ApiPromise } from "@polkadot/api";
import { Keyring } from "@polkadot/keyring";
import { WsProvider } from "@polkadot/api";

const chainSpec = fs.readFileSync('./node/chain-specs/bulletin-polkadot.json', 'utf8');
const client = smoldot.start();

// Bulletin address
const provider = new WsProvider('ws://localhost:10000');
const api = await ApiPromise.create({ provider });
await api.isReady;

const keyring = new Keyring({ type: 'sr25519' });
const sudo_pair = keyring.addFromUri('//Alice');
const who_pair = keyring.addFromUri('//Alice');

// data
const who = who_pair.address; // ✅ base58 string
const transactions = 32;
const bytes = 64 * 1024 * 1024; // 64 MB

const authorizeTx = api.tx.transactionStorage.authorizeAccount(
    who,
    transactions,
    bytes
  );

const signedTx = await authorizeTx.signAsync(sudo_pair);
console.log("✅ Signed transaction:", signedTx.toHex());

await client
    .addChain({ chainSpec })
    .then((chain) => {
        chain.sendJsonRpc(`{"jsonrpc":"2.0","id":2,"method":"author_submitAndWatchExtrinsic","params":["${signedTx.toHex()}"]}`);
        return chain;
      })
      .then(async (chain) => {
        const response = await chain.nextJsonRpcResponse();
        console.log("✅ JSON-RPC response:", response);
      })

