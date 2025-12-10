import { blake2AsU8a } from '@polkadot/util-crypto'
import * as multihash from 'multiformats/hashes/digest'
import { CID } from 'multiformats/cid'
import { Keyring } from '@polkadot/keyring';
import { getPolkadotSigner } from '@polkadot-api/signer';

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
