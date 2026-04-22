#!/usr/bin/env npx tsx
// HOP (Hand-off Protocol) stress test.
//
// Exercises hop_submit / hop_claim / hop_ack RPCs against Bulletin collators.
//
// Usage:
//   npx tsx test/stress/hop-stress.ts \
//     --ws-url wss://collator-0,wss://collator-1 \
//     --scenario full-cycle --items 100 --payload-size 1024

import { parseArgs } from "node:util"
import { u8aToHex } from "@polkadot/util"
import {
  blake2AsHex,
  ed25519PairFromRandom,
  ed25519Sign,
} from "@polkadot/util-crypto"
import { createClient as createSubstrateClient } from "@polkadot-api/substrate-client"
import { withPolkadotSdkCompat } from "polkadot-api/polkadot-sdk-compat"
import { getWsProvider } from "polkadot-api/ws-provider/node"

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

const SCENARIOS = [
  "submit-only",
  "full-cycle",
  "group",
  "pool-fill",
  "mixed",
  "errors",
] as const
type Scenario = (typeof SCENARIOS)[number]

const { values } = parseArgs({
  options: {
    "ws-url": { type: "string", default: "ws://127.0.0.1:9944" },
    scenario: { type: "string", default: "full-cycle" },
    items: { type: "string", default: "100" },
    "payload-size": { type: "string", default: "1024" },
    recipients: { type: "string", default: "1" },
    concurrency: { type: "string", default: "4" },
    duration: { type: "string", default: "300" },
    help: { type: "boolean", default: false },
  },
  strict: true,
})

if (values.help) {
  console.log(`
HOP stress test for Bulletin Chain collators

Options:
  --ws-url <urls>         Comma-separated collator RPC WebSocket URLs
  --scenario <name>       Test scenario: ${SCENARIOS.join(", ")}
  --items <n>             Number of entries (default: 100)
  --payload-size <bytes>  Payload size per entry in bytes (default: 1024)
  --recipients <n>        Recipients per entry (default: 1)
  --concurrency <n>       Parallel submit/claim streams (default: 4)
  --duration <secs>       Duration for sustained scenarios (default: 300)
`)
  process.exit(0)
}

const wsUrls = (values["ws-url"] ?? "ws://127.0.0.1:9944")
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean)
const scenario = values.scenario as Scenario
if (!SCENARIOS.includes(scenario)) {
  console.error(
    `Unknown scenario: ${scenario}. Choose from: ${SCENARIOS.join(", ")}`,
  )
  process.exit(1)
}
const numItems = parseInt(values.items ?? "100", 10)
const payloadSize = parseInt(values["payload-size"] ?? "1024", 10)
const numRecipients = parseInt(values.recipients ?? "1", 10)
const concurrency = parseInt(values.concurrency ?? "4", 10)
const durationSecs = parseInt(values.duration ?? "300", 10)

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface EphemeralKeypair {
  publicKey: Uint8Array
  secretKey: Uint8Array
}

interface SubmittedEntry {
  cid: string // hex-encoded blake2-256 hash
  data: Uint8Array
  recipients: EphemeralKeypair[]
  collatorUrl: string
  submitLatencyMs: number
}

