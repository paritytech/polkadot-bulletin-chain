# Authorizations design

## Motivation

Two distinct problems shape the allowance design. They map onto the two limits in [Allowance Limits](#allowance-limits).

Block-throughput reference numbers:

| Parameter | Value |
|---|---|
| `MaxTransactionSize` | 2 MiB |
| `MaxBlockTransactions` | 512 |
| `MAX_BLOCK_LENGTH` × `NORMAL_DISPATCH_RATIO` (90%) | **~9 MiB / block** (binding constraint) |
| Block time | 6 s ⇒ 14 400 blocks/day |
| Sustained max throughput | **~127 GiB/day**, **~1.73 TiB / 2 weeks** |
| `RetentionPeriod` | `2 × 100 800` blocks = 14 days |
| `AuthorizationPeriod` | `14 days` (Westend / Paseo configs) |

### 1. Wasted block space (soft)

The chain has ~9 MiB of body capacity per block. If `store` rejects every call the moment a user crosses their per-account allowance, blocks frequently sit empty even when authorized users have data ready to send — capacity is left on the table.

Accept over-allowance `store` calls at a lower priority instead. In-budget users still go first; over-budget calls fill whatever block space is left. ⇒ motivates a **soft limit** on temporary-storage allowance, enforced by priority rather than rejection.

### 2. Unbounded renewed storage on collators (hard)

`renew` re-anchors an existing stored item: when the original entry's `RetentionPeriod` is about to elapse, a `renew` lands a fresh `Transactions[block]` entry pointing at the same content, and the *renewed* entry's own `RetentionPeriod` clock starts from that block. Repeat indefinitely and a single piece of content can stay on chain forever.

Without bounds, at sustained-peak block usage one window of fresh `store` data alone is ~1.73 TiB, and re-renewals stack on top. Crucially, content that is kept alive permanently is re-renewed every `RetentionPeriod` — each re-renewal must not double-count against the per-account quota, or the account would exhaust its allowance after just one cycle. ⇒ motivates a **hard limit** on renewed storage (per account and chain-wide), with re-renewal detection to avoid double-counting.

## Storage types

- **Temporary storage** — happens through the `store` call. Lives on chain for one `RetentionPeriod` from its `store` block.
- **Renewed storage** — happens through the `renew` call. The renewed entry itself also lives one `RetentionPeriod` (from its renewal block); the original `Transactions` entry it pointed at ages out on its own clock.

`store`, `store_with_cid_config`, `renew`, and `renew_content_hash` are unconditionally feeless. Authorization is the sole economic gate. Wrapper calls (e.g. `utility::batch`) are rejected by `ValidateStorageCalls`.

Each `TransactionInfo` is stamped with `kind: TransactionKind { Store, Renew }`. The kind is what `on_initialize`'s obsolete-block cleanup uses to tell which entries should decrement the chain-wide renewed-bytes counter when they age out — see [Hard limit on renewed storage](#hard-limit-on-renewed-storage).

## Allowance limits

PoP grants two numbers per account: `bytes_allowance` (size budget) and `transactions_allowance` (count budget). The same `bytes_allowance` is reused on the soft and hard sides, with different semantics.

- **Soft (temporary)** — `bytes_allowance` and `transactions_allowance` are priority thresholds only. The boost stays on while in-budget on both axes (`bytes <= bytes_allowance` *and* `transactions <= transactions_allowance`) and drops to `0` once *either* is strictly over cap. A missing or `0`-allowance grant also yields no boost. `store` calls are never rejected; they queue behind in-budget signers when over.
- **Hard (renewed)** — `bytes_allowance` is a real cap on renewed bytes for the account within the authorization window. `renew` is **rejected** when `bytes_permanent + size > bytes_allowance`. Re-renewing content that already has `kind == Renew` skips this check (see [Per-account quota](#per-account-quota)). The transaction-count axis does not gate renew. A separate chain-wide cap (`MaxPermanentStorageSize`) bounds the total renewed bytes on chain across all signers.

### Authorization storage

- One `AuthorizationExtent` per scope is kept in `Authorizations`, keyed by `AuthorizationScope::{Account, Preimage}`.
- `AuthorizationExtent { transactions, transactions_allowance, bytes, bytes_permanent, bytes_allowance }` holds the soft-side counters (`bytes`, `transactions`), the per-account renew usage (`bytes_permanent`), and the caps.
- `bytes` and `transactions` bump on `store` / `store_with_cid_config`. The `transactions` axis bumps on both store and renew, since both consume a transaction slot.
- `AccountRenewals` is a `StorageDoubleMap<AccountId, ContentHash, ()>` that tracks which (account, content_hash) pairs have already been charged `bytes_permanent` in the current authorization window. It is inserted on the first successful renew and checked for re-renewal detection. Cleared via `clear_prefix` when the authorization window resets (expired-but-present re-authorize path, or `remove_expired_authorization`).

`bytes_permanent` bumps on each first-time renewal per (account, content_hash) pair. The same account re-renewing the same content skips the `bytes_permanent` check and increment — the content was already counted in a prior renewal within this window. A different account renewing the same content is always charged, matching `store` semantics where each account pays independently. Without the re-renewal skip, content kept alive permanently (re-renewed every `RetentionPeriod`) would double-count against the quota on every cycle, eventually exhausting the allowance even though no new distinct content was added. The counter resets to zero when the authorization window expires and a fresh grant is issued.

### `authorize_account` semantics

Per existing entry state:

- **Unexpired**: caps are **additive** (`bytes_allowance += bytes`, `transactions_allowance += transactions`). Matches PoP's `claim_long_term_storage` flow, which calls this once per claim and expects each to extend the caps. Consumed counters are preserved, expiry is left untouched.
- **Expired-but-present**: caps are **re-granted** (`bytes_allowance = bytes`, `transactions_allowance = transactions`) and **all** consumed counters reset to `0`. The new window's renew quota is independent of the old window's renewals — the old data is still on chain and is tracked by the chain-wide `PermanentStorageUsed` counter, but it does not spend the new window's quota.
- **Missing**: create a fresh entry with all counters at `0`.

`authorize_preimage` follows the same shape, but `transactions_allowance` is fixed at `1` (a preimage grant is a single-shot store right) and the unexpired path **replaces** rather than adds.

### `refresh_account_authorization`

Extends `expiration` by another `AuthorizationPeriod` and leaves all consumed counters (`bytes`, `transactions`, `bytes_permanent`) untouched. Refresh does **not** grant additional capacity. To start a fresh window, let the authorization expire and re-authorize. Origin is `T::Authorizer` (e.g. PoP).

### `remove_expired_account_authorization`

Removes the authorization entry. The chain-wide `PermanentStorageUsed` counter is unaffected — renewed bytes still on chain are tracked by `Transactions` and aged out by `on_initialize`.

## Soft limit (priority boost)

Implemented by the [`AllowanceBasedPriority`][ext] transaction extension via a runtime-selected `BoostStrategy`:

- `check_authorization` saturates `bytes` and `transactions` upward and never rejects.
- The boost only applies to **signed account-scoped `store` / `store_with_cid_config`**. `renew` and preimage-scoped stores get `0`.
- The strategy is fed the **post-this-tx** extent so the decision is "would this leave the holder in-budget on both axes?".
- `FlatBoost` (default in both runtimes): `ALLOWANCE_PRIORITY_BOOST` while in-budget, `0` once over.
- `ProportionalBoost` (alternative): scales with the tighter of the byte- and tx-budget remainders.

In-budget `store` txs sort strictly above over-budget ones. Pool nonce / arrival ordering breaks ties.

[ext]: ../pallets/transaction-storage/src/extension.rs

## Hard limit on renewed storage

The hard cap is enforced at two levels, and a renewal that would breach **either** is rejected.

### Per-account quota

`renew` of `size` bytes for account `A` is rejected with `Error::PermanentAllowanceExceeded` when

```
bytes_permanent + size > A.bytes_allowance
```

`bytes_permanent` bumps on each first-time renewal per (account, content_hash) pair.

**Re-renewal detection is per-(account, content_hash)** via the `AccountRenewals` storage map, not per-entry `kind`. When the same account renews the same content again within the same authorization window, the `AccountRenewals` entry already exists and the `bytes_permanent` check and increment are skipped. A different account renewing the same content hash will NOT find an `AccountRenewals` entry (it only tracks their own renewals), so they are charged independently — matching `store` semantics where each account pays for the same content.

This is essential for content kept alive permanently: each `RetentionPeriod` the user must re-renew the same content, and without this skip, every cycle would double-count against the per-account quota, eventually exhausting the allowance even though no new distinct content was added.

The counter resets to zero when the authorization window expires and a fresh grant is issued (the expired-but-present re-authorize path). `AccountRenewals` entries are also cleared at that point via `clear_prefix`.

### Chain-wide cap

`renew` is rejected with `Error::ChainPermanentCapReached` when

```
PermanentStorageUsed + size > T::MaxPermanentStorageSize::get()
```

`PermanentStorageUsed` is bumped on every successful `renew` (including re-renewals of the same content, since each creates a new physical `Transactions` entry). It is decremented in `on_initialize` (mandatory weight, bounded by `MaxBlockTransactions`) when an obsolete block is removed: each `Transactions[obsolete][i]` with `kind == Renew` contributes its `size` to a single saturating decrement, then `Transactions[obsolete]` is removed.

That obsolete-block cleanup is the only path that ever decrements `PermanentStorageUsed`. `Transactions` is the authoritative record of which renewed bytes are still on chain; the counter is just a precomputed sum maintained alongside it.

`MaxPermanentStorageSize` is a `Config` trait constant. The runtime picks the backing — `parameter_types! { pub const … }` (runtime-upgrade only) or `parameter_types! { pub storage … }` (storage-backed; mutable at runtime via `system.set_storage`).

### Capacity planning signals

- `Event::PermanentStorageUsedUpdated { used }` fires once per change to `PermanentStorageUsed`: once per successful `renew` (increment), and once per obsolete-block cleanup that ages out at least one `kind == Renew` entry (decrement).
- `Event::PermanentStorageNearCap { used, cap }` fires on the rising edge across `PERMANENT_STORAGE_NEAR_CAP_PERCENT` (80%) of `MaxPermanentStorageSize`. Off-chain consumers can use this as a "raise the cap or coordinate another bulletin chain" trigger.

## Why renewed bytes can't grow unboundedly

Stated up front: at any block `n`, total renewed bytes on chain are bounded by `MaxPermanentStorageSize` (chain-wide cap). A single account's `bytes_permanent` is bounded by `bytes_allowance`. The actual on-chain footprint can temporarily exceed `bytes_allowance` because re-renewals create new physical `Transactions` entries (see Example 3), but re-renewals of the same content skip the per-account check so they don't block — this is critical for content kept alive permanently across multiple `RetentionPeriod` cycles.

Why: every renewed byte ages out exactly `RetentionPeriod` blocks after its renew block (the obsolete-block cleanup in `on_initialize`). New renews are gated by the chain-wide cap, so the counter can only enter the in-bounds region. As old data ages out, the cap recovers.

The examples below trace the counters block-by-block to make the bound visible.

### Example 1 — single user, two different content items

PoP authorizes Alice for `bytes_allowance = 10 MiB`. Alice does:

| Block | Action | `bytes_permanent` | `PermanentStorageUsed` | Outcome |
|---:|---|---:|---:|---|
| 1 | `store` A (5 MiB); `renew` A | 5 MiB | 5 MiB | OK (within quota) |
| 2 | `store` B (5 MiB); `renew` B | 10 MiB | 10 MiB | OK (at quota) |
| 3 | `store` C (1 MiB); `renew` C | — | — | **`PermanentAllowanceExceeded`** (10 + 1 > 10) |

The per-account cap holds: at most `bytes_allowance` bytes of distinct renewed content simultaneously.

### Example 2 — re-renewing the same content

PoP authorizes Alice for `bytes_allowance = 5 MiB`. Alice stores content `X` (5 MiB) and renews it repeatedly:

| Block | Action | `bytes_permanent` | `PermanentStorageUsed` | Outcome |
|---:|---|---:|---:|---|
| 1 | `store` X; `renew` X | 5 MiB | 5 MiB | OK — first renew, kind=Store → bumps counter |
| 3 | re-`renew` X (same content) | 5 MiB | 10 MiB | OK — existing entry has kind=Renew → skip check/increment |
| 5 | re-`renew` X again | 5 MiB | 15 MiB | OK — same content, always fits |

Re-renewing the same content never increases `bytes_permanent` (the existing entry has `kind == Renew`, so it's detected as a re-renewal). `PermanentStorageUsed` increases because each physical `Transactions` entry exists on chain until aged out by `on_initialize`. The chain-wide cap is the bound on physical entries.

### Example 3 — single user, lifecycle across one `AuthorizationPeriod`

`AuthorizationPeriod = RetentionPeriod = 14 days`. PoP authorizes Alice with `bytes_allowance = 10 MiB` at block `0`. Alice stores 10 MiB (as a single content item) and renews it at block `1`. The authorization is `expired` from block `14 days` onward (`now >= expiration`); the renewed entry was indexed at block `1`, so its `RetentionPeriod` clock fires at block `1 + 14 days + 1` (the `on_initialize` cleanup once `obsolete` reaches `1`).

| Block | Authorization state | `bytes_permanent` | Alice's on-chain renewed bytes | `PermanentStorageUsed` |
|---:|---|---:|---:|---:|
| 0 | unexpired (expires `14 days`) | 0 | 0 | 0 |
| 1 | unexpired; Alice: `store(10 MiB)` + `renew` | 10 MiB | 10 MiB | 10 MiB |
| 1 → `14 days − 1` | unexpired, idle | 10 MiB | 10 MiB | 10 MiB |
| `14 days` | **expired-but-present**; Alice's further `store` / `renew` reject with `InvalidTransaction::Payment` | 10 MiB | 10 MiB | 10 MiB |
| `14 days + 2` | expired-but-present; `on_initialize` ages out the renew (`obsolete = 1`) | 10 MiB | 0 | 0 |

From here Alice's path branches:

- **PoP re-authorizes** (`authorize_account` on the expired-but-present path) — the caps are re-granted, all consumed counters (including `bytes_permanent`) reset to `0`. Alice gets a fresh window and can `store` / `renew` again.
- **PoP does not re-authorize** — the authorization sits expired-but-present until anyone calls `remove_expired_account_authorization`. Alice cannot `store` or `renew`. Her renewed data has already aged out.

Note: `bytes_permanent` stays at 10 MiB (per-window counter, does not decrement on age-out). `PermanentStorageUsed` drops to 0 because the `on_initialize` cleanup removes the obsolete `Transactions` entry. On re-authorize, `bytes_permanent` resets to 0.

### Example 4 — end-of-window renew with different content (worst case for on-chain footprint)

Worst case for on-chain footprint: renew right at the end of one window, re-claim immediately at the start of the next, renew *different* content. Both renewals overlap on chain until the older one ages out.

| Day | Action | `bytes_permanent` | On-chain renewed bytes |
|---:|---|---:|---:|
| 13 | renew content A (10 MiB) | 10 MiB | 10 MiB |
| 14 | window 1 expired; re-claim (counters reset); renew content B (10 MiB) | 10 MiB | **20 MiB** |
| 15–26 | both batches on chain | 10 MiB | 20 MiB |
| 27 | day 13's batch ages out (cleanup decrements) | 10 MiB | 10 MiB |
| 28 | day 14's batch ages out; re-claim; new renew | … | … |

Note that `bytes_permanent` never exceeds `bytes_allowance` (10 MiB) in any single window — each re-authorize resets all consumed counters so the new window starts from 0. The on-chain footprint (20 MiB) temporarily exceeds the per-account allowance, but this is bounded by the chain-wide cap.

If the user re-renewed the **same** content A on day 14 instead, the re-renewal would see the existing entry has `kind == Renew` and skip the `bytes_permanent` check entirely. The on-chain footprint would be 20 MiB (two physical entries), but the per-account allowance check would succeed trivially.

### Example 5 — chain-wide cap at scale

`MaxPermanentStorageSize = 1.7 TiB`. Many users renewing concurrently:

| Block | Action | `PermanentStorageUsed` | Outcome |
|---:|---|---:|---|
| n | aggregate renews bring counter to 1.6 TiB | 1.6 TiB | `PermanentStorageNearCap` event fires (≥ 80% of cap) |
| n+k | further renews would exceed 1.7 TiB | 1.7 TiB | `ChainPermanentCapReached` rejects new renews |
| n+k+RetentionPeriod | obsolete cleanup decrements as old renewals age out | < 1.7 TiB | new renews accepted again |

The chain-wide cap is a hard ceiling on `PermanentStorageUsed`; the on-chain renewed bytes equal the counter (modulo a transient lag inside `on_initialize`). The system self-corrects: as soon as the counter falls below the cap, renewals resume.

### Example 6 — adversarial single-user renew spam

A user with maximum claim rate and full `bytes_allowance` every period can only renew up to `bytes_allowance` worth of **distinct** content per window. Re-renewing the same content is free for capacity. To put more distinct renewed bytes on chain, they would need a larger `bytes_allowance` — exactly what `Error::PermanentAllowanceExceeded` prevents.

A user across many accounts (Sybil-like) is bounded by the chain-wide cap (Example 5), regardless of how many accounts they control.

## Migration

`STORAGE_VERSION = 3`. Migrations are only relevant for the Paseo/Westend testnets carrying pre-existing on-chain state forward; see the `pallet_bulletin_transaction_storage::migrations::{v1, v2, v3}` modules for the wiring.

## Capacity planning operational steps

When `PermanentStorageNearCap` fires governance can either:

- Pass a referendum to upgrade collator disk capacity and raise `MaxPermanentStorageSize` (via runtime upgrade for `ConstU64`-backed configs, or `system.set_storage` for `parameter_types! { pub storage }`-backed configs).
- Coordinate spawning another bulletin chain.

## Auto-renewal

Auto-renewal reuses the manual `renew` code path so the [Hard limit on renewed storage](#hard-limit-on-renewed-storage) accounting fires consistently — per-account `bytes_permanent` tracking with re-renewal detection, chain-wide `PermanentStorageUsed` cap check, `kind = Renew` stamp in `Transactions`, obsolete-cleanup decrement. No separate accounting path.

`process_auto_renewals` runs as part of the mandatory `apply_block_inherents` inherent. If `check_authorization` rejects an auto-renewal (expired auth, permanent allowance exceeded, or chain-wide cap reached), the auto-renewal registration is removed, `AutoRenewalFailed` is emitted, and the data expires normally.
