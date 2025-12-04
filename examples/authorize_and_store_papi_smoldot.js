import * as smoldot from 'smoldot';
import { ApiPromise, WsProvider } from "@polkadot/api";
import { Keyring } from "@polkadot/keyring";
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { create } from 'ipfs-http-client';
import { cidFromBytes } from './common.js';


async function main() {
    await cryptoWaitReady();

    // Alice's address - for transaction creation
    console.log('Connecting to Alice node for transaction creation...');
    const aliceWs = new WsProvider('ws://localhost:10000');
    const aliceApi = await ApiPromise.create({ provider: aliceWs });
    await aliceApi.isReady;
    
    // Bob's address - to get the chainspec
    console.log('Fetching chainspec from Bob node...');
    const bobWs = new WsProvider('ws://localhost:12346');
    const bobApi = await ApiPromise.create({ provider: bobWs });
    await bobApi.isReady;

    // Create keyring and accounts
    const keyring = new Keyring({ type: 'sr25519' });
    const sudoAccount = keyring.addFromUri('//Alice');
    const whoAccount = keyring.addFromUri('//Alice');

    // Data
    const who = whoAccount.address;
    const transactions = 32;
    const bytes = 64 * 1024 * 1024; // 64 MB

    // Prepare data for storage
    const dataToStore = "Hello, Bulletin with PAPI + Smoldot - " + new Date().toString();
    const cid = cidFromBytes(dataToStore);

    // Start smoldot with logging enabled
    console.log('\n🚀 Starting smoldot...');
    const client = smoldot.start({
        maxLogLevel: 0, // 0=off, 1=error, 2=warn, 3=info, 4=debug, 5=trace
        logCallback: (level, target, message) => {
            const levelNames = ['ERROR', 'WARN', 'INFO', 'DEBUG', 'TRACE'];
            const levelName = levelNames[level - 1] || 'UNKNOWN';
            console.log(`[smoldot:${levelName}] ${target}: ${message}`);
        }
    });

    // Get chain spec from Bob node and remove protocolId to allow smoldot to sync with local chain.
    const chainSpec = (await bobApi.rpc.syncstate.genSyncSpec(true)).toString();
    const chainSpecObj = JSON.parse(chainSpec);
    chainSpecObj.protocolId = null;
    const modifiedChainSpec = JSON.stringify(chainSpecObj);

    await client
        .addChain({ chainSpec: modifiedChainSpec })
        .then(async (chain) => {
            // Give smoldot a moment to sync
            console.log("⏳ Waiting for smoldot to sync...");
            await new Promise(resolve => setTimeout(resolve, 12000));
            
            const authorizeTx = aliceApi.tx.transactionStorage.authorizeAccount(
                who,
                transactions,
                bytes
            );
            const sudoTx = aliceApi.tx.sudo.sudo(authorizeTx);
            const signedAuthTx = await sudoTx.signAsync(sudoAccount);
            
            const authBlockHash = await submitAndWatch(chain, signedAuthTx, 'authorizeAccount');
            console.log('✅ Authorized in block:', authBlockHash);
            
            return chain;
        })
        .then(async (chain) => {
            const dataBytes = Buffer.from(dataToStore);
            const storeTx = aliceApi.tx.transactionStorage.store(dataBytes);
            const signedStoreTx = await storeTx.signAsync(whoAccount);
            
            const storeBlockHash = await submitAndWatch(chain, signedStoreTx, 'store');
            console.log('✅ Stored data with CID:', cid);
            console.log('   In block:', storeBlockHash);
            
            return chain;
        })
        .then(async () => {
            // Cleanup
            await aliceApi.disconnect();
            await bobApi.disconnect();
        })
        .then(() => client.terminate())
        .catch(async (error) => {
            console.error('Error while executing the example: ', error.message);
            await aliceApi.disconnect();
            await bobApi.disconnect();
            client.terminate();
            throw error;
        });
}

async function submitAndWatch(chain, signedTx, description) {
    console.log(`✅ Signed ${description} transaction:`, signedTx.toHex());
    
    // Submit transaction with watch
    chain.sendJsonRpc(`{"jsonrpc":"2.0","id":2,"method":"author_submitAndWatchExtrinsic","params":["${signedTx.toHex()}"]}`);
    
    // Get subscription ID
    const response = await chain.nextJsonRpcResponse();
    const subscriptionId = JSON.parse(response).result;
    console.log(`${description} subscription ID:`, subscriptionId);
    
    // Listen for transaction status updates
    while (true) {
        const statusUpdate = await chain.nextJsonRpcResponse();
        const parsed = JSON.parse(statusUpdate);
        console.log(`${description} transaction status:`, parsed);
        
        // Check if transaction is in a block
        if (parsed.params?.result?.inBlock) {
            console.log(`✅ ${description} transaction included in block:`, parsed.params.result.inBlock);
            return parsed.params.result.inBlock;
        }
        
        // Check for failure conditions
        if (parsed.params?.result === 'dropped' || parsed.params?.result === 'invalid') {
            throw new Error(`${description} transaction failed: ${parsed.params.result}`);
        }
        if (parsed.params?.result?.Invalid || parsed.params?.result?.Dropped) {
            throw new Error(`${description} transaction failed: ${JSON.stringify(parsed.params.result)}`);
        }
    }
}

await main();
