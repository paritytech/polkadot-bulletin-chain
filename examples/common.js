import { blake2AsU8a } from '@polkadot/util-crypto'
import * as multihash from 'multiformats/hashes/digest'
import { CID } from 'multiformats/cid'
import { Keyring } from '@polkadot/keyring';
import { getPolkadotSigner } from '@polkadot-api/signer';
import * as dagPB from '@ipld/dag-pb'
import { UnixFS } from 'ipfs-unixfs'
import { createCanvas } from "canvas";
import fs from "fs";
import assert from "assert";

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
    console.log(`[CID]: Using cidCodec: ${cidCodec} and mhCode: ${mhCode}`);
    let mh;
    switch (mhCode) {
        case 0xb220: // blake2b-256
            mh = multihash.create(mhCode, blake2AsU8a(bytes));
            break;

        default:
            throw new Error("Unhandled multihash code: " + mhCode)
    }
    return CID.createV1(cidCodec, mh)
}

export function convertCid(cid, cidCodec) {
    const mh = cid.multihash;
    return CID.createV1(cidCodec, mh);
}

/**
 * Creates a PAPI-compatible signer from a Keyring account
 */
export function createSigner(account) {
    return getPolkadotSigner(
        account.publicKey,
        'Sr25519',
        (input) => account.sign(input)
    );
}

export function setupKeyringAndSigners(sudoSeed, accountSeed) {
    const keyring = new Keyring({ type: 'sr25519' });
    const sudoAccount = keyring.addFromUri(sudoSeed);
    const whoAccount = keyring.addFromUri(accountSeed);
    
    const sudoSigner = createSigner(sudoAccount);
    const whoSigner = createSigner(whoAccount);
    
    return {
        sudoSigner,
        whoSigner,
        whoAddress: whoAccount.address
    };
}

/**
 * Build a UnixFS DAG-PB file node from raw chunks.
 *
 * (By default with SHA2 multihash)
 */
export async function buildUnixFSDagPB(chunks, mhCode = 0x12) {
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
    const rootCid = await cidFromBytes(dagBytes, dagPB.code, mhCode)

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

export function filesAreEqual(path1, path2) {
    const data1 = fs.readFileSync(path1);
    const data2 = fs.readFileSync(path2);
    assert.deepStrictEqual(data1.length, data2.length)

    for (let i = 0; i < data1.length; i++) {
        assert.deepStrictEqual(data1[i], data2[i])
    }
}
