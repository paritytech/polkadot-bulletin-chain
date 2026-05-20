/**
 * End-to-end HOP demo: authorization, round-trip claim/ack, and promotion.
 *
 * Exercises:
 *   - All four HOP RPC methods via hop-sdk: poolStatus, send, claim, ack
 *   - HopRuntimeApi methods:
 *       can_account_promote   — papi typed runtime call + hop-sdk
 *       max_promotion_size    — hop-sdk (HopClient.maxPromotionSize)
 *       is_promoted_on_chain  — hop-sdk (HopClient.isPromotedOnChain)
 *
 * Actors:
 *   - sudo     signs `authorize_account` on-chain.
 *   - sender   submits HOP blobs (needs raw signBytes).
 *   - receiver claims and acks (anonymous client — no signer needed).
 *
 * Scenario:
 *   1. Pool baseline assertion.
 *   2. Pre-auth: can_account_promote → false; max_promotion_size sanity.
 *   3. Sudo authorizes sender.
 *   4. Post-auth: can_account_promote → true (cross-checked papi vs SDK).
 *   5. Round-trip submit → claim → ack → assert pool empty + not promoted.
 *   6. Promotion submit → wait for on-chain promotion → assert pool empty.
 *
 * Promotion timing (zombienet config):
 *   --hop-retention-secs  60   → HOP pool entry expires after 60s ~ 10 blocks
 *   --hop-check-interval  10   → maintenance task fires every 10 seconds
 *
 * Usage:
 *   node examples/hop_round_trip.js [ws_url] [sudo_derivation_path]
 *   node examples/hop_round_trip.js ws://localhost:10000 //Alice
 */

import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws';
import { getPolkadotSigner } from 'polkadot-api/signer';
import { HopClient, HopNotFoundError } from 'hop-sdk';
import { sr25519CreateDerive } from '@polkadot-labs/hdkd';
import {
	DEV_PHRASE,
	entropyToMiniSecret,
	mnemonicToEntropy,
	ss58Address,
	blake2b256,
} from '@polkadot-labs/hdkd-helpers';
import { waitForBlockProduction, toHex } from './common.js';
import { authorizeAccount, TX_MODE_FINALIZED_BLOCK } from './api.js';
import {
	logHeader,
	logConnection,
	logStep,
	logInfo,
	logSuccess,
	logError,
	logTestResult,
} from './logger.js';
import { bulletin } from './.papi/descriptors/dist/index.js';

// ── CLI args ─────────────────────────────────────────────────────────────────
const args = process.argv.slice(2);
const NODE_WS = args[0] || 'ws://localhost:10000';
const SUDO_PATH = args[1] || '//Alice';
const SENDER_PATH = '//CustomSigner';

// ── Timing constants ─────────────────────────────────────────────────────────
const CLAIM_POLL_INTERVAL_MS = 2_000;
const CLAIM_TIMEOUT_MS = 60_000;
const PROMOTION_POLL_INTERVAL_MS = 6_000;   // ~1 block at 6 s/block
const PROMOTION_TIMEOUT_MS = 300_000;       // ~50 blocks

// Per-recipient metadata overhead the HOP pool charges against capacity,
// matching `METADATA_COST_PER_RECIPIENT` in substrate/client/hop/src/types.rs.
const METADATA_COST_PER_RECIPIENT = 40;

function entryAccountedSize(dataLen, recipientCount) {
	return dataLen + recipientCount * METADATA_COST_PER_RECIPIENT;
}

// HOP requires raw byte signing — never use getPolkadotSigner, which wraps the
// payload in <Bytes>…</Bytes> and the node would reject the signature.
function rawSigner(keyPair) {
	return {
		publicKey: keyPair.publicKey,
		signTx: () => Promise.reject(new Error('signTx is not used by HOP')),
		signBytes: async (data) => keyPair.sign(data),
	};
}

function logPoolStatus(label, status) {
	logInfo(`${label} — entries: ${status.entryCount}  bytes: ${status.totalBytes} / ${status.maxBytes}`);
}

function assertPoolStatus(label, status, expectedEntries, expectedBytes) {
	if (status.entryCount !== expectedEntries) {
		throw new Error(`${label}: expected ${expectedEntries} entries, got ${status.entryCount}`);
	}
	if (BigInt(status.totalBytes) !== BigInt(expectedBytes)) {
		throw new Error(`${label}: expected ${expectedBytes} bytes, got ${status.totalBytes}`);
	}
}

