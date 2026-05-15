/**
 * Simple HOP round-trip: sender submits, receiver polls + claims + acks.
 *
 * Assumes the sender account is already authorized for HOP submission.
 * No on-chain calls here — pure HOP protocol over JSON-RPC.
 *
 * Usage:
 *   node examples/hop_round_trip.js [ws_url] [sender_seed]
 *   node examples/hop_round_trip.js ws://localhost:10000 //Alice
 */

import { Keyring } from '@polkadot/keyring';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import {
	hopSubmit,
	hopClaim,
	hopAck,
	HOP_ERROR_NOT_FOUND,
} from './hop.js';

const NODE_WS = process.argv[2] || 'ws://localhost:10000';
const SENDER_SEED = process.argv[3] || '//Alice';

const POLL_INTERVAL_MS = 2_000;
const POLL_TIMEOUT_MS = 60_000;

/** Poll hop_claim, treating NOT_FOUND as "not yet, retry". */
async function pollAndClaim(wsUrl, ticket) {
	const deadline = Date.now() + POLL_TIMEOUT_MS;
	while (Date.now() < deadline) {
		try {
			return await hopClaim(wsUrl, ticket);
		} catch (err) {
			if (err.code !== HOP_ERROR_NOT_FOUND) throw err;
			console.log(`  …not in pool yet, retrying in ${POLL_INTERVAL_MS / 1000}s`);
			await new Promise(r => setTimeout(r, POLL_INTERVAL_MS));
		}
	}
	throw new Error(`Timed out after ${POLL_TIMEOUT_MS / 1000}s`);
}

async function main() {
	await cryptoWaitReady();

	const keyring = new Keyring({ type: 'sr25519' });
	const sender = keyring.addFromUri(SENDER_SEED);

	const message = `Hello HOP! ${new Date().toISOString()}`;
	const data = new TextEncoder().encode(message);

	console.log(`Node    : ${NODE_WS}`);
	console.log(`Sender  : ${sender.address}`);
	console.log(`Message : "${message}"`);

	// 1. Sender submits — sender.sign() returns raw sr25519 bytes (no <Bytes> wrap).
	console.log('\n[sender] submit');
	const ticket = await hopSubmit(
		NODE_WS,
		data,
		sender.publicKey,
		(msg) => sender.sign(msg),
	);
	console.log(`  ticket = ${Buffer.from(ticket).toString('hex')}`);

	// 2. Receiver polls hop_claim until the entry appears.
	console.log('\n[receiver] poll + claim');
	const received = await pollAndClaim(NODE_WS, ticket);
	const decoded = new TextDecoder().decode(received);
	console.log(`  got ${received.length} bytes: "${decoded}"`);

	if (decoded !== message) throw new Error(`mismatch: got "${decoded}"`);

	// 3. Receiver acks — node deletes the entry once every recipient acks.
	console.log('\n[receiver] ack');
	try {
		await hopAck(NODE_WS, ticket);
		console.log('  ack ok');
	} catch (err) {
		if (err.code !== HOP_ERROR_NOT_FOUND) throw err;
		console.log('  entry already gone (benign)');
	}

	console.log('\nround-trip ok');
}

main().catch((err) => {
	console.error(err);
	process.exit(1);
});
