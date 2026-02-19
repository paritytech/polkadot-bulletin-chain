/**
 * P2P IPFS fetch module — importable from plain JS test scripts.
 *
 * Uses Helia (from bulletin-helia's own node_modules) to connect directly
 * to Bulletin chain nodes via libp2p and fetch blocks by CID.
 *
 * Usage:
 *   import { fetchCidViaP2P } from './bulletin-helia/p2p_fetch.js';
 *   const data = await fetchCidViaP2P(peerMultiaddrs, cidString);
 */

import { createHelia } from 'helia';
import { CID } from 'multiformats/cid';
import { multiaddr } from '@multiformats/multiaddr';
import { blake2b256 } from '@multiformats/blake2/blake2b';

/**
 * Fetch raw bytes for a CID via direct P2P connection to Bulletin nodes.
 *
 * @param {string[]} peerMultiaddrs - Multiaddrs of Bulletin nodes to connect to
 * @param {string|CID} cidInput - CID to fetch (string or CID object)
 * @param {object} [options]
 * @param {number} [options.timeoutMs=60000] - Timeout in ms for the fetch
 * @returns {Promise<Buffer>} Raw bytes of the fetched block
 */
export async function fetchCidViaP2P(peerMultiaddrs, cidInput, options = {}) {
  const { timeoutMs = 60000 } = options;
  const cidString = typeof cidInput === 'string' ? cidInput : cidInput.toString();

  console.log(`🔗 P2P fetch: CID=${cidString}, peers=${peerMultiaddrs.length}`);

  // Extract peer IDs for connection gating
  const allowedPeerIds = new Set();
  for (const addr of peerMultiaddrs) {
    const match = addr.match(/\/p2p\/([^/]+)/);
    if (match && match[1]) {
      allowedPeerIds.add(match[1]);
    }
  }

  // Create Helia node with connection gating and blake2b-256 hasher
  const helia = await createHelia({
    hashers: [blake2b256],
    libp2p: {
      connectionGater: {
        denyDialMultiaddr: async (maAddr) => {
          const addr = maAddr.toString();
          const match = addr.match(/\/p2p\/([^/]+)/);
          if (match && match[1] && allowedPeerIds.has(match[1])) {
            return false; // allow
          }
          return true; // deny
        },
      },
    },
  });

  try {
    // Connect to peers
    for (const addr of peerMultiaddrs) {
      try {
        await helia.libp2p.dial(multiaddr(addr));
        console.log(`   ✅ Connected to ${addr}`);
      } catch (err) {
        console.warn(`   ⚠️ Failed to connect to ${addr}: ${err.message}`);
      }
    }

    const connections = helia.libp2p.getConnections();
    if (connections.length === 0) {
      throw new Error('Failed to connect to any peer');
    }

    // Fetch the block
    const cid = CID.parse(cidString);
    console.log(`   ⬇️ Fetching block...`);

    const blockData = await helia.blockstore.get(cid);

    // Convert to Uint8Array — blockstore may return async iterable
    let block;
    if (blockData instanceof Uint8Array) {
      block = blockData;
    } else if (typeof blockData === 'object' && Symbol.asyncIterator in Object(blockData)) {
      const chunks = [];

      const timeoutPromise = new Promise((_, reject) => {
        setTimeout(() => reject(new Error(
          `Timeout after ${timeoutMs / 1000}s waiting for data`
        )), timeoutMs);
      });

      const iterator = blockData[Symbol.asyncIterator]();
      let done = false;
      while (!done) {
        const result = await Promise.race([iterator.next(), timeoutPromise]);
        if (result.done) {
          done = true;
        } else {
          chunks.push(result.value);
        }
      }

      if (chunks.length === 0) {
        throw new Error('Block not found — peer may not have this CID');
      }

      const totalLength = chunks.reduce((acc, c) => acc + c.length, 0);
      block = new Uint8Array(totalLength);
      let offset = 0;
      for (const chunk of chunks) {
        block.set(chunk, offset);
        offset += chunk.length;
      }
    } else {
      block = new Uint8Array(blockData);
    }

    if (block.length === 0) {
      throw new Error('Block is empty — peer may not have this CID');
    }

    console.log(`   ✅ Fetched ${block.length} bytes`);
    return Buffer.from(block);
  } finally {
    await helia.stop();
  }
}
