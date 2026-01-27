"use strict";
var __create = Object.create;
var __defProp = Object.defineProperty;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __getProtoOf = Object.getPrototypeOf;
var __hasOwnProp = Object.prototype.hasOwnProperty;
var __export = (target, all) => {
  for (var name in all)
    __defProp(target, name, { get: all[name], enumerable: true });
};
var __copyProps = (to, from, except, desc) => {
  if (from && typeof from === "object" || typeof from === "function") {
    for (let key of __getOwnPropNames(from))
      if (!__hasOwnProp.call(to, key) && key !== except)
        __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
  }
  return to;
};
var __toESM = (mod, isNodeMode, target) => (target = mod != null ? __create(__getProtoOf(mod)) : {}, __copyProps(
  // If the importer is in node compatibility mode or this is not an ESM
  // file that has been converted to a CommonJS file using a Babel-
  // compatible transform (i.e. "__esModule" has not been set), then set
  // "default" to the CommonJS "module.exports" for node compatibility.
  isNodeMode || !mod || !mod.__esModule ? __defProp(target, "default", { value: mod, enumerable: true }) : target,
  mod
));
var __toCommonJS = (mod) => __copyProps(__defProp({}, "__esModule", { value: true }), mod);

// src/index.ts
var index_exports = {};
__export(index_exports, {
  AsyncBulletinClient: () => AsyncBulletinClient,
  AuthorizationScope: () => AuthorizationScope,
  BulletinClient: () => BulletinClient,
  BulletinError: () => BulletinError,
  CID: () => import_cid2.CID,
  CidCodec: () => CidCodec,
  DEFAULT_CHUNKER_CONFIG: () => DEFAULT_CHUNKER_CONFIG,
  DEFAULT_STORE_OPTIONS: () => DEFAULT_STORE_OPTIONS,
  FixedSizeChunker: () => FixedSizeChunker,
  HashAlgorithm: () => HashAlgorithm,
  MAX_CHUNK_SIZE: () => MAX_CHUNK_SIZE,
  PAPITransactionSubmitter: () => PAPITransactionSubmitter,
  UnixFsDagBuilder: () => UnixFsDagBuilder,
  VERSION: () => VERSION,
  batch: () => batch,
  bytesToHex: () => bytesToHex,
  calculateCid: () => calculateCid,
  calculateThroughput: () => calculateThroughput,
  cidFromBytes: () => cidFromBytes,
  cidToBytes: () => cidToBytes,
  convertCid: () => convertCid,
  createProgressTracker: () => createProgressTracker,
  deepClone: () => deepClone,
  estimateFees: () => estimateFees,
  formatBytes: () => formatBytes,
  formatThroughput: () => formatThroughput,
  getContentHash: () => getContentHash,
  hexToBytes: () => hexToBytes,
  isBrowser: () => isBrowser,
  isNode: () => isNode,
  isValidSS58: () => isValidSS58,
  limitConcurrency: () => limitConcurrency,
  measureTime: () => measureTime,
  optimalChunkSize: () => optimalChunkSize,
  parseCid: () => parseCid,
  reassembleChunks: () => reassembleChunks,
  retry: () => retry,
  sleep: () => sleep,
  truncate: () => truncate,
  validateChunkSize: () => validateChunkSize
});
module.exports = __toCommonJS(index_exports);

// src/types.ts
var CidCodec = /* @__PURE__ */ ((CidCodec2) => {
  CidCodec2[CidCodec2["Raw"] = 85] = "Raw";
  CidCodec2[CidCodec2["DagPb"] = 112] = "DagPb";
  CidCodec2[CidCodec2["DagCbor"] = 113] = "DagCbor";
  return CidCodec2;
})(CidCodec || {});
var HashAlgorithm = /* @__PURE__ */ ((HashAlgorithm2) => {
  HashAlgorithm2[HashAlgorithm2["Blake2b256"] = 45600] = "Blake2b256";
  HashAlgorithm2[HashAlgorithm2["Sha2_256"] = 18] = "Sha2_256";
  HashAlgorithm2[HashAlgorithm2["Keccak256"] = 27] = "Keccak256";
  return HashAlgorithm2;
})(HashAlgorithm || {});
var DEFAULT_CHUNKER_CONFIG = {
  chunkSize: 1024 * 1024,
  // 1 MiB
  maxParallel: 8,
  createManifest: true
};
var DEFAULT_STORE_OPTIONS = {
  cidCodec: 85 /* Raw */,
  hashingAlgorithm: 45600 /* Blake2b256 */,
  waitForFinalization: false
};
var AuthorizationScope = /* @__PURE__ */ ((AuthorizationScope2) => {
  AuthorizationScope2["Account"] = "Account";
  AuthorizationScope2["Preimage"] = "Preimage";
  return AuthorizationScope2;
})(AuthorizationScope || {});
var BulletinError = class extends Error {
  constructor(message, code, cause) {
    super(message);
    this.code = code;
    this.cause = cause;
    this.name = "BulletinError";
  }
};

