// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

import { withBitswap } from "@helia/bitswap";
import { withLibp2p, type HeliaWithLibp2p } from "@helia/libp2p";
import { createHeliaLight } from "helia";
import { CID } from "multiformats/cid";
import { multiaddr } from "@multiformats/multiaddr";
import { blake2b256 } from "@multiformats/blake2/blake2b";
import { sha256 } from "multiformats/hashes/sha2";
import { from as hasherFrom } from "multiformats/hashes/hasher";
import { keccak_256 } from "@noble/hashes/sha3.js";

const keccak256Hasher = hasherFrom({
  name: "keccak-256",
  code: 0x1b,
  encode: (input: Uint8Array) => keccak_256(input),
});

export interface HeliaClientConfig {
  peerMultiaddrs: string[];
  onLog?: (level: "info" | "debug" | "error" | "success", message: string, data?: unknown) => void;
}

export interface ConnectionInfo {
  peerId: string;
  remoteAddr: string;
  direction: string;
}

export interface FetchResult {
  data: Uint8Array;
  isJSON: boolean;
  parsedJSON?: unknown;
}

export class HeliaClient {
  private config: HeliaClientConfig;
  private helia?: HeliaWithLibp2p;
  private connectedPeers: ConnectionInfo[] = [];

  constructor(config: HeliaClientConfig) {
    this.config = config;
  }

  private log(level: "info" | "debug" | "error" | "success", message: string, data?: unknown) {
    if (this.config.onLog) {
      this.config.onLog(level, message, data);
    } else {
      const prefix = { info: "INFO", debug: "DEBUG", error: "ERROR", success: "OK" }[level];
      console.log(`[${prefix}] ${message}`, data ?? "");
    }
  }

  async initialize(): Promise<{ peerId: string; connections: ConnectionInfo[] }> {
    this.log("info", "Initializing Helia P2P client...");

    // Extract peer IDs from provided multiaddrs for whitelist
    const allowedPeerIds = new Set<string>();
    for (const addr of this.config.peerMultiaddrs) {
      const match = addr.match(/\/p2p\/([^/]+)/);
      if (match && match[1]) {
        allowedPeerIds.add(match[1]);
        this.log("debug", `Whitelisted peer: ${match[1]}`);
      }
    }

    this.log("info", `Connection gater: allowing ${allowedPeerIds.size} whitelisted peer(s)`);

    // Create Helia node with blake2b256 hasher for Polkadot/Substrate CID compatibility
    this.helia = withBitswap(
      withLibp2p(createHeliaLight({ hashers: [blake2b256, sha256, keccak256Hasher] }), {
        connectionGater: {
          denyDialMultiaddr: async (maAddr) => {
            const addr = maAddr.toString();
            const match = addr.match(/\/p2p\/([^/]+)/);
            if (match && match[1] && allowedPeerIds.has(match[1])) {
              return false; // Allow whitelisted peers
            }
            return true; // Deny all others
          },
        },
      }),
    );
    await this.helia.start();

    const peerId = this.helia.libp2p.peerId.toString();
    this.log("success", `Helia node created with peer ID: ${peerId}`);

    // Connect to specified peers
    this.log("info", `Connecting to ${this.config.peerMultiaddrs.length} peer(s)...`);

    for (const addr of this.config.peerMultiaddrs) {
      try {
        this.log("debug", `Dialing peer: ${addr}`);
        const ma = multiaddr(addr);
        await this.helia.libp2p.dial(ma);
        this.log("success", `Connected to peer: ${addr}`);
      } catch (error) {
        this.log("error", `Failed to connect to peer: ${addr}`, error);
      }
    }

    // Get connection info
    const connections = this.helia.libp2p.getConnections();
    this.connectedPeers = connections.map((conn) => ({
      peerId: conn.remotePeer.toString(),
      remoteAddr: conn.remoteAddr.toString(),
      direction: conn.direction,
    }));

    this.log("success", `Connected to ${this.connectedPeers.length} peer(s)`);

    return { peerId, connections: this.connectedPeers };
  }

  async fetchData(cidOrString: string | CID): Promise<FetchResult> {
    if (!this.helia) {
      throw new Error("Helia not initialized");
    }

    let cid: CID;
    if (typeof cidOrString === "string") {
      try {
        cid = CID.parse(cidOrString);
      } catch (error) {
        throw new Error(`Invalid CID: ${error instanceof Error ? error.message : String(error)}`);
      }
    } else {
      cid = cidOrString;
    }

    this.log("info", `Fetching CID: ${cid.toString()}`);
    this.log("debug", `CID parsed: version=${cid.version}, codec=0x${cid.code.toString(16)}`);

    this.log("debug", "Requesting block from blockstore...");
    const chunks: Uint8Array[] = [];
    const timeoutMs = 30000;
    try {
      for await (const chunk of this.helia.blockstore.get(cid, {
        signal: AbortSignal.timeout(timeoutMs),
      })) {
        chunks.push(chunk);
      }
    } catch (error) {
      if (error instanceof Error && (error.name === "TimeoutError" || error.name === "AbortError")) {
        throw new Error(`Timeout after ${timeoutMs / 1000}s`);
      }
      throw error;
    }

    if (chunks.length === 0) {
      throw new Error("No data received from peer");
    }

    const totalLength = chunks.reduce((acc, chunk) => acc + chunk.length, 0);
    const data = new Uint8Array(totalLength);
    let offset = 0;
    for (const chunk of chunks) {
      data.set(chunk, offset);
      offset += chunk.length;
    }

    this.log("success", `Fetched ${data.length} bytes`);

    // Try to parse as JSON
    try {
      const text = new TextDecoder().decode(data);
      const parsed = JSON.parse(text);
      return { data, isJSON: true, parsedJSON: parsed };
    } catch {
      return { data, isJSON: false };
    }
  }

  getConnections(): ConnectionInfo[] {
    return this.connectedPeers;
  }

  isInitialized(): boolean {
    return !!this.helia;
  }

  async stop(): Promise<void> {
    if (this.helia) {
      await this.helia.stop();
      this.helia = undefined;
      this.connectedPeers = [];
      this.log("info", "Helia client stopped");
    }
  }
}
