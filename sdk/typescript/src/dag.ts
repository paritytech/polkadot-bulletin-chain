// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * DAG-PB (Directed Acyclic Graph - Protocol Buffers) utilities
 * for creating IPFS-compatible manifests
 */

import * as dagPB from "@ipld/dag-pb"
import { UnixFS } from "ipfs-unixfs"
import type { CID } from "multiformats/cid"
import {
  BulletinError,
  type Chunk,
  CidCodec,
  ErrorCode,
  HashAlgorithm,
} from "./types.js"
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
        ErrorCode.EMPTY_DATA,
      )
    }

    // Ensure all chunks have CIDs
    const chunkCids = chunks.map((chunk) => {
      if (!chunk.cid) {
        throw new BulletinError(
          `Chunk at index ${chunk.index} does not have a CID`,
          ErrorCode.DAG_ENCODING_FAILED,
        )
      }
      return chunk.cid
    })
    const chunkSizes = chunks.map((chunk) => chunk.data.length)
    return this.buildFromParts(chunkCids, chunkSizes, hashAlgorithm)
  }

  /**
   * Build a manifest from chunk CIDs + sizes alone — no chunk data needed.
   * Lets the streaming plan path produce a manifest without holding the file
   * in memory (CIDs are ~36 bytes each).
   */
  async buildFromParts(
    chunkCids: CID[],
    chunkSizes: number[],
    hashAlgorithm: HashAlgorithm = HashAlgorithm.Blake2b256,
  ): Promise<DagManifest> {
    if (chunkCids.length === 0) {
      throw new BulletinError(
        "Cannot build DAG from empty chunks",
        ErrorCode.EMPTY_DATA,
      )
    }
    if (chunkCids.length !== chunkSizes.length) {
      throw new BulletinError(
        "chunkCids and chunkSizes length mismatch",
        ErrorCode.DAG_ENCODING_FAILED,
      )
    }

    const totalSize = chunkSizes.reduce((sum, n) => sum + n, 0)
    const blockSizes = chunkSizes.map((n) => BigInt(n))

    // Build UnixFS file metadata (no inline data here)
    const fileData = new UnixFS({
      type: "file",
      blockSizes,
    })

    // DAG-PB node: our file with chunk links
    const dagNode = dagPB.prepare({
      Data: fileData.marshal(),
      Links: chunkCids.map((cid, i) => ({
        Name: "",
        Tsize: chunkSizes[i],
        Hash: cid,
      })),
    })

    // Encode DAG-PB
    const dagBytes = dagPB.encode(dagNode)

    // Calculate root CID using DAG-PB codec
    const rootCid = await calculateCid(dagBytes, CidCodec.DagPb, hashAlgorithm)

    return {
      rootCid,
      chunkCids,
      totalSize,
      dagBytes,
    }
  }

  /**
   * Parse a DAG-PB manifest back into its components
   */
  async parse(dagBytes: Uint8Array): Promise<{
    chunkCids: CID[]
    totalSize: number
  }> {
    try {
      const dagNode = dagPB.decode(dagBytes)

      if (!dagNode.Data) {
        throw new Error("DAG node has no data")
      }

      const unixfs = UnixFS.unmarshal(dagNode.Data)

      if (unixfs.type !== "file") {
        throw new Error(`Expected file type, got ${unixfs.type}`)
      }

      const chunkCids = dagNode.Links.map((link) => link.Hash)
      const totalSize = unixfs.fileSize()

      return {
        chunkCids,
        totalSize: Number(totalSize),
      }
    } catch (error) {
      throw new BulletinError(
        `Failed to decode DAG-PB manifest: ${error}`,
        ErrorCode.DAG_ENCODING_FAILED,
        error,
      )
    }
  }
}