/** Poll claim, treating NOT_FOUND as "not yet, retry". */
async function pollAndClaim(client, ticket) {
	const deadline = Date.now() + CLAIM_TIMEOUT_MS;
	while (Date.now() < deadline) {
		try {
			return await client.claim(ticket);
		} catch (err) {
			if (!(err instanceof HopNotFoundError)) throw err;
			logInfo(`Ticket not in pool yet — retrying in ${CLAIM_POLL_INTERVAL_MS / 1000}s`);
			await new Promise(r => setTimeout(r, CLAIM_POLL_INTERVAL_MS));
		}
	}
	throw new Error(`Timed out after ${CLAIM_TIMEOUT_MS / 1000}s waiting to claim`);
}

async function pollPromotion(hopClient, contentHash) {
	const deadline = Date.now() + PROMOTION_TIMEOUT_MS;
	while (Date.now() < deadline) {
		if (await hopClient.isPromotedOnChain(contentHash)) return;
		logInfo(`Not promoted yet — retrying in ${PROMOTION_POLL_INTERVAL_MS / 1000}s`);
		await new Promise(r => setTimeout(r, PROMOTION_POLL_INTERVAL_MS));
	}
	throw new Error(`Timed out after ${PROMOTION_TIMEOUT_MS / 1000}s waiting for promotion`);
}