// src/chunker.ts
var MAX_CHUNK_SIZE = 8 * 1024 * 1024;
var FixedSizeChunker = class {
  constructor(config) {
    this.config = { ...DEFAULT_CHUNKER_CONFIG, ...config };
    if (this.config.chunkSize <= 0) {
      throw new BulletinError(
        "Chunk size must be greater than 0",
        "INVALID_CONFIG"
      );
    }
    if (this.config.chunkSize > MAX_CHUNK_SIZE) {
      throw new BulletinError(
        `Chunk size ${this.config.chunkSize} exceeds maximum allowed size of ${MAX_CHUNK_SIZE}`,
        "CHUNK_TOO_LARGE"
      );
    }
  }
  /**
   * Split data into chunks
   */
  chunk(data) {
    if (data.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA");
    }
    const chunks = [];
    const totalChunks = this.numChunks(data.length);
    for (let i = 0; i < totalChunks; i++) {
      const start = i * this.config.chunkSize;
      const end = Math.min(start + this.config.chunkSize, data.length);
      const chunkData = data.slice(start, end);
      chunks.push({
        data: chunkData,
        index: i,
        totalChunks
      });
    }
    return chunks;
  }
  /**
   * Calculate the number of chunks needed for the given data size
   */
  numChunks(dataSize) {
    if (dataSize === 0) return 0;
    return Math.ceil(dataSize / this.config.chunkSize);
  }
  /**
   * Get the chunk size
   */
  get chunkSize() {
    return this.config.chunkSize;
  }
};
function reassembleChunks(chunks) {
  if (chunks.length === 0) {
    throw new BulletinError("Cannot reassemble empty chunks", "EMPTY_DATA");
  }
  for (let i = 0; i < chunks.length; i++) {
    if (chunks[i].index !== i) {
      throw new BulletinError(
        `Chunk index mismatch: expected ${i}, got ${chunks[i].index}`,
        "CHUNKING_FAILED"
      );
    }
  }
  const totalSize = chunks.reduce((sum, chunk) => sum + chunk.data.length, 0);
  const result = new Uint8Array(totalSize);
  let offset = 0;
  for (const chunk of chunks) {
    result.set(chunk.data, offset);
    offset += chunk.data.length;
  }
  return result;
}

// src/dag.ts
var dagPB = __toESM(require("@ipld/dag-pb"));
var import_ipfs_unixfs = require("ipfs-unixfs");

