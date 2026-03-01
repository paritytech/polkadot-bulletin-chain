// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * DAG-PB (Directed Acyclic Graph - Protocol Buffers) utilities
 * for creating IPFS-compatible manifests
 */

import * as dagPB from "@ipld/dag-pb"
import { UnixFS } from "ipfs-unixfs"
import type { CID } from "multiformats/cid"
import { BulletinError, type Chunk, HashAlgorithm } from "./types.js"
import { calculateCid } from "./utils.js"

/**
 * DAG-PB manifest representing a file composed of multiple chunks
 */
export interface DagManifest {
  /** The root CID of the manifest */
  rootCid: CID
  /** CIDs of all chunks in order */
  chunkCids: CID[]
  /** Total size of the file in bytes */
  totalSize: number
  /** Encoded DAG-PB bytes */
  dagBytes: Uint8Array
}

/**
 * UnixFS DAG-PB builder following IPFS UnixFS v1 specification
 */
export class UnixFsDagBuilder {
  /**
   * Build a UnixFS DAG-PB file node from raw chunks
   */
  async build(
    chunks: Chunk[],
    hashAlgorithm: HashAlgorithm = HashAlgorithm.Blake2b256,
  ): Promise<DagManifest> {
    if (!chunks || chunks.length === 0) {
      throw new BulletinError(
        "Cannot build DAG from empty chunks",
        "EMPTY_DATA",
      )
    }

    // Ensure all chunks have CIDs
    const chunkCids = chunks.map((chunk) => {
      if (!chunk.cid) {
        throw new BulletinError(
          `Chunk at index ${chunk.index} does not have a CID`,
          "DAG_ENCODING_FAILED",
        )
      }
      return chunk.cid
    })

    // Calculate total size and block sizes
    const totalSize = chunks.reduce((sum, chunk) => sum + chunk.data.length, 0)
    const blockSizes = chunks.map((chunk) => BigInt(chunk.data.length))

    // Build UnixFS file metadata (no inline data here)
    const fileData = new UnixFS({
      type: "file",
      blockSizes,
    })

    // DAG-PB node: our file with chunk links
    const dagNode = dagPB.prepare({
      Data: fileData.marshal(),
      Links: chunks.map((chunk, i) => ({
        Name: "",
        Tsize: chunk.data.length,
        Hash: chunkCids[i],
      })),
    })

    // Encode DAG-PB
    const dagBytes = dagPB.encode(dagNode)

    // Calculate root CID using DAG-PB codec
    const rootCid = await calculateCid(dagBytes, 0x70, hashAlgorithm)

    return {
      rootCid,
      chunkCids,
      totalSize,
      dagBytes,
    }
  }
}
