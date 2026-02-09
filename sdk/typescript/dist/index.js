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
  MockBulletinClient: () => MockBulletinClient,
  MockStoreBuilder: () => MockStoreBuilder,
  StoreBuilder: () => StoreBuilder,
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
var HashAlgorithm = /* @__PURE__ */ ((HashAlgorithm4) => {
  HashAlgorithm4[HashAlgorithm4["Blake2b256"] = 45600] = "Blake2b256";
  HashAlgorithm4[HashAlgorithm4["Sha2_256"] = 18] = "Sha2_256";
  HashAlgorithm4[HashAlgorithm4["Keccak256"] = 27] = "Keccak256";
  return HashAlgorithm4;
})(HashAlgorithm || {});
var DEFAULT_CHUNKER_CONFIG = {
  chunkSize: 1024 * 1024,
  // 1 MiB (default)
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
var MAX_CHUNK_SIZE = 2 * 1024 * 1024;
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
  return `${(bytes / Math.pow(k, i)).toFixed(dm)} ${sizes[i]}`;
}
function validateChunkSize(size) {
  if (size <= 0) {
    throw new BulletinError(
      "Chunk size must be positive",
      "INVALID_CHUNK_SIZE"
    );
  }
  if (size > MAX_CHUNK_SIZE) {
    throw new BulletinError(
      `Chunk size ${formatBytes(size)} exceeds maximum ${formatBytes(MAX_CHUNK_SIZE)}`,
      "CHUNK_TOO_LARGE"
    );
  }
}
function optimalChunkSize(dataSize) {
  const MIN_CHUNK_SIZE = 1024 * 1024;
  const OPTIMAL_CHUNKS = 100;
  if (dataSize <= MIN_CHUNK_SIZE) {
    return dataSize;
  }
  const optimalSize = Math.floor(dataSize / OPTIMAL_CHUNKS);
  if (optimalSize < MIN_CHUNK_SIZE) {
    return MIN_CHUNK_SIZE;
  } else if (optimalSize > MAX_CHUNK_SIZE) {
    return MAX_CHUNK_SIZE;
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
  const { maxRetries = 3, delayMs = 1e3, exponentialBackoff = true } = options;
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
  const front = str.slice(0, Math.ceil((maxLength - ellipsis.length) / 2));
  const back = str.slice(-Math.floor((maxLength - ellipsis.length) / 2));
  return front + ellipsis + back;
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
      throw new BulletinError(
        "Cannot build DAG from empty chunks",
        "EMPTY_DATA"
      );
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
      createManifest: config.createManifest ?? true,
      chunkingThreshold: config.chunkingThreshold ?? 2 * 1024 * 1024,
      checkAuthorizationBeforeUpload: config.checkAuthorizationBeforeUpload ?? true
    };
  }
  /**
   * Prepare a simple store operation (data < 2 MiB)
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

// src/async-client.ts
var StoreBuilder = class {
  constructor(client, data) {
    this.client = client;
    this.options = { ...DEFAULT_STORE_OPTIONS };
    this.data = data instanceof Uint8Array ? data : data.asBytes();
  }
  /** Set the CID codec */
  withCodec(codec) {
    this.options.cidCodec = codec;
    return this;
  }
  /** Set the hash algorithm */
  withHashAlgorithm(algorithm) {
    this.options.hashingAlgorithm = algorithm;
    return this;
  }
  /** Set whether to wait for finalization */
  withFinalization(wait) {
    this.options.waitForFinalization = wait;
    return this;
  }
  /** Set custom store options */
  withOptions(options) {
    this.options = options;
    return this;
  }
  /** Set progress callback for chunked uploads */
  withCallback(callback) {
    this.callback = callback;
    return this;
  }
  /** Execute the store operation (signed transaction, uses account authorization) */
  async send() {
    return this.client.storeWithOptions(this.data, this.options, this.callback);
  }
  /**
   * Execute store operation as unsigned transaction (for preimage-authorized content)
   *
   * Use this when the content has been pre-authorized via `authorizePreimage()`.
   * Unsigned transactions don't require fees and can be submitted by anyone.
   *
   * @example
   * ```typescript
   * // First authorize the content hash
   * const hash = blake2b256(data);
   * await client.authorizePreimage(hash, BigInt(data.length));
   *
   * // Anyone can now store this content without fees
   * const result = await client.store(data).sendUnsigned();
   * ```
   */
  async sendUnsigned() {
    return this.client.storeWithPreimageAuth(this.data, this.options, this.callback);
  }
};
var AsyncBulletinClient = class {
  /**
   * Create a new async client with PAPI client and signer
   *
   * The PAPI client must be configured with the correct chain metadata
   * for your Bulletin Chain node.
   *
   * @param api - Configured PAPI TypedApi instance
   * @param signer - Polkadot signer for transaction signing
   * @param config - Optional client configuration
   */
  constructor(api, signer, config) {
    this.api = api;
    this.signer = signer;
    this.config = {
      defaultChunkSize: config?.defaultChunkSize ?? 1024 * 1024,
      // 1 MiB
      maxParallel: config?.maxParallel ?? 8,
      createManifest: config?.createManifest ?? true,
      chunkingThreshold: config?.chunkingThreshold ?? 2 * 1024 * 1024,
      // 2 MiB
      checkAuthorizationBeforeUpload: config?.checkAuthorizationBeforeUpload ?? true
    };
  }
  /**
   * Set the account for authorization checks
   *
   * If set and `checkAuthorizationBeforeUpload` is enabled, the client will
   * query authorization state before uploading and fail fast if insufficient.
   */
  withAccount(account) {
    this.account = account;
    return this;
  }
  /**
   * Store data on Bulletin Chain using builder pattern
   *
   * Returns a builder that allows fluent configuration of store options.
   *
   * @param data - Data to store (PAPI Binary or Uint8Array)
   *
   * @example
   * ```typescript
   * import { Binary } from 'polkadot-api';
   *
   * // Using PAPI's Binary class (recommended)
   * const result = await client
   *   .store(Binary.fromText('Hello, Bulletin!'))
   *   .withCodec(CidCodec.DagPb)
   *   .withHashAlgorithm('blake2b-256')
   *   .withCallback((event) => {
   *     console.log('Progress:', event);
   *   })
   *   .send();
   *
   * // Or with Uint8Array
   * const result = await client
   *   .store(new Uint8Array([1, 2, 3]))
   *   .send();
   * ```
   */
  store(data) {
    return new StoreBuilder(this, data);
  }
  /**
   * Store data with custom options (internal, used by builder)
   *
   * **Note**: This method is public for use by the builder but users should prefer
   * the builder pattern via `store()`.
   *
   * Automatically chunks data if it exceeds the configured threshold.
   */
  async storeWithOptions(data, options, progressCallback) {
    const dataBytes = data instanceof Uint8Array ? data : data.asBytes();
    if (dataBytes.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA");
    }
    if (dataBytes.length > this.config.chunkingThreshold) {
      return this.storeInternalChunked(
        dataBytes,
        void 0,
        options,
        progressCallback
      );
    } else {
      return this.storeInternalSingle(dataBytes, options);
    }
  }
  /**
   * Internal: Store data in a single transaction (no chunking)
   */
  async storeInternalSingle(data, options) {
    if (data.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA");
    }
    const opts = { ...DEFAULT_STORE_OPTIONS, ...options };
    const cid = await calculateCid(
      data,
      opts.cidCodec ?? 85 /* Raw */,
      opts.hashingAlgorithm
    );
    throw new BulletinError(
      "Direct PAPI integration not yet implemented - see examples for current usage patterns",
      "NOT_IMPLEMENTED"
    );
  }
  /**
   * Internal: Store data with chunking
   */
  async storeInternalChunked(data, config, options, progressCallback) {
    throw new BulletinError(
      "Chunked upload not yet implemented for direct PAPI integration",
      "NOT_IMPLEMENTED"
    );
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
   *
   * @param data - Data to store (PAPI Binary or Uint8Array)
   */
  async storeChunked(data, config, options, progressCallback) {
    const dataBytes = data instanceof Uint8Array ? data : data.asBytes();
    if (dataBytes.length === 0) {
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
    const chunks = chunker.chunk(dataBytes);
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
      totalSize: dataBytes.length,
      numChunks: chunks.length
    };
  }
  /**
   * Authorize an account to store data
   *
   * Requires sudo/authorizer privileges
   */
  async authorizeAccount(who, transactions, bytes) {
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
  /**
   * Authorize a preimage (by content hash) to be stored
   *
   * Requires sudo/authorizer privileges
   */
  async authorizePreimage(contentHash, maxSize) {
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
  /**
   * Renew/extend retention period for stored data
   */
  async renew(block, index) {
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
  /**
   * Store preimage-authorized content as unsigned transaction
   *
   * Use this for content that has been pre-authorized via `authorizePreimage()`.
   * Unsigned transactions don't require fees and can be submitted by anyone who
   * has the preauthorized content.
   *
   * @param data - The preauthorized content to store
   * @param options - Store options (codec, hashing algorithm, etc.)
   * @param progressCallback - Optional progress callback for chunked uploads
   *
   * @example
   * ```typescript
   * import { blake2b256 } from '@noble/hashes/blake2b';
   *
   * // First, authorize the content hash (requires sudo)
   * const data = Binary.fromText('Hello, Bulletin!');
   * const hash = blake2b256(data.asBytes());
   * await sudoClient.authorizePreimage(hash, BigInt(data.asBytes().length));
   *
   * // Anyone can now submit without fees
   * const result = await client.store(data).sendUnsigned();
   * ```
   */
  async storeWithPreimageAuth(data, options, progressCallback) {
    const dataBytes = data instanceof Uint8Array ? data : data.asBytes();
    if (dataBytes.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA");
    }
    if (dataBytes.length > this.config.chunkingThreshold) {
      throw new BulletinError(
        "Chunked unsigned transactions not yet supported. Use signed transactions for large files.",
        "UNSUPPORTED_OPERATION"
      );
    }
    const opts = { ...DEFAULT_STORE_OPTIONS, ...options };
    const cid = await calculateCid(
      dataBytes,
      opts.cidCodec ?? 85 /* Raw */,
      opts.hashingAlgorithm
    );
    try {
      const tx = this.api.tx.TransactionStorage.store({ data: dataBytes });
      const result = await tx.submit();
      const finalized = await result.waitFor("finalized");
      const storedEvent = finalized.events.find(
        (e) => e.type === "TransactionStorage" && e.value.type === "Stored"
      );
      const extrinsicIndex = storedEvent?.value.value?.index;
      const blockNumber = finalized.blockNumber;
      return {
        cid,
        size: dataBytes.length,
        blockNumber,
        extrinsicIndex,
        chunks: void 0
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to store with preimage auth: ${error}`,
        "TRANSACTION_FAILED",
        error
      );
    }
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

// src/mock-client.ts
var MockStoreBuilder = class {
  constructor(client, data) {
    this.client = client;
    this.options = { ...DEFAULT_STORE_OPTIONS };
    this.data = data instanceof Uint8Array ? data : data.asBytes();
  }
  /** Set the CID codec */
  withCodec(codec) {
    this.options.cidCodec = codec;
    return this;
  }
  /** Set the hash algorithm */
  withHashAlgorithm(algorithm) {
    this.options.hashingAlgorithm = algorithm;
    return this;
  }
  /** Set whether to wait for finalization */
  withFinalization(wait) {
    this.options.waitForFinalization = wait;
    return this;
  }
  /** Set custom store options */
  withOptions(options) {
    this.options = options;
    return this;
  }
  /** Set progress callback for chunked uploads */
  withCallback(callback) {
    this.callback = callback;
    return this;
  }
  /** Execute the mock store operation */
  async send() {
    return this.client.storeWithOptions(this.data, this.options, this.callback);
  }
};
var MockBulletinClient = class {
  /**
   * Create a new mock client with optional configuration
   */
  constructor(config) {
    /** Operations performed (for testing verification) */
    this.operations = [];
    this.config = {
      defaultChunkSize: config?.defaultChunkSize ?? 1024 * 1024,
      // 1 MiB
      maxParallel: config?.maxParallel ?? 8,
      createManifest: config?.createManifest ?? true,
      chunkingThreshold: config?.chunkingThreshold ?? 2 * 1024 * 1024,
      // 2 MiB
      checkAuthorizationBeforeUpload: config?.checkAuthorizationBeforeUpload ?? true,
      simulateAuthFailure: config?.simulateAuthFailure ?? false,
      simulateStorageFailure: config?.simulateStorageFailure ?? false
    };
  }
  /**
   * Set the account for authorization checks
   */
  withAccount(account) {
    this.account = account;
    return this;
  }
  /**
   * Get all operations performed by this client
   */
  getOperations() {
    return [...this.operations];
  }
  /**
   * Clear recorded operations
   */
  clearOperations() {
    this.operations = [];
  }
  /**
   * Store data using builder pattern
   *
   * @param data - Data to store (PAPI Binary or Uint8Array)
   */
  store(data) {
    return new MockStoreBuilder(this, data);
  }
  /**
   * Store data with custom options (internal, used by builder)
   */
  async storeWithOptions(data, options, _progressCallback) {
    const dataBytes = data instanceof Uint8Array ? data : data.asBytes();
    if (dataBytes.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA");
    }
    if (this.config.checkAuthorizationBeforeUpload && this.config.simulateAuthFailure) {
      throw new BulletinError(
        "Insufficient authorization: need 100 bytes, have 0 bytes",
        "INSUFFICIENT_AUTHORIZATION",
        { need: 100, available: 0 }
      );
    }
    if (this.config.simulateStorageFailure) {
      throw new BulletinError(
        "Simulated storage failure",
        "TRANSACTION_FAILED"
      );
    }
    const opts = { ...DEFAULT_STORE_OPTIONS, ...options };
    const cid = await calculateCid(
      dataBytes,
      opts.cidCodec ?? 85 /* Raw */,
      opts.hashingAlgorithm
    );
    this.operations.push({
      type: "store",
      dataSize: dataBytes.length,
      cid: cid.toString()
    });
    return {
      cid,
      size: dataBytes.length,
      blockNumber: 1
    };
  }
  /**
   * Authorize an account to store data
   */
  async authorizeAccount(who, transactions, bytes) {
    if (this.config.simulateAuthFailure) {
      throw new BulletinError(
        "Simulated authorization failure",
        "AUTHORIZATION_FAILED"
      );
    }
    this.operations.push({
      type: "authorize_account",
      who,
      transactions,
      bytes
    });
    return {
      blockHash: "0x0000000000000000000000000000000000000000000000000000000000000001",
      txHash: "0x0000000000000000000000000000000000000000000000000000000000000002",
      blockNumber: 1
    };
  }
  /**
   * Authorize a preimage to be stored
   */
  async authorizePreimage(contentHash, maxSize) {
    if (this.config.simulateAuthFailure) {
      throw new BulletinError(
        "Simulated authorization failure",
        "AUTHORIZATION_FAILED"
      );
    }
    this.operations.push({
      type: "authorize_preimage",
      contentHash,
      maxSize
    });
    return {
      blockHash: "0x0000000000000000000000000000000000000000000000000000000000000001",
      txHash: "0x0000000000000000000000000000000000000000000000000000000000000002",
      blockNumber: 1
    };
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
  MockBulletinClient,
  MockStoreBuilder,
  StoreBuilder,
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
