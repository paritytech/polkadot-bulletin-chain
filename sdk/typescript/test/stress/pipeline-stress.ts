#!/usr/bin/env npx tsx
// Pipeline stress test — uses the SDK's pipelineStore against Versi/dev chain.
//
// Usage:
//   npx tsx test/stress/pipeline-stress.ts \
//     --ws-url wss://bc-3000-rpc-node-0.parity-versi.parity.io,wss://bc-3000-rpc-node-1.parity-versi.parity.io,wss://bc-3000-rpc-node-2.parity-versi.parity.io,wss://bc-3000-rpc-node-3.parity-versi.parity.io \
//     --items 100 --payload-size 1024 --authorizer-seed "//Alice"

import { mkdirSync, writeFileSync } from "node:fs"
import { dirname, resolve as resolvePath } from "node:path"
import { parseArgs } from "node:util"
import { createClient as createSubstrateClient } from "@polkadot-api/substrate-client"
import { sr25519CreateDerive } from "@polkadot-labs/hdkd"
import { DEV_MINI_SECRET, ss58Address } from "@polkadot-labs/hdkd-helpers"
import { createClient as createPolkadotClient } from "polkadot-api"
import { withPolkadotSdkCompat } from "polkadot-api/polkadot-sdk-compat"
import { getPolkadotSigner } from "polkadot-api/signer"
import { getWsProvider } from "polkadot-api/ws-provider/node"
import type { BulletinTypedApi } from "../../src/async-client.js"
import {
  type BlockLimits,
  type LatencyStats,
  type MultiAccountSigner,
  type PipelineStats,
  pipelineStore,
  pipelineStoreMulti,
} from "../../src/pipeline.js"

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

const { values } = parseArgs({
  options: {
    "ws-url": { type: "string", default: "ws://127.0.0.1:9944" },
    items: { type: "string", default: "20" },
    "payload-size": { type: "string", default: "1024" },
    "authorizer-seed": { type: "string", default: "//Alice" },
    "submitter-seed": { type: "string" },
    "authorize-budget-mb": { type: "string", default: "50" },
    accounts: { type: "string", default: "1" },
    "skip-authorize": { type: "boolean", default: false },
    "output-json": { type: "string" },
    help: { type: "boolean", default: false },
  },
  strict: true,
})

if (values.help) {
  console.log(`
Pipeline stress test for Bulletin Chain SDK

Options:
  --ws-url <urls>           Comma-separated RPC WebSocket URLs
  --items <n>               Number of store transactions (default: 20)
  --payload-size <bytes>    Payload size per item in bytes (default: 1024)
  --authorizer-seed <seed>  Authorizer key URI (default: //Alice)
  --submitter-seed <seed>   Submitter key URI (default: same as authorizer)
  --accounts <n>            Number of parallel submitter accounts (default: 1)
  --authorize-budget-mb <n> Authorization budget in MB (default: 50)
  --output-json <path>      Write full result JSON to this path
`)
  process.exit(0)
}

const wsUrls = (values["ws-url"] ?? "ws://127.0.0.1:9944")
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean)
const numItems = parseInt(values.items ?? "20", 10)
const payloadSize = parseInt(values["payload-size"] ?? "1024", 10)
const authorizerSeed = values["authorizer-seed"] ?? "//Alice"
const submitterSeed = values["submitter-seed"] ?? authorizerSeed
const authBudgetMb = parseInt(values["authorize-budget-mb"] ?? "50", 10)
const numAccounts = parseInt(values.accounts ?? "1", 10)

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function createSigner(seed: string) {
  const derive = sr25519CreateDerive(DEV_MINI_SECRET)
  const keyPair = derive(seed)
  return {
    signer: getPolkadotSigner(keyPair.publicKey, "Sr25519", keyPair.sign),
    rawSign: keyPair.sign as (message: Uint8Array) => Promise<Uint8Array>,
    address: ss58Address(keyPair.publicKey, 42),
    publicKey: keyPair.publicKey,
  }
}

