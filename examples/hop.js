/**
 * Minimal HOP (Hand-Off Protocol) client for the Bulletin Chain.
 *
 * HOP is an off-chain data pool exposed by the node over JSON-RPC. A sender
 * pushes data + a proof of authorization; the node holds it for a retention
 * window, then either:
 *   - hands it to recipients via hop_claim / hop_ack, or
 *   - promotes it to permanent on-chain storage (pallet_hop_promotion).
 *
 * This file implements all four HOP RPC methods minimally:
 *   hop_submit(data_hex, recipients_hex[], signature_hex, signer_hex, timestamp_ms)
 *   hop_claim(data_hash_hex, signature_hex) -> data_hex
 *   hop_ack(data_hash_hex, signature_hex)
 *   hop_poolStatus() -> { entryCount, totalBytes, maxBytes }
 *
 * Only sr25519 is supported. To extend, switch the SR25519 variant byte to
 * 0x00 (ed25519), 0x02 (ecdsa), or 0x03 (eth) and use the matching signer.
 */

import WebSocket from 'ws';
import {
	blake2AsU8a,
	randomAsU8a,
	sr25519PairFromSeed,
	sr25519Sign,
} from '@polkadot/util-crypto';

// Domain-separation tags matching `pallet-hop-promotion` on the Rust side.
const HOP_SUBMIT_CONTEXT = new TextEncoder().encode('hop-submit-v1:');
const HOP_CLAIM_CONTEXT = new TextEncoder().encode('hop-claim-v1:');
const HOP_ACK_CONTEXT = new TextEncoder().encode('hop-ack-v1:');

// SCALE variant byte for MultiSigner / MultiSignature.
const SR25519 = 0x01;

const TICKET_LEN = 65; // scheme (1) + data_hash (32) + ephemeral_seed (32)
const MAX_DATA_SIZE = 64 * 1024 * 1024; // 64 MiB

// JSON-RPC error codes returned by the node's HOP endpoint.
export const HOP_ERROR_DATA_TOO_LARGE = 1001;
export const HOP_ERROR_POOL_FULL = 1002;
export const HOP_ERROR_NOT_FOUND = 1003;
export const HOP_ERROR_INVALID_TICKET = 1004;
export const HOP_ERROR_QUOTA_EXCEEDED = 1005;

function toHex(bytes) {
	return '0x' + Buffer.from(bytes).toString('hex');
}

function fromHex(hex) {
	return Uint8Array.from(Buffer.from(hex.replace(/^0x/, ''), 'hex'));
}

function encodeMultiSr25519(bytes) {
	const out = new Uint8Array(1 + bytes.length);
	out[0] = SR25519;
	out.set(bytes, 1);
	return out;
}

/**
 * Build the 32-byte payload the sender signs for a hop_submit call.
 *
 *   payload = blake2_256( "hop-submit-v1:" ‖ blake2_256(data) ‖ submit_timestamp_le64 )
 */
function submitSigningPayload(dataHash, submitTimestamp) {
	const ctx = HOP_SUBMIT_CONTEXT;
	const buf = new Uint8Array(ctx.length + 32 + 8);
	buf.set(ctx, 0);
	buf.set(dataHash, ctx.length);
	new DataView(buf.buffer).setBigUint64(ctx.length + 32, BigInt(submitTimestamp), true);
	return blake2AsU8a(buf);
}

/**
 * Build the 32-byte payload a recipient signs for claim / ack.
 *
 *   payload = blake2_256( context ‖ data_hash )
 */
function contextPayload(context, dataHash) {
	const buf = new Uint8Array(context.length + 32);
	buf.set(context, 0);
	buf.set(dataHash, context.length);
	return blake2AsU8a(buf);
}

/** Parse a 65-byte ticket into its components. Only sr25519 is supported. */
function parseTicket(ticket) {
	if (ticket.length !== TICKET_LEN) {
		throw new Error(`Ticket must be ${TICKET_LEN} bytes, got ${ticket.length}`);
	}
	if (ticket[0] !== SR25519) {
		throw new Error(`Only sr25519 tickets supported (scheme byte 0x${ticket[0].toString(16)})`);
	}
	return {
		dataHash: ticket.slice(1, 33),
		seed: ticket.slice(33, 65),
	};
}

/** Sign `payload` with the sr25519 keypair derived from `seed`. */
function signWithSeed(seed, payload) {
	const pair = sr25519PairFromSeed(seed);
	return sr25519Sign(payload, pair);
}

/** One-shot JSON-RPC call over a fresh WebSocket. */
function rpcCall(wsUrl, method, params) {
	return new Promise((resolve, reject) => {
		const ws = new WebSocket(wsUrl);
		const id = 1;

		ws.on('open', () => {
			ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }));
		});

		ws.on('message', (raw) => {
			let resp;
			try {
				resp = JSON.parse(raw.toString());
			} catch {
				return;
			}
			if (resp.id !== id) return;
			ws.close();
			if (resp.error) {
				const err = new Error(`${method} failed: [${resp.error.code}] ${resp.error.message}`);
				err.code = resp.error.code;
				reject(err);
			} else {
				resolve(resp.result);
			}
		});

		ws.on('error', (err) => {
			reject(new Error(`WebSocket error on ${wsUrl}: ${err.message}`));
		});
	});
}

/**
 * Submit `data` into the HOP off-chain pool as `senderPublicKey`.
 *
 * The sender must already be authorized for HOP submission (see
 * pallet-hop-promotion::authorize_account). The `signRaw` callback must sign
 * the raw 32-byte payload — do NOT wrap it in <Bytes>…</Bytes>, the node will
 * reject the signature.
 *
 * Returns the 65-byte claim ticket:
 *   scheme (1) ‖ blake2_256(data) (32) ‖ ephemeral_seed (32)
 * Hand the ticket to the recipient out-of-band; they pass it to hopClaim /
 * hopAck. In the promotion flow we don't claim — the node lifts the entry
 * into TransactionStorage after the retention window.
 *
 * @param {string} wsUrl
 * @param {Uint8Array} data - 1 byte – 64 MiB
 * @param {Uint8Array} senderPublicKey - 32-byte sr25519 public key
 * @param {(msg: Uint8Array) => Uint8Array | Promise<Uint8Array>} signRaw
 * @returns {Promise<Uint8Array>} the 65-byte ticket
 */
export async function hopSubmit(wsUrl, data, senderPublicKey, signRaw) {
	if (data.length === 0 || data.length > MAX_DATA_SIZE) {
		throw new Error(`HOP data must be 1 byte – ${MAX_DATA_SIZE} bytes (got ${data.length})`);
	}

	// One ephemeral sr25519 keypair per recipient. The seed becomes part of the
	// ticket; the pubkey is the recipient identity submitted to the node.
	const ephemeralSeed = randomAsU8a(32);
	const ephemeralPair = sr25519PairFromSeed(ephemeralSeed);
	const recipient = encodeMultiSr25519(ephemeralPair.publicKey);

	// Bind (data hash, timestamp) to the sender's authorized account.
	const dataHash = blake2AsU8a(data);
	const submitTimestamp = Date.now();
	const payload = submitSigningPayload(dataHash, submitTimestamp);
	const rawSig = await signRaw(payload);

	await rpcCall(wsUrl, 'hop_submit', [
		toHex(data),
		[toHex(recipient)],
		toHex(encodeMultiSr25519(rawSig)),
		toHex(encodeMultiSr25519(senderPublicKey)),
		submitTimestamp,
	]);

	const ticket = new Uint8Array(TICKET_LEN);
	ticket[0] = SR25519;
	ticket.set(dataHash, 1);
	ticket.set(ephemeralSeed, 33);
	return ticket;
}

/**
 * Download data from the pool using a claim ticket. Does NOT mark the entry
 * as acknowledged — call `hopAck` after processing.
 *
 * @param {string} wsUrl
 * @param {Uint8Array} ticket - 65-byte ticket from `hopSubmit`
 * @returns {Promise<Uint8Array>} the original data
 */
export async function hopClaim(wsUrl, ticket) {
	const { dataHash, seed } = parseTicket(ticket);
	const payload = contextPayload(HOP_CLAIM_CONTEXT, dataHash);
	const rawSig = signWithSeed(seed, payload);

	const dataHex = await rpcCall(wsUrl, 'hop_claim', [
		toHex(dataHash),
		toHex(encodeMultiSr25519(rawSig)),
	]);
	return fromHex(dataHex);
}

/**
 * Acknowledge receipt of claimed data. Once all recipients ack, the node
 * deletes the blob. Idempotent — acking twice is safe.
 *
 * @param {string} wsUrl
 * @param {Uint8Array} ticket
 */
export async function hopAck(wsUrl, ticket) {
	const { dataHash, seed } = parseTicket(ticket);
	const payload = contextPayload(HOP_ACK_CONTEXT, dataHash);
	const rawSig = signWithSeed(seed, payload);

	await rpcCall(wsUrl, 'hop_ack', [
		toHex(dataHash),
		toHex(encodeMultiSr25519(rawSig)),
	]);
}

/**
 * Return current pool statistics: { entryCount, totalBytes, maxBytes }.
 *
 * @param {string} wsUrl
 */
export async function hopPoolStatus(wsUrl) {
	return rpcCall(wsUrl, 'hop_poolStatus', []);
}

// ── HopRuntimeApi calls (via state_call) ─────────────────────────────────────
// These hit the Substrate `state_call` RPC, which executes a runtime API method
// against the current block. The result is the SCALE-encoded return value;
// for `bool` returns this is a single byte (0x00 = false, 0x01 = true).

function u32LeHex(n) {
	const buf = new Uint8Array(4);
	new DataView(buf.buffer).setUint32(0, n, true);
	return Buffer.from(buf).toString('hex');
}

/**
 * Runtime API: `HopRuntimeApi::can_account_promote(who, data_len) -> bool`.
 *
 * Returns true iff `accountPublicKey` is currently authorized to submit a HOP
 * blob of `dataLen` bytes for promotion.
 *
 * @param {string} wsUrl
 * @param {Uint8Array} accountPublicKey - 32-byte AccountId32 (sr25519 pubkey)
 * @param {number} dataLen
 * @returns {Promise<boolean>}
 */
export async function canAccountPromote(wsUrl, accountPublicKey, dataLen) {
	if (accountPublicKey.length !== 32) {
		throw new Error(`AccountId must be 32 bytes, got ${accountPublicKey.length}`);
	}
	// SCALE-encoded args: AccountId32 (32 raw bytes) ‖ u32 LE (4 bytes)
	const argsHex = '0x' + Buffer.from(accountPublicKey).toString('hex') + u32LeHex(dataLen);
	const resultHex = await rpcCall(wsUrl, 'state_call', [
		'HopRuntimeApi_can_account_promote',
		argsHex,
	]);
	return resultHex === '0x01';
}

/**
 * Runtime API: `HopRuntimeApi::is_promoted_on_chain(hash) -> bool`.
 *
 * Returns true once the HOP entry with `contentHash` has been lifted into
 * on-chain `TransactionStorage` by the promotion task — preferred over polling
 * `TransactionStorage.TransactionByContentHash`.
 *
 * @param {string} wsUrl
 * @param {Uint8Array} contentHash - 32-byte blake2b-256(data)
 * @returns {Promise<boolean>}
 */
export async function isPromotedOnChain(wsUrl, contentHash) {
	if (contentHash.length !== 32) {
		throw new Error(`contentHash must be 32 bytes, got ${contentHash.length}`);
	}
	const argsHex = '0x' + Buffer.from(contentHash).toString('hex');
	const resultHex = await rpcCall(wsUrl, 'state_call', [
		'HopRuntimeApi_is_promoted_on_chain',
		argsHex,
	]);
	return resultHex === '0x01';
}
