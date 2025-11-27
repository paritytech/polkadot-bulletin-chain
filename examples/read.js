import * as smoldot from 'smoldot';
import fs from 'fs';
import { ApiPromise } from "@polkadot/api";
import { Keyring } from "@polkadot/keyring";
import { WsProvider } from "@polkadot/api";


async function main() {
    // Bob's address
    const ws = new WsProvider('ws://localhost:12346');
    const bobApi = await ApiPromise.create({ provider: ws });
    await bobApi.isReady;
    const chainSpec = (await bobApi.rpc.syncstate.genSyncSpec(true)).toString();

    // Alice's address
    const provider = new WsProvider('ws://localhost:10000');
    const api = await ApiPromise.create({ provider });
    await api.isReady;
    
    
    // Check if chainSpec has bootnodes
    const chainSpecObj = JSON.parse(chainSpec);
    console.log("🔗 Bootnodes in chainSpec:", chainSpecObj.bootNodes || []);
    if (!chainSpecObj.bootNodes || chainSpecObj.bootNodes.length === 0) {
        console.warn("⚠️  No bootnodes found! Smoldot won't be able to sync.");
    }
    
    // Set protocolId to null
    console.log("Original protocolId:", chainSpecObj.protocolId);
    chainSpecObj.protocolId = null;
    console.log("Modified protocolId:", chainSpecObj.protocolId);
    
    // Convert back to string
    const modifiedChainSpec = JSON.stringify(chainSpecObj);

    const keyring = new Keyring({ type: 'sr25519' });
    const sudo_pair = keyring.addFromUri('//Alice');
    const who_pair = keyring.addFromUri('//Alice');

    // data
    const who = who_pair.address;
    const transactions = 32;
    const bytes = 64 * 1024 * 1024; // 64 MB

    const authorizeTx = api.tx.transactionStorage.authorizeAccount(
        who,
        transactions,
        bytes
    );

    // Wrap in sudo since authorizeAccount requires root privileges
    const sudoTx = api.tx.sudo.sudo(authorizeTx);
    const signedTx = await sudoTx.signAsync(sudo_pair);
    console.log("✅ Signed transaction:", signedTx.toHex());

    // Start smoldot with logging enabled
    const client = smoldot.start({
        maxLogLevel: 4, // 0=off, 1=error, 2=warn, 3=info, 4=debug, 5=trace
        logCallback: (level, target, message) => {
            const levelNames = ['ERROR', 'WARN', 'INFO', 'DEBUG', 'TRACE'];
            const levelName = levelNames[level - 1] || 'UNKNOWN';
            console.log(`[smoldot:${levelName}] ${target}: ${message}`);
        }
    });
    await client
        .addChain({ chainSpec: modifiedChainSpec })
        .then(async (chain) => {
            // Give smoldot a moment to sync
            console.log("⏳ Waiting for smoldot to sync...");
            await new Promise(resolve => setTimeout(resolve, 2000));
            
            // First, test with a simple storage query
            console.log("🔍 Testing smoldot with a storage query...");
            chain.sendJsonRpc('{"jsonrpc":"2.0","id":1,"method":"chain_getBlockHash","params":[0]}');
            const queryResponse = await chain.nextJsonRpcResponse();
            const queryParsed = JSON.parse(queryResponse);
            console.log("✅ Genesis block hash:", queryParsed.result);
            
            // Check current head with timeout
            console.log("🔍 Checking smoldot's current head...");
            chain.sendJsonRpc('{"jsonrpc":"2.0","id":3,"method":"chain_getHead","params":[]}');
            
            const headResponse = await Promise.race([
                chain.nextJsonRpcResponse(),
                new Promise((_, reject) => setTimeout(() => reject(new Error("Timeout waiting for head")), 12000))
            ]).catch(err => {
                throw err;
            });
            
            const headParsed = JSON.parse(headResponse);
            console.log("Current head hash:", headParsed.result);
            
            // Get the block number for current head
            chain.sendJsonRpc(`{"jsonrpc":"2.0","id":4,"method":"chain_getHeader","params":["${headParsed.result}"]}`);
            const headerResponse = await chain.nextJsonRpcResponse();
            const headerParsed = JSON.parse(headerResponse);
            console.log("Current head block number:", parseInt(headerParsed.result.number, 16));

            chain.sendJsonRpc(`{"jsonrpc":"2.0","id":2,"method":"author_submitAndWatchExtrinsic","params":["${signedTx.toHex()}"]}`);
            return chain;
        })
        .then(async (chain) => {
            // Get subscription ID
            const response = await chain.nextJsonRpcResponse();
            console.log("Subscription ID:", JSON.parse(response).result);
            // Listen for transaction status updates
            while (true) {
                const statusUpdate = await chain.nextJsonRpcResponse();
                const parsed = JSON.parse(statusUpdate);
                console.log("Transaction status:", parsed);
                
                // Check if transaction is finalized
                if (parsed.params?.result?.inBlock) {
                    console.log("✅ Transaction finalized in block:", parsed.params.result.inBlock);
                    break;
                }
                
                if (parsed.params?.result === 'dropped' || parsed.params?.result === 'invalid') {
                    console.error("❌ Transaction failed:", parsed.params.result);
                    break;
                }
                if (parsed.params?.result?.Invalid || parsed.params?.result?.Dropped) {
                    console.error("❌ Transaction failed:", parsed.params.result);
                    break;
                }
            }
            
            return chain;
        })
        .then(() => client.terminate())
}

await main();