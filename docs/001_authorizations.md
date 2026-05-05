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

Without bounds, at sustained-peak block usage one window of fresh `store` data alone is ~1.73 TiB, and re-renewals stack on top. ⇒ motivates a **hard limit** on renewed storage (per account and chain-wide).

## Storage types

- **Temporary storage** — happens through the `store` call. Lives on chain for one `RetentionPeriod` from its `store` block.
- **Renewed storage** — happens through the `renew` call. The renewed entry itself also lives one `RetentionPeriod` (from its renewal block); the original `Transactions` entry it pointed at ages out on its own clock.

Both `store`/`store_with_cid_config` and `renew` are unconditionally feeless. Authorization is the sole economic gate. Wrapper calls (e.g. `utility::batch`) are rejected by `ValidateStorageCalls`.

Each `TransactionInfo` is stamped with `kind: TransactionKind { Store, Renew }`. The kind is what `on_initialize`'s obsolete-block cleanup uses to tell which entries should decrement the chain-wide renewed-bytes counter when they age out — see [Hard limit on renewed storage](#hard-limit-on-renewed-storage).

## Allowance limits

PoP grants two numbers per account: `bytes_allowance` (size budget) and `transactions_allowance` (count budget). The same `bytes_allowance` is reused on the soft and hard sides, with different semantics.

- **Soft (temporary)** — `bytes_allowance` and `transactions_allowance` are priority thresholds only. The boost drops to `0` once *either* axis is at-or-over cap (`bytes >= bytes_allowance` or `transactions >= transactions_allowance`). `store` calls are never rejected; they queue behind in-budget signers when over.
- **Hard (renewed)** — `bytes_allowance` is a real cap on the per-window renew quota. `renew` is **rejected** when `bytes_permanent + size > bytes_allowance`. The transaction-count axis does not gate renew. A separate chain-wide cap (`MaxPermanentStorageSize`) bounds the total renewed bytes on chain across all signers.

### Authorization storage

- One `AuthorizationExtent` per scope is kept in `Authorizations`, keyed by `AuthorizationScope::{Account, Preimage}`.
- `AuthorizationExtent { transactions, transactions_allowance, bytes, bytes_permanent, bytes_allowance }` holds the soft-side counters (`bytes`, `transactions`), the per-window renew quota (`bytes_permanent`), and the caps.
- `bytes` and `transactions` bump on `store` / `store_with_cid_config`. `bytes_permanent` bumps on `renew`. The `transactions` axis bumps on both, since both consume a transaction slot.

### `authorize_account` semantics

Per existing entry state:

- **Unexpired**: caps are **additive** (`bytes_allowance += bytes`, `transactions_allowance += transactions`). Matches PoP's `claim_long_term_storage` flow, which calls this once per claim and expects each to extend the caps. Consumed counters are preserved, expiry is left untouched.
- **Expired-but-present**: caps are **re-granted** (`bytes_allowance = bytes`, `transactions_allowance = transactions`) and **all** consumed counters reset to `0`, including `bytes_permanent`. The new window's renew quota is independent of the old window's renewals — the old data is still on chain and is tracked by the chain-wide `PermanentStorageUsed` counter, but it does not spend the new window's quota.
- **Missing**: create a fresh entry with all counters at `0`.

`authorize_preimage` follows the same shape, but `transactions_allowance` is fixed at `1` (a preimage grant is a single-shot store right) and the unexpired path **replaces** rather than adds.

### `refresh_account_authorization`

Extends `expiration` by another `AuthorizationPeriod` and leaves all consumed counters (`bytes`, `bytes_permanent`, `transactions`) untouched. Refresh does **not** grant additional capacity. To start a fresh window, let the authorization expire and re-authorize. Origin is `T::Authorizer` (e.g. PoP).

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

`renew` of `size` bytes for scope `S` is rejected with `Error::PermanentAllowanceExceeded` when

```
S.bytes_permanent + size > S.bytes_allowance
```

`bytes_permanent` is **increment-only within a window** and resets to `0` on (re-)authorize via the expired-but-present path. It measures "renew bytes consumed in the current authorization window", not lifetime on-chain footprint. The chain-wide cap is the source of truth for actual on-chain bytes.

### Chain-wide cap

`renew` is rejected with `Error::ChainPermanentCapReached` when

```
PermanentStorageUsed + size > T::MaxPermanentStorageSize::get()
```