// src/utils.ts
var import_cid = require("multiformats/cid");
var digest = __toESM(require("multiformats/hashes/digest"));
var import_util_crypto = require("@polkadot/util-crypto");
async function getContentHash(data, hashAlgorithm) {
  switch (hashAlgorithm) {
    case 45600 /* Blake2b256 */: {
      return (0, import_util_crypto.blake2AsU8a)(data);
    }
    case 18 /* Sha2_256 */: {
      return (0, import_util_crypto.sha256AsU8a)(data);
    }
    case 27 /* Keccak256 */:
      throw new BulletinError(
        "Keccak256 hashing requires integration with the pallet via PAPI",
        "UNSUPPORTED_HASH_ALGORITHM"
      );
    default:
      throw new BulletinError(
        `Unsupported hash algorithm: ${hashAlgorithm}`,
        "INVALID_HASH_ALGORITHM"
      );
  }
}
async function calculateCid(data, cidCodec = 85, hashAlgorithm = 45600 /* Blake2b256 */) {
  try {
    const hash = await getContentHash(data, hashAlgorithm);
    const mh = digest.create(hashAlgorithm, hash);
    return import_cid.CID.createV1(cidCodec, mh);
  } catch (error) {
    throw new BulletinError(
      `Failed to calculate CID: ${error}`,
      "CID_CALCULATION_FAILED",
      error
    );
  }
}
function convertCid(cid, newCodec) {
  return import_cid.CID.createV1(newCodec, cid.multihash);
}
function parseCid(cidString) {
  try {
    return import_cid.CID.parse(cidString);
  } catch (error) {
    throw new BulletinError(
      `Failed to parse CID: ${error}`,
      "INVALID_CID",
      error
    );
  }
}
function cidFromBytes(bytes) {
  try {
    return import_cid.CID.decode(bytes);
  } catch (error) {
    throw new BulletinError(
      `Failed to decode CID from bytes: ${error}`,
      "INVALID_CID",
      error
    );
  }
}
function cidToBytes(cid) {
  return cid.bytes;
}
function hexToBytes(hex) {
  const cleanHex = hex.startsWith("0x") ? hex.slice(2) : hex;
  const bytes = new Uint8Array(cleanHex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(cleanHex.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}
function bytesToHex(bytes) {
  return "0x" + Array.from(bytes).map((b) => b.toString(16).padStart(2, "0")).join("");
}
function formatBytes(bytes, decimals = 2) {
  if (bytes === 0) return "0 Bytes";
  const k = 1024;
  const dm = decimals < 0 ? 0 : decimals;
  const sizes = ["Bytes", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(dm))} ${sizes[i]}`;
}
function validateChunkSize(size) {
  const MAX_CHUNK_SIZE2 = 8 * 1024 * 1024;
  if (size <= 0) {
    throw new BulletinError("Chunk size must be positive", "INVALID_CHUNK_SIZE");
  }
  if (size > MAX_CHUNK_SIZE2) {
    throw new BulletinError(
      `Chunk size ${formatBytes(size)} exceeds maximum ${formatBytes(MAX_CHUNK_SIZE2)}`,
      "CHUNK_TOO_LARGE"
    );
  }
}
function optimalChunkSize(dataSize) {
  const MIN_CHUNK_SIZE = 1024 * 1024;
  const MAX_CHUNK_SIZE2 = 4 * 1024 * 1024;
  const OPTIMAL_CHUNKS = 100;
  if (dataSize <= MIN_CHUNK_SIZE) {
    return dataSize;
  }
  const optimalSize = Math.floor(dataSize / OPTIMAL_CHUNKS);
  if (optimalSize < MIN_CHUNK_SIZE) {
    return MIN_CHUNK_SIZE;
  } else if (optimalSize > MAX_CHUNK_SIZE2) {
    return MAX_CHUNK_SIZE2;
  } else {
    return Math.floor(optimalSize / 1048576) * 1048576;
  }
}
function estimateFees(dataSize) {
  const BASE_FEE = 1000000n;
  const PER_BYTE_FEE = 100n;
  return BASE_FEE + BigInt(dataSize) * PER_BYTE_FEE;
}
async function retry(fn, options = {}) {
  const {
    maxRetries = 3,
    delayMs = 1e3,
    exponentialBackoff = true
  } = options;
  let lastError;
  for (let attempt = 0; attempt <= maxRetries; attempt++) {
    try {
      return await fn();
    } catch (error) {
      lastError = error;
      if (attempt < maxRetries) {
        const delay = exponentialBackoff ? delayMs * Math.pow(2, attempt) : delayMs;
        await sleep(delay);
      }
    }
  }
  throw lastError || new Error("Retry failed");
}
function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
function batch(array, size) {
  const batches = [];
  for (let i = 0; i < array.length; i += size) {
    batches.push(array.slice(i, i + size));
  }
  return batches;
}
async function limitConcurrency(tasks, limit) {
  const results = [];
  const executing = [];
  for (const task of tasks) {
    const promise = task().then((result) => {
      results.push(result);
    });
    executing.push(promise);
    if (executing.length >= limit) {
      await Promise.race(executing);
      const index = await Promise.race(
        executing.map((p, i) => p.then(() => i))
      );
      executing.splice(index, 1);
    }
  }
  await Promise.all(executing);
  return results;
}
function createProgressTracker(total) {
  let current = 0;
  return {
    get current() {
      return current;
    },
    get total() {
      return total;
    },
    get percentage() {
      return total > 0 ? current / total * 100 : 0;
    },
    increment(amount = 1) {
      current = Math.min(current + amount, total);
      return this.percentage;
    },
    set(value) {
      current = Math.max(0, Math.min(value, total));
      return this.percentage;
    },
    reset() {
      current = 0;
    },
    isComplete() {
      return current >= total;
    }
  };
}
async function measureTime(fn) {
  const start = Date.now();
  const result = await fn();
  const duration = Date.now() - start;
  return [result, duration];
}
function calculateThroughput(bytes, ms) {
  if (ms === 0) return 0;
  return bytes / ms * 1e3;
}
function formatThroughput(bytesPerSecond) {
  return `${formatBytes(bytesPerSecond)}/s`;
}
function isValidSS58(address) {
  const ss58Regex = /^[1-9A-HJ-NP-Za-km-z]{47,48}$/;
  return ss58Regex.test(address);
}
function truncate(str, maxLength, ellipsis = "...") {
  if (str.length <= maxLength) {
    return str;
  }
  const partLength = Math.floor((maxLength - ellipsis.length) / 2);
  return str.slice(0, partLength) + ellipsis + str.slice(-partLength);
}
function deepClone(obj) {
  return JSON.parse(JSON.stringify(obj));
}
function isNode() {
  return typeof process !== "undefined" && process.versions != null && process.versions.node != null;
}
function isBrowser() {
  return typeof window !== "undefined" && typeof window.document !== "undefined";
}

// src/dag.ts
var UnixFsDagBuilder = class {
  /**
   * Build a UnixFS DAG-PB file node from raw chunks
   */
  async build(chunks, hashAlgorithm = 45600 /* Blake2b256 */) {
    if (!chunks || chunks.length === 0) {
      throw new BulletinError("Cannot build DAG from empty chunks", "EMPTY_DATA");
    }
    const chunkCids = chunks.map((chunk) => {
      if (!chunk.cid) {
        throw new BulletinError(
          `Chunk at index ${chunk.index} does not have a CID`,
          "DAG_ENCODING_FAILED"
        );
      }
      return chunk.cid;
    });
    const totalSize = chunks.reduce((sum, chunk) => sum + chunk.data.length, 0);
    const blockSizes = chunks.map((chunk) => BigInt(chunk.data.length));
    const fileData = new import_ipfs_unixfs.UnixFS({
      type: "file",
      blockSizes
    });
    const dagNode = dagPB.prepare({
      Data: fileData.marshal(),
      Links: chunks.map((chunk, i) => ({
        Name: "",
        Tsize: chunk.data.length,
        Hash: chunkCids[i]
      }))
    });
    const dagBytes = dagPB.encode(dagNode);
    const rootCid = await calculateCid(dagBytes, 112, hashAlgorithm);
    return {
      rootCid,
      chunkCids,
      totalSize,
      dagBytes
    };
  }
  /**
   * Parse a DAG-PB manifest back into its components
   */
  async parse(dagBytes) {
    try {
      const dagNode = dagPB.decode(dagBytes);
      if (!dagNode.Data) {
        throw new Error("DAG node has no data");
      }
      const unixfs = import_ipfs_unixfs.UnixFS.unmarshal(dagNode.Data);
      if (unixfs.type !== "file") {
        throw new Error(`Expected file type, got ${unixfs.type}`);
      }
      const chunkCids = dagNode.Links.map((link) => link.Hash);
      const totalSize = unixfs.fileSize();
      return {
        chunkCids,
        totalSize: Number(totalSize)
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to parse DAG-PB manifest: ${error}`,
        "DAG_DECODING_FAILED",
        error
      );
    }
  }
};

