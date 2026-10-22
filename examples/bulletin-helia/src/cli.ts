#!/usr/bin/env node

import { writeFileSync } from "node:fs"
// Imported via the package's public exports, exactly as an external client
// would — the CLI is a reference consumer of the library, not part of it.
import { IPFSClient } from "bulletin-helia/ipfs"
import { CLILogger } from "bulletin-helia/logger-cli"

async function main() {
  const args = process.argv.slice(2)

  // Extract the optional `-o/--out <file>` flag; the rest are positional
  // (CID followed by one or more peer multiaddrs).
  let outPath: string | undefined
  const positional: string[] = []
  for (let i = 0; i < args.length; i++) {
    const arg = args[i]
    if (arg === "-o" || arg === "--out") {
      outPath = args[++i]
    } else if (arg.startsWith("--out=")) {
      outPath = arg.slice("--out=".length)
    } else {
      positional.push(arg)
    }
  }

  const cid = positional[0]
  const peerMultiaddrs = positional.slice(1)

  if (
    peerMultiaddrs.length < 1 ||
    (outPath !== undefined && outPath.length === 0)
  ) {
    console.error(
      "Usage: bulletin-helia [-o <file>] <CID> <peer-multiaddr1> [peer-multiaddr2] ...",
    )
    console.error("")
    console.error(
      "  -o, --out <file>  Write the fetched raw bytes to <file> instead of stdout",
    )
    console.error("")
    console.error("Examples:")
    console.error(
      "  bulletin-helia bafyreifhj6h... /ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooW...",
    )
    console.error(
      "  bulletin-helia -o block.bin bafyreifhj6h... /ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooW...",
    )
    process.exit(1)
  }

  const logger = new CLILogger()

  logger.info("Bulletin Helia CLI - P2P Mode")
  logger.info(`CID: ${cid}`)
  logger.info(`Peers: ${peerMultiaddrs.length}`)
  peerMultiaddrs.forEach((addr, i) => {
    logger.debug(`  [${i + 1}] ${addr}`)
  })

  const client = new IPFSClient({
    logger,
    peerMultiaddrs,
  })

  try {
    await client.initialize()

    if (outPath !== undefined) {
      // Write the exact downloaded bytes (already verified against the CID by
      // bitswap) to disk, with no hex/JSON transformation.
      const bytes = await client.fetchRawBytes(cid)
      writeFileSync(outPath, bytes)
      logger.success(`Wrote ${bytes.length} bytes to ${outPath}`)
    } else {
      const result = await client.fetchData(cid)

      console.log("\n=== RAW BYTES ===")
      console.log(result.rawHex)

      if (result.isJSON) {
        console.log("\n=== PARSED JSON ===")
        console.log(JSON.stringify(result.data, null, 2))
      }
    }

    await client.stop()
    process.exit(0)
  } catch (error) {
    logger.error("Failed to fetch data", error)
    process.exit(1)
  }
}

main()