function generatePayloads(count: number, size: number): Uint8Array[] {
  const items: Uint8Array[] = []
  for (let i = 0; i < count; i++) {
    const buf = new Uint8Array(size)
    // Fill with deterministic but unique data
    const header = new TextEncoder().encode(`stress-item-${i}-`)
    buf.set(header)
    // Fill rest with pseudo-random bytes (seeded by index)
    for (let j = header.length; j < size; j++) {
      buf[j] = ((i * 31 + j * 7) ^ 0xa5) & 0xff
    }
    items.push(buf)
  }
  return items
}

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(2)} MB`
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(2)} KB`
  return `${bytes} B`
}

function formatDuration(ms: number): string {
  const sec = ms / 1000
  if (sec >= 60) return `${Math.floor(sec / 60)}m${(sec % 60).toFixed(1)}s`
  return `${sec.toFixed(1)}s`
}

// ---------------------------------------------------------------------------
// Block limits for Bulletin Chain
// ---------------------------------------------------------------------------

// Values from runtimes/bulletin-westend/src/lib.rs and pallet benchmarks.
// The Rust stress-test queries them from the runtime (ChainLimits::query);
// a future version should read them from storage dynamically.
const BLOCK_LIMITS: BlockLimits = {
  maxNormalWeight: 1_500_000_000_000n, // 75% of 2s weight budget
  normalBlockLength: 9_437_184, // 90% of 10 MiB MAX_BLOCK_LENGTH
  maxBlockTransactions: 512, // TransactionStorage::MaxBlockTransactions
  storeWeightBase: 35_489_000n, // from pallet benchmark weights.rs
  storeWeightPerByte: 6_912n, // from pallet benchmark weights.rs
  extrinsicOverhead: 110, // signature + address + extensions
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

async function main() {
  console.log("=== Pipeline Stress Test ===")
  console.log(`  RPC endpoints: ${wsUrls.length}`)
  for (const url of wsUrls) console.log(`    - ${url}`)
  console.log(`  Items:         ${numItems}`)
  console.log(`  Payload size:  ${formatBytes(payloadSize)}`)
  console.log(`  Total data:    ${formatBytes(numItems * payloadSize)}`)
  console.log(`  Accounts:      ${numAccounts}`)
  console.log()

  // Create accounts
  const authorizer = createSigner(authorizerSeed)

  // Generate submitter accounts: single account or N derived accounts
  const submitters: ReturnType<typeof createSigner>[] = []
  if (numAccounts <= 1) {
    const s =
      submitterSeed === authorizerSeed
        ? authorizer
        : createSigner(submitterSeed)
    submitters.push(s)
  } else {
    for (let i = 0; i < numAccounts; i++) {
      submitters.push(createSigner(`${submitterSeed}/${i}`))
    }
  }

  console.log(`  Authorizer: ${authorizer.address} (${authorizerSeed})`)
  for (let i = 0; i < submitters.length; i++) {
    const s = submitters[i]!
    const seed = numAccounts <= 1 ? submitterSeed : `${submitterSeed}/${i}`
    console.log(`  Submitter ${i}: ${s.address} (${seed})`)
  }
  console.log()

  // Connect PAPI client for authorization
  console.log("Connecting to chain...")
  const papiClient = createPolkadotClient(
    withPolkadotSdkCompat(getWsProvider(wsUrls[0]!)),
  )
  const api = papiClient.getUnsafeApi() as unknown as BulletinTypedApi

  // Authorize submitter accounts (use fire-and-forget to avoid signAndSubmit hang)
  if (values["skip-authorize"]) {
    console.log("Skipping authorization (--skip-authorize)")
  } else {
    // Budget must cover total payload per account
    const itemsPerAccount = Math.ceil(numItems / submitters.length)
    const dataSizeMb =
      Math.ceil((itemsPerAccount * payloadSize) / (1024 * 1024)) + 10
    const effectiveMb = Math.max(authBudgetMb, dataSizeMb)
    const budgetBytes = BigInt(effectiveMb) * 1024n * 1024n
    const budgetTxs = itemsPerAccount + 100

    const rawClient = createSubstrateClient(
      withPolkadotSdkCompat(getWsProvider(wsUrls[0]!)),
    )
    // Fetch authorizer nonce once, then increment manually to avoid
    // pool-based nonce collision when submitting multiple auth txs
    let authNonce = await rawClient.request<number>("system_accountNextIndex", [
      authorizer.address,
    ])
    for (const sub of submitters) {
      console.log(
        `Authorizing ${sub.address} for ${budgetTxs} txs / ${formatBytes(Number(budgetBytes))} (nonce ${authNonce})...`,
      )
      try {
        const authTx = api.tx.TransactionStorage.authorize_account({
          who: sub.address,
          transactions: budgetTxs,
          bytes: budgetBytes,
        })
        const hex = await (authTx as any).sign(authorizer.signer, {
          nonce: authNonce,
        })
        await rawClient.request("author_submitExtrinsic", [hex])
        authNonce++
      } catch (e: any) {
        console.log(
          `Authorization for ${sub.address}: ${e.message?.slice(0, 80) ?? e}`,
        )
        authNonce++ // still increment to avoid stuck nonce
      }
    }
    // Wait a block for inclusion
    await new Promise((r) => setTimeout(r, 4000))
    rawClient.destroy()
    console.log(`Authorization submitted for ${submitters.length} account(s)`)
  }

  // Generate payloads
  console.log(
    `Generating ${numItems} payloads of ${formatBytes(payloadSize)}...`,
  )
  const items = generatePayloads(numItems, payloadSize)
  console.log("Payloads ready")
  console.log()

  // Run pipeline
  console.log("Starting pipeline...")

  const pipelineConfig = {
    wsUrls,
    createProvider: (url: string) => withPolkadotSdkCompat(getWsProvider(url)),
    blockLimits: BLOCK_LIMITS,
    signingType: "Sr25519" as const,
  }

  if (numAccounts > 1) {
    // Multi-account mode
    const signers: MultiAccountSigner[] = submitters.map((s) => ({
      signer: s.signer,
      rawSign: s.rawSign,
    }))

    const result = await pipelineStoreMulti(api, signers, items, {
      ...pipelineConfig,
      onProgress: (stats: PipelineStats) => {
        const pct =
          stats.totalItems > 0
            ? ((stats.finalized / stats.totalItems) * 100).toFixed(1)
            : "0"
        const elapsed = formatDuration(stats.elapsedMs)
        console.log(
          `  [${elapsed}] wave ${stats.waves}: ` +
            `${stats.confirmed} best, ${stats.finalized}/${stats.totalItems} fin (${pct}%), ` +
            `${stats.txsBroadcast} broadcast, ${stats.broadcastErrors} errs, ` +
            `${stats.txPerSec.toFixed(2)} tx/s, ${formatBytes(stats.throughputBytesPerSec)}/s`,
        )
      },
    })

    console.log()
    console.log("=== Results (multi-account) ===")
    console.log(`  Accounts:      ${result.accounts}`)
    console.log(`  Duration:      ${formatDuration(result.durationMs)}`)
    console.log(`  Finalized:     ${result.finalized} / ${result.totalItems}`)
    console.log(`  Throughput:    ${result.txPerSec.toFixed(4)} tx/s`)
    console.log(
      `  Data rate:     ${formatBytes(result.throughputBytesPerSec)}/s`,
    )
    console.log(`  Total data:    ${formatBytes(result.totalBytes)}`)
    console.log()

    for (let i = 0; i < result.perAccount.length; i++) {
      const r = result.perAccount[i]!
      console.log(
        `  Account ${i}: ${r.finalized}/${r.totalItems} fin, ` +
          `${r.txPerSec.toFixed(2)} tx/s, ${formatBytes(r.throughputBytesPerSec)}/s, ` +
          `nonce ${r.startNonce}->${r.expectedFinalNonce}`,
      )
    }
    console.log()

    console.log("=== Latency (per-item, broadcast → block, aggregated) ===")
    printLatency("Inclusion (best)   ", result.inclusionLatency)
    printLatency("Finalization       ", result.finalizationLatency)
    console.log()

    writeResultsJson(result)
    papiClient.destroy()
    process.exit(result.finalized === result.totalItems ? 0 : 1)
  } else {
    // Single-account mode
    const submitter = submitters[0]!
    const result = await pipelineStore(api, submitter.signer, items, {
      ...pipelineConfig,
      rawSign: submitter.rawSign,
      onProgress: (stats: PipelineStats) => {
        const pct =
          stats.totalItems > 0
            ? ((stats.finalized / stats.totalItems) * 100).toFixed(1)
            : "0"
        const elapsed = formatDuration(stats.elapsedMs)
        console.log(
          `  [${elapsed}] wave ${stats.waves}: ` +
            `${stats.confirmed} best, ${stats.finalized}/${stats.totalItems} fin (${pct}%), ` +
            `${stats.txsBroadcast} broadcast, ${stats.broadcastErrors} errs, ` +
            `${stats.txPerSec.toFixed(2)} tx/s, ${formatBytes(stats.throughputBytesPerSec)}/s`,
        )
      },
    })

    console.log()
    console.log("=== Results ===")
    console.log(`  Duration:      ${formatDuration(result.durationMs)}`)
    console.log(`  Waves:         ${result.waves}`)
    console.log(
      `  Broadcast:     ${result.txsBroadcast} (${result.broadcastErrors} errors)`,
    )
    console.log(`  Confirmed:     ${result.confirmed} (best)`)
    console.log(`  Finalized:     ${result.finalized} / ${result.totalItems}`)
    console.log(`  Throughput:    ${result.txPerSec.toFixed(4)} tx/s`)
    console.log(
      `  Data rate:     ${formatBytes(result.throughputBytesPerSec)}/s`,
    )
    console.log(`  Total data:    ${formatBytes(result.totalBytes)}`)
    console.log(
      `  Nonce range:   ${result.startNonce} -> ${result.expectedFinalNonce}`,
    )
    console.log()
    console.log("=== Latency (per-item, broadcast → block) ===")
    printLatency("Inclusion (best)   ", result.inclusionLatency)
    printLatency("Finalization       ", result.finalizationLatency)
    console.log()

    writeResultsJson(result)
    papiClient.destroy()
    process.exit(result.finalized === result.totalItems ? 0 : 1)
  }
}

function writeResultsJson(result: unknown): void {
  const outputPath = values["output-json"]
  if (!outputPath) return
  const absPath = resolvePath(outputPath)
  mkdirSync(dirname(absPath), { recursive: true })
  const payload = {
    config: {
      wsUrls,
      items: numItems,
      payloadSize,
      authorizerSeed,
      submitterSeed,
      authBudgetMb,
      accounts: numAccounts,
    },
    result,
    generatedAt: new Date().toISOString(),
  }
  writeFileSync(
    absPath,
    JSON.stringify(
      payload,
      (_k, v) => (typeof v === "bigint" ? v.toString() : v),
      2,
    ),
  )
  console.log(`Wrote results JSON to ${absPath}`)
}

function printLatency(label: string, lat: LatencyStats | null): void {
  if (!lat) {
    console.log(`  ${label}: n/a (no samples)`)
    return
  }
  console.log(
    `  ${label}: n=${lat.count} ` +
      `min=${lat.min.toFixed(0)}ms ` +
      `p50=${lat.p50.toFixed(0)}ms ` +
      `p90=${lat.p90.toFixed(0)}ms ` +
      `p95=${lat.p95.toFixed(0)}ms ` +
      `p99=${lat.p99.toFixed(0)}ms ` +
      `max=${lat.max.toFixed(0)}ms ` +
      `mean=${lat.mean.toFixed(0)}ms`,
  )
}

main().catch((e) => {
  console.error("Fatal:", e)
  process.exit(1)
})
