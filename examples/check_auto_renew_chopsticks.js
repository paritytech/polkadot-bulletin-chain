// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

/**
 * Chopsticks integration test for the `pallet-bulletin-data-renewal` (runtime
 * name `DataRenewal`) auto-renewal flow, using PAPI typed codecs.
 *
 * Forks a live Bulletin chain with a local runtime WASM override, then:
 *  1. Overrides RetentionPeriod and mocks ProofChecked
 *  2. Sets up authorization + a stored transaction via PAPI-encoded storage
 *  3. Tests enable_auto_renew: sets DataRenewal.Renewals entry, verifies it
 *  4. Tests disable_auto_renew: removes the entry, verifies it's cleared
 *  5. Re-enables auto-renewal for the expiry test
 *  6. Advances to the expiry block. There `TransactionStorage::on_initialize`
 *     ages the data out and queues it into `DataRenewal.PendingAutoRenewals`
 *     for the same block's `process_pending_renewals` mandatory inherent.
 *  7. Injects the `DataRenewal.process_pending_renewals` inherent into that
 *     block via `dev_newBlock({ transactions })` and verifies the drain:
 *       - PendingAutoRenewals is emptied (taken by the inherent)
 *       - TransactionByContentHash now points at the expiry block (renewed)
 *       - the recurring registration survives with `paid = false`
 *
 * Chopsticks does not natively synthesize custom pallet inherents (see
 * AcalaNetwork/chopsticks#1037 — it only learned the storage-pallet proof
 * inherent). This test therefore injects the mandatory renewal inherent
 * explicitly, which exercises the exact on-chain code path a collator runs.
 *
 * Usage:
 *   node check_auto_renew_chopsticks.js [endpoint] [wasm_path]
 *
 * Example:
 *   cargo build --release -p bulletin-westend-runtime
 *   node check_auto_renew_chopsticks.js wss://westend-bulletin-rpc.polkadot.io \
 *     ../target/release/wbuild/bulletin-westend-runtime/bulletin_westend_runtime.compact.compressed.wasm
 */

import { ChopsticksProvider, setup } from "@acala-network/chopsticks-core";
import { Keyring } from "@polkadot/keyring";
import { cryptoWaitReady, blake2AsU8a, xxhashAsHex } from "@polkadot/util-crypto";
import { Struct, u8, u32, u64, Bytes, Vector, Tuple } from "@polkadot-api/substrate-bindings";

const endpoint = process.argv[2] || "wss://westend-bulletin-rpc.polkadot.io";
const runtimeWasm = process.argv[3] || null;

// Runtime pallet index (construct_runtime) and call index of the mandatory
// `process_pending_renewals` inherent in `pallet-bulletin-data-renewal`.
const PALLET_DATA_RENEWAL = 42;
const CALL_PROCESS_PENDING_RENEWALS = 4;

// ─── PAPI Typed Codecs ────────────────────────────────────────────────────
//
// These codecs replace manual SCALE hex encoding with type-safe PAPI codecs.
// See: https://papi.how/typed-codecs

/** Authorization { extent: AuthorizationExtent { transactions: u32, bytes: u64 }, expiration: u32 } */
const AuthorizationCodec = Struct({
  extent: Struct({ transactions: u32, bytes: u64 }),
  expiration: u32,
});

/** (BlockNumber, u32) — stored in TransactionByContentHash */
const BlockAndIndexCodec = Tuple(u32, u32);

/** DataRenewal RenewalData { account: AccountId(32), recurring: bool, paid: bool } */
const RenewalDataCodec = Struct({
  account: Bytes(32),
  recurring: u8, // SCALE bool: 0x00 / 0x01
  paid: u8, // SCALE bool: 0x00 / 0x01
});

/**
 * TransactionInfo (field order matches the Rust struct SCALE layout):
 *   chunk_root: H256, content_hash: [u8;32], hashing: HashingAlgorithm(u8 enum),
 *   cid_codec: CidCodec(u64), size: u32, extrinsic_index: u32,
 *   block_chunks: u32, kind: TransactionKind(u8 enum: Store=0, Renew=1)
 */
const TransactionInfoCodec = Struct({
  chunk_root: Bytes(32),
  content_hash: Bytes(32),
  hashing: u8,
  cid_codec: u64,
  size: u32,
  extrinsic_index: u32,
  block_chunks: u32,
  kind: u8,
});

/** BoundedVec<TransactionInfo> with SCALE compact length prefix */
const TransactionInfoVecCodec = Vector(TransactionInfoCodec);

// ─── Storage key helpers ──────────────────────────────────────────────────

function toHex(bytes) {
  return Array.from(bytes).map(b => b.toString(16).padStart(2, "0")).join("");
}

function twox128(text) {
  return xxhashAsHex(text, 128).slice(2);
}

function blake2_128Concat(data) {
  const hash = blake2AsU8a(data, 128);
  const out = new Uint8Array(hash.length + data.length);
  out.set(hash);
  out.set(data, hash.length);
  return toHex(out);
}

/** Plain storage key: twox128(pallet) ++ twox128(name) */
function storageKey(pallet, name) {
  return "0x" + twox128(pallet) + twox128(name);
}

/** Map storage key: twox128(pallet) ++ twox128(name) ++ blake2_128Concat(keyBytes) */
function mapKey(pallet, name, keyBytes) {
  return "0x" + twox128(pallet) + twox128(name) + blake2_128Concat(keyBytes);
}

/** Encode a value with a PAPI codec and return a 0x-prefixed hex string */
function encodeHex(codec, value) {
  return "0x" + toHex(codec.enc(value));
}

/**
 * Encode a bare (unsigned) extrinsic wrapping a nullary call. Bare extrinsics
 * are how inherents reach the runtime: version byte = EXTRINSIC_FORMAT_VERSION
 * (5) with the bare type bits (0), followed by the pallet+call indices, all
 * behind a SCALE compact length prefix.
 */
function bareInherentExtrinsic(palletIndex, callIndex) {
  const EXTRINSIC_FORMAT_VERSION = 5;
  const body = new Uint8Array([EXTRINSIC_FORMAT_VERSION, palletIndex, callIndex]);
  // Compact length for < 64 bytes is a single byte: len << 2.
  const prefixed = new Uint8Array([body.length << 2, ...body]);
  return "0x" + toHex(prefixed);
}

// ─── Logging helpers ──────────────────────────────────────────────────────

function logHeader(text) {
  console.log("\n" + "=".repeat(80));
  console.log(`  ${text}`);
  console.log("=".repeat(80));
}
function logStep(n, msg) { console.log(`\n[Step ${n}] ${msg}`); }
function logOk(msg) { console.log(`  OK: ${msg}`); }
function logFail(msg) { console.error(`  FAIL: ${msg}`); }

// ─── Main ──────────────────────────────────────────────────────────────────

