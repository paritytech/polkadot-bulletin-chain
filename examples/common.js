import { blake2AsU8a } from '@polkadot/util-crypto'
import * as multihash from 'multiformats/hashes/digest'
import { CID } from 'multiformats/cid'

export async function waitForNewBlock() {
    // TODO: wait for a new block.
    console.log('🛰 Waiting for new block...')
    return new Promise(resolve => setTimeout(resolve, 7000))
}

/**
 * helper: create CID for raw data
 */
export function cidFromBytes(bytes, cidCodec) {
    const hash = blake2AsU8a(bytes)

    // 0xb2 = the multihash algorithm family for BLAKE2b
    // 0x20 = the digest length in bytes (32 bytes = 256 bits)
    const mh = multihash.create(0xb220, hash)

    // Default to `0x55 (raw)` if cidCodec is not provided.
    const codec = cidCodec != null ? cidCodec : 0x55;

    console.log("Generate CID - using codec:", codec);
    return CID.createV1(codec, mh)
}
