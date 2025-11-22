import * as smoldot from 'smoldot';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { ApiPromise, WsProvider } from '@polkadot/api';


async function main() {
    await cryptoWaitReady();

    // Download the raw chain spec from an RPC node.
    // Note: The dApps or the actual application should not connect to any external RPC node,
    //       but should instead use chain specs provided directly.
    // 12346 is Bob's validator rpc port, because Alice does not have `bootNodes`.
    const ws = new WsProvider('ws://localhost:12346');
    const api = await ApiPromise.create({ provider: ws });
    await api.isReady;
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
