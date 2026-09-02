import { bitswap } from "@helia/block-brokers"
import { blake2b256 } from "@multiformats/blake2/blake2b"
import { multiaddr } from "@multiformats/multiaddr"
import { createHelia, type Helia, libp2pDefaults } from "helia"
import { equals as bytesEquals, toHex } from "multiformats/bytes"
import { CID } from "multiformats/cid"
import type { BaseLogger } from "./logger-base.js"

// Minimal shape of @helia/bitswap's want-list, which emits a 'presence' event
// per peer response — `has: false` is a DoNotHave. Used to terminate a fetch
// early once every connected peer has said it lacks the block.
type PresenceListener = (evt: {
  detail: { sender: { toString(): string }; cid: CID; has: boolean }
}) => void
interface PresenceTarget {
  addEventListener(type: "presence", listener: PresenceListener): void
  removeEventListener(type: "presence", listener: PresenceListener): void
}

export interface IPFSConfig {
  logger: BaseLogger
  peerMultiaddrs: string[]
}

// A content router that finds nothing. bitswap's want() always kicks off a
// provider lookup (network.findAndConnect -> routing.findProviders); with an
// empty routers list that lookup throws NoRoutersAvailableError, which bitswap
// logs as an error on every fetch. Supplying this router keeps the lookup a
// no-op — zero providers, no DHT or HTTP gateway query — so blocks are served
// only by the connected whitelisted peer(s), with no spurious error log.
const noopRouting = {
  async *findProviders(): AsyncGenerator<never> {},
  toString: () => "NoopRouter()",
}

// Peer id embedded in a multiaddr string, i.e. the value after `/p2p/`.
function peerIdFromMultiaddr(addr: string): string | undefined {
  return addr.match(/\/p2p\/([^/]+)/)?.[1]
}

function concatBytes(chunks: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(chunks.reduce((n, c) => n + c.length, 0))
  let offset = 0
  for (const chunk of chunks) {
    out.set(chunk, offset)
    offset += chunk.length
  }
  return out
}

export class IPFSClient {
  private config: IPFSConfig
  private helia?: Helia
  private bitswapWantList?: PresenceTarget

  constructor(config: IPFSConfig) {
    this.config = config
  }

  async initialize(): Promise<void> {
    this.config.logger.info("Initializing IPFS P2P client...", {
      peerMultiaddrs: this.config.peerMultiaddrs,
    })

    try {
      await this.initializeHeliaP2P()
      this.config.logger.success("IPFS client initialized successfully")
    } catch (error) {
      this.config.logger.error("Failed to initialize IPFS client", error)
      throw error
    }
  }

