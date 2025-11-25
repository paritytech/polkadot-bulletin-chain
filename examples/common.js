import assert from "assert";

import {blake2AsU8a, keccak256AsU8a, sha256AsU8a} from '@polkadot/util-crypto'

import { Enum } from '@polkadot-api/substrate-bindings';
import * as multihash from 'multiformats/hashes/digest'
import { CID } from 'multiformats/cid'
import * as sha256 from 'multiformats/hashes/sha2';

import { UnixFS } from 'ipfs-unixfs'
import * as dagPB from '@ipld/dag-pb'

import { createCanvas } from "canvas";
import fs from "fs";

export async function waitForNewBlock() {
    // TODO: wait for a new block.
    console.log('🛰 Waiting for new block...')
    return new Promise(resolve => setTimeout(resolve, 7000))
}

/**
 * Create CID for data.
 * Default to `0x55 (raw)` with blake2b_256 hash.
 *
 * 0xb220:
 * - 0xb2 = the multihash algorithm family for BLAKE2b
 * - 0x20 = the digest length in bytes (32 bytes = 256 bits)
 *
 * See: https://github.com/multiformats/multicodec/blob/master/table.csv
 */
export async function cidFromBytes(bytes, cidCodec = 0x55, mhCode = 0xb220) {
    console.log(`Using cidCodec: ${cidCodec} and mhCode: ${mhCode}`);
    let mh;
    switch (mhCode) {
        case 0xb220: // blake2b-256
            mh = multihash.create(mhCode, blake2AsU8a(bytes));
            break;
        case 0x12:   // sha2-256
            mh = multihash.create(mhCode, sha256AsU8a(bytes));
            // Equivalent to:
            //  import * as sha256 from "multiformats/hashes/sha2";
            //  mh = sha256.sha256.digest(bytes);
            break;
        case 0x1b:   // keccak-256
            mh = multihash.create(mhCode, keccak256AsU8a(bytes));
            break;

        default:
            throw new Error("Unhandled multihash code: " + mhCode)
    }
    console.log("Multihash:", mh);
    return CID.createV1(cidCodec, mh)
}

export function to_hashing_enum(hashing) {
    switch (hashing) {
        case 0xb220: // blake2b-256
            return Enum("Blake2b256");
        case 0x12:   // sha2-256
            return Enum("Sha2_256");
        case 0x1b:   // keccak-256
            return Enum("Keccak256");
            break;
        default:
            throw new Error("Unhandled multihash code: " + mhCode)
    }
}

/**
 * Build a UnixFS DAG-PB file node from raw chunks.
 *
 * @param {Array<{ cid: CID, length: number }>} chunks
 * @returns {Promise<{ rootCid: CID, dagBytes: Uint8Array }>}
 */
export async function buildUnixFSDagPB(chunks) {
    if (!chunks?.length) {
        throw new Error('❌ buildUnixFSDag: chunks[] is empty')
    }

    // UnixFS blockSizes = sizes of child blocks
    const blockSizes = chunks.map(c => c.len)

    console.log(`\n🧩 Building UnixFS DAG from chunks:
  • totalChunks: ${chunks.length}
  • blockSizes: ${blockSizes.join(', ')}`)

    // Build UnixFS file metadata (no inline data here)
    const fileData = new UnixFS({
        type: 'file',
        blockSizes
    })

    // DAG-PB node: our file with chunk links
    const dagNode = dagPB.prepare({
        Data: fileData.marshal(),
        Links: chunks.map(c => ({
            Name: '',
            Tsize: c.len,
            Hash: c.cid
        }))
    })

    // Encode DAG-PB
    const dagBytes = dagPB.encode(dagNode)

    // Hash DAG to produce CIDv1
    const dagHash = await sha256.sha256.digest(dagBytes)
    const rootCid = CID.createV1(dagPB.code, dagHash)

    console.log(`✅ DAG root CID: ${rootCid.toString()}`)

    return { rootCid, dagBytes }
}

export function generateTextImage(file, text, width = 800, height = 600) {
    const canvas = createCanvas(width, height);
    const ctx = canvas.getContext("2d");

    // 🎨 Background
    ctx.fillStyle = randomColor();
    ctx.fillRect(0, 0, width, height);

    // 🟠 Random shapes
    for (let i = 0; i < 15; i++) {
        ctx.beginPath();
        ctx.fillStyle = randomColor();
        ctx.arc(
            Math.random() * width,
            Math.random() * height,
            Math.random() * 120,
            0,
            Math.PI * 2
        );
        ctx.fill();
    }

    // ✍️ Draw your text
    ctx.font = "bold 40px Sans";
    ctx.fillStyle = "white";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";

    // Add text with shadow for readability
    ctx.shadowColor = "black";
    ctx.shadowBlur = 8;

    ctx.fillText(text, width / 2, height / 2);

    let jpegBytes = canvas.toBuffer("image/jpeg");
    fs.writeFileSync(file, jpegBytes);
    console.log("Saved to file:", file);
}

function randomColor() {
    return `rgb(${rand255()}, ${rand255()}, ${rand255()})`;
}

function rand255() {
    return Math.floor(Math.random() * 256);
}

async function test() {
    let bytes = new Uint8Array(Buffer.from("Hello, Bulletin with PAPI - Fri Nov 21 2025 11:09:18 GMT+0000"));
    let cid;

    console.log("\n == blake2b_256 ==");
    // Raw with blake2b_256 hash:
    cid = await cidFromBytes(bytes);
    console.log("Generated CID:", cid.toString(), "\n");
    // Generated CID: bafk2bzacedvk4eijklisgdjijnxky24pmkg7jgk5vsct4mwndj3nmx7plzz7m

    // DAG-PB with blake2b_256 hash:
    cid = await cidFromBytes(bytes, 0x70);
    console.log("Generated CID:", cid.toString(), "\n");
    // Generated CID: bafykbzacedvk4eijklisgdjijnxky24pmkg7jgk5vsct4mwndj3nmx7plzz7m

    console.log("\n == ssha2_256 ==");
    // Raw with ssha2_256 hash:
    cid = await cidFromBytes(bytes, 0x55, 0x12);
    console.log("Generated CID:", cid.toString(), "\n");

    // DAG-PB with ssha2_256 hash:
    cid = await cidFromBytes(bytes, 0x70, 0x12);
    console.log("Generated CID:", cid.toString(), "\n");

    console.log("\n == keccak_256 ==");
    // Raw with keccak_256 hash:
    cid = await cidFromBytes(bytes, 0x55, 0x1b);
    console.log("Generated CID:", cid.toString(), "\n");

    // DAG-PB with ssha2_256 hash:
    cid = await cidFromBytes(bytes, 0x70, 0x1b);
    console.log("Generated CID:", cid.toString(), "\n");

    // Make sure sha equivalent works:
    console.log("\n\n == SHA256 equivalent ==");
    let hash = await sha256.sha256.digest(bytes);
    let cid_sha256 = CID.createV1(0x70, hash);
    cid = await cidFromBytes(bytes, 0x70, 0x12);
    assert.deepStrictEqual(
        cid_sha256,
        cid,
        '❌ SHA CID calculation not compatible!'
    );
    console.log("Matches!");
}

// test().catch(console.error);
