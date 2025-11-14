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
export function cidFromBytes(bytes) {
    const hash = blake2AsU8a(bytes)
    const mh = multihash.create(0xb220, hash)
    return CID.createV1(0x55, mh) // 0x55 = raw
}