/**
 * HOP full-flow demo
 *
 * Scenario:
 *   1. Authorize `who` to store data on-chain (via authorizationSigner / sudo).
 *   2. Send data into the HOP off-chain pool as `who`.
 *   3. Wait for the node's maintenance task to promote the blob to
 *      TransactionStorage (pallet_hop_promotion::promote).
 *   4. Verify the on-chain entry via TransactionByContentHash storage query.
 *   5. Renew the entry to reset its retention window (permanent storage).
 *   6. Read the RetentionPeriod and show the expiry block.
 *   7. Wait for the expiry block to pass and verify TransactionByContentHash
 *      is absent — data swept by on_initialize.
 *
 * Promotion timing (zombienet config):
 *   --hop-retention-blocks 10   → HOP pool entry expires after 10 blocks
 *   --hop-check-interval  10   → maintenance task fires every 10 blocks
 *   Expect promotion within ~20 blocks (~120 s at 6 s/block).
 *
 * Usage:
 *   node examples/fullflow-hop.js [ws_url] [auth_seed] [ipfs_api_url]
 *
 *   node examples/fullflow-hop.js ws://localhost:10000 //Alice
 */

import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws';
import { HopClient } from 'hop-sdk';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import {
	DEFAULT_IPFS_GATEWAY_URL,
	setupKeyringAndSigners,
	waitForBlockProduction,
	waitForBlock,
	getContentHash,
	toHex,
} from './common.js';
import {
	authorizeAccount,
	waitForTransaction,
	TX_MODE_FINALIZED_BLOCK,
} from './api.js';
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
const NODE_WS        = args[0] || 'ws://localhost:10000';
const SEED           = args[1] || '//Alice';
const HTTP_IPFS_API  = args[2] || DEFAULT_IPFS_GATEWAY_URL;

// ── Timing constants ──────────────────────────────────────────────────────────
/** Poll TransactionByContentHash every block until the blob appears on-chain. */
const POLL_INTERVAL_MS     = 6_000;   // ~1 block at 6 s/block
/** Give promotion up to 5 min (50 blocks) before giving up. */
const PROMOTION_TIMEOUT_MS = 300_000;
/** Give data-drop verification up to 15 min (150 blocks) beyond retention expiry. */
const DROP_TIMEOUT_MS = 900_000;

// ─────────────────────────────────────────────────────────────────────────────

/**
 * Poll TransactionStorage.TransactionByContentHash until the HOP blob has been
 * promoted to permanent on-chain storage or the timeout elapses.
 *
 * Returns `{ block, index }` — the block number and intra-block extrinsic index
 * where the data was stored, which are needed for a subsequent `renew` call.
 *
 * @param {object} bulletinAPI - PAPI typed API
 * @param {Uint8Array} contentHash - blake2b-256 of the submitted data
 * @param {number} timeoutMs
 * @returns {Promise<{ block: number, index: number }>}
 */
async function pollUntilPromoted(bulletinAPI, contentHash, timeoutMs) {
	const deadline = Date.now() + timeoutMs;
	const hexHash  = toHex(contentHash);

	while (Date.now() < deadline) {
		const entry = await bulletinAPI.query.TransactionStorage.TransactionByContentHash
			.getValue(hexHash);

		if (entry != null) {
			// entry = [blockNumber, extrinsicIndex]
			const [block, index] = entry;
			return { block, index };
		}

		logInfo(`Not yet on-chain (hash ${hexHash.slice(0, 14)}…) — retrying in ${POLL_INTERVAL_MS / 1000}s`);
		await new Promise(r => setTimeout(r, POLL_INTERVAL_MS));
	}

	throw new Error(
		`Timed out after ${timeoutMs / 1000}s waiting for on-chain promotion of ${hexHash}`,
	);
}

// ─────────────────────────────────────────────────────────────────────────────

async function main() {
	await cryptoWaitReady();

	logHeader('HOP FULL FLOW TEST');
	logConnection(NODE_WS, SEED, HTTP_IPFS_API);

	// ── Data to send ───────────────────────────────────────────────────────────
	const message     = `Hello from HOP full flow! (${new Date().toISOString()})`;
	const data        = new TextEncoder().encode(message);
	// blake2b-256(data) — the same hash the pallet stores in TransactionByContentHash
	const contentHash = getContentHash(data);
	logInfo(`Message      : "${message}"`);
	logInfo(`Content hash : ${toHex(contentHash)}`);

	// ── Signers ────────────────────────────────────────────────────────────────
	// authorizationSigner  → PAPI signer for sudo/governance txs (authorize_account)
	// whoSigner            → PAPI signer for on-chain txs (renew)
	// whoAccount           → raw @polkadot/keyring account; .sign() for HOP submit
	const {
		authorizationSigner,
		whoSigner,
		whoAddress,
		whoAccount,
	} = setupKeyringAndSigners(SEED, '//CustomSigner');

	logInfo(`Auth account : ${SEED}`);
	logInfo(`Who account  : ${whoAddress}`);

	// ── PAPI client ────────────────────────────────────────────────────────────
	const papiClient = createClient(getWsProvider(NODE_WS));
	const bulletinAPI = papiClient.getTypedApi(bulletin);

	// ── HOP client ────────────────────────────────────────────────────────────
	// Use whoAccount.sign directly — NOT polkadot-api's signBytes, which wraps
	// messages in <Bytes>…</Bytes> and causes InvalidSignature on the node.
	const hopClient = HopClient.connectWithAccount(
		NODE_WS,
		whoAccount.publicKey,
		(msg) => whoAccount.sign(msg),
		'sr25519',
	);

	let resultCode = 0;

	try {
		await waitForBlockProduction(bulletinAPI);

		// ── Step 1: authorize `who` ────────────────────────────────────────────
		logStep('1️⃣', `Authorizing ${whoAddress} to store data…`);
		await authorizeAccount(
			bulletinAPI,
			authorizationSigner,
			whoAddress,
			/* transactions */ 10,
			/* bytes */ BigInt(10 * 1024 * 1024), // 10 MiB
			TX_MODE_FINALIZED_BLOCK,
		);
		logSuccess('Authorization confirmed on-chain.');

		// ── Step 2: send data via HOP ──────────────────────────────────────────
		logStep('2️⃣', `Sending ${data.length} bytes into the HOP pool…`);
		const [ticket] = await hopClient.send(data);
		logSuccess(`Submitted. Ticket: ${Buffer.from(ticket.encode()).toString('hex')}`);
		logInfo('Node maintenance task will promote this blob after --hop-retention-blocks.');

		// ── Step 3: wait for on-chain promotion ────────────────────────────────
		logStep('3️⃣', 'Waiting for pallet_hop_promotion::promote to land on-chain…');
		const { block: storedBlock, index: storedIndex } = await pollUntilPromoted(
			bulletinAPI,
			contentHash,
			PROMOTION_TIMEOUT_MS,
		);
		logSuccess(`Promoted! TransactionStorage entry at block #${storedBlock}, index ${storedIndex}.`);

		// ── Step 4: verify on-chain entry ──────────────────────────────────────
		logStep('4️⃣', 'Reading on-chain TransactionInfo…');
		const transactions = await bulletinAPI.query.TransactionStorage.Transactions
			.getValue(storedBlock);

		const txInfo = transactions?.[storedIndex];
		if (!txInfo) {
			throw new Error(
				`TransactionInfo missing at block ${storedBlock} index ${storedIndex} — ` +
				`TransactionByContentHash may be stale`,
			);
		}

		logSuccess('On-chain TransactionInfo:');
		logInfo(`  size         : ${txInfo.size} bytes`);
		logInfo(`  content_hash : ${txInfo.content_hash}`);
		logInfo(`  kind         : ${JSON.stringify(txInfo.kind)}`); // Store | Renew

		// Sanity: content hash must match what we submitted.
		// PAPI returns content_hash as a hex string; toHex() produces the same format.
		if (txInfo.content_hash !== toHex(contentHash)) {
			throw new Error(
				`Content hash mismatch! expected ${toHex(contentHash)}, ` +
				`got ${txInfo.content_hash}`,
			);
		}
		logSuccess('Content hash matches — data integrity verified.');

		// ── Step 5: renew the entry (permanent storage) ────────────────────────
		// renew(block, index) resets the retention clock for another RetentionPeriod.
		// The pallet marks the entry as TransactionKind::Renew so the chain-wide
		// PermanentStorageUsed counter tracks it correctly.
		logStep('5️⃣', `Renewing entry at block=${storedBlock} index=${storedIndex}…`);
		const renewTx = bulletinAPI.tx.TransactionStorage.renew({
			block: storedBlock,
			index: storedIndex,
		});
		const renewResult = await waitForTransaction(
			renewTx,
			whoSigner,
			'Renew',
			TX_MODE_FINALIZED_BLOCK,
		);
		logSuccess(`Renewed in block ${renewResult.block?.hash ?? '(unknown)'}.`);

		// ── Step 6: show retention info ────────────────────────────────────────
		logStep('6️⃣', 'Reading RetentionPeriod…');
		const retentionPeriod = await bulletinAPI.query.TransactionStorage.RetentionPeriod
			.getValue();
		const currentBlock = await bulletinAPI.query.System.Number.getValue();
		const expiresAtBlock = Number(currentBlock) + Number(retentionPeriod);

		logInfo(`  RetentionPeriod : ${retentionPeriod} blocks (~${Math.round(Number(retentionPeriod) * 6 / 3600)} h at 6 s/block)`);
		logInfo(`  Current block   : ${currentBlock}`);
		logInfo(`  Expires at      : ~block ${expiresAtBlock}`);
		logInfo('  Data will be swept from chain by on_initialize at that block.');

		// ── Step 7: wait for retention expiry and verify data is dropped ───────
		logStep('7️⃣', `Waiting for block #${expiresAtBlock} (retention expiry)…`);
		await waitForBlock(bulletinAPI, expiresAtBlock, DROP_TIMEOUT_MS, POLL_INTERVAL_MS);

		// TransactionByContentHash must now be absent — on_initialize swept the entry.
		const droppedEntry = await bulletinAPI.query.TransactionStorage.TransactionByContentHash
			.getValue(toHex(contentHash));

		if (droppedEntry != null) {
			throw new Error(
				`Data was NOT dropped after retention expiry — ` +
				`TransactionByContentHash still has an entry at block ${droppedEntry[0]} index ${droppedEntry[1]}`,
			);
		}
		logSuccess('TransactionByContentHash entry is gone — data successfully dropped by on_initialize.');

		logTestResult(true, 'HOP Full Flow Test');
	} catch (err) {
		logError(err.message);
		console.error(err);
		logTestResult(false, 'HOP Full Flow Test');
		resultCode = 1;
	} finally {
		hopClient.destroy();
		papiClient.destroy();
		process.exit(resultCode);
	}
}

await main();
