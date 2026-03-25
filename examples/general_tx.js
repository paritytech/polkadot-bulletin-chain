/**
 * General extrinsic construction utilities.
 *
 * With #[pallet::authorize] replacing ValidateUnsigned, unsigned transactions must be
 * submitted as "general" extrinsics (Preamble::General) rather than "bare" extrinsics
 * (Preamble::Bare). General extrinsics include the transaction extension pipeline but
 * no signature, allowing AuthorizeCall to process the call's authorization logic.
 *
 * Extrinsic v5 format:
 *   Bare:    compact_length | 0x05 | call_data
 *   Signed:  compact_length | 0xC5 | address | signature | extension_data | call_data
 *   General: compact_length | 0x45 | 0x00    | extension_data | call_data
 *
 * For unsigned general transactions, all extensions use default/zero values since they
 * skip validation when the origin is None/Authorized.
 */

const EXTRINSIC_FORMAT_VERSION = 5;
const GENERAL_EXTRINSIC = 0b0100_0000;
const BARE_EXTRINSIC = 0b0000_0000;
const EXTENSION_VERSION = 0;

const GENERAL_PREAMBLE = EXTRINSIC_FORMAT_VERSION | GENERAL_EXTRINSIC; // 0x45
const BARE_PREAMBLE = EXTRINSIC_FORMAT_VERSION | BARE_EXTRINSIC;       // 0x05

/**
 * Default extension explicit bytes for unsigned general transactions per runtime.
 *
 * For unsigned transactions, all extensions skip validation when there's no signer,
 * so default zero values are safe. The bytes are the SCALE-encoded concatenation of
 * each extension's explicit type in TxExtension order.
 *
 * Extensions with unit type encode as 0 bytes.
 * Non-unit extensions all happen to encode as 0x00 for their default:
 *   - CheckEra: Era::Immortal = 0x00
 *   - CheckNonce: Compact<u32>(0) = 0x00
 *   - ChargeTransactionPayment: Compact<u128>(0) = 0x00 (tip = 0)
 *   - CheckMetadataHash: Mode::Disabled = 0x00
 */
const RUNTIME_EXTENSION_DEFAULTS = {
    // bulletin-polkadot (solochain): TxExtension = (
    //   AuthorizeCall, CheckNonZeroSender, CheckSpecVersion, CheckTxVersion,
    //   CheckGenesis, CheckEra, CheckNonce, CheckWeight,
    //   ValidateStorageCalls, AllowedSignedCalls, BridgeRejectObsoleteHeadersAndMessages
    // )
    // Non-unit: CheckEra(0x00) + CheckNonce(0x00)
    'bulletin-polkadot': new Uint8Array([0x00, 0x00]),

    // bulletin-westend (parachain): TxExtension = StorageWeightReclaim<Runtime, (
    //   AuthorizeCall, CheckNonZeroSender, CheckSpecVersion, CheckTxVersion,
    //   CheckGenesis, CheckEra, CheckNonce, CheckWeight,
    //   SkipCheckIfFeeless<ChargeTransactionPayment>,
    //   ValidateStorageCalls, CheckMetadataHash
    // )>
    // Non-unit: CheckEra(0x00) + CheckNonce(0x00) + ChargeTransactionPayment(0x00) + CheckMetadataHash(0x00)
    'bulletin-westend': new Uint8Array([0x00, 0x00, 0x00, 0x00]),
};

// ---- SCALE compact encoding/decoding ----

function encodeCompact(value) {
    if (value <= 0x3f) {
        return new Uint8Array([(value << 2)]);
    } else if (value <= 0x3fff) {
        const v = (value << 2) | 0x01;
        return new Uint8Array([v & 0xff, (v >> 8) & 0xff]);
    } else if (value <= 0x3fffffff) {
        const v = (value << 2) | 0x02;
        return new Uint8Array([v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff, (v >> 24) & 0xff]);
    } else {
        throw new Error(`Value ${value} too large for compact encoding`);
    }
}

