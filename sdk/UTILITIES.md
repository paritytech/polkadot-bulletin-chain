# Bulletin SDK Utility Functions

This document describes all utility functions available in both the Rust and TypeScript SDKs.

## Overview

Both SDKs provide a comprehensive set of utility functions to simplify common operations when working with the Bulletin Chain:

- **Data Conversion** - Hex/bytes conversion, formatting
- **Validation** - Chunk size, address validation
- **Optimization** - Optimal chunk size calculation
- **Progress Tracking** - Progress trackers for long operations
- **Performance** - Throughput calculation, execution time measurement
- **Async Helpers** - Retry logic, sleep, concurrency limiting
- **Array Operations** - Batching, chunking

## Rust SDK Utilities

### Module: `sdk/rust/src/utils.rs`

#### Data Conversion

##### `hex_to_bytes(hex: &str) -> Result<Vec<u8>>`
Convert hex string (with or without `0x` prefix) to bytes.

```rust
use bulletin_sdk_rust::utils::hex_to_bytes;

let bytes = hex_to_bytes("deadbeef")?;
// Vec<u8>: [0xde, 0xad, 0xbe, 0xef]
```

##### `bytes_to_hex(bytes: &[u8]) -> String`
Convert bytes to hex string (without `0x` prefix).

```rust
use bulletin_sdk_rust::utils::bytes_to_hex;

let hex = bytes_to_hex(&[0xde, 0xad, 0xbe, 0xef]);
// "deadbeef"
```

##### `format_cid(cid: &[u8]) -> String`
Format CID bytes as hex string with `0x` prefix.

```rust
use bulletin_sdk_rust::utils::format_cid;

let formatted = format_cid(&cid_bytes);
// "0x1220abcd..."
```

#### SS58 Address Conversion (std-only)

##### `ss58_to_account_id(ss58: &str) -> Result<AccountId32>`
Convert SS58 address to AccountId32.

```rust
use bulletin_sdk_rust::utils::ss58_to_account_id;

let account = ss58_to_account_id("5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY")?;
```

##### `account_id_to_ss58(account: &AccountId32, prefix: u16) -> String`
Convert AccountId32 to SS58 address.

```rust
use bulletin_sdk_rust::utils::account_id_to_ss58;

let ss58 = account_id_to_ss58(&account, 42); // 42 = Bulletin Chain SS58 prefix
```

#### Hashing

##### `hash_data(data: &[u8]) -> ContentHash`
Calculate Blake2b-256 hash of data.

```rust
use bulletin_sdk_rust::utils::hash_data;

let hash = hash_data(b"Hello, Bulletin!");
// [u8; 32]
```

#### Validation

##### `validate_chunk_size(size: u64) -> Result<()>`
Validate chunk size (0 < size <= 8 MiB).

```rust
use bulletin_sdk_rust::utils::validate_chunk_size;

validate_chunk_size(1_048_576)?; // OK
validate_chunk_size(10_000_000)?; // Error: exceeds 8 MiB
```

#### Optimization

##### `optimal_chunk_size(data_size: u64) -> u64`
Calculate optimal chunk size for given data size.

```rust
use bulletin_sdk_rust::utils::optimal_chunk_size;

let size = optimal_chunk_size(100_000_000); // 100 MB
// Returns 1_048_576 (1 MiB)
```

##### `estimate_fees(data_size: u64) -> u64`
Estimate transaction fees for data size.

```rust
use bulletin_sdk_rust::utils::estimate_fees;

let fees = estimate_fees(1_000_000); // 1 MB
```

#### Codec/Algorithm Names

##### `codec_name(codec: CidCodec) -> &'static str`
Get human-readable codec name.

```rust
use bulletin_sdk_rust::utils::codec_name;
use bulletin_sdk_rust::cid::CidCodec;

assert_eq!(codec_name(CidCodec::Raw), "raw");
assert_eq!(codec_name(CidCodec::DagPb), "dag-pb");
```

##### `hash_algorithm_name(algo: HashAlgorithm) -> &'static str`
Get human-readable hash algorithm name.

```rust
use bulletin_sdk_rust::utils::hash_algorithm_name;
use bulletin_sdk_rust::cid::HashAlgorithm;

assert_eq!(hash_algorithm_name(HashAlgorithm::Blake2b256), "blake2b-256");
```

#### Async Helpers (std-only)

##### `retry_async<F, Fut, T>(max_retries: u32, delay_ms: u64, f: F) -> Result<T>`
Retry async operation with exponential backoff.