// src/client.ts
var BulletinClient = class {
  constructor(config) {
    this.config = {
      endpoint: config.endpoint,
      defaultChunkSize: config.defaultChunkSize ?? 1024 * 1024,
      maxParallel: config.maxParallel ?? 8,
      createManifest: config.createManifest ?? true
    };
  }
  /**
   * Prepare a simple store operation (data < 8 MiB)
   *
   * Returns the data and its CID. Use PAPI to submit to TransactionStorage.store
   */
  async prepareStore(data, options) {
    if (data.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA");
    }
    const opts = { ...DEFAULT_STORE_OPTIONS, ...options };
    const cid = await calculateCid(
      data,
      opts.cidCodec ?? 85 /* Raw */,
      opts.hashingAlgorithm
    );
    return { data, cid };
  }
  /**
   * Prepare a chunked store operation for large files
   *
   * This chunks the data, calculates CIDs, and optionally creates a DAG-PB manifest.
   * Returns chunk data and manifest that can be submitted via PAPI.
   */
  async prepareStoreChunked(data, config, options, progressCallback) {
    if (data.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA");
    }
    const chunkerConfig = {
      ...DEFAULT_CHUNKER_CONFIG,
      chunkSize: config?.chunkSize ?? this.config.defaultChunkSize,
      maxParallel: config?.maxParallel ?? this.config.maxParallel,
      createManifest: config?.createManifest ?? this.config.createManifest
    };
    const opts = { ...DEFAULT_STORE_OPTIONS, ...options };
    const chunker = new FixedSizeChunker(chunkerConfig);
    const chunks = chunker.chunk(data);
    for (const chunk of chunks) {
      if (progressCallback) {
        progressCallback({
          type: "chunk_started",
          index: chunk.index,
          total: chunks.length
        });
      }
      try {
        chunk.cid = await calculateCid(
          chunk.data,
          opts.cidCodec ?? 85 /* Raw */,
          opts.hashingAlgorithm
        );
        if (progressCallback) {
          progressCallback({
            type: "chunk_completed",
            index: chunk.index,
            total: chunks.length,
            cid: chunk.cid
          });
        }
      } catch (error) {
        if (progressCallback) {
          progressCallback({
            type: "chunk_failed",
            index: chunk.index,
            total: chunks.length,
            error
          });
        }
        throw error;
      }
    }
    let manifest;
    if (chunkerConfig.createManifest) {
      if (progressCallback) {
        progressCallback({ type: "manifest_started" });
      }
      const builder = new UnixFsDagBuilder();
      const dagManifest = await builder.build(chunks, opts.hashingAlgorithm);
      manifest = {
        data: dagManifest.dagBytes,
        cid: dagManifest.rootCid
      };
      if (progressCallback) {
        progressCallback({
          type: "manifest_created",
          cid: dagManifest.rootCid
        });
      }
    }
    if (progressCallback) {
      progressCallback({
        type: "completed",
        manifestCid: manifest?.cid
      });
    }
    return { chunks, manifest };
  }
  /**
   * Estimate authorization needed for storing data
   *
   * Returns (num_transactions, total_bytes) needed for authorization
   */
  estimateAuthorization(dataSize) {
    const numChunks = Math.ceil(dataSize / this.config.defaultChunkSize);
    let transactions = numChunks;
    let bytes = dataSize;
    if (this.config.createManifest) {
      transactions += 1;
      bytes += numChunks * 10 + 1e3;
    }
    return { transactions, bytes };
  }
};

