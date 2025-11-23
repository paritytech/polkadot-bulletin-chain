import {blake2AsU8a, keccak256AsU8a, sha256AsU8a} from '@polkadot/util-crypto'
import * as multihash from 'multiformats/hashes/digest'
import { CID } from 'multiformats/cid'
import * as sha256 from "multiformats/hashes/sha2";
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

test().catch(console.error);