`PermanentStorageUsed` is bumped on every successful `renew`. It is decremented in `on_initialize` (mandatory weight, bounded by `MaxBlockTransactions`) when an obsolete block is removed: each `Transactions[obsolete][i]` with `kind == Renew` contributes its `size` to a single saturating decrement, then `Transactions[obsolete]` is removed.

That obsolete-block cleanup is the only path that ever decrements `PermanentStorageUsed`. There is no lazy ledger, no cursor, no `on_poll` drain. `Transactions` is itself the authoritative record of which renewed bytes are still on chain; any other accounting structure would be redundant.

`MaxPermanentStorageSize` is a `Config` trait constant. The runtime picks the backing — `parameter_types! { pub const … }` (runtime-upgrade only) or `parameter_types! { pub storage … }` (storage-backed; mutable at runtime via `system.set_storage`).

### Capacity planning signals

- `Event::PermanentStorageUsedUpdated { used }` fires once per change to `PermanentStorageUsed` (one per renew, one per obsolete-block cleanup).
- `Event::PermanentStorageNearCap { used, cap }` fires on the rising edge across `PERMANENT_STORAGE_NEAR_CAP_PERCENT` (80%) of `MaxPermanentStorageSize`. Off-chain consumers can use this as a "raise the cap or coordinate another bulletin chain" trigger.

## Why renewed bytes can't grow unboundedly

Stated up front: at any block `n`, total renewed bytes on chain are bounded by `MaxPermanentStorageSize` (chain-wide cap) and a single account's renewed bytes are bounded by `2 × bytes_allowance` for `RetentionPeriod = AuthorizationPeriod` configurations.

Why: every renewed byte ages out exactly `RetentionPeriod` blocks after its renew block (the obsolete-block cleanup in `on_initialize`). New renews are gated by the chain-wide cap, so the counter can only enter the in-bounds region. As old data ages out, the cap recovers.

The examples below trace the counters block-by-block to make the bound visible.

### Example 1 — single user, single window

PoP authorizes Alice for `bytes_allowance = 10 MiB`. Alice does:

| Block | Action | `bytes_permanent` | `PermanentStorageUsed` | Outcome |
|---:|---|---:|---:|---|
| 1 | `store` 5 MiB; `renew` it | 5 MiB | 5 MiB | OK (within quota) |
| 2 | `store` 5 MiB; `renew` it | 10 MiB | 10 MiB | OK (at quota) |
| 3 | `store` 1 MiB; `renew` it | — | — | **`PermanentAllowanceExceeded`** |

The per-account cap holds: at most `bytes_allowance` bytes renewed per window.

### Example 2 — single user, aligned periods (steady state)

`AuthorizationPeriod = RetentionPeriod = 14 days`. Alice claims at day 0, renews 10 MiB at day 0, re-claims at day 14, renews 10 MiB at day 14, …

| Day | Authorization state | `bytes_permanent` | On-chain renewed bytes |
|---:|---|---:|---:|
| 0 | window 1 begins; renew 10 MiB | 10 MiB | 10 MiB |
| 1–13 | window 1 in progress; no further action | 10 MiB | 10 MiB |
| 14 | window 1 expired; obsolete cleanup decrements; re-claim → `bytes_permanent = 0`; renew 10 MiB | 10 MiB | 10 MiB |
| 28 | window 2 expired; re-claim; renew 10 MiB | 10 MiB | 10 MiB |
| … | … | 10 MiB | 10 MiB |

Steady-state on-chain footprint per account = `bytes_allowance` (= 10 MiB).

### Example 3 — single user, end-of-window renew (worst case)

Worst case for per-account on-chain footprint: renew right at the end of one window, re-claim immediately at the start of the next, renew again. Both renewals overlap on chain until the older one ages out.

| Day | Action | `bytes_permanent` | On-chain renewed bytes |
|---:|---|---:|---:|
| 13 | renew 10 MiB | 10 MiB | 10 MiB |
| 14 | window 1 expired; re-claim → `bytes_permanent = 0`; renew 10 MiB | 10 MiB | **20 MiB** |
| 15–26 | both batches on chain | 10 MiB | 20 MiB |
| 27 | day 13's batch ages out (cleanup decrements) | 10 MiB | 10 MiB |
| 28 | day 14's batch ages out; re-claim; new renew | … | … |