// src/transaction.ts
var PAPITransactionSubmitter = class {
  constructor(api, signer) {
    this.api = api;
    this.signer = signer;
  }
  async submitStore(data) {
    try {
      const tx = this.api.tx.TransactionStorage.store({ data });
      const result = await tx.signAndSubmit(this.signer);
      const finalized = await result.waitFor("finalized");
      return {
        blockHash: finalized.blockHash,
        txHash: finalized.txHash,
        blockNumber: finalized.blockNumber
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to submit store transaction: ${error}`,
        "TRANSACTION_FAILED",
        error
      );
    }
  }
  async submitAuthorizeAccount(who, transactions, bytes) {
    try {
      const tx = this.api.tx.TransactionStorage.authorize_account({
        who,
        transactions,
        bytes
      });
      const result = await tx.signAndSubmit(this.signer);
      const finalized = await result.waitFor("finalized");
      return {
        blockHash: finalized.blockHash,
        txHash: finalized.txHash,
        blockNumber: finalized.blockNumber
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to authorize account: ${error}`,
        "AUTHORIZATION_FAILED",
        error
      );
    }
  }
  async submitAuthorizePreimage(contentHash, maxSize) {
    try {
      const tx = this.api.tx.TransactionStorage.authorize_preimage({
        content_hash: contentHash,
        max_size: maxSize
      });
      const result = await tx.signAndSubmit(this.signer);
      const finalized = await result.waitFor("finalized");
      return {
        blockHash: finalized.blockHash,
        txHash: finalized.txHash,
        blockNumber: finalized.blockNumber
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to authorize preimage: ${error}`,
        "AUTHORIZATION_FAILED",
        error
      );
    }
  }
  async submitRenew(block, index) {
    try {
      const tx = this.api.tx.TransactionStorage.renew({ block, index });
      const result = await tx.signAndSubmit(this.signer);
      const finalized = await result.waitFor("finalized");
      return {
        blockHash: finalized.blockHash,
        txHash: finalized.txHash,
        blockNumber: finalized.blockNumber
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to renew: ${error}`,
        "TRANSACTION_FAILED",
        error
      );
    }
  }
  async submitRefreshAccountAuthorization(who) {
    try {
      const tx = this.api.tx.TransactionStorage.refresh_account_authorization({ who });
      const result = await tx.signAndSubmit(this.signer);
      const finalized = await result.waitFor("finalized");
      return {
        blockHash: finalized.blockHash,
        txHash: finalized.txHash,
        blockNumber: finalized.blockNumber
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to refresh authorization: ${error}`,
        "TRANSACTION_FAILED",
        error
      );
    }
  }
  async submitRefreshPreimageAuthorization(contentHash) {
    try {
      const tx = this.api.tx.TransactionStorage.refresh_preimage_authorization({
        content_hash: contentHash
      });
      const result = await tx.signAndSubmit(this.signer);
      const finalized = await result.waitFor("finalized");
      return {
        blockHash: finalized.blockHash,
        txHash: finalized.txHash,
        blockNumber: finalized.blockNumber
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to refresh preimage authorization: ${error}`,
        "TRANSACTION_FAILED",
        error
      );
    }
  }
  async submitRemoveExpiredAccountAuthorization(who) {
    try {
      const tx = this.api.tx.TransactionStorage.remove_expired_account_authorization({ who });
      const result = await tx.signAndSubmit(this.signer);
      const finalized = await result.waitFor("finalized");
      return {
        blockHash: finalized.blockHash,
        txHash: finalized.txHash,
        blockNumber: finalized.blockNumber
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to remove expired authorization: ${error}`,
        "TRANSACTION_FAILED",
        error
      );
    }
  }
  async submitRemoveExpiredPreimageAuthorization(contentHash) {
    try {
      const tx = this.api.tx.TransactionStorage.remove_expired_preimage_authorization({
        content_hash: contentHash
      });
      const result = await tx.signAndSubmit(this.signer);
      const finalized = await result.waitFor("finalized");
      return {
        blockHash: finalized.blockHash,
        txHash: finalized.txHash,
        blockNumber: finalized.blockNumber
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to remove expired preimage authorization: ${error}`,
        "TRANSACTION_FAILED",
        error
      );
    }
  }
};

