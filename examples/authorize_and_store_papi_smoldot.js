import * as smoldot from 'smoldot';
import { ApiPromise, WsProvider } from "@polkadot/api";
import { createClient } from 'polkadot-api';
import { Keyring } from "@polkadot/keyring";
import { getSmProvider } from 'polkadot-api/sm-provider';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { create } from 'ipfs-http-client';
import { cidFromBytes } from './common.js';
import { bulletin } from './.papi/descriptors/dist/index.mjs';
import { sr25519CreateDerive } from "@polkadot-labs/hdkd"
import {
  DEV_PHRASE,
  entropyToMiniSecret,
  mnemonicToEntropy,
} from "@polkadot-labs/hdkd-helpers"
import { getPolkadotSigner } from "polkadot-api/signer"

// Generate PAPI descriptors using local node:
// npx papi add -w ws://localhost:10000 bulletin
// npx papi
async function main() {
    await cryptoWaitReady();
    
    // Bob's address - to get the chainspec
    console.log('Fetching chainspec from Bob node...');
    const bobWs = new WsProvider('ws://localhost:12346');
    const bobApi = await ApiPromise.create({ provider: bobWs });
    await bobApi.isReady;

    // Create keyring and accounts
    const miniSecret = entropyToMiniSecret(mnemonicToEntropy(DEV_PHRASE))
    const derive = sr25519CreateDerive(miniSecret)
    const hdkdKeyPair = derive("//Alice")
 
    const aliceSigner = getPolkadotSigner(
        hdkdKeyPair.publicKey,
        "Sr25519",
        hdkdKeyPair.sign,
    )

    // Data
    const who = aliceSigner.publicKey;
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
    const sd = smoldot.start({
        maxLogLevel: 4, // 0=off, 1=error, 2=warn, 3=info, 4=debug, 5=trace
        logCallback: (level, target, message) => {
            const levelNames = ['ERROR', 'WARN', 'INFO', 'DEBUG', 'TRACE'];
            const levelName = levelNames[level - 1] || 'UNKNOWN';
            console.log(`[smoldot:${levelName}] ${target}: ${message}`);
        }
    });
    const chain = await sd.addChain({ chainSpec: modifiedChainSpec });
    const client = createClient(getSmProvider(chain));
    const bulletinAPI = client.getTypedApi(bulletin);

    console.log('✅ who is who: ', who.toString());
    const w = who.toString();
    bulletinAPI.tx.transactionStorage.authorizeAccount({
        w,
        transactions,
        bytes
    }).signAndSubmit(aliceSigner)
        .then(() => console.log("✅ Authorized!"))
        .catch((err) => {
            console.error("❌ authorize error: ", err);
            process.exit(1);
    });

    // console.log('✅ storing...');
    // bulletinAPI.tx.transactionStorage.store(dataToStore)
    //     .signSubmitAndWatch(aliceSigner).subscribe({
    //         next: (ev) => {
    //             console.log("⏭️ store next: ", ev);
    //         },
    //         error: (err) => {
    //             console.error("❌ store error: ", err);
    //             process.exit(1);
    //         },
    //     });
}

await main();
