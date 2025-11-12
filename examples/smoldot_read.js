import * as smoldot from 'smoldot';
import fs from 'fs';

const chainSpec = fs.readFileSync('./node/chain-specs/bulletin-polkadot.json', 'utf8');
const client = smoldot.start();

await client
    .addChain({ chainSpec })
    .then((chain) => {
        chain.sendJsonRpc(`{"jsonrpc":"2.0","id":12,"method":"chain_getFinalizedHead","params":[]}`);
        return chain;
      })
      .then(async (chain) => {
        const response = await chain.nextJsonRpcResponse();
        console.log("✅ JSON-RPC response:", response);
      })
      .then(() => client.terminate())