// src/async-client.ts
var AsyncBulletinClient = class {
  constructor(submitter, config) {
    this.submitter = submitter;
    this.config = {
      defaultChunkSize: config?.defaultChunkSize ?? 1024 * 1024,
      maxParallel: config?.maxParallel ?? 8,
      createManifest: config?.createManifest ?? true
    };
  }
  /**
   * Store data on Bulletin Chain (simple, < 8 MiB)
   *
   * Handles the complete workflow:
   * 1. Calculate CID
   * 2. Submit transaction
   * 3. Wait for finalization
   */
  async store(data, options) {
    if (data.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA");
    }
    const opts = { ...DEFAULT_STORE_OPTIONS, ...options };
    const cid = await calculateCid(
      data,
      opts.cidCodec ?? 85 /* Raw */,
      opts.hashingAlgorithm
    );
    const receipt = await this.submitter.submitStore(data);
    return {
      cid,
      size: data.length,
      blockNumber: receipt.blockNumber
    };
  }
  /**
   * Store large data with automatic chunking and manifest creation
   *
   * Handles the complete workflow:
   * 1. Chunk the data
   * 2. Calculate CIDs for each chunk
   * 3. Submit each chunk as a separate transaction
   * 4. Create and submit DAG-PB manifest (if enabled)
   * 5. Return all CIDs and receipt information
   */
  async storeChunked(data, config, options, progressCallback) {
    if (data.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA");
    }
    const chunkerConfig = {
      ...DEFAULT_CHUNKER_CONFIG,
      chunkSize: config?.chunkSize ?? this.config.defaultChunkSize,
      maxParallel: config?.maxParallel ?? this.config.maxParallel,
      createManifest: config?.createManifest ?? this.config.createManifest
    };
    const opts = { ...DEFAULT_STORE_OPTIONS, ...options };
    const chunker = new FixedSizeChunker(chunkerConfig);
    const chunks = chunker.chunk(data);
    const chunkCids = [];
    for (const chunk of chunks) {
      if (progressCallback) {
        progressCallback({
          type: "chunk_started",
          index: chunk.index,
          total: chunks.length
        });
      }
      try {
        const cid = await calculateCid(
          chunk.data,
          opts.cidCodec ?? 85 /* Raw */,
          opts.hashingAlgorithm
        );
        chunk.cid = cid;
        await this.submitter.submitStore(chunk.data);
        chunkCids.push(cid);
        if (progressCallback) {
          progressCallback({
            type: "chunk_completed",
            index: chunk.index,
            total: chunks.length,
            cid
          });
        }
      } catch (error) {
        if (progressCallback) {
          progressCallback({
            type: "chunk_failed",
            index: chunk.index,
            total: chunks.length,
            error
          });
        }
        throw error;
      }
    }
    let manifestCid;
    if (chunkerConfig.createManifest) {
      if (progressCallback) {
        progressCallback({ type: "manifest_started" });
      }
      const builder = new UnixFsDagBuilder();
      const manifest = await builder.build(chunks, opts.hashingAlgorithm);
      await this.submitter.submitStore(manifest.dagBytes);
      manifestCid = manifest.rootCid;
      if (progressCallback) {
        progressCallback({
          type: "manifest_created",
          cid: manifest.rootCid
        });
      }
    }
    if (progressCallback) {
      progressCallback({
        type: "completed",
        manifestCid
      });
    }
    return {
      chunkCids,
      manifestCid,
      totalSize: data.length,
      numChunks: chunks.length
    };
  }
  /**
   * Authorize an account to store data
   *
   * Requires sudo/authorizer privileges
   */
  async authorizeAccount(who, transactions, bytes) {
    return this.submitter.submitAuthorizeAccount(who, transactions, bytes);
  }
  /**
   * Authorize a preimage (by content hash) to be stored
   *
   * Requires sudo/authorizer privileges
   */
  async authorizePreimage(contentHash, maxSize) {
    return this.submitter.submitAuthorizePreimage(contentHash, maxSize);
  }
  /**
   * Renew/extend retention period for stored data
   */
  async renew(block, index) {
    return this.submitter.submitRenew(block, index);
  }
  /**
   * Refresh an account authorization (extends expiry)
   *
   * Requires sudo/authorizer privileges
   */
  async refreshAccountAuthorization(who) {
    return this.submitter.submitRefreshAccountAuthorization(who);
  }
  /**
   * Refresh a preimage authorization (extends expiry)
   *
   * Requires sudo/authorizer privileges
   */
  async refreshPreimageAuthorization(contentHash) {
    return this.submitter.submitRefreshPreimageAuthorization(contentHash);
  }
  /**
   * Remove an expired account authorization
   */
  async removeExpiredAccountAuthorization(who) {
    return this.submitter.submitRemoveExpiredAccountAuthorization(who);
  }
  /**
   * Remove an expired preimage authorization
   */
  async removeExpiredPreimageAuthorization(contentHash) {
    return this.submitter.submitRemoveExpiredPreimageAuthorization(contentHash);
  }
  /**
   * Estimate authorization needed for storing data
   */
  estimateAuthorization(dataSize) {
    const numChunks = Math.ceil(dataSize / this.config.defaultChunkSize);
    let transactions = numChunks;
    let bytes = dataSize;
    if (this.config.createManifest) {
      transactions += 1;
      bytes += numChunks * 10 + 1e3;
    }
    return { transactions, bytes };
  }
};

// src/index.ts
var import_cid2 = require("multiformats/cid");
var VERSION = "0.1.0";
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  AsyncBulletinClient,
  AuthorizationScope,
  BulletinClient,
  BulletinError,
  CID,
  CidCodec,
  DEFAULT_CHUNKER_CONFIG,
  DEFAULT_STORE_OPTIONS,
  FixedSizeChunker,
  HashAlgorithm,
  MAX_CHUNK_SIZE,
  PAPITransactionSubmitter,
  UnixFsDagBuilder,
  VERSION,
  batch,
  bytesToHex,
  calculateCid,
  calculateThroughput,
  cidFromBytes,
  cidToBytes,
  convertCid,
  createProgressTracker,
  deepClone,
  estimateFees,
  formatBytes,
  formatThroughput,
  getContentHash,
  hexToBytes,
  isBrowser,
  isNode,
  isValidSS58,
  limitConcurrency,
  measureTime,
  optimalChunkSize,
  parseCid,
  reassembleChunks,
  retry,
  sleep,
  truncate,
  validateChunkSize
});
