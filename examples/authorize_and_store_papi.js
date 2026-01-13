import assert from "assert";
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { authorizeAccount, fetchCid, store} from './api.js';
import { setupKeyringAndSigners } from './common.js';
import { cidFromBytes } from "./cid_dag_metadata.js";
import { bulletin } from './.papi/descriptors/dist/index.mjs';

const NODE_WS = 'ws://localhost:10000';
const HTTP_IPFS_API = 'http://127.0.0.1:8080'   // Local IPFS HTTP gateway

async function main() {
    await cryptoWaitReady();

    let client, resultCode;
    try {
        // Init WS PAPI client and typed api.
        client = createClient(getWsProvider(NODE_WS));
        const bulletinAPI = client.getTypedApi(bulletin);

        // Signers.
        const { sudoSigner, whoSigner, whoAddress } = setupKeyringAndSigners('//Alice', '//Alice');

        // Data to store.
        const dataToStore = "Hello, Bulletin with PAPI - " + new Date().toString();
        let expectedCid = await cidFromBytes(dataToStore);

        // Authorize an account.
        await authorizeAccount(
            bulletinAPI,
            sudoSigner,
            whoAddress,
            1,
            BigInt(dataToStore.length)
        );

        // Store data.
        const cid = await store(bulletinAPI, whoSigner, dataToStore);
        console.log("✅ Data stored successfully with CID:", cid);

        // Read back from IPFS
        let downloadedContent = await fetchCid(HTTP_IPFS_API, cid);
        console.log("✅ Downloaded content:", downloadedContent.toString());
        assert.deepStrictEqual(
            cid,
            expectedCid,
            '❌ expectedCid does not match cid!'
        );
        assert.deepStrictEqual(
            dataToStore,
            downloadedContent.toString(),
            '❌ dataToStore does not match downloadedContent!'
        );
        console.log(`✅ Verified content!`);

        console.log(`\n\n\n✅✅✅ Test passed! ✅✅✅`);
        resultCode = 0;
    } catch (error) {
        console.error("❌ Error:", error);
        resultCode = 1;
    } finally {
        if (client) client.destroy();
        process.exit(resultCode);
    }
}

await main();