async function main() {
	logHeader('HOP END-TO-END TEST');
	logConnection(NODE_WS, SUDO_PATH, '');

	// ── Keypair setup ────────────────────────────────────────────────────────
	const miniSecret = entropyToMiniSecret(mnemonicToEntropy(DEV_PHRASE));
	const derive = sr25519CreateDerive(miniSecret);

	// Sudo signs an on-chain extrinsic — getPolkadotSigner is fine here
	// (only the HOP signBytes path must avoid <Bytes> wrapping).
	const sudoKeyPair = derive(SUDO_PATH);
	const sudoSigner = getPolkadotSigner(sudoKeyPair.publicKey, 'Sr25519', sudoKeyPair.sign);

	const senderKeyPair = derive(SENDER_PATH);
	const senderAddress = ss58Address(senderKeyPair.publicKey);

	// ── Payloads ─────────────────────────────────────────────────────────────
	const roundTripMessage = `Hello round-trip! (${new Date().toISOString()})`;
	const roundTripData = new TextEncoder().encode(roundTripMessage);
	const roundTripHash = blake2b256(roundTripData);

	const promotionMessage = `Hello promotion! (${new Date().toISOString()})`;
	const promotionData = new TextEncoder().encode(promotionMessage);
	const promotionHash = blake2b256(promotionData);

	logInfo(`Sudo account   : ${SUDO_PATH}`);
	logInfo(`Sender account : ${senderAddress}`);
	logInfo(`Round-trip msg : "${roundTripMessage}" (hash ${toHex(roundTripHash)})`);
	logInfo(`Promotion msg  : "${promotionMessage}" (hash ${toHex(promotionHash)})`);

	const papiClient = createClient(getWsProvider(NODE_WS));
	const bulletinAPI = papiClient.getTypedApi(bulletin);
	const senderHop = HopClient.connectWithAccount(NODE_WS, rawSigner(senderKeyPair), 'sr25519');
	// Anonymous client — claim/ack don't need a signer, only the ticket.
	const receiverHop = HopClient.connect(NODE_WS);

	const recipientCount = 1;
	let resultCode = 0;

	try {
		await waitForBlockProduction(bulletinAPI);

		// ── Step 1: pool baseline ────────────────────────────────────────────
		logStep('1️⃣', 'hop_poolStatus (baseline)…');
		const baselineStatus = await senderHop.poolStatus();
		logPoolStatus('baseline', baselineStatus);
		assertPoolStatus('baseline', baselineStatus, 0, 0);

		// ── Step 2: pre-auth runtime checks ──────────────────────────────────
		logStep('2️⃣', 'Pre-auth runtime checks (expect can_account_promote = false)…');
		const preAuthCanRoundTrip = await senderHop.canAccountPromote(senderAddress, roundTripData.length);
		const preAuthCanPromotion = await senderHop.canAccountPromote(senderAddress, promotionData.length);
		if (preAuthCanRoundTrip || preAuthCanPromotion) {
			throw new Error(
				`Pre-auth can_account_promote should be false but got `
				+ `roundTrip=${preAuthCanRoundTrip}, promotion=${preAuthCanPromotion}`,
			);
		}
		const maxSize = await senderHop.maxPromotionSize();
		if (maxSize <= 0) {
			throw new Error(`max_promotion_size returned non-positive value: ${maxSize}`);
		}
		if (roundTripData.length > maxSize || promotionData.length > maxSize) {
			throw new Error(`Payload exceeds max_promotion_size (${maxSize})`);
		}
		const preAuthPromotedRoundTrip = await senderHop.isPromotedOnChain(roundTripHash);
		const preAuthPromotedPromotion = await senderHop.isPromotedOnChain(promotionHash);
		if (preAuthPromotedRoundTrip || preAuthPromotedPromotion) {
			throw new Error('is_promoted_on_chain returned true before any submission');
		}
		logSuccess(`Pre-auth checks ok (max_promotion_size = ${maxSize}).`);

		// ── Step 3: authorize sender ─────────────────────────────────────────
		logStep('3️⃣', `Sudo authorizing ${senderAddress}…`);
		await authorizeAccount(
			bulletinAPI,
			sudoSigner,
			senderAddress,
			10 /* transactions */,
			BigInt(10 * 1024 * 1024) /* bytes */,
			TX_MODE_FINALIZED_BLOCK,
		);
		logSuccess('Authorization confirmed on-chain.');

		// ── Step 4: post-auth runtime checks ─────────────────────────────────
		logStep('4️⃣', 'Post-auth runtime checks (expect can_account_promote = true)…');
		const postAuthCanViaPapi = await bulletinAPI.apis.HopRuntimeApi.can_account_promote(
			senderAddress,
			roundTripData.length,
		);
		const postAuthCanViaSdk = await senderHop.canAccountPromote(senderAddress, roundTripData.length);
		if (!postAuthCanViaPapi || !postAuthCanViaSdk) {
			throw new Error(
				`Post-auth can_account_promote should be true but got `
				+ `papi=${postAuthCanViaPapi}, sdk=${postAuthCanViaSdk}`,
			);
		}
		logSuccess('Runtime confirms sender may promote (papi + SDK agree).');

		// ── Step 5: round-trip submit / claim / ack ──────────────────────────
		logStep('5️⃣', `Round-trip: sender submits ${roundTripData.length} bytes…`);
		const [roundTripTicket] = await senderHop.send(roundTripData, recipientCount);
		logSuccess(`Submitted. Ticket: ${Buffer.from(roundTripTicket.encode()).toString('hex')}`);

		const afterRoundTripSubmit = await senderHop.poolStatus();
		logPoolStatus('after round-trip submit', afterRoundTripSubmit);
		assertPoolStatus(
			'after round-trip submit',
			afterRoundTripSubmit,
			1,
			entryAccountedSize(roundTripData.length, recipientCount),
		);

		logInfo('Receiver (anonymous client) polling claim…');
		const received = await pollAndClaim(receiverHop, roundTripTicket);
		const decoded = new TextDecoder().decode(received);
		if (decoded !== roundTripMessage) {
			throw new Error(`Round-trip mismatch: expected "${roundTripMessage}", got "${decoded}"`);
		}
		logSuccess(`Claimed ${received.length} bytes — content matches.`);

		await receiverHop.ack(roundTripTicket);
		logSuccess('Ack accepted — entry should be consumed.');

		const afterAck = await senderHop.poolStatus();
		logPoolStatus('after ack', afterAck);
		assertPoolStatus('after ack', afterAck, 0, 0);

		if (await senderHop.isPromotedOnChain(roundTripHash)) {
			throw new Error('Acked entry must not be promoted on-chain');
		}
		logSuccess('Round-trip flow complete: claim+ack consumed the entry, no promotion.');

		// ── Step 6: promotion submit + wait ──────────────────────────────────
		logStep('6️⃣', `Promotion: sender submits ${promotionData.length} bytes…`);
		const [promotionTicket] = await senderHop.send(promotionData, recipientCount);
		logSuccess(`Submitted. Ticket: ${Buffer.from(promotionTicket.encode()).toString('hex')}`);

		const afterPromotionSubmit = await senderHop.poolStatus();
		logPoolStatus('after promotion submit', afterPromotionSubmit);
		assertPoolStatus(
			'after promotion submit',
			afterPromotionSubmit,
			1,
			entryAccountedSize(promotionData.length, recipientCount),
		);

		logInfo('Polling is_promoted_on_chain (no ack — let maintenance task promote)…');
		await pollPromotion(senderHop, promotionHash);
		logSuccess('Runtime confirms entry is now on-chain (promoted).');

		// Sanity: the earlier acked entry must still report as not-promoted.
		if (await senderHop.isPromotedOnChain(roundTripHash)) {
			throw new Error('Round-trip (acked) entry unexpectedly reports as promoted');
		}

		// Ack after promotion is expected to return NOT_FOUND.
		try {
			await senderHop.ack(promotionTicket);
			logInfo('Ack accepted after promotion (unusual but benign).');
		} catch (err) {
			if (err instanceof HopNotFoundError) {
				logInfo('Ack returned NOT_FOUND — expected after promotion.');
			} else {
				throw err;
			}
		}

		logTestResult(true, 'HOP End-to-End Test');
	} catch (err) {
		logError(err.message);
		console.error(err);
		logTestResult(false, 'HOP End-to-End Test');
		resultCode = 1;
	} finally {
		senderHop.destroy();
		receiverHop.destroy();
		papiClient.destroy();
		process.exit(resultCode);
	}
}

await main();