Peak on-chain bytes per account: `2 × bytes_allowance`. Generalising, with `RetentionPeriod / AuthorizationPeriod = K`, the bound is `(K + 1) × bytes_allowance`: at any moment up to `K + 1` consecutive windows can overlap on chain (the current window's renew plus up to `K` earlier windows still inside their `RetentionPeriod`). Aligned periods (Westend / Paseo) give `K = 1`, so peak = `2 × bytes_allowance` (during overlap windows).

### Example 4 — chain-wide cap at scale

`MaxPermanentStorageSize = 1.7 TiB`. Many users renewing concurrently:

| Block | Action | `PermanentStorageUsed` | Outcome |
|---:|---|---:|---|
| n | aggregate renews bring counter to 1.6 TiB | 1.6 TiB | `PermanentStorageNearCap` event fires (≥ 80% of cap) |
| n+k | further renews would exceed 1.7 TiB | 1.7 TiB | `ChainPermanentCapReached` rejects new renews |
| n+k+RetentionPeriod | obsolete cleanup decrements as old renewals age out | < 1.7 TiB | new renews accepted again |

The chain-wide cap is a hard ceiling on `PermanentStorageUsed`; the on-chain renewed bytes equal the counter (modulo a transient lag inside `on_initialize`). The system self-corrects: as soon as the counter falls below the cap, renewals resume.

### Example 5 — adversarial single-user renew spam

A user with maximum claim rate and full `bytes_allowance` every period contributes at most `(K + 1) × bytes_allowance` on-chain bytes simultaneously (Example 3). To exceed that, they would need to renew **more** in a single window than their `bytes_allowance` permits — exactly what `Error::PermanentAllowanceExceeded` rejects.

A user across many accounts (Sybil-like) is bounded by the chain-wide cap (Example 4), regardless of how many accounts they control.

## Migration

`STORAGE_VERSION = 3`. Migrations are only relevant for the Paseo/Westend testnets carrying pre-existing on-chain state forward; see the `pallet_bulletin_transaction_storage::migrations::{v1, v2, v3}` modules for the wiring.

## Capacity planning operational steps

When `PermanentStorageNearCap` fires governance can either:

- Pass a referendum to upgrade collator disk capacity and raise `MaxPermanentStorageSize` (via runtime upgrade for `ConstU64`-backed configs, or `system.set_storage` for `parameter_types! { pub storage }`-backed configs).
- Coordinate spawning another bulletin chain.

## Auto-renewal

Whatever auto-renewal mechanism lands must reuse the manual `renew` code path so the [Hard limit on renewed storage](#hard-limit-on-renewed-storage) accounting fires consistently — per-account `bytes_permanent` increment, chain-wide `PermanentStorageUsed` cap check, `kind = Renew` stamp in `Transactions`, obsolete-cleanup decrement. No separate accounting path.

## TODO

### Align with auto-renewal ([PR #313](https://github.com/paritytech/polkadot-bulletin-chain/pull/313))

PR #313 introduces `TransactionByContentHash`, `AutoRenewals`, `PendingAutoRenewals`, and `process_auto_renewals`. Items to resolve at merge time:

- **Centralize accounting in `do_renew`.** Hard-cap checks (per-account, chain-wide) and the `kind = Renew` stamp must live in `do_renew`, called by `renew`, `renew_content_hash`, and `process_auto_renewals`.
- **Specify `process_auto_renewals` behavior on chain-wide cap rejection.** If `do_renew` rejects an auto-renewal because of `MaxPermanentStorageSize`, treat it the same as PR #313's "block full" path: remove the registration, emit `AutoRenewalFailed`, let the data expire normally.
- **Drop the snapshot check in `enable_auto_renew`** (or replace with a real reservation). The current check (`extent.transactions > 0 && extent.bytes >= tx_info.size`) is misleading and the per-window quota framing makes it even less meaningful — it suggests "this will work" while making no guarantees beyond the current block.
- **Reserve block-transaction slots for user txs.** `process_auto_renewals` is mandatory and pushes into the same `BlockTransactions` slot as user `store`/`renew`. Cap auto-renewals to a fraction of `MaxBlockTransactions` or partition the slot budget.
- **Audit `on_initialize` weight.** Per expiring tx: 2 reads + up to 2 writes, all in mandatory weight. Worst case at `MaxBlockTransactions = 512` is ~1500 reads + ~500 writes per `on_initialize`. Bench it.
- **Per-content dedup of re-renewals (nice-to-have).** On `renew(X)`, look up the previous `(block, idx)` for `X` via `TransactionByContentHash` and cancel its pending decrement — drops the per-content double-count when the same content is renewed in multiple consecutive windows.
