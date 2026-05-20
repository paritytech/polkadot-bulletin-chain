/**
 * HOP promotion flow demo.
 *
 * Exercises:
 *   - All four HOP RPC methods via hop-sdk: poolStatus, send, claim, ack
 *   - HopRuntimeApi methods:
 *       can_account_promote   — papi typed runtime call
 *       max_promotion_size    — hop-sdk (HopClient.maxPromotionSize)
 *       is_promoted_on_chain  — hop-sdk (HopClient.isPromotedOnChain)
 *
 * Scenario:
 *   1. Sudo authorizes `who` to submit HOP blobs.
 *   2. Sanity-check via `can_account_promote(who, data.len)` and
 *      `max_promotion_size()` that the runtime sees the authorization and the
 *      payload fits the promotion limit.
 *   3. Read pool status (baseline).
 *   4. Submit data via HOP.
 *   5. Read pool status (entry present).
 *   6. Claim the data back through HOP and assert byte equality — this is the
 *      authoritative data-integrity check.
 *   7. Poll `is_promoted_on_chain(content_hash)` until true.
 *   8. Read pool status (entry consumed by promotion).
 *   9. Ack the entry. After promotion the node has already deleted it, so the
 *      ack is expected to return NOT_FOUND — handled as benign.
 *
 * Promotion timing (zombienet config):
 *   --hop-retention-secs  12   → HOP pool entry expires after 12s ~ 2 blocks
 *   --hop-check-interval  10   → maintenance task fires every 10 blocks
 *   Expect promotion within ~20 blocks (~120 s at 6 s/block).
 *
 * Usage:
 *   node examples/hop_promotion.js [ws_url] [sudo_derivation_path]
 *   node examples/hop_promotion.js ws://localhost:10000 //Alice
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
const WHO_PATH = '//CustomSigner';

// ── Timing constants ─────────────────────────────────────────────────────────
const PROMOTION_POLL_INTERVAL_MS = 6_000;   // ~1 block at 6 s/block
const PROMOTION_TIMEOUT_MS = 300_000;       // ~50 blocks

// HOP requires raw byte signing — never use getPolkadotSigner, which wraps the
// payload in <Bytes>…</Bytes> and the node would reject the signature.
function rawSigner(keyPair) {
	return {
		publicKey: keyPair.publicKey,
		signTx: () => Promise.reject(new Error('signTx is not used by HOP')),
		signBytes: async (data) => keyPair.sign(data),
	};
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

function logPoolStatus(label, status) {
	logInfo(`${label} — entries: ${status.entryCount}  bytes: ${status.totalBytes} / ${status.maxBytes}`);
}

async function main() {
	logHeader('HOP PROMOTION TEST');
	logConnection(NODE_WS, SUDO_PATH, '');

	// ── Keypair setup ─────────────────────────────────────────────────────────
	const miniSecret = entropyToMiniSecret(mnemonicToEntropy(DEV_PHRASE));
	const derive = sr25519CreateDerive(miniSecret);

	// Sudo signs `authorize_account` on-chain — getPolkadotSigner is fine here
	// (it's only the HOP signBytes path that mustn't wrap in <Bytes>).
	const sudoKeyPair = derive(SUDO_PATH);
	const sudoSigner = getPolkadotSigner(sudoKeyPair.publicKey, 'Sr25519', sudoKeyPair.sign);

	// `who` submits HOP blobs and needs raw signBytes.
	const whoKeyPair = derive(WHO_PATH);
	const whoAddress = ss58Address(whoKeyPair.publicKey);

	// ── Data to send ──────────────────────────────────────────────────────────
	const message = `Hello from HOP promotion! (${new Date().toISOString()})`;
	const data = new TextEncoder().encode(message);
	const contentHash = blake2b256(data); // matches the hash the runtime indexes
	logInfo(`Message      : "${message}"`);
	logInfo(`Content hash : ${toHex(contentHash)}`);

	logInfo(`Sudo account : ${SUDO_PATH}`);
	logInfo(`Who account  : ${whoAddress}`);

	const papiClient = createClient(getWsProvider(NODE_WS));
	const bulletinAPI = papiClient.getTypedApi(bulletin);
	const hopClient = HopClient.connectWithAccount(NODE_WS, rawSigner(whoKeyPair), 'sr25519');

	let resultCode = 0;

	try {
		await waitForBlockProduction(bulletinAPI);

		// ── Step 1: authorize `who` ───────────────────────────────────────────
		logStep('1️⃣', `Authorizing ${whoAddress} to submit…`);
		await authorizeAccount(
			bulletinAPI,
			sudoSigner,
			whoAddress,
			/* transactions */ 10,
			/* bytes */ BigInt(10 * 1024 * 1024),
			TX_MODE_FINALIZED_BLOCK,
		);
		logSuccess('Authorization confirmed on-chain.');

		// ── Step 2: runtime sanity check ──────────────────────────────────────
		logStep('2️⃣', 'Runtime API: can_account_promote + max_promotion_size…');
		/* another option using bulletinApi
		 * `const canPromote = bulletinAPI.apis.HopRuntimeApi.can_account_promote(whoAddress, data.length);`
		 */
		const canPromote = await hopClient.canAccountPromote(whoAddress, data.length);
		if (!canPromote) {
			throw new Error('can_account_promote returned false — authorization did not take effect');
		}
		const maxSize = await hopClient.maxPromotionSize();
		if (data.length > maxSize) {
			throw new Error(`Payload ${data.length} bytes exceeds max_promotion_size ${maxSize}`);
		}
		logSuccess(`Runtime confirms \`who\` may promote this blob (max_promotion_size = ${maxSize}).`);

		// ── Step 3: pool status (baseline) ────────────────────────────────────
		logStep('3️⃣', 'hop_poolStatus (baseline)…');
		logPoolStatus('baseline', await hopClient.poolStatus());

		// ── Step 4: submit ────────────────────────────────────────────────────
		logStep('4️⃣', `hop_submit (${data.length} bytes)…`);
		const [ticket] = await hopClient.send(data);
		logSuccess(`Submitted. Ticket: ${Buffer.from(ticket.encode()).toString('hex')}`);

		// ── Step 5: pool status (entry present) ───────────────────────────────
		logStep('5️⃣', 'hop_poolStatus (after submit)…');
		logPoolStatus('after submit', await hopClient.poolStatus());

		// ── Step 6: claim and verify data integrity ───────────────────────────
		logStep('6️⃣', 'hop_claim (data round-trip)…');
		const received = await hopClient.claim(ticket);
		const decoded = new TextDecoder().decode(received);
		if (decoded !== message) {
			throw new Error(`Data mismatch: expected "${message}", got "${decoded}"`);
		}
		logSuccess(`Claimed ${received.length} bytes — content matches.`);

		// ── Step 7: wait for promotion via runtime API ────────────────────────
		logStep('7️⃣', 'Runtime API: polling is_promoted_on_chain…');
		await pollPromotion(hopClient, contentHash);
		logSuccess('Runtime confirms entry is now on-chain (promoted).');

		// ── Step 8: pool status (entry consumed) ──────────────────────────────
		logStep('8️⃣', 'hop_poolStatus (after promotion)…');
		logPoolStatus('after promotion', await hopClient.poolStatus());

		// ── Step 9: ack (idempotent best-effort cleanup) ──────────────────────
		// Promotion deletes the pool entry, so ack is expected to NOT_FOUND.
		logStep('9️⃣', 'hop_ack (best-effort cleanup)…');
		try {
			await hopClient.ack(ticket);
			logSuccess('Ack accepted.');
		} catch (err) {
			if (err instanceof HopNotFoundError) {
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
		hopClient.destroy();
		papiClient.destroy();
		process.exit(resultCode);
	}
}

await main();