function decodeCompact(bytes, offset = 0) {
    const mode = bytes[offset] & 0x03;
    if (mode === 0) {
        return { value: bytes[offset] >> 2, bytesRead: 1 };
    } else if (mode === 1) {
        const value = ((bytes[offset] | (bytes[offset + 1] << 8)) >> 2) >>> 0;
        return { value, bytesRead: 2 };
    } else if (mode === 2) {
        const value = ((bytes[offset] | (bytes[offset + 1] << 8) |
            (bytes[offset + 2] << 16) | (bytes[offset + 3] << 24)) >> 2) >>> 0;
        return { value, bytesRead: 4 };
    } else {
        throw new Error('Big integer compact encoding not supported');
    }
}

// ---- Hex conversion ----

function hexToBytes(hex) {
    if (hex.startsWith('0x')) hex = hex.slice(2);
    const bytes = new Uint8Array(hex.length / 2);
    for (let i = 0; i < bytes.length; i++) {
        bytes[i] = parseInt(hex.substr(i * 2, 2), 16);
    }
    return bytes;
}

function bytesToHex(bytes) {
    return '0x' + Array.from(bytes).map(b => b.toString(16).padStart(2, '0')).join('');
}

/**
 * Query the runtime's specName via JSON-RPC.
 *
 * @param {string} wsUrl - WebSocket URL (converted to HTTP for the RPC call)
 * @returns {Promise<string>} The runtime's specName
 */
export async function getRuntimeSpecName(wsUrl) {
    const httpUrl = wsUrl.replace(/^ws(s?):\/\//, 'http$1://');
    const response = await fetch(httpUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
            jsonrpc: '2.0',
            id: 1,
            method: 'state_getRuntimeVersion',
            params: [],
        }),
    });

    if (!response.ok) {
        throw new Error(`Failed to fetch runtime version: HTTP ${response.status}`);
    }

    const json = await response.json();
    if (json.error) {
        throw new Error(`RPC error: ${json.error.message}`);
    }

    return json.result.specName;
}

/**
 * Get the default extension bytes for a given runtime spec name.
 *
 * @param {string} specName - Runtime spec name (e.g., "bulletin-polkadot", "bulletin-westend")
 * @returns {Uint8Array} Default extension bytes for unsigned general transactions
 */
export function getExtensionDefaults(specName) {
    const defaults = RUNTIME_EXTENSION_DEFAULTS[specName];
    if (!defaults) {
        throw new Error(
            `Unknown runtime "${specName}". Known runtimes: ${Object.keys(RUNTIME_EXTENSION_DEFAULTS).join(', ')}. ` +
            `Add extension defaults for this runtime in general_tx.js.`
        );
    }
    return defaults;
}

/**
 * Convert a bare extrinsic (from PAPI's getBareTx) to a general extrinsic.
 *
 * @param {string} bareTxHex - Hex-encoded bare extrinsic from PAPI's tx.getBareTx()
 * @param {Uint8Array} extensionBytes - SCALE-encoded default extension data
 * @returns {string} Hex-encoded general extrinsic ready for submission
 */
export function bareToGeneralTx(bareTxHex, extensionBytes) {
    const bareTxBytes = hexToBytes(bareTxHex);

    // Parse compact length prefix of the bare extrinsic
    const { value: bodyLen, bytesRead } = decodeCompact(bareTxBytes);
    const bodyStart = bytesRead;

    // Verify bare preamble byte
    const preamble = bareTxBytes[bodyStart];
    if (preamble !== BARE_PREAMBLE) {
        throw new Error(
            `Expected bare extrinsic preamble 0x${BARE_PREAMBLE.toString(16)}, ` +
            `got 0x${preamble.toString(16)}`
        );
    }

    // Extract call data (everything after the preamble byte within the body)
    const callBytes = bareTxBytes.slice(bodyStart + 1, bodyStart + bodyLen);

    // Build general extrinsic body: preamble + ext_version + extension_data + call_data
    const body = new Uint8Array(1 + 1 + extensionBytes.length + callBytes.length);
    body[0] = GENERAL_PREAMBLE;
    body[1] = EXTENSION_VERSION;
    body.set(extensionBytes, 2);
    body.set(callBytes, 2 + extensionBytes.length);

    // Prepend compact-encoded body length
    const lengthPrefix = encodeCompact(body.length);
    const generalTx = new Uint8Array(lengthPrefix.length + body.length);
    generalTx.set(lengthPrefix);
    generalTx.set(body, lengthPrefix.length);

    return bytesToHex(generalTx);
}