  private async initializeHeliaP2P(): Promise<void> {
    this.config.logger.debug("Creating minimal Helia P2P node with libp2p...")
    this.config.logger.info(
      "Starting a libp2p node restricted to bitswap against whitelisted peers (no DHT/bootstrap/gateway)",
    )

    // Extract peer IDs from provided multiaddrs
    const allowedPeerIds = new Set<string>()
    for (const addr of this.config.peerMultiaddrs) {
      const peerId = peerIdFromMultiaddr(addr)
      if (peerId) {
        allowedPeerIds.add(peerId)
        this.config.logger.debug(`Whitelisted peer: ${peerId}`)
      } else {
        this.config.logger.warning(`No peer ID found in multiaddr: ${addr}`)
      }
    }

    this.config.logger.info(
      `Connection gater: Only allowing ${allowedPeerIds.size} whitelisted peer(s)`,
    )

    // Start from Helia's default libp2p config, then strip everything that
    // reaches out to the wider network. We only want bitswap against the
    // whitelisted peer(s) — no public-network chatter.
    const libp2p = libp2pDefaults()

    // No inbound: this is a fetch-only client, so it never needs to listen,
    // advertise addresses, or map ports.
    libp2p.addresses = { listen: [] }

    // No automatic peer discovery (mdns LAN scan + dialing public IPFS
    // bootstrap nodes).
    libp2p.peerDiscovery = []

    // Remove services that probe the network or call external HTTP endpoints.
    // upnp is what emits the "M-SEARCH for gateways" logs; the rest (autoNAT,
    // dcutr, dht, delegatedRouting, relay, auto-tls, http) all assume a node
    // participating in the public DHT/relay mesh, which we explicitly are not.
    // identify (+push) is kept because bitswap relies on it to learn that a
    // connected peer speaks the bitswap protocol; ping/keychain are local.
    const services = libp2p.services as unknown as Record<string, unknown>
    for (const name of [
      "upnp",
      "autoNAT",
      "autoTLS",
      "dcutr",
      "dht",
      "delegatedRouting",
      "relay",
      "http",
    ]) {
      delete services[name]
    }

    // Only allow dials to whitelisted peers.
    libp2p.connectionGater = {
      denyDialMultiaddr: async (maAddr) => {
        const addr = maAddr.toString()
        const peerId = peerIdFromMultiaddr(addr)
        if (peerId && allowedPeerIds.has(peerId)) {
          this.config.logger.debug(`Allowing whitelisted peer: ${addr}`)
          return false // false = don't deny = allow
        }
        this.config.logger.warning(
          `Blocking non-whitelisted connection: ${addr}`,
        )
        return true // true = deny
      },
    }

    // Wrap the bitswap block broker so we can grab a reference to the
    // underlying @helia/bitswap instance. Its want-list emits 'presence' events
    // that tell us which peers answered DoNotHave — see fetchBlock().
    const makeBitswapBroker = bitswap()
    const captureBitswapBroker: typeof makeBitswapBroker = (components) => {
      const broker = makeBitswapBroker(components)
      this.bitswapWantList = (
        broker as unknown as { bitswap?: { wantList?: PresenceTarget } }
      ).bitswap?.wantList
      if (this.bitswapWantList == null) {
        // Internal shape changed in a Helia upgrade; fetches still work but lose
        // the early DoNotHave abort and fall back to the timeout.
        this.config.logger.warning(
          "bitswap want-list not found; DoNotHave early-abort disabled (fetches wait for the timeout)",
        )
      }
      return broker
    }

    // Create the Helia node:
    // - blockBrokers is bitswap() only (no trustlessGateway), so blocks are
    //   fetched solely over libp2p and never from a public HTTP IPFS gateway.
    // - routers is the no-op router (not Helia's default httpGatewayRouting(),
    //   which would hand bitswap a list of public gateways as "providers").
    //   Bitswap still broadcasts wants to the connected whitelisted peer(s),
    //   which is the only source we want.
    // - blake2b-256 hasher is included for Polkadot/Substrate compatibility.
    this.helia = await createHelia({
      hashers: [blake2b256],
      blockBrokers: [captureBitswapBroker],
      routers: [noopRouting],
      libp2p,
    })

    const peerId = this.helia.libp2p.peerId.toString()
    this.config.logger.success("Helia P2P node created", {
      peerId: peerId,
    })

    // Log multiaddrs
    const multiaddrs = this.helia.libp2p
      .getMultiaddrs()
      .map((ma) => ma.toString())
    this.config.logger.network("Node listening on multiaddrs", {
      count: multiaddrs.length,
      addresses: multiaddrs,
    })

    // Connect to specified peers
    this.config.logger.info(
      `Connecting to ${this.config.peerMultiaddrs.length} peer(s)...`,
    )

    for (const addr of this.config.peerMultiaddrs) {
      try {
        this.config.logger.debug(`Dialing peer: ${addr}`)
        const ma = multiaddr(addr)
        await this.helia.libp2p.dial(ma)
        this.config.logger.success(`Connected to peer: ${addr}`)
      } catch (error) {
        this.config.logger.error(`Failed to connect to peer: ${addr}`, error)
        // Continue with other peers even if one fails
      }
    }

    // Log current connections
    const connections = this.helia.libp2p.getConnections()
    this.config.logger.network("Active connections", {
      count: connections.length,
      connections: connections.map((conn) => ({
        peer: conn.remotePeer.toString(),
        remoteAddr: conn.remoteAddr.toString(),
        direction: conn.direction,
      })),
    })
  }

  /**
   * Fetch raw bytes for a CID. Returns a Uint8Array with no encoding overhead.
   */
  async fetchRawBytes(cidString: string): Promise<Uint8Array> {
    const cid = this.parseCid(cidString)
    return this.fetchBlock(cid)
  }

  async fetchData(
    cidString: string,
  ): Promise<{ data: unknown; isJSON: boolean; rawHex: string }> {
    const cid = this.parseCid(cidString)
    const block = await this.fetchBlock(cid)
    const rawHex = toHex(block)

    const text = new TextDecoder().decode(block)
    const json = this.tryParseJSON(text)

    if (json.success) {
      this.config.logger.success("Data fetched and parsed as JSON successfully")
      this.config.logger.debug("JSON preview", {
        type: typeof json.data,
        keys:
          json.data && typeof json.data === "object"
            ? Object.keys(json.data)
            : undefined,
      })
      return { data: json.data, isJSON: true, rawHex }
    }

    this.config.logger.success("Data fetched successfully (raw bytes)")
    this.config.logger.debug("Raw data info", { bytes: block.length })
    return { data: text, isJSON: false, rawHex }
  }

