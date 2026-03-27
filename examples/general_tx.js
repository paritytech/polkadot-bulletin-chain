/**
 * General transaction signer for #[pallet::authorize] calls.
 *
 * With #[pallet::authorize] replacing ValidateUnsigned, unsigned transactions must be
 * submitted as "general" extrinsics (Preamble::General) rather than "bare" extrinsics
 * (Preamble::Bare). General extrinsics include the transaction extension pipeline but
 * no signature, allowing AuthorizeCall to process the call's authorization logic.
 *
 * This uses a custom PolkadotSigner that constructs v5 general transactions using the
 * runtime metadata to properly encode extension data, rather than hardcoding defaults.
 *
 * See: https://github.com/polkadot-api/polkadot-api/issues/760
 */

import {
    compact,
    decAnyMetadata,
    extrinsicFormat,
    unifyMetadata,
} from "@polkadot-api/substrate-bindings"
import { mergeUint8 } from "polkadot-api/utils"

const EXTENSION_VERSION = 0;

/**
 * Create a PolkadotSigner that produces v5 general transactions (unsigned with extensions).
 *
 * Usage:
 *   const signer = createGeneralSigner();
 *   await tx.signSubmitAndWatch(signer).subscribe(...);
 *
 * @returns {import("polkadot-api").PolkadotSigner}
 */
export function createGeneralSigner() {
    return {
        publicKey: new Uint8Array(32),
        signBytes() {
            throw new Error("Unsupported: generalSigner does not support signBytes")
        },
        async signTx(callData, signedExtensions, metadata) {
            const decMeta = unifyMetadata(decAnyMetadata(metadata))

            const extra = decMeta.extrinsic.signedExtensions[EXTENSION_VERSION].map(
                ({ identifier }) => {
                    const signedExtension = signedExtensions[identifier]
                    if (!signedExtension)
                        throw new Error(`Missing ${identifier} signed extension`)
                    return signedExtension.value
                },
            )

            const preResult = mergeUint8([
                extrinsicFormat.enc({
                    version: 5,
                    type: "general",
                }),
                new Uint8Array([EXTENSION_VERSION]),
                ...extra,
                callData,
            ])

            return mergeUint8([compact.enc(preResult.length), preResult])
        },
    }
}
