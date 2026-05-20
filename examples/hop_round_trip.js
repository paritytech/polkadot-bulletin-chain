/**
 * Simple HOP round-trip: sender submits, receiver poll claims + acks.
 *
 * Assumes the sender account is already authorized for HOP submission.
 * No on-chain calls here — pure HOP protocol over JSON-RPC.
 *
 * Usage:
 *   node examples/hop_round_trip.js [ws_url] [sender_seed]
 *   node examples/hop_round_trip.js ws://localhost:10000 //Alice
 */

import { HopClient, HopNotFoundError } from 'hop-sdk';
import { sr25519CreateDerive } from '@polkadot-labs/hdkd';
import {
	DEV_PHRASE,
	entropyToMiniSecret,
	mnemonicToEntropy,
} from '@polkadot-labs/hdkd-helpers';

const NODE_WS = process.argv[2] || 'ws://localhost:10000';
const DERIVATION_PATH = process.argv[3] || '//Alice';

const POLL_INTERVAL_MS = 2_000;
const POLL_TIMEOUT_MS = 60_000;

// HOP needs raw byte signing — never use getPolkadotSigner, which wraps the
// payload in <Bytes>…</Bytes> and the node would reject the signature.
function rawSigner(keyPair) {
	return {
		publicKey: keyPair.publicKey,
		signTx: () => Promise.reject(new Error('signTx is not used by HOP')),
		signBytes: async (data) => keyPair.sign(data),
	};
}

/** Poll claim, treating NOT_FOUND as "not yet, retry". */
async function pollAndClaim(client, ticket) {
	const deadline = Date.now() + POLL_TIMEOUT_MS;
	while (Date.now() < deadline) {
		try {
			return await client.claim(ticket);
		} catch (err) {
			if (!(err instanceof HopNotFoundError)) throw err;
			console.log(`  …not in pool yet, retrying in ${POLL_INTERVAL_MS / 1000}s`);
			await new Promise(r => setTimeout(r, POLL_INTERVAL_MS));
		}
	}
	throw new Error(`Timed out after ${POLL_TIMEOUT_MS / 1000}s`);
}

async function main() {
	const miniSecret = entropyToMiniSecret(mnemonicToEntropy(DEV_PHRASE));
	const derive = sr25519CreateDerive(miniSecret);
	const senderKeyPair = derive(DERIVATION_PATH);

	const message = `Hello HOP! ${new Date().toISOString()}`;
	const data = new TextEncoder().encode(message);

	console.log(`Node       : ${NODE_WS}`);
	console.log(`Derivation : ${DERIVATION_PATH}`);
	console.log(`Message    : "${message}"`);

	const senderClient = HopClient.connectWithAccount(NODE_WS, rawSigner(senderKeyPair), 'sr25519');
	const receiverClient = HopClient.connect(NODE_WS);

	try {
		console.log('\n[sender] submit');
		const [ticket] = await senderClient.send(data);
		console.log(`  ticket = ${Buffer.from(ticket.encode()).toString('hex')}`);

		console.log('\n[receiver] poll + claim');
		const received = await pollAndClaim(receiverClient, ticket);
		const decoded = new TextDecoder().decode(received);
		console.log(`  got ${received.length} bytes: "${decoded}"`);

		if (decoded !== message) throw new Error(`mismatch: got "${decoded}"`);

		console.log('\n[receiver] ack');
		try {
			await receiverClient.ack(ticket);
			console.log('  ack ok');
		} catch (err) {
			if (!(err instanceof HopNotFoundError)) throw err;
			console.log('  entry already gone (benign)');
		}

		console.log('\nround-trip ok');
	} finally {
		senderClient.destroy();
		receiverClient.destroy();
	}
}

main().catch((err) => {
	console.error(err);
	process.exit(1);
});