  private parseCid(cidString: string): CID {
    this.config.logger.info(`Starting fetch for CID: ${cidString}`)

    try {
      const cid = CID.parse(cidString)
      const codecName = this.getCodecName(cid.code)

      this.config.logger.debug("CID parsed successfully", {
        version: cid.version,
        codec: `${codecName} (0x${cid.code.toString(16)})`,
        multihash: cid.multihash.toString(),
      })

      return cid
    } catch (error) {
      this.config.logger.error("Invalid CID format", error)
      throw new Error(
        `Invalid CID: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  private tryParseJSON(text: string): { success: boolean; data?: unknown } {
    try {
      const data = JSON.parse(text)
      return { success: true, data }
    } catch {
      return { success: false }
    }
  }

  /**
   * Fetch a raw block from the blockstore and return it as Uint8Array.
   */
  private async fetchBlock(cid: CID): Promise<Uint8Array> {
    if (!this.helia) {
      throw new Error("Helia not initialized")
    }

    this.config.logger.network("Fetching via Helia...")

    // Abort on whichever comes first: the timeout, or every connected peer
    // reporting DoNotHave (see watchDoNotHave). The signal is forwarded to
    // bitswap, so the fetch is cancelled and the want is retracted.
    const timeoutMs = 3000
    const controller = new AbortController()
    const timer = setTimeout(
      () =>
        controller.abort(
          new Error(`Timeout after ${timeoutMs / 1000}s waiting for data`),
        ),
      timeoutMs,
    )
    const stopWatching = this.watchDoNotHave(cid, controller)

    try {
      // blockstore.get is an async generator; a single block arrives as one
      // chunk, so avoid a second copy in that common case.
      const chunks: Uint8Array[] = []
      for await (const chunk of this.helia.blockstore.get(cid, {
        signal: controller.signal,
      })) {
        chunks.push(chunk)
      }

      const block = chunks.length === 1 ? chunks[0] : concatBytes(chunks)

      if (block.length === 0) {
        throw new Error("Block is empty - the peer may not have this CID")
      }

      this.config.logger.success(`Fetched ${block.length} bytes`)
      return block
    } catch (error) {
      // Surface the abort reason (timeout, or "no peer has it") rather than a
      // generic AbortError.
      if (controller.signal.aborted) {
        const reason = controller.signal.reason
        throw reason instanceof Error
          ? reason
          : new Error("Block not found - no connected peer has this CID")
      }
      throw error
    } finally {
      clearTimeout(timer)
      stopWatching()
    }
  }

  /**
   * Abort the fetch as soon as every peer we're connected to has answered
   * DoNotHave for `cid`. Because we only connect to the whitelisted peers, the
   * set of connections is exactly the set we're waiting on — once they've all
   * declined there is nothing left to wait for. Returns a cleanup function.
   */
  private watchDoNotHave(cid: CID, controller: AbortController): () => void {
    const wantList = this.bitswapWantList
    if (wantList == null || this.helia == null) {
      return () => {}
    }

    const pending = new Set(
      this.helia.libp2p
        .getConnections()
        .map((conn) => conn.remotePeer.toString()),
    )
    if (pending.size === 0) {
      return () => {}
    }

    const listener: PresenceListener = (evt) => {
      const { sender, cid: responded, has } = evt.detail
      if (
        has ||
        !bytesEquals(responded.multihash.digest, cid.multihash.digest)
      ) {
        return // only DoNotHave responses for this CID
      }
      pending.delete(sender.toString())
      this.config.logger.debug(`Peer reported DoNotHave: ${sender.toString()}`)
      if (pending.size === 0) {
        controller.abort(
          new Error("Block not found - no connected peer has this CID"),
        )
      }
    }

    wantList.addEventListener("presence", listener)
    return () => wantList.removeEventListener("presence", listener)
  }

  async stop(): Promise<void> {
    this.config.logger.info("Stopping IPFS client...")

    try {
      if (this.helia) {
        await this.helia.stop()
        this.config.logger.debug("Helia instance stopped")
      }

      this.helia = undefined

      this.config.logger.success("IPFS client stopped")
    } catch (error) {
      this.config.logger.error("Error stopping IPFS client", error)
      throw error
    }
  }

  private getCodecName(code: number): string {
    // Common IPFS codec codes
    // See: https://github.com/multiformats/multicodec/blob/master/table.csv
    const codecs: { [key: number]: string } = {
      85: "raw",
      112: "dag-pb",
      113: "dag-cbor",
      297: "dag-json",
      663: "dag-jose",
      133: "json",
    }

    return codecs[code] || `unknown (0x${code.toString(16)})`
  }
}