interface PhaseStats {
  count: number
  errors: number
  errorsByCode: Map<number, number>
  latencies: number[]
  totalBytes: number
  startMs: number
  endMs: number
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function generatePayload(index: number, size: number): Uint8Array {
  const buf = new Uint8Array(size)
  const header = new TextEncoder().encode(`hop-${index}-`)
  buf.set(header)
  for (let j = header.length; j < size; j++) {
    buf[j] = ((index * 31 + j * 7) ^ 0xa5) & 0xff
  }
  return buf
}

function generateRecipients(count: number): EphemeralKeypair[] {
  const pairs: EphemeralKeypair[] = []
  for (let i = 0; i < count; i++) {
    pairs.push(ed25519PairFromRandom())
  }
  return pairs
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

function percentile(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0
  const idx = Math.ceil((p / 100) * sorted.length) - 1
  return sorted[Math.max(0, idx)] ?? 0
}

function newPhaseStats(): PhaseStats {
  return {
    count: 0,
    errors: 0,
    errorsByCode: new Map(),
    latencies: [],
    totalBytes: 0,
    startMs: Date.now(),
    endMs: 0,
  }
}

function printPhaseStats(name: string, stats: PhaseStats) {
  stats.endMs = stats.endMs || Date.now()
  const durationMs = stats.endMs - stats.startMs
  const sorted = [...stats.latencies].sort((a, b) => a - b)
  const throughput =
    durationMs > 0 ? (stats.count / (durationMs / 1000)).toFixed(2) : "N/A"
  const dataRate =
    durationMs > 0 ? formatBytes(stats.totalBytes / (durationMs / 1000)) : "N/A"

  console.log(`\n--- ${name} ---`)
  console.log(`  Count:      ${stats.count} (${stats.errors} errors)`)
  console.log(`  Duration:   ${formatDuration(durationMs)}`)
  console.log(`  Throughput: ${throughput} ops/s, ${dataRate}/s`)
  if (sorted.length > 0) {
    console.log(
      `  Latency:    p50=${(sorted[Math.floor(sorted.length * 0.5)] ?? 0).toFixed(0)}ms ` +
        `p95=${percentile(sorted, 95).toFixed(0)}ms ` +
        `p99=${percentile(sorted, 99).toFixed(0)}ms`,
    )
  }
  if (stats.errorsByCode.size > 0) {
    const codes = [...stats.errorsByCode.entries()]
      .map(([code, count]) => `${code}:${count}`)
      .join(", ")
    console.log(`  Errors:     ${codes}`)
  }
}

// ---------------------------------------------------------------------------
// RPC helpers
// ---------------------------------------------------------------------------

function createRpcClient(url: string) {
  return createSubstrateClient(withPolkadotSdkCompat(getWsProvider(url)))
}

async function hopSubmit(
  client: ReturnType<typeof createRpcClient>,
  data: Uint8Array,
  recipientPubkeys: Uint8Array[],
): Promise<{
  hash: string
  pool_status: { entry_count: number; total_bytes: number; max_bytes: number }
}> {
  const dataHex = u8aToHex(data)
  const recipientHexes = recipientPubkeys.map((pk) => u8aToHex(pk))
  return client.request("hop_submit", [dataHex, recipientHexes])
}

async function hopClaim(
  client: ReturnType<typeof createRpcClient>,
  hash: string,
  keypair: EphemeralKeypair,
): Promise<string> {
  // Sign the content hash with the ephemeral private key
  const hashBytes = hexToBytes(hash)
  const sig = ed25519Sign(hashBytes, keypair)
  return client.request("hop_claim", [hash, u8aToHex(sig)])
}

async function hopAck(
  client: ReturnType<typeof createRpcClient>,
  hash: string,
  keypair: EphemeralKeypair,
): Promise<void> {
  const hashBytes = hexToBytes(hash)
  const sig = ed25519Sign(hashBytes, keypair)
  await client.request("hop_ack", [hash, u8aToHex(sig)])
}

async function hopPoolStatus(
  client: ReturnType<typeof createRpcClient>,
): Promise<{ entry_count: number; total_bytes: number; max_bytes: number }> {
  return client.request("hop_poolStatus", [])
}

function hexToBytes(hex: string): Uint8Array {
  const clean = hex.startsWith("0x") ? hex.slice(2) : hex
  const bytes = new Uint8Array(clean.length / 2)
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(clean.slice(i * 2, i * 2 + 2), 16)
  }
  return bytes
}

function extractErrorCode(err: unknown): number {
  if (err && typeof err === "object" && "code" in err) {
    return (err as { code: number }).code
  }
  const msg = String(err)
  const match = msg.match(/\b(100[0-9]|101[0-1])\b/)
  return match?.[1] ? parseInt(match[1], 10) : 0
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

async function runSubmitOnly() {
  console.log("\n=== S1: Submit Throughput ===")

  const stats = newPhaseStats()
  const clients = wsUrls.map(createRpcClient)
  const submitted: SubmittedEntry[] = []

  // Distribute items across concurrent streams
  const itemsPerStream = Math.ceil(numItems / concurrency)

  const streams = Array.from({ length: concurrency }, (_, streamIdx) => {
    const clientIdx = streamIdx % clients.length
    const client = clients[clientIdx] as ReturnType<typeof createRpcClient>
    const url = wsUrls[clientIdx] as string
    const start = streamIdx * itemsPerStream
    const end = Math.min(start + itemsPerStream, numItems)

    return (async () => {
      for (let i = start; i < end; i++) {
        const data = generatePayload(i, payloadSize)
        const recipients = generateRecipients(numRecipients)
        const t0 = performance.now()
        try {
          const result = await hopSubmit(
            client,
            data,
            recipients.map((r) => r.publicKey),
          )
          const latency = performance.now() - t0
          stats.count++
          stats.latencies.push(latency)
          stats.totalBytes += data.length
          submitted.push({
            cid: result.hash,
            data,
            recipients,
            collatorUrl: url,
            submitLatencyMs: latency,
          })
        } catch (err) {
          stats.errors++
          const code = extractErrorCode(err)
          stats.errorsByCode.set(code, (stats.errorsByCode.get(code) ?? 0) + 1)
          if (stats.errors <= 5) {
            console.error(`  submit error [${i}]: ${err}`)
          }
        }
      }
    })()
  })

  await Promise.all(streams)
  stats.endMs = Date.now()

  printPhaseStats("Submit", stats)

  // Print pool status from first collator
  try {
    const poolStatus = await hopPoolStatus(
      clients[0] as ReturnType<typeof createRpcClient>,
    )
    console.log(
      `\n  Pool: ${poolStatus.entry_count} entries, ` +
        `${formatBytes(poolStatus.total_bytes)} / ${formatBytes(poolStatus.max_bytes)}`,
    )
  } catch (err) {
    console.log(`  Pool status unavailable: ${err}`)
  }

  for (const c of clients) c.destroy()
  return submitted
}

async function runClaimAndAck(submitted: SubmittedEntry[]) {
  if (submitted.length === 0) {
    console.log("\nNo entries to claim/ack.")
    return
  }

  // Claim phase
  console.log(`\nClaiming ${submitted.length} entries...`)
  const claimStats = newPhaseStats()

  for (const entry of submitted) {
    const client = createRpcClient(entry.collatorUrl)
    for (const kp of entry.recipients) {
      const t0 = performance.now()
      try {
        const claimedDataHex = await hopClaim(client, entry.cid, kp)
        const latency = performance.now() - t0
        claimStats.count++
        claimStats.latencies.push(latency)
        claimStats.totalBytes += entry.data.length

        // Verify data integrity
        const claimedData = hexToBytes(claimedDataHex)
        if (claimedData.length !== entry.data.length) {
          console.error(
            `  Data mismatch for ${entry.cid}: expected ${entry.data.length}B, got ${claimedData.length}B`,
          )
        }
      } catch (err) {
        claimStats.errors++
        const code = extractErrorCode(err)
        claimStats.errorsByCode.set(
          code,
          (claimStats.errorsByCode.get(code) ?? 0) + 1,
        )
        if (claimStats.errors <= 5) {
          console.error(`  claim error [${entry.cid.slice(0, 16)}...]: ${err}`)
        }
      }
    }
    client.destroy()
  }
  claimStats.endMs = Date.now()
  printPhaseStats("Claim", claimStats)

  // Ack phase
  console.log(`\nAcknowledging ${submitted.length} entries...`)
  const ackStats = newPhaseStats()

  for (const entry of submitted) {
    const client = createRpcClient(entry.collatorUrl)
    for (const kp of entry.recipients) {
      const t0 = performance.now()
      try {
        await hopAck(client, entry.cid, kp)
        const latency = performance.now() - t0
        ackStats.count++
        ackStats.latencies.push(latency)
      } catch (err) {
        ackStats.errors++
        const code = extractErrorCode(err)
        ackStats.errorsByCode.set(
          code,
          (ackStats.errorsByCode.get(code) ?? 0) + 1,
        )
        if (ackStats.errors <= 5) {
          console.error(`  ack error [${entry.cid.slice(0, 16)}...]: ${err}`)
        }
      }
    }
    client.destroy()
  }
  ackStats.endMs = Date.now()
  printPhaseStats("Ack", ackStats)
}

async function runFullCycle() {
  console.log("\n=== S2: Full Cycle (submit + claim + ack) ===")
  const submitted = await runSubmitOnly()
  await runClaimAndAck(submitted)
}

async function runGroup() {
  console.log(`\n=== S3: Group Recipients (${numRecipients} recipients) ===`)
  // Same as full-cycle but numRecipients is typically > 1
  const submitted = await runSubmitOnly()

  // Claim with all recipients in parallel
  if (submitted.length === 0) return

  console.log(`\nClaiming with ${numRecipients} recipients per entry...`)
  const claimStats = newPhaseStats()

  for (const entry of submitted) {
    const client = createRpcClient(entry.collatorUrl)
    // Claim with all recipients concurrently
    const claimPromises = entry.recipients.map(async (kp) => {
      const t0 = performance.now()
      try {
        await hopClaim(client, entry.cid, kp)
        const latency = performance.now() - t0
        claimStats.count++
        claimStats.latencies.push(latency)
        claimStats.totalBytes += entry.data.length
      } catch (err) {
        claimStats.errors++
        const code = extractErrorCode(err)
        claimStats.errorsByCode.set(
          code,
          (claimStats.errorsByCode.get(code) ?? 0) + 1,
        )
      }
    })
    await Promise.all(claimPromises)

    // Ack with all recipients
    const ackPromises = entry.recipients.map(async (kp) => {
      try {
        await hopAck(client, entry.cid, kp)
      } catch (_) {}
    })
    await Promise.all(ackPromises)
    client.destroy()
  }
  claimStats.endMs = Date.now()
  printPhaseStats("Claim (parallel recipients)", claimStats)

  // Verify entries are cleaned up
  try {
    const client = createRpcClient(wsUrls[0] as string)
    const status = await hopPoolStatus(client)
    console.log(
      `\n  Pool after ack: ${status.entry_count} entries, ${formatBytes(status.total_bytes)}`,
    )
    client.destroy()
  } catch (_) {}
}

async function runPoolFill() {
  console.log("\n=== S4: Pool Fill ===")
  const client = createRpcClient(wsUrls[0] as string)

  // Check initial pool status
  let status = await hopPoolStatus(client)
  console.log(
    `  Initial pool: ${status.entry_count} entries, ${formatBytes(status.total_bytes)} / ${formatBytes(status.max_bytes)}`,
  )

  const stats = newPhaseStats()
  let i = 0

  while (true) {
    const data = generatePayload(i, payloadSize)
    const recipients = generateRecipients(1)
    const t0 = performance.now()
    try {
      const result = await hopSubmit(
        client,
        data,
        recipients.map((r) => r.publicKey),
      )
      const latency = performance.now() - t0
      stats.count++
      stats.latencies.push(latency)
      stats.totalBytes += data.length

      // Print progress every 100 items
      if (stats.count % 100 === 0) {
        console.log(
          `  ${stats.count} submitted, pool: ${result.pool_status.entry_count} entries, ` +
            `${formatBytes(result.pool_status.total_bytes)} / ${formatBytes(result.pool_status.max_bytes)}`,
        )
      }
    } catch (err) {
      const code = extractErrorCode(err)
      if (code === 1002) {
        // PoolFull — we're done
        console.log(`\n  PoolFull hit after ${stats.count} entries`)
        break
      }
      stats.errors++
      stats.errorsByCode.set(code, (stats.errorsByCode.get(code) ?? 0) + 1)
      if (stats.errors > 10) {
        console.error(`  Too many errors, stopping.`)
        break
      }
    }
    i++

    // Safety cap
    if (stats.count >= 100_000) {
      console.log("  Hit 100k entries safety cap")
      break
    }
  }

  stats.endMs = Date.now()
  printPhaseStats("Pool Fill", stats)

  status = await hopPoolStatus(client)
  console.log(
    `  Final pool: ${status.entry_count} entries, ${formatBytes(status.total_bytes)} / ${formatBytes(status.max_bytes)}`,
  )

  client.destroy()
}

async function runMixed() {
  console.log(`\n=== S5: Mixed Read/Write (${durationSecs}s) ===`)

  const clients = wsUrls.map(createRpcClient)
  const submitStats = newPhaseStats()
  const claimStats = newPhaseStats()

  // Shared queue of submitted entries for readers to consume
  const pending: SubmittedEntry[] = []

  const endTime = Date.now() + durationSecs * 1000

  // Writer streams
  const writerCount = Math.max(1, Math.floor(concurrency / 2))
  const readerCount = Math.max(1, concurrency - writerCount)

  const writers = Array.from({ length: writerCount }, (_, idx) => {
    const client = clients[idx % clients.length] as ReturnType<
      typeof createRpcClient
    >
    const url = wsUrls[idx % wsUrls.length] as string
    let itemIdx = idx * 1_000_000 // unique per writer

    return (async () => {
      while (Date.now() < endTime) {
        const data = generatePayload(itemIdx++, payloadSize)
        const recipients = generateRecipients(1)
        const t0 = performance.now()
        try {
          const result = await hopSubmit(
            client,
            data,
            recipients.map((r) => r.publicKey),
          )
          submitStats.count++
          submitStats.latencies.push(performance.now() - t0)
          submitStats.totalBytes += data.length
          pending.push({
            cid: result.hash,
            data,
            recipients,
            collatorUrl: url,
            submitLatencyMs: performance.now() - t0,
          })
        } catch (err) {
          submitStats.errors++
          const code = extractErrorCode(err)
          submitStats.errorsByCode.set(
            code,
            (submitStats.errorsByCode.get(code) ?? 0) + 1,
          )
        }
      }
    })()
  })

  // Reader streams
  const readers = Array.from({ length: readerCount }, () =>
    (async () => {
      while (Date.now() < endTime || pending.length > 0) {
        const entry = pending.shift()
        if (!entry) {
          await new Promise((r) => setTimeout(r, 50))
          continue
        }
        const client = createRpcClient(entry.collatorUrl)
        const kp = entry.recipients[0] as EphemeralKeypair
        const t0 = performance.now()
        try {
          await hopClaim(client, entry.cid, kp)
          claimStats.count++
          claimStats.latencies.push(performance.now() - t0)
          claimStats.totalBytes += entry.data.length
          // Ack
          await hopAck(client, entry.cid, kp)
        } catch (err) {
          claimStats.errors++
          const code = extractErrorCode(err)
          claimStats.errorsByCode.set(
            code,
            (claimStats.errorsByCode.get(code) ?? 0) + 1,
          )
        }
        client.destroy()
      }
    })(),
  )

  // Print progress periodically
  const progressInterval = setInterval(() => {
    const elapsed = formatDuration(Date.now() - submitStats.startMs)
    console.log(
      `  [${elapsed}] submitted: ${submitStats.count}, claimed: ${claimStats.count}, ` +
        `pending: ${pending.length}, errors: ${submitStats.errors}/${claimStats.errors}`,
    )
  }, 5000)

  await Promise.all(writers)
  await Promise.all(readers)
  clearInterval(progressInterval)

  submitStats.endMs = Date.now()
  claimStats.endMs = Date.now()

  printPhaseStats("Submit (writers)", submitStats)
  printPhaseStats("Claim+Ack (readers)", claimStats)

  for (const c of clients) c.destroy()
}

async function runErrorTests() {
  console.log("\n=== Error Handling Tests ===")
  const client = createRpcClient(wsUrls[0] as string)
  let passed = 0
  let failed = 0

  async function expectError(
    name: string,
    expectedCode: number,
    fn: () => Promise<unknown>,
  ) {
    try {
      await fn()
      console.log(
        `  FAIL: ${name} — expected error ${expectedCode}, got success`,
      )
      failed++
    } catch (err) {
      const code = extractErrorCode(err)
      if (code === expectedCode) {
        console.log(`  PASS: ${name} (code ${code})`)
        passed++
      } else {
        console.log(
          `  FAIL: ${name} — expected ${expectedCode}, got ${code}: ${String(err).slice(0, 100)}`,
        )
        failed++
      }
    }
  }

  // 1. Empty data
  await expectError("EmptyData", 1005, () =>
    hopSubmit(client, new Uint8Array(0), [ed25519PairFromRandom().publicKey]),
  )

  // 2. No recipients
  await expectError("NoRecipients", 1010, () =>
    hopSubmit(client, new Uint8Array([1, 2, 3]), []),
  )

  // 3. Claim non-existent hash
  const fakeHash = `0x${"ab".repeat(32)}`
  const fakeKp = ed25519PairFromRandom()
  await expectError("NotFound", 1004, () => hopClaim(client, fakeHash, fakeKp))

  // 4. Claim with wrong keypair
  const validData = generatePayload(999, 1024)
  const validRecipient = ed25519PairFromRandom()
  const wrongKp = ed25519PairFromRandom()
  try {
    const result = await hopSubmit(client, validData, [
      validRecipient.publicKey,
    ])
    await expectError("NotRecipient", 1009, () =>
      hopClaim(client, result.hash, wrongKp),
    )
    // Clean up: claim + ack with the real recipient
    try {
      await hopClaim(client, result.hash, validRecipient)
      await hopAck(client, result.hash, validRecipient)
    } catch (_) {}
  } catch (err) {
    console.log(`  SKIP: NotRecipient — submit failed: ${err}`)
  }

  // 5. Duplicate entry
  const dupData = generatePayload(998, 512)
  const dupRecipient = ed25519PairFromRandom()
  try {
    await hopSubmit(client, dupData, [dupRecipient.publicKey])
    await expectError("DuplicateEntry", 1003, () =>
      hopSubmit(client, dupData, [dupRecipient.publicKey]),
    )
    // Clean up
    const cid = blake2AsHex(dupData, 256)
    try {
      await hopClaim(client, cid, dupRecipient)
      await hopAck(client, cid, dupRecipient)
    } catch (_) {}
  } catch (err) {
    console.log(`  SKIP: DuplicateEntry — submit failed: ${err}`)
  }

  // 6. DataTooLarge (9 MiB, over 8 MiB limit)
  // Only run if payload would be reasonable to allocate
  await expectError("DataTooLarge", 1001, () =>
    hopSubmit(client, new Uint8Array(9 * 1024 * 1024), [
      ed25519PairFromRandom().publicKey,
    ]),
  )

  console.log(`\n  Results: ${passed} passed, ${failed} failed`)

  client.destroy()
  return failed === 0
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

async function main() {
  console.log("=== HOP Stress Test ===")
  console.log(`  Collators:     ${wsUrls.length}`)
  for (const url of wsUrls) console.log(`    - ${url}`)
  console.log(`  Scenario:      ${scenario}`)
  console.log(`  Items:         ${numItems}`)
  console.log(`  Payload size:  ${formatBytes(payloadSize)}`)
  console.log(`  Total data:    ${formatBytes(numItems * payloadSize)}`)
  console.log(`  Recipients:    ${numRecipients}`)
  console.log(`  Concurrency:   ${concurrency}`)

  switch (scenario) {
    case "submit-only": {
      await runSubmitOnly()
      break
    }
    case "full-cycle": {
      await runFullCycle()
      break
    }
    case "group": {
      await runGroup()
      break
    }
    case "pool-fill": {
      await runPoolFill()
      break
    }
    case "mixed": {
      await runMixed()
      break
    }
    case "errors": {
      const ok = await runErrorTests()
      process.exit(ok ? 0 : 1)
    }
  }

  process.exit(0)
}

main().catch((e) => {
  console.error("Fatal:", e)
  process.exit(1)
})
