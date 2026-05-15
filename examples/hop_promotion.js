/**
 * HOP promotion flow demo.
 *
 * Exercises:
 *   - All four HOP RPC methods: hop_poolStatus, hop_submit, hop_claim, hop_ack
 *   - Both HopRuntimeApi methods: can_account_promote, is_promoted_on_chain
 *
 * Scenario:
 *   1. Sudo authorizes `who` to submit HOP blobs.
 *   2. Sanity-check via `can_account_promote(who, data.len)` that the runtime
 *      sees the authorization.
 *   3. Read pool status (baseline).
 *   4. Submit data via HOP.
 *   5. Read pool status (entry present).
 *   6. Claim the data back through HOP and assert byte equality — this is the
 *      authoritative data-integrity check; we no longer need to read
 *      `TransactionStorage.Transactions` to verify content.
 *   7. Poll `is_promoted_on_chain(content_hash)` until true — replaces polling
 *      `TransactionStorage.TransactionByContentHash`.
 *   8. Read pool status (entry consumed by promotion).
 *   9. Ack the entry. After promotion the node has already deleted it, so the
 *      ack is expected to return NOT_FOUND — handled as benign.
 *
 * Promotion timing (zombienet config):
 *   --hop-retention-blocks 10   → HOP pool entry expires after 10 blocks
 *   --hop-check-interval  10   → maintenance task fires every 10 blocks
 *   Expect promotion within ~20 blocks (~120 s at 6 s/block).
 *
 * Usage:
 *   node examples/hop_promotion.js [ws_url] [auth_seed]
 *   node examples/hop_promotion.js ws://localhost:10000 //Alice
 */

import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import {
	hopSubmit,
	hopClaim,
	hopAck,
	hopPoolStatus,
	canAccountPromote,
	isPromotedOnChain,
	HOP_ERROR_NOT_FOUND,
} from './hop.js';
import {
	setupKeyringAndSigners,
	waitForBlockProduction,
	getContentHash,
	toHex,
} from './common.js';
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
const SEED = args[1] || '//Alice';

// ── Timing constants ─────────────────────────────────────────────────────────
const PROMOTION_POLL_INTERVAL_MS = 6_000;   // ~1 block at 6 s/block
const PROMOTION_TIMEOUT_MS = 300_000;       // ~50 blocks

/**
 * Poll `HopRuntimeApi::is_promoted_on_chain` until it flips to true.
 * Replaces the old TransactionByContentHash polling — one runtime API call
 * answers the question authoritatively per block.
 */
async function pollPromotion(wsUrl, contentHash) {
	const deadline = Date.now() + PROMOTION_TIMEOUT_MS;
	while (Date.now() < deadline) {
		if (await isPromotedOnChain(wsUrl, contentHash)) return;
		logInfo(`Not promoted yet — retrying in ${PROMOTION_POLL_INTERVAL_MS / 1000}s`);
		await new Promise(r => setTimeout(r, PROMOTION_POLL_INTERVAL_MS));
	}
	throw new Error(`Timed out after ${PROMOTION_TIMEOUT_MS / 1000}s waiting for promotion`);
}

function logPoolStatus(label, status) {
	logInfo(`${label} — entries: ${status.entryCount}  bytes: ${status.totalBytes} / ${status.maxBytes}`);
}

