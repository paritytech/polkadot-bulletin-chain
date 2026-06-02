/**
 * Store a chunked file on Bulletin Chain with a UnixFS DAG-PB root and verify
 * it round-trips through an IPFS gateway.
 *
 * Usage:
 *   node native_ipfs_dag_pb_chunked_data.js [ws_url] [seed] [ipfs_api_url]
 *
 * Flags:
 *   --smoldot=<relay-spec>:<para-spec>   Use smoldot light client.
 *   --smoldot-sync-wait=N                Seconds to wait for smoldot sync (default 30).
 *   --signer-disc=XX                     Append discriminator to user seed.
 */

import { cryptoWaitReady } from '@polkadot/util-crypto';
import { cidFromBytes, buildUnixFSDagPB, convertCid } from './cid_dag_metadata.js';
import {
    generateTextImage,
    fileToDisk,
    filesAreEqual,
    newSigner,
    waitForChainReady,
    waitForBlockProduction,
    parseProviderArgs,
    buildProviders,
    DEFAULT_IPFS_GATEWAY_URL,
} from './common.js';
import { fetchCid } from './api.js';
import { bulletin } from './.papi/descriptors/dist/index.js';
import { blobFromItems, BulletinClient, WaitFor } from '../sdk/typescript/dist/index.mjs';
import assert from 'assert';

import fs from 'fs';
import os from 'os';
import path from 'path';
import * as dagPB from '@ipld/dag-pb';

const args = process.argv.slice(2).filter(arg => !arg.startsWith('--'));
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Eve';
const HTTP_IPFS_API = args[2] || DEFAULT_IPFS_GATEWAY_URL;
const PROVIDER_CFG = parseProviderArgs(process.argv);
const signerDiscriminator =
    process.argv.find(arg => arg.startsWith('--signer-disc='))?.split('=')[1] ?? null;

const CHUNK_SIZE = 6 * 1024; // 6 KB

async function main() {
    await cryptoWaitReady();

    let client, providersHandle, resultCode;
    try {
        const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'bulletinimggen-'));
        const filePath = path.join(tmpDir, 'image.jpeg');
        const downloadedFilePath = path.join(tmpDir, 'downloaded.jpeg');
        generateTextImage(filePath, 'Hello, Bulletin dag - ' + new Date().toString());

        providersHandle = await buildProviders({ ...PROVIDER_CFG, wsUrl: NODE_WS });

        const { signer: authorizationSigner } = newSigner(SEED);
        const userSeed = signerDiscriminator
            ? `//Nativeipfsdagsigner${signerDiscriminator}`
            : '//Nativeipfsdagsigner';
        const { signer: whoSigner, address: whoAddress } = newSigner(userSeed);

        client = new BulletinClient({
            descriptor: bulletin,
            providers: providersHandle.providers,
            uploadSigner: whoSigner,
            authorizerSigner: authorizationSigner,
        });

        await waitForChainReady(client.api);
        await waitForBlockProduction(client.api);

        console.log('✅ Connected to Bulletin chain');
        console.log(`💳 Using account: ${whoAddress}`);

        // Authorize the user account for chunks + DAG storage.
        await client
            .authorizeAccount(whoAddress, 128, BigInt(64 * 1024 * 1024))
            .withWaitFor(WaitFor.Finalized)
            .send();

        // Chunk the file locally, then store all chunks via one SDK upload.
        const fileData = fs.readFileSync(filePath);
        const chunks = [];
        for (let i = 0; i < fileData.length; i += CHUNK_SIZE) {
            const bytes = fileData.subarray(i, i + CHUNK_SIZE);
            const cid = await cidFromBytes(bytes);
            chunks.push({ cid, bytes, len: bytes.length });
        }
        console.log(`✂️ Split into ${chunks.length} chunks`);

        const items = chunks.map((c) => ({ data: c.bytes }));
        const { cids: chunkCids } = await client
            .submit(await client.estimateUpload(items), blobFromItems(items))
            .withWaitFor(WaitFor.Finalized)
            .send();
        for (let i = 0; i < chunks.length; i++) {
            assert.deepStrictEqual(chunkCids[i].toString(), chunks[i].cid.toString());
        }
        console.log(`✅ Stored ${chunks.length} chunks`);

        // Build the UnixFS DAG-PB root from the chunks.
        const { rootCid: expectedRootCid, dagBytes } = await buildUnixFSDagPB(chunks, 0x12);
        const calculatedRootCid = await cidFromBytes(dagBytes, 0x70, 0x12);
        assert.deepStrictEqual(expectedRootCid.toString(), calculatedRootCid.toString());

        // Store the DAG bytes through the SDK with the DAG-PB / SHA2-256 codec.
        const dagItems = [{ data: dagBytes, codec: 0x70, hashAlgo: 0x12 }];
        const { cids: rootCids } = await client
            .submit(await client.estimateUpload(dagItems), blobFromItems(dagItems))
            .withWaitFor(WaitFor.Finalized)
            .send();
        const rootCid = rootCids[0];
        assert.deepStrictEqual(expectedRootCid.toString(), rootCid.toString());

        console.log('🧱 DAG stored on Bulletin with CID:', rootCid.toString());
        console.log('\n🌐 Try opening in browser:');
        console.log(`   ${HTTP_IPFS_API}/ipfs/${rootCid.toString()}`);
        console.log(`   ${HTTP_IPFS_API}/ipfs/${convertCid(rootCid, 0x55)}`);

        // Download via IPFS gateway and verify round-trip.
        const fullBuffer = await fetchCid(HTTP_IPFS_API, rootCid);
        console.log(`✅ Reconstructed file size: ${fullBuffer.length} bytes`);
        await fileToDisk(downloadedFilePath, fullBuffer);
        filesAreEqual(filePath, downloadedFilePath);

        const rootCidAsRaw = convertCid(rootCid, 0x55);
        const storedDagNode = dagPB.decode(await fetchCid(HTTP_IPFS_API, rootCidAsRaw));
        const decodedDagNode = dagPB.decode(Buffer.from(dagBytes));
        assert.deepStrictEqual(storedDagNode, decodedDagNode);

        console.log(`\n\n\n✅✅✅ Passed all tests ✅✅✅`);
        resultCode = 0;
    } catch (error) {
        console.error('❌ Error:', error);
        resultCode = 1;
    } finally {
        if (client) await client.destroy();
        if (providersHandle) await providersHandle.cleanup();
        process.exit(resultCode);
    }
}

await main();
