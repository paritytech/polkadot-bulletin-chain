import { blake2AsU8a } from '@polkadot/util-crypto'
import * as multihash from 'multiformats/hashes/digest'
import { CID } from 'multiformats/cid'
import { Keyring } from '@polkadot/keyring';
import { getPolkadotSigner } from '@polkadot-api/signer';

// Authorization constants
export const AUTH_TRANSACTIONS = 32;
export const AUTH_BYTES = 64n * 1024n * 1024n; // 64 MB
export const ALICE_ADDRESS = "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY";

export async function waitForNewBlock() {
    // TODO: wait for a new block.
    console.log('🛰 Waiting for new block...')
    return new Promise(resolve => setTimeout(resolve, 7000))
}

export function cidFromBytes(bytes) {
    const hash = blake2AsU8a(bytes)
    // 0xb2 = the multihash algorithm family for BLAKE2b
    // 0x20 = the digest length in bytes (32 bytes = 256 bits)
    const mh = multihash.create(0xb220, hash)
    return CID.createV1(0x55, mh) // 0x55 = raw
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

export function setupKeyringAndSigners() {
    const keyring = new Keyring({ type: 'sr25519' });
    const sudoAccount = keyring.addFromUri('//Alice');
    const whoAccount = keyring.addFromUri('//Alice');
    
    const sudoSigner = createSigner(sudoAccount);
    const whoSigner = createSigner(whoAccount);
    
    return {
        sudoSigner,
        whoSigner,
        whoAddress: whoAccount.address
    };
}