async function main() {
	await cryptoWaitReady();

	logHeader('HOP PROMOTION TEST');
	logConnection(NODE_WS, SEED, '');

	// ── Data to send ──────────────────────────────────────────────────────────
	const message = `Hello from HOP promotion! (${new Date().toISOString()})`;
	const data = new TextEncoder().encode(message);
	const contentHash = getContentHash(data); // blake2b-256(data) — matches the hash the runtime indexes
	logInfo(`Message      : "${message}"`);
	logInfo(`Content hash : ${toHex(contentHash)}`);

	// authorizationSigner — sudo signer, used for authorize_account.
	// whoAccount         — derived account that submits HOP blobs.
	const {
		authorizationSigner,
		whoAddress,
		whoAccount,
	} = setupKeyringAndSigners(SEED, '//CustomSigner');

	logInfo(`Auth account : ${SEED}`);
	logInfo(`Who account  : ${whoAddress}`);

	const papiClient = createClient(getWsProvider(NODE_WS));
	const bulletinAPI = papiClient.getTypedApi(bulletin);

	let resultCode = 0;

	try {
		await waitForBlockProduction(bulletinAPI);

		// ── Step 1: authorize `who` ───────────────────────────────────────────
		logStep('1️⃣', `Authorizing ${whoAddress} to submit…`);
		await authorizeAccount(
			bulletinAPI,
			authorizationSigner,
			whoAddress,
			/* transactions */ 10,
			/* bytes */ BigInt(10 * 1024 * 1024),
			TX_MODE_FINALIZED_BLOCK,
		);
		logSuccess('Authorization confirmed on-chain.');

		// ── Step 2: runtime sanity check ──────────────────────────────────────
		logStep('2️⃣', 'Runtime API: can_account_promote(who, data.len)…');
		const canPromote = await canAccountPromote(NODE_WS, whoAccount.publicKey, data.length);
		if (!canPromote) {
			throw new Error('can_account_promote returned false — authorization did not take effect');
		}
		logSuccess('Runtime confirms `who` may promote this blob.');

		// ── Step 3: pool status (baseline) ────────────────────────────────────
		logStep('3️⃣', 'hop_poolStatus (baseline)…');
		logPoolStatus('baseline', await hopPoolStatus(NODE_WS));

		// ── Step 4: submit ────────────────────────────────────────────────────
		// account.sign() signs raw bytes — do NOT use polkadot-api's signBytes,
		// which wraps the message in <Bytes>…</Bytes> and would fail verification.
		logStep('4️⃣', `hop_submit (${data.length} bytes)…`);
		const ticket = await hopSubmit(
			NODE_WS,
			data,
			whoAccount.publicKey,
			(msg) => whoAccount.sign(msg),
		);
		logSuccess(`Submitted. Ticket: ${Buffer.from(ticket).toString('hex')}`);

		// ── Step 5: pool status (entry present) ───────────────────────────────
		logStep('5️⃣', 'hop_poolStatus (after submit)…');
		logPoolStatus('after submit', await hopPoolStatus(NODE_WS));

		// ── Step 6: claim and verify data integrity ───────────────────────────
		// This replaces reading TransactionStorage.Transactions and comparing
		// content_hash — round-tripping the bytes is a stronger check.
		logStep('6️⃣', 'hop_claim (data round-trip)…');
		const received = await hopClaim(NODE_WS, ticket);
		const decoded = new TextDecoder().decode(received);
		if (decoded !== message) {
			throw new Error(`Data mismatch: expected "${message}", got "${decoded}"`);
		}
		logSuccess(`Claimed ${received.length} bytes — content matches.`);

		// ── Step 7: wait for promotion via runtime API ────────────────────────
		logStep('7️⃣', 'Runtime API: polling is_promoted_on_chain…');
		await pollPromotion(NODE_WS, contentHash);
		logSuccess('Runtime confirms entry is now on-chain (promoted).');

		// ── Step 8: pool status (entry consumed) ──────────────────────────────
		logStep('8️⃣', 'hop_poolStatus (after promotion)…');
		logPoolStatus('after promotion', await hopPoolStatus(NODE_WS));

		// ── Step 9: ack (idempotent best-effort cleanup) ──────────────────────
		// Promotion deletes the pool entry, so ack is expected to NOT_FOUND.
		// We still exercise the RPC to show the safe idempotent pattern.
		logStep('9️⃣', 'hop_ack (best-effort cleanup)…');
		try {
			await hopAck(NODE_WS, ticket);
			logSuccess('Ack accepted.');
		} catch (err) {
			if (err.code === HOP_ERROR_NOT_FOUND) {
				logInfo('Ack returned NOT_FOUND — expected after promotion. Treating as success.');
			} else {
				throw err;
			}
		}

		logTestResult(true, 'HOP Promotion Test');
	} catch (err) {
		logError(err.message);
		console.error(err);
		logTestResult(false, 'HOP Promotion Test');
		resultCode = 1;
	} finally {
		papiClient.destroy();
		process.exit(resultCode);
	}
}

await main();