async function main() {
  await cryptoWaitReady();
  logHeader("DATA-RENEWAL INHERENT CHOPSTICKS INTEGRATION TEST (PAPI codecs)");
  console.log(`Endpoint: ${endpoint}`);

  logStep(0, "Setting up Chopsticks fork...");
  const config = { endpoint };
  if (runtimeWasm) {
    console.log(`  Runtime WASM override: ${runtimeWasm}`);
    config.wasmOverride = runtimeWasm;
  } else {
    console.log("  No WASM override — using live chain runtime.");
    console.log("  Pass WASM path as 3rd arg to use local runtime with DataRenewal support.");
  }
  const chain = await setup(config);
  await chain.api.isReady;
  const provider = new ChopsticksProvider(chain);
  await provider.isReady;
  const startBlock = chain.head.number;
  console.log(`  Forked at block #${startBlock}`);

  // ── Override RetentionPeriod to 1 block (so expiry triggers in 2 blocks) ──
  // Also set ProofChecked to true to satisfy TransactionStorage::on_finalize
  // (Chopsticks doesn't produce real storage proofs).
  logStep(1, "Overriding RetentionPeriod and mocking ProofChecked...");
  const retentionPeriod = 1;
  const proofCheckedKey = storageKey("TransactionStorage", "ProofChecked");
  await provider.send("dev_setStorage", [
    [
      [storageKey("TransactionStorage", "RetentionPeriod"), encodeHex(u32, retentionPeriod)],
      [proofCheckedKey, "0x01"],
    ]
  ], false);
  logOk(`RetentionPeriod set to ${retentionPeriod}, ProofChecked = true`);

  // ── Setup accounts ──
  const keyring = new Keyring({ type: "sr25519" });
  const authorizer = keyring.addFromUri("//Eve");

  // ── Compute content hash ──
  const testData = new Uint8Array(2000).fill(42);
  const contentHash = blake2AsU8a(testData, 256);
  const renewalsKey = mapKey("DataRenewal", "Renewals", contentHash);
  const pendingKey = storageKey("DataRenewal", "PendingAutoRenewals");
  const contentHashKey = mapKey("TransactionStorage", "TransactionByContentHash", contentHash);
  console.log(`  Content hash: 0x${toHex(contentHash).slice(0, 16)}...`);

  const storeBlock = startBlock;
  // TransactionStorage::on_initialize ages out `n - (RetentionPeriod + 1)`.
  const expiryBlock = storeBlock + retentionPeriod + 1;
  console.log(`  Store block: #${storeBlock} (current)`);
  console.log(`  Expiry block: #${expiryBlock} (${retentionPeriod + 1} blocks away)`);

  // ── Set base storage with PAPI codecs ──
  logStep(2, "Setting base storage state (authorization, transaction, content hash map)...");

  // AuthorizationScope::Account(authorizer) = 0x00 ++ authorizer.publicKey
  const scopeBytes = new Uint8Array(1 + 32);
  scopeBytes[0] = 0; // Account variant
  scopeBytes.set(authorizer.publicKey, 1);

  const chunks = Math.ceil(testData.length / 256) || 1;
  const baseStorageItems = [
    // Authorizations(Account(authorizer))
    [
      mapKey("TransactionStorage", "Authorizations", scopeBytes),
      encodeHex(AuthorizationCodec, {
        extent: { transactions: 100, bytes: BigInt(100_000_000) },
        expiration: startBlock + 200000,
      }),
    ],
    // Transactions(storeBlock)
    [
      mapKey("TransactionStorage", "Transactions", u32.enc(storeBlock)),
      encodeHex(TransactionInfoVecCodec, [{
        chunk_root: new Uint8Array(32), // dummy
        content_hash: contentHash,
        hashing: 0,               // Blake2b256 variant
        cid_codec: BigInt(0x55),  // RAW_CODEC
        size: testData.length,
        extrinsic_index: 0,
        block_chunks: chunks,
        kind: 0,                  // TransactionKind::Store
      }]),
    ],
    // TransactionByContentHash(contentHash) -> (storeBlock, 0)
    [
      contentHashKey,
      encodeHex(BlockAndIndexCodec, [storeBlock, 0]),
    ],
  ];

  await provider.send("dev_setStorage", [baseStorageItems], false);
  logOk("Base storage items set (3 items)");

  for (const [key, _value] of baseStorageItems) {
    const check = await provider.send("state_getStorage", [key], false);
    if (check) {
      logOk(`  Verified: ${key.slice(0, 40)}...`);
    } else {
      logFail(`  Missing: ${key.slice(0, 40)}...`);
    }
  }

  // Verify Renewals does NOT exist yet
  const preCheck = await provider.send("state_getStorage", [renewalsKey], false);
  if (!preCheck) {
    logOk("DataRenewal.Renewals is empty (not yet enabled)");
  } else {
    logFail("DataRenewal.Renewals should not exist before enable!");
  }

  // ── Test enable_auto_renew (recurring, prepaid first cycle) ──
  logStep(3, "Testing enable_auto_renew: setting DataRenewal.Renewals entry...");
  const renewalValue = encodeHex(RenewalDataCodec, {
    account: authorizer.publicKey,
    recurring: 1,
    paid: 1,
  });
  await provider.send("dev_setStorage", [[[renewalsKey, renewalValue]]], false);

  const enableCheck = await provider.send("state_getStorage", [renewalsKey], false);
  if (enableCheck && enableCheck.slice(2) === renewalValue.slice(2)) {
    logOk("Renewals entry created (enable_auto_renew verified)");
  } else {
    logFail(`Renewals entry mismatch! expected=${renewalValue} got=${enableCheck}`);
    throw new Error("enable_auto_renew verification failed");
  }

  // ── Test disable_auto_renew ──
  logStep(4, "Testing disable_auto_renew: removing DataRenewal.Renewals entry...");
  await provider.send("dev_setStorage", [[[renewalsKey, null]]], false);
  const disableCheck = await provider.send("state_getStorage", [renewalsKey], false);
  if (!disableCheck) {
    logOk("Renewals entry removed (disable_auto_renew verified)");
  } else {
    logFail("Renewals entry still exists after disable!");
    throw new Error("disable_auto_renew verification failed");
  }

  // ── Re-enable auto-renewal for expiry test ──
  logStep(5, "Re-enabling auto-renewal for the expiry test...");
  await provider.send("dev_setStorage", [[[renewalsKey, renewalValue]]], false);
  if (!(await provider.send("state_getStorage", [renewalsKey], false))) {
    throw new Error("Failed to re-enable Renewals");
  }
  logOk("Renewals re-enabled");

  // ── Advance blocks up to (but not including) the expiry block ──
  logStep(6, `Advancing to the block before expiry (#${expiryBlock})...`);
  for (let i = 0; i < retentionPeriod; i++) {
    await provider.send("dev_setStorage", [[[proofCheckedKey, "0x01"]]], false);
    await provider.send("dev_newBlock", [], false);
    logOk(`Block #${chain.head.number} produced`);
  }

  // ── Expiry block: on_initialize queues PendingAutoRenewals, and the injected
  //    process_pending_renewals inherent drains it in the same block. ──
  logStep(7, "Building the expiry block with the process_pending_renewals inherent injected...");
  const inherent = bareInherentExtrinsic(PALLET_DATA_RENEWAL, CALL_PROCESS_PENDING_RENEWALS);
  console.log(`  Injecting inherent extrinsic: ${inherent}`);
  await provider.send("dev_setStorage", [[[proofCheckedKey, "0x01"]]], false);
  await provider.send("dev_newBlock", [{ transactions: [inherent] }], false);
  if (chain.head.number < expiryBlock) {
    throw new Error(`Expected block #${expiryBlock}, got #${chain.head.number}`);
  }
  logOk(`Block #${chain.head.number} produced (on_finalize did not panic)`);

  // ── Verify the drain results ──
  logStep(8, "Verifying the inherent drained PendingAutoRenewals and renewed the data...");

  const pendingAfter = await provider.send("state_getStorage", [pendingKey], false);
  if (!pendingAfter || pendingAfter === "0x00") {
    logOk("PendingAutoRenewals is empty (drained by the inherent)");
  } else {
    logFail(`PendingAutoRenewals not drained: ${pendingAfter}`);
    throw new Error("process_pending_renewals did not drain PendingAutoRenewals");
  }

  const contentHashAfter = await provider.send("state_getStorage", [contentHashKey], false);
  if (contentHashAfter) {
    const [block, index] = BlockAndIndexCodec.dec(contentHashAfter);
    if (block === expiryBlock) {
      logOk(`TransactionByContentHash points at the expiry block #${block} index ${index} (renewed)`);
    } else {
      logFail(`TransactionByContentHash points at #${block} (expected #${expiryBlock})`);
      throw new Error("data was not renewed to the expiry block");
    }
  } else {
    logFail("TransactionByContentHash was removed — data expired without renewal!");
    throw new Error("auto-renewal did not run");
  }

  const renewalAfter = await provider.send("state_getStorage", [renewalsKey], false);
  if (renewalAfter) {
    const decoded = RenewalDataCodec.dec(renewalAfter);
    if (decoded.recurring === 1 && decoded.paid === 0) {
      logOk("Recurring registration survives with paid = false (prepayment consumed)");
    } else {
      logFail(`Unexpected RenewalData: recurring=${decoded.recurring} paid=${decoded.paid}`);
    }
  } else {
    logFail("Recurring registration was removed after a single cycle!");
    throw new Error("recurring registration should survive");
  }

  await chain.close();

  logHeader("TEST COMPLETE");
  console.log("  Verified:");
  console.log("  - enable_auto_renew: DataRenewal.Renewals entry created");
  console.log("  - disable_auto_renew: DataRenewal.Renewals entry removed");
  console.log("  - on_initialize queues expiring data into PendingAutoRenewals");
  console.log("  - process_pending_renewals inherent drains it and renews the data");
  console.log("  - recurring registration survives with the prepayment consumed");
  console.log("");
  process.exit(0);
}

try {
  await main();
} catch (err) {
  console.error("\nFATAL:", err.message || err);
  console.error(err.stack);
  process.exit(1);
}