```rust
use bulletin_sdk_rust::utils::retry_async;

let result = retry_async(3, 1000, || async {
    // Your async operation
    Ok(())
}).await?;
```

## TypeScript SDK Utilities

### Module: `sdk/typescript/src/utils.ts`

#### Data Conversion

##### `hexToBytes(hex: string): Uint8Array`
Convert hex string (with or without `0x` prefix) to bytes.

```typescript
import { hexToBytes } from '@bulletin/sdk';

const bytes = hexToBytes('deadbeef');
// Uint8Array([0xde, 0xad, 0xbe, 0xef])
```

##### `bytesToHex(bytes: Uint8Array): string`
Convert bytes to hex string with `0x` prefix.

```typescript
import { bytesToHex } from '@bulletin/sdk';

const hex = bytesToHex(new Uint8Array([0xde, 0xad, 0xbe, 0xef]));
// '0xdeadbeef'
```

##### `formatBytes(bytes: number, decimals?: number): string`
Format bytes as human-readable size.

```typescript
import { formatBytes } from '@bulletin/sdk';

formatBytes(1024); // '1.00 KB'
formatBytes(1048576); // '1.00 MB'
formatBytes(1073741824, 0); // '1 GB'
```

#### CID Operations

##### `parseCid(cidString: string): CID`
Parse CID from string.

```typescript
import { parseCid } from '@bulletin/sdk';

const cid = parseCid('bafkreiabcd1234...');
```

##### `formatCid(cid: CID, base?: string): string`
Format CID as string with optional multibase.

```typescript
import { formatCid } from '@bulletin/sdk';

formatCid(cid); // base58btc format
formatCid(cid, 'base32'); // base32 format
```

#### Validation

##### `validateChunkSize(size: number): void`
Validate chunk size (throws on invalid).

```typescript
import { validateChunkSize } from '@bulletin/sdk';

validateChunkSize(1_048_576); // OK
validateChunkSize(10_000_000); // Throws: exceeds 8 MiB
```

##### `isValidSS58(address: string): boolean`
Basic SS58 address format validation.

```typescript
import { isValidSS58 } from '@bulletin/sdk';

isValidSS58('5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY'); // true
isValidSS58('invalid'); // false
```

#### Optimization

##### `optimalChunkSize(dataSize: number): number`
Calculate optimal chunk size for given data size.

```typescript
import { optimalChunkSize } from '@bulletin/sdk';

const size = optimalChunkSize(100_000_000); // 100 MB
// Returns 1048576 (1 MiB)
```

##### `estimateFees(dataSize: number): bigint`
Estimate transaction fees for data size.

```typescript
import { estimateFees } from '@bulletin/sdk';

const fees = estimateFees(1_000_000); // 1 MB
```

#### Async Helpers

##### `retry<T>(fn, options?): Promise<T>`
Retry async operation with configurable backoff.

```typescript
import { retry } from '@bulletin/sdk';

const result = await retry(
  async () => await someOperation(),
  {
    maxRetries: 3,
    delayMs: 1000,
    exponentialBackoff: true
  }
);
```

##### `sleep(ms: number): Promise<void>`
Sleep for specified milliseconds.

```typescript
import { sleep } from '@bulletin/sdk';

await sleep(1000); // Wait 1 second
```

##### `limitConcurrency<T>(tasks, limit): Promise<T[]>`
Run promises with concurrency limit.

```typescript
import { limitConcurrency } from '@bulletin/sdk';

const tasks = urls.map(url => () => fetch(url));
const results = await limitConcurrency(tasks, 5); // Max 5 concurrent
```

#### Array Operations

##### `batch<T>(array: T[], size: number): T[][]`
Batch array into chunks.

```typescript
import { batch } from '@bulletin/sdk';

const items = [1, 2, 3, 4, 5];
const batches = batch(items, 2);
// [[1, 2], [3, 4], [5]]
```

#### Progress Tracking

##### `createProgressTracker(total: number)`
Create a progress tracker.

```typescript
import { createProgressTracker } from '@bulletin/sdk';

const tracker = createProgressTracker(100);

tracker.increment(25);
console.log(tracker.percentage); // 25

tracker.increment(25);
console.log(tracker.isComplete()); // false

tracker.set(100);
console.log(tracker.isComplete()); // true
```

#### Performance Measurement

##### `measureTime<T>(fn): Promise<[T, number]>`
Measure execution time of async function.

