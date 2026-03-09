# TypeScript SDK Review

Review of `sdk/typescript/` as a standalone library intended for consumption by other projects.

Incorporates findings from [#282](https://github.com/paritytech/polkadot-bulletin-chain/issues/282) (sections 12–20).

---

## 1. Package Configuration (`package.json`)

**`exports` field nesting is wrong.** Condition keys (`import`/`require`) should be nested inside path keys, not the other way around:

```jsonc
// Current (wrong)
"exports": {
  "import": { ".": "./dist/esm/index.js" },
  "require": { ".": "./dist/cjs/index.js" }
}

// Correct
"exports": {
  ".": {
    "import": "./dist/esm/index.js",
    "require": "./dist/cjs/index.js",
    "types": "./dist/types/index.d.ts"
  }
}
```

**`polkadot-api` should be a peer dependency**, not a direct dependency. Consumers will have their own `polkadot-api` instance — duplicates cause `instanceof` failures and bloated bundles. Same for `multiformats`.

**Missing `prepare` script.** A library should have:
```json
"scripts": {
  "prepare": "npm run build",
  "prepublishOnly": "npm test && npm run lint"
}
```

**`tsconfig.json` issues:**
- ~~`"lib": ["ES2020", "DOM"]` — `DOM` shouldn't be in a library that targets Node.js~~ **Fixed:** Switched to `@total-typescript/tsconfig/bundler/no-dom/library` preset. The SDK's only DOM reference (`window.document` in `isBrowser()`) was rewritten to use `globalThis`.
- `"moduleResolution": "bundler"` — correct for tsup; hand-rolled config replaced with `@total-typescript/tsconfig/bundler/no-dom/library` preset

---

## 2. Three Client Classes, No Shared Interface

`AsyncBulletinClient`, `BulletinClient`, and `MockBulletinClient` have **different APIs**:
- `AsyncBulletinClient` has `store`, `storeChunked`, `renew`, `authorizeAccount`, `storeWithPreimageAuth`, `sendUnsigned`
- `BulletinClient` has `prepareStore`, `prepareStoreChunked` (no submission)
- `MockBulletinClient` is missing `storeChunked`, `renew`, `storeWithPreimageAuth`, `sendUnsigned`

**Fix:** Define a shared `IBulletinClient` interface. `MockBulletinClient` should implement the same interface for tests to be meaningful.

---

## 3. `BulletinClient` Is Confused

`BulletinClient` takes a `ClientConfig` with an `endpoint` but never connects. `maxParallel` is accepted but never used for parallelism. Its methods are pure functions that don't need a class:

```typescript
// Current
const client = new BulletinClient({ endpoint: "ws://..." });
const prepared = client.prepareStore(data, signer);

// Should be
import { prepareStore } from "@polkadot/bulletin-sdk";
const prepared = prepareStore(data, signer);
```

**Fix:** Export these as standalone functions, or make `BulletinClient` actually connect and submit.

---

## 4. `api: any` Destroys Type Safety

In `async-client.ts`, the constructor takes `api: any` despite `TypedApi` being imported:

```typescript
// Line 191 — current
constructor(api: any, signer: PolkadotSigner) { ... }

// Should be
constructor(api: TypedApi<typeof bulletin>, signer: PolkadotSigner) { ... }
```

With `any`, every `.tx.`, `.query.`, `.event.` call is untyped — no autocomplete, no compile-time checks, wrong pallet/method names only caught at runtime.

---

## 5. `signAndSubmitWithProgress` Returns `any`

The progress callback and return types are all `any`:

```typescript
// Current
async signAndSubmitWithProgress(tx: any, callback?: (event: any) => void): Promise<any>

// Should be
async signAndSubmitWithProgress(
  tx: Transaction<...>,
  callback?: (event: TransactionStatusEvent) => void
): Promise<TxFinalized>
```

This means consumers get zero type guidance on what events look like or what the result contains.

---

## 6. `authorizeAccount` Hardcodes Sudo

```typescript
// Line 747
const sudoTx = this.api.tx.Sudo.sudo({ call: authorizeTx.decodedCall });
```

This assumes all authorization goes through sudo. If governance or a multisig authorizes, this method is useless.

**Fix:** Accept an optional `wrapper` or let the caller compose:
```typescript
// Option A: Accept wrapper
authorizeAccount(account, options, wrapper?: (tx) => tx)

// Option B: Return raw tx, let caller wrap
const tx = client.prepareAuthorize(account, options);
const sudoTx = api.tx.Sudo.sudo({ call: tx.decodedCall });
```

---

## 7. `utils.ts` Kitchen Sink (518 lines)

Generic utilities that don't belong in a Bulletin Chain SDK:

| Function | Issue |
|---|---|
| `deepClone` | Uses `JSON.parse(JSON.stringify())` — breaks on `BigInt`, `Uint8Array`, `Map`, `Set`. Use `structuredClone()` (Node 18+) |
| `retry` | Generic, belongs in a utility library |
| `limitConcurrency` | Returns results in completion order, not input order — breaks `Promise.all` semantics |
| `hexToBytes` | Silently corrupts data: non-hex chars become `0`, odd-length strings drop last nibble (line 121-128) |
| `isValidSS58` | Regex-only, no checksum validation — name overpromises |
| `formatBytes` | Doesn't handle negative numbers (returns `"NaN undefined"`) |
| `estimateFees` | Uses hardcoded placeholder values — should be marked `@experimental` |
| `timeout` | 5-line generic utility |
| `debounce` | Generic |
| `groupBy` | Generic |

**Fix:** Delete generic utilities. Keep only Bulletin-specific helpers (CID computation, codec helpers). If consumers need `retry` or `debounce`, they have their own libraries.

---

## 8. Inconsistent Naming

| Current | Better |
|---|---|
| `storeInternalSingle` | `submitStore` |
| `storeInternalChunked` | `submitStoreChunked` |
| `storeWithPreimageAuth` | `storePreauthorized` |
| `signAndSubmitFinalized` | `submitAndWait` |
| `prepareStore` | `buildStoreTx` |

Internal methods are exposed as if public. Method names should describe what they do from the consumer's perspective.

---

## 9. `storeWithPreimageAuth` Is Speculative

Uses API patterns (`tx.submit()`, `result.waitFor("finalized")`) that may not exist in the current PAPI version. This looks like aspirational code rather than tested functionality.

**Fix:** Either implement against the actual PAPI API or remove and document as future work.

---

## 10. Chunked Store Ignores `waitFor`

In `storeInternalChunked` (line ~491), the `waitFor` option from `StoreOptions` is ignored — it always calls `signAndSubmitFinalized`, meaning chunked stores always wait for finalization regardless of what the user requested.

Single-item `storeInternalSingle` correctly respects the `waitFor` option.

**Fix:** Apply the same `waitFor` logic to chunked operations.

---

## 11. `estimateAuthorization` Duplicated 3x

The same pure function is copy-pasted into `BulletinClient`, `AsyncBulletinClient`, and `MockBulletinClient`. It's a pure computation with no dependencies on client state.

**Fix:** Export as a standalone function:
```typescript
export function estimateAuthorization(dataSize: number, options?: AuthOptions): AuthEstimate
```

---

## 12. Integration Tests Out of Sync

`test/integration/client.test.ts` is out of sync with the SDK API. It references nonexistent `PAPITransactionSubmitter`, uses wrong `store()` signatures, and calls undefined methods like `refreshAccountAuthorization`. The test file will not compile.

**Fix:** Rewrite integration tests against the current `AsyncBulletinClient` API, or delete and track as a follow-up.

---

## 13. `hexToBytes` Silently Corrupts Data

`utils.ts:121-128` — Beyond being a generic utility (#7), `hexToBytes` has a critical correctness bug: non-hex characters silently become `0` bytes, and odd-length hex strings silently drop the last nibble. No error is thrown on invalid input.

```typescript
hexToBytes("zzzz"); // Returns Uint8Array [0, 0] instead of throwing
hexToBytes("abc");  // Returns Uint8Array [171] — drops 'c'
```

**Fix:** Validate input, reject non-hex characters, reject odd-length strings. Or remove and use `@polkadot/util`'s `hexToU8a`.

---

## 14. Examples Reference Undefined Methods

`examples/complete-workflow.ts` references methods that don't exist on `AsyncBulletinClient`: `refreshAccountAuthorization`, `removeExpiredAccountAuthorization`, among others. These examples will fail at runtime.

`examples/simple-store.ts:35` has a placeholder `getTypedApi(/* your chain descriptors */)` that will also cause a runtime error. Examples should either work end-to-end or clearly document the required setup.

**Fix:** Update examples to match the current API surface, or remove and replace with a tested example in the README.

---

## 15. `setTimeout` Leak in `signAndSubmitWithProgress`

`async-client.ts:363-369` — The `setTimeout` used for progress tracking is never cleared on normal resolution. The timer handle leaks, which can delay Node.js process exit and cause flaky test hangs.

**Fix:** Store the timer handle and clear it in a `finally` block.

---

## 16. Chunk Submission Logic Duplicated

`async-client.ts:491-599` (`storeInternalChunked`) and `async-client.ts:613-735` (`storeChunked`) contain duplicated chunk submission logic. Changes to one must be mirrored in the other, which is error-prone.

**Fix:** Extract shared chunk submission into a private helper method.

---

## 17. DAG Builder Issues

Two issues in `dag.ts`:

1. **Magic number** (line 82) — Uses `0x70` instead of a named constant like `CidCodec.DagPb`. Makes the code harder to understand and audit.

2. **Precision loss** (line 113) — `Number(totalSize)` loses precision for files larger than `2^53` bytes. The return type should be `bigint` or the function should guard against unsafe integer conversion.

---

## 18. `BulletinError` Drops Error Cause

`types.ts:219` — `BulletinError` doesn't pass the `cause` option to `super()`. This breaks error chain inspection — consumers cannot access the original error via `error.cause`, making debugging harder.

```typescript
// Current
class BulletinError extends Error {
  constructor(message: string, public code: string, cause?: Error) {
    super(message); // cause is lost
  }
}

// Fix
super(message, { cause });
```

---

## 19. Hardcoded `VERSION` Drifts from `package.json`

`index.ts:58` — `VERSION = "0.1.0"` is hardcoded. This will inevitably drift from `package.json` on version bumps.

**Fix:** Import from `package.json` at build time, or use a build plugin (e.g., `tsup`'s `define`) to inject the version.

---

## 20. `chunker.ts` Copies Data Unnecessarily

`chunker.ts:57` — `data.slice()` creates a copy of the chunk data. `data.subarray()` returns a zero-copy view into the same buffer, which is significantly more efficient for large files.

---

## 21. Proposed Library Structure

```
sdk/typescript/
├── src/
│   ├── index.ts          # Curated public exports (not export *)
│   ├── client.ts         # Single connected client implementing IBulletinClient
│   ├── types.ts          # Public types and interfaces
│   ├── builder.ts        # Transaction builders (pure functions)
│   ├── chunker.ts        # Chunking logic (keep as-is, it's clean)
│   ├── dag.ts            # DAG builder (keep as-is)
│   ├── codec.ts          # Bulletin-specific encoding helpers
│   └── errors.ts         # Error types with codes
├── package.json          # Proper exports, peer deps, prepare script
├── tsconfig.json         # node16 resolution, no DOM
└── tsconfig.build.json   # Separate build config
```

**Key changes:**
- **One client class** with connected + offline modes instead of three different classes
- **Curated exports** — only types, client, and builder functions
- **Peer dependencies** — `polkadot-api`, `multiformats`
- **No generic utils** — delete `deepClone`, `retry`, `debounce`, etc.
- **Typed throughout** — no `any` in public API surface
- **`IBulletinClient` interface** — enables mocking via any test framework

---

## Priority Order

| Priority | Issue | Impact |
|---|---|---|
| **P0** | Fix `api: any` (#4) | Type safety across entire SDK |
| **P0** | Fix `exports` field (#1) | Library unusable with some bundlers |
| **P0** | Peer dependencies (#1) | Duplicate packages break `instanceof` |
| **P0** | `hexToBytes` silent data corruption (#13) | Correctness — wrong data stored on-chain |
| **P0** | Integration tests out of sync (#12) | Tests don't compile, false confidence |
| **P1** | Shared interface (#2) | Testability and API consistency |
| **P1** | Chunked `waitFor` (#10) | Behavioral bug |
| **P1** | Remove `utils.ts` bloat (#7) | Public API surface, bundle size |
| **P1** | Fix examples (#14) | Broken examples block adoption |
| **P1** | `BulletinError` drops cause (#18) | Debugging impossible |
| **P2** | Typed progress events (#5) | DX improvement |
| **P2** | Decouple sudo (#6) | Flexibility |
| **P2** | Deduplicate `estimateAuthorization` (#11) | Code hygiene |
| **P2** | `setTimeout` leak (#15) | Resource leak, flaky tests |
| **P2** | Deduplicate chunk submission (#16) | Maintainability |
| **P2** | DAG builder issues (#17) | Correctness for large files |
| **P3** | Naming consistency (#8) | Readability |
| **P3** | Remove speculative API (#9) | API clarity |
| **P3** | Flatten to one client (#3) | Architecture simplification |
| **P3** | Hardcoded `VERSION` (#19) | Version drift on release |
| **P3** | Zero-copy chunking (#20) | Performance for large files |
