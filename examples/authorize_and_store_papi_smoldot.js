import * as smoldot from 'smoldot';
import { ApiPromise, WsProvider } from "@polkadot/api";
import { createClient } from 'polkadot-api';
import { Keyring } from "@polkadot/keyring";
import { getSmProvider } from 'polkadot-api/sm-provider';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { create } from 'ipfs-http-client';
import { cidFromBytes } from './common.js';
import { bulletin } from './.papi/descriptors/dist/index.mjs';


async function main() {
    await cryptoWaitReady();
    
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

    // Note: In real usage, this step is not required — the chain spec with bootNodes should be included as part of the dApp.
    //       For local testing, we use this to fetch the actual chain spec from the local node.
    // Get chain spec from Bob node and remove protocolId to allow smoldot to sync with local chain.
    const chainSpec = (await bobApi.rpc.syncstate.genSyncSpec(true)).toString();
    const chainSpecObj = JSON.parse(chainSpec);
    chainSpecObj.protocolId = null;
    const modifiedChainSpec = JSON.stringify(chainSpecObj);

    // Initialize Smoldot client
    const chain = await smoldot.start().addChain({ chainSpec: modifiedChainSpec });
    const client = createClient(getSmProvider(chain));
    const bulletinAPI = client.getTypedApi(bulletin);

    bulletinAPI.tx.transactionStorage.authorizeAccount({
        who,
        transactions,
        bytes
    }).signAndSubmit(sudoAccount)
        .then(() => console.log("✅ Authorized!"))
        .catch((err) => {
            console.error("authorize error: ", err);
            process.exit(1);
    });

    bulletinAPI.tx.transactionStorage.store(dataToStore)
        .signSubmitAndWatch(whoAccount).subscribe({
            next: (ev) => {
                console.log("⏭️ store next: ", ev);
            },
            error: (err) => {
                console.error("❌ store error: ", err);
                process.exit(1);
            },
        });
}

await main();
