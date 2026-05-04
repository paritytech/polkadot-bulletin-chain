---
name: update-sdk-docs
description: Update the SDK book documentation to reflect changes in the SDK source code
---

When SDK source code changes (Rust or TypeScript), update the corresponding book documentation to stay in sync.

If no arguments are passed, detect what changed by diffing the current branch against `main` and update all affected docs.
If arguments are passed (e.g., `rust`, `typescript`, `api-reference`), only update those sections.

## Steps

### 1. Detect SDK changes

Diff against the base branch to find what changed:

```bash
git diff main --name-only -- sdk/rust/src/ sdk/typescript/src/
```

Categorize changes:
- **New/removed public exports** (structs, classes, enums, traits, functions, constants)
- **Changed method signatures** (parameters, return types)
- **New/removed methods** on existing types
- **Changed enum variants**
- **Changed default values** in configs/options

### 2. Read the current SDK source

For **Rust SDK** changes, read:
- `sdk/rust/src/lib.rs` — root re-exports and constants
- `sdk/rust/src/prelude.rs` — prelude module
- `sdk/rust/src/client.rs` — BulletinClient
- `sdk/rust/src/transaction.rs` — TransactionClient (std)
- `sdk/rust/src/types.rs` — all shared types, enums, configs
- `sdk/rust/src/storage.rs` — StorageOperation, BatchStorageOperation
- `sdk/rust/src/chunker.rs` — Chunker trait, FixedSizeChunker
- `sdk/rust/src/dag.rs` — DagBuilder, UnixFsDagBuilder
- `sdk/rust/src/authorization.rs` — AuthorizationManager
- `sdk/rust/src/renewal.rs` — RenewalOperation, RenewalTracker
- `sdk/rust/src/cid.rs` — CID functions
- `sdk/rust/src/error.rs` — Error enum (if separate)

For **TypeScript SDK** changes, read:
- `sdk/typescript/src/index.ts` — all re-exports
- `sdk/typescript/src/client.ts` — AsyncBulletinClient
- `sdk/typescript/src/builder.ts` — StoreBuilder, CallBuilder, AuthCallBuilder
- `sdk/typescript/src/preparer.ts` — BulletinPreparer
- `sdk/typescript/src/mock.ts` — MockBulletinClient
- `sdk/typescript/src/chunker.ts` — FixedSizeChunker
- `sdk/typescript/src/dag.ts` — UnixFsDagBuilder
- `sdk/typescript/src/cid.ts` — CID utility functions
- `sdk/typescript/src/types.ts` — all types, enums, interfaces
- `sdk/typescript/src/error.ts` — BulletinError, ErrorCode
- `sdk/typescript/src/constants.ts` — constants

### 3. Update affected documentation pages

Map SDK changes to book pages:

| SDK area | Book pages to update |
|----------|---------------------|
| Client constructors/setup | `README.md`, `quickstart.md`, `{lang}/README.md`, `{lang}/basic-storage.md` |
| Store methods | `{lang}/basic-storage.md`, `{lang}/chunked-uploads.md` |
| Authorization methods | `{lang}/authorization.md` |
| Renewal methods | `{lang}/renewal.md` |
| Error types/codes | `{lang}/error-handling.md` |
| CID functions | `concepts/storage.md`, `{lang}/basic-storage.md` |
| Chunking/DAG | `{lang}/chunked-uploads.md`, `concepts/manifests.md` |
| Types/enums/constants | `{lang}/api-reference.md` |
| Any public API change | `{lang}/api-reference.md` |
| PAPI integration | `typescript/papi-integration.md` |
| Mock client | `rust/mock-testing.md` |
| no_std changes | `rust/no_std.md` |

Where `{lang}` is `typescript/` or `rust/` depending on which SDK changed.

### 4. Update rules

When updating documentation:

- **Code examples must compile/run**: Use actual SDK APIs with correct signatures. Never invent methods or parameters.
- **API reference pages are comprehensive**: Every public class, method, type, enum, constant must be listed in `{lang}/api-reference.md`.
- **Constructor examples must be complete**: Show all required parameters, not zero-arg constructors if the real constructor requires args.
- **Introduction and quickstart use the high-level clients**: TypeScript uses `AsyncBulletinClient`, Rust uses `TransactionClient`.
- **Keep existing page structure**: Don't reorganize sections unless the API change requires it.
- **Don't add features that don't exist**: If a method was removed, remove it from docs. If return types changed, update them.
- **Match the exact type names**: `StoreOptions` not `StoreOpts`, `HashAlgorithm` not `HashAlgo`, etc.

### 5. Verify

After updating, run:

```bash
cd docs/book && mdbook build
```

Fix any broken links or build errors.

### 6. Summary

After completing updates, provide a summary of:
- Which SDK files changed
- Which doc pages were updated
- What specific APIs were added/changed/removed in the docs
