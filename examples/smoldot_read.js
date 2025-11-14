import * as smoldot from 'smoldot';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { ApiPromise, WsProvider } from '@polkadot/api';


async function main() {
    await cryptoWaitReady();

    const ws = new WsProvider('wss://bulletin.rpc.amforc.com');
    const api = await ApiPromise.create({ provider: ws });
    await api.isReady;

    // true: raw chain spec
    const chainSpec = await api.rpc.syncstate.genSyncSpec(true);

    const client = smoldot.start();

    await client
            .addChain({ chainSpec: chainSpec.toString() })
            .then((chain) => {
                chain.sendJsonRpc(`{"jsonrpc":"2.0","id":12,"method":"chain_getFinalizedHead","params":[]}`);
                return chain;
            })
            .then(async (chain) => {
                const response = await chain.nextJsonRpcResponse();
                console.log("✅ JSON-RPC response:", response);
                return chain;
            })
            .then(() => client.terminate());
}

await main();
