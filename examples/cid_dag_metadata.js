import { blake2AsU8a } from '@polkadot/util-crypto';
import * as multihash from 'multiformats/hashes/digest';
import { CID } from 'multiformats/cid';
import * as dagPB from '@ipld/dag-pb';
import { UnixFS } from 'ipfs-unixfs';

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

    console.log(`🧩 Building UnixFS DAG from chunks:
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