```typescript
import { measureTime } from '@bulletin/sdk';

const [result, duration] = await measureTime(async () => {
  return await someOperation();
});

console.log(`Operation took ${duration}ms`);
```

##### `calculateThroughput(bytes: number, ms: number): number`
Calculate throughput in bytes per second.

```typescript
import { calculateThroughput } from '@bulletin/sdk';

const bytesPerSecond = calculateThroughput(1_048_576, 1000); // 1 MB in 1 second
// 1048576 (bytes/s)
```

##### `formatThroughput(bytesPerSecond: number): string`
Format throughput as human-readable string.

```typescript
import { formatThroughput } from '@bulletin/sdk';

formatThroughput(1_048_576); // '1.00 MB/s'
```

#### String Operations

##### `truncate(str, maxLength, ellipsis?): string`
Truncate string with ellipsis.

```typescript
import { truncate } from '@bulletin/sdk';

truncate('bafkreiabcd1234567890', 15); // 'bafkr...67890'
truncate('longstring', 8, '--'); // 'lon--ing'
```

#### Environment Detection

##### `isNode(): boolean`
Check if running in Node.js environment.

```typescript
import { isNode } from '@bulletin/sdk';

if (isNode()) {
  // Use Node.js APIs
}
```

##### `isBrowser(): boolean`
Check if running in browser environment.

```typescript
import { isBrowser } from '@bulletin/sdk';

if (isBrowser()) {
  // Use browser APIs
}
```

## Usage in Prelude

### Rust

All utilities are available in the prelude:

```rust
use bulletin_sdk_rust::prelude::*;

let bytes = utils::hex_to_bytes("deadbeef")?;
let size = utils::optimal_chunk_size(data.len() as u64);
```

### TypeScript

All utilities are exported from the main module:

```typescript
import {
  hexToBytes,
  formatBytes,
  retry,
  createProgressTracker
} from '@bulletin/sdk';
```

## Best Practices

### When to Use Utilities

1. **Data Conversion**: Always use `hexToBytes`/`bytesToHex` for hex conversions
2. **Chunk Size**: Use `optimalChunkSize()` for large files, then validate with `validateChunkSize()`
3. **Retry Logic**: Use `retry()` for network operations that may fail transiently
4. **Progress Tracking**: Use `createProgressTracker()` for long-running uploads
5. **Performance**: Use `measureTime()` to profile operations during development

### Error Handling

Rust utilities return `Result<T>` - always handle errors:

```rust
match hex_to_bytes("invalid") {
    Ok(bytes) => { /* use bytes */ },
    Err(e) => { /* handle error */ },
}
```

TypeScript utilities throw errors - use try-catch:

```typescript
try {
  const bytes = hexToBytes('invalid');
} catch (error) {
  // handle error
}
```

### Performance Considerations

- `deepClone()` (TS): Only for JSON-serializable objects, not for large data
- `retry()`: Use reasonable retry counts to avoid excessive delays
- `limitConcurrency()`: Balance between speed and resource usage
- `formatBytes()`: Lightweight, safe for frequent calls

## Testing

All utilities are thoroughly tested:

- **Rust**: See `sdk/rust/src/utils.rs` (tests at bottom of file)
- **TypeScript**: See `sdk/typescript/test/unit/utils.test.ts`

Run tests:

```bash
# Rust
cargo test --lib utils

# TypeScript
npm run test:unit -- utils.test.ts
```

## Examples

### Complete Upload with Progress

```typescript
import {
  optimalChunkSize,
  createProgressTracker,
  measureTime,
  formatThroughput,
  calculateThroughput
} from '@bulletin/sdk';

async function uploadWithMetrics(data: Uint8Array) {
  const chunkSize = optimalChunkSize(data.length);
  const tracker = createProgressTracker(data.length);

  const [result, duration] = await measureTime(async () => {
    return await client.storeChunked(data, { chunkSize }, undefined, (event) => {
      if (event.type === 'chunk_completed') {
        tracker.increment(chunkSize);
        console.log(`Progress: ${tracker.percentage.toFixed(1)}%`);
      }
    });
  });

  const throughput = calculateThroughput(data.length, duration);
  console.log(`Upload complete: ${formatThroughput(throughput)}`);

  return result;
}
```

### Retry with Backoff

```rust
use bulletin_sdk_rust::utils::retry_async;

async fn upload_with_retry(client: &AsyncBulletinClient, data: Vec<u8>) -> Result<StoreResult> {
    retry_async(3, 1000, || async {
        client.store(data.clone(), StoreOptions::default()).await
    }).await
}
```
