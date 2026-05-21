# Authorizations design

## Motivation

Two distinct problems shape the allowance design. They map onto the two limits in [Allowance semantics](#allowance-semantics).

Block-throughput reference numbers:

| Parameter | Value |
|---|---|
| `MaxTransactionSize` | 2 MiB |
| `MaxBlockTransactions` | 512 |
| `MAX_BLOCK_LENGTH` × `NORMAL_DISPATCH_RATIO` (90%) | **~9 MiB / block** (binding constraint) |
| Parachain block time | 6 s ⇒ 14 400 blocks/day |
| Sustained max throughput | **~127 GiB/day**, **~1.73 TiB / 2 weeks** |
| `RetentionPeriod` | `201 600 blocks` = 14 days |
| `DefaultAuthorizationWindow` (Westend / Paseo) | `14 × 14 400` = 201 600 **relay** blocks (14 days) |

### 1. Wasted block space (soft)

The chain has ~9 MiB of body capacity per block. If `store` rejected every call the moment a user crossed their per-account allowance, blocks would frequently sit empty even when authorized users had data ready to send — capacity left on the table.

Accept over-allowance `store` calls at a lower priority instead. In-budget users still go first; over-budget calls fill whatever block space is left. ⇒ motivates a **soft limit** on temporary-storage allowance, enforced by priority rather than rejection.

### 2. Unbounded renewed storage on collators (hard)

`renew` re-anchors an existing stored item: when the original entry's `RetentionPeriod` is about to elapse, a `renew` lands a fresh `Transactions[block]` entry pointing at the same content, and the *renewed* entry's own `RetentionPeriod` clock starts from that block. Repeat indefinitely and a single piece of content can stay on chain forever.

Without bounds, at sustained-peak block usage one window of fresh `store` data alone is ~1.73 TiB, and re-renewals stack on top. ⇒ motivates a **hard limit** on renewed storage (per slot and chain-wide).

## Storage types

- **Temporary storage** — happens through the `store` call. Lives on chain for one `RetentionPeriod` from its `store` block.
- **Renewed storage** — re-anchors an existing entry. The renewed entry itself lives one `RetentionPeriod` (from its renewal block); the original `Transactions` entry it pointed at ages out on its own clock.

The renew-family extrinsics:

- `force_renew(entry: TransactionRef)` — synchronous renewal at dispatch time. `TransactionRef::Position { block, index }` or `TransactionRef::ContentHash(_)`.
- `renew(entry: TransactionRef)` — one-shot scheduler. The extension's `check_signed` pre-pays the renewal at registration (same hard-cap charge as `force_renew`); the cycle delivers without re-charging.
- `enable_auto_renew(content_hash)` — recurring scheduler. Same pre-paid first cycle as `renew`; cycle 1 delivers without re-charging, cycle 2 onward charges per cycle.
- `disable_auto_renew(content_hash)` — cancels the registration. Signed callers are rejected with `CannotDisablePrepaidAutoRenewal` while the registration is in its prepaid window (`paid: true`); they must wait for the first cycle to consume the prepayment. Root can disable regardless (governance / cleanup), but the prepayment is forfeit.

`store`, `store_with_cid_config`, `force_renew`, `renew`, `enable_auto_renew`, and `disable_auto_renew` are unconditionally feeless. Authorization is the sole economic gate. Wrapper calls (e.g. `utility::batch`) are rejected by `ValidateStorageCalls`.

Each `TransactionInfo` is stamped with `kind: TransactionKind { Store, Renew }`. The kind is what `on_initialize`'s obsolete-block cleanup uses to tell which entries should decrement the chain-wide renewed-bytes counter when they age out — see [Hard limit on renewed storage](#hard-limit-on-renewed-storage).

## Authorization model: slots

Per-scope state is `AuthorizationSlots`, a map from `AuthorizationScope::{Account, Preimage}` to a `BoundedVec<TimedAuthorization, MaxAuthorizationSlots>`. Each slot is independent:

```rust
struct TimedAuthorization {
    extent: AuthorizationExtent,
    starts_at: u32,   // inclusive relay block
    expiration: u32,  // exclusive relay block
}
```

`starts_at` and `expiration` are **relay-chain** block numbers, sourced from `Config::RelayChainBlockNumberProvider` (cumulus parachain-system on live chains). A slot is **active** at `relay_now` iff `starts_at <= relay_now < expiration`; **future** while `starts_at > relay_now`; **expired** once `expiration <= relay_now`.

The bounded vec is kept sorted by `expiration` ascending (tiebreak `starts_at`) by `add_slot`. The order keeps the SCALE encoding deterministic, makes the `try_state` invariant a single forward pass, and makes "earliest-expiring active slot" a forward scan.

**Lazy prune.** Every read or mutate path goes through `prune_expired`, which drops slots where `expiration <= relay_now`. Drained-but-active slots (those at the soft cap on `bytes` / `transactions`) are intentionally kept — `store` never gates on those caps, so a drained slot still serves low-priority stores until it expires. Once every slot for a scope is dropped, the entry itself is removed and the provider-ref (for `Account` scope) is decremented.

Westend / Paseo wiring:

| Constant | Value |
|---|---|
| `MaxAuthorizationSlots` | `8` |
| `DefaultAuthorizationWindow` | `14 × 14 400` = 201 600 relay blocks (14 days) |
| `MaxStartsAtFuture` | `30 × 14 400` = 432 000 relay blocks (30 days) |

## Allowance semantics

PoP grants two numbers per slot: `bytes_allowance` and `transactions_allowance`. `AuthorizationExtent` is unchanged from the pre-slot design:

```rust
struct AuthorizationExtent {
    transactions: u32,
    transactions_allowance: u32,
    bytes: u64,
    bytes_permanent: u64,
    bytes_allowance: u64,
}
```

Counters live **inside a slot** — there is no cross-slot subsidy. `bytes` / `transactions` bump on `store`. `bytes_permanent` bumps on `renew`. `transactions` bumps on both (it feeds the priority boost).

- **Soft (temporary).** `store` never rejects. The chosen slot's `bytes` and `transactions` saturate upward on every consume. The priority boost reads a **folded** view of the scope (see [Soft limit](#soft-limit-priority-boost)).
- **Hard (renewed).** `renew` of `size` bytes is rejected with `Error::PermanentAllowanceExceeded` unless **some active slot** satisfies `bytes_permanent + size <= bytes_allowance` *on that slot alone*. A separate chain-wide cap (`MaxPermanentStorageSize`) is checked first.

### Slot selection on `store` / `renew`

`pick_slot_for_consumption` (in `lib.rs`) picks the earliest-expiring **active** slot:

- `store`: any active slot is acceptable; the soft counters never gate. The earliest-expiring slot is chosen so capacity expiring soonest is consumed first.
- `renew`: additionally filters to slots whose per-slot renew cap covers `size` (`bytes_permanent + size <= bytes_allowance`).

If no slot qualifies, `renew` returns `PermanentAllowanceExceeded` (when at least one slot is active but none has renew capacity) or `InvalidTransaction::Payment` (when no slot is active). On success, the chosen slot's `bytes` (store) or `bytes_permanent` (renew) is bumped by `size` and its `transactions` by 1. On a successful renew, the chain-wide `PermanentStorageUsed` is also bumped.

### `authorize_account` / `authorize_preimage` semantics

Both extrinsics resolve a window:

- **Default form** (`authorize_account`, `authorize_preimage`): `starts_at = relay_now`, `expiration = relay_now + DefaultAuthorizationWindow`.
- **Explicit form** (`authorize_account_window`, `authorize_preimage_window`): caller supplies `(starts_at: Option<u32>, expiration: u32)`. `starts_at = None` means "active immediately" (`relay_now`). The window is validated by `ensure_valid_window`:
  - `expiration > effective_starts_at`
  - `expiration > relay_now`
  - `effective_starts_at - relay_now <= MaxStartsAtFuture`

  A `starts_at` in the past is accepted (treated as already-active).

Then `add_slot` inserts or merges:

- **Additive merge** when an existing slot matches *either*:
  1. **the same exact window** (`starts_at` and `expiration` equal), **or**
  2. **the same `expiration` AND both slots are already active** (`existing.starts_at <= relay_now` AND `new.starts_at <= relay_now`). A `starts_at` in the past is observationally equivalent to `relay_now` for an active slot, so two such slots that expire at the same time can be folded with no semantic loss.

  The merge pre-clamps the existing slot's `bytes` and `transactions` to the **old** cap before widening the allowance, so the folded view stays equivalent to keeping two slots side-by-side.
- Otherwise **push** a new slot. Fails with `Error::TooManySlots` when the bounded vec is full (after the lazy prune).

Preimage slots carry `transactions_allowance = 2`. The slot model gates the transaction-count axis on consume, and the canonical preimage flow is store-then-renew — a single-tx budget would block the renew.

**`refresh_account_authorization` no longer exists as an extrinsic.** Re-extending an account's authorization is now "call `authorize_account` again": if the same default window is still active, the caps merge additively (consumed counters preserved); if it has expired, the lazy prune drops the old slot and a fresh one is pushed.

**Events.** `AccountAuthorized` and `PreimageAuthorized` now carry the resolved `starts_at` and `expiration`, so off-chain consumers can index slots without re-deriving from the call.

**Errors introduced by the slot model.**

| Error | When |
|---|---|
| `TooManySlots` | `add_slot` could not push because the bounded vec is full after the lazy prune. |
| `InvalidWindow` | `expiration <= starts_at`, `expiration <= relay_now`, or `starts_at - relay_now > MaxStartsAtFuture`. |
| `RelayChainTimeUnavailable` | `relay_now == 0` (parachain-system inherent has not yet populated validation data — applies to genesis and the very first block). |

## Soft limit (priority boost)

Implemented by the [`AllowanceBasedPriority`][ext] transaction extension via a runtime-selected `BoostStrategy`:

- The boost only applies to **signed account-scoped `store` / `store_with_cid_config`**. `renew` and preimage-scoped stores get `0`.
- The strategy is fed the **post-this-tx, folded** extent — `Pallet::account_authorization_extent(who)` sums per-slot `bytes` and `transactions` across *active* slots, **clamping each slot's used counters to that slot's own allowance** before adding. The clamp matters: without it an over-cap soft store on one slot would inflate the folded `bytes` past the folded `bytes_allowance` and mask another slot's remaining room from the boost.
- `FlatBoost` (default in both runtimes): `ALLOWANCE_PRIORITY_BOOST` while folded-in-budget on both axes, `0` once over.
- `ProportionalBoost` (alternative): scales with the tighter of the byte- and tx-budget remainders.

In-budget `store` txs sort strictly above over-budget ones. Pool nonce / arrival ordering breaks ties.

[ext]: ../pallets/transaction-storage/src/extension.rs

## Hard limit on renewed storage

The hard cap is enforced at two levels, and a renewal that would breach **either** is rejected.

### Per-slot quota

`renew` of `size` bytes for scope `S` is rejected with `Error::PermanentAllowanceExceeded` when no active slot of `S` satisfies

```
slot.bytes_permanent + size <= slot.bytes_allowance
```

`bytes_permanent` is **increment-only within a slot** and never resets — when the slot's `expiration` passes, the lazy prune drops the whole slot. It measures "renew bytes consumed by this slot", not lifetime on-chain footprint. The chain-wide cap is the source of truth for actual on-chain bytes.

### Chain-wide cap

`renew` is rejected with `Error::ChainPermanentCapReached` when

```
PermanentStorageUsed + size > T::MaxPermanentStorageSize::get()
```

`PermanentStorageUsed` is bumped on every successful `renew`. It is decremented in `on_initialize` (mandatory weight, bounded by `MaxBlockTransactions`) when an obsolete block is removed: each `Transactions[obsolete][i]` with `kind == Renew` contributes its `size` to a single saturating decrement, then `Transactions[obsolete]` is removed.

That obsolete-block cleanup is the only path that ever decrements `PermanentStorageUsed`. `Transactions` is the authoritative record of which renewed bytes are still on chain; the counter is just a precomputed sum maintained alongside it.

`MaxPermanentStorageSize` is a `Config` trait constant. The runtime picks the backing — `parameter_types! { pub const … }` (runtime-upgrade only) or `parameter_types! { pub storage … }` (storage-backed; mutable at runtime via `system.set_storage`). Paseo uses the storage-backed form, seeded at 1.7 TiB.

### Capacity planning signals

- `Event::PermanentStorageUsedUpdated { used }` fires once per change to `PermanentStorageUsed`: once per successful `renew` (increment), and once per obsolete-block cleanup that ages out at least one `kind == Renew` entry (decrement).
- `Event::PermanentStorageNearCap { used, cap }` fires on the rising edge across `PERMANENT_STORAGE_NEAR_CAP_PERCENT` (80%) of `MaxPermanentStorageSize`. Off-chain consumers can use this as a "raise the cap or coordinate another bulletin chain" trigger.

## Why renewed bytes can't grow unboundedly

Stated up front:

- Chain-wide bound at any block: total renewed bytes ≤ `MaxPermanentStorageSize`.
- Per-account bound: with `K = RetentionPeriod / DefaultAuthorizationWindow` (Westend / Paseo: `K = 1`) and at most `S = MaxAuthorizationSlots = 8` concurrently active slots, the on-chain peak per account is bounded by `(K + 1) × S × bytes_allowance` in the worst case. Under the normal flow where a caller does not stack overlapping `_window` grants, the bound collapses to `(K + 1) × bytes_allowance`.

Why: every renewed byte ages out exactly `RetentionPeriod` blocks after its renew block (the obsolete-block cleanup in `on_initialize`). New renews are gated by the chain-wide cap, so the counter can only enter the in-bounds region; as old data ages out, the cap recovers. Slot expiry only ends the *capacity to renew further* in that slot — it does not retroactively evict already-renewed bytes.

The examples below trace the counters block-by-block to make the bounds visible.

### Example 1 — single slot, single window

PoP calls `authorize_account(Alice, transactions = 3, bytes = 10 MiB)` at relay block `R0`. One slot is pushed with `starts_at = R0`, `expiration = R0 + 14 days`, `bytes_allowance = 10 MiB`. Alice does:

| Block | Action | `slot.bytes_permanent` | `PermanentStorageUsed` | Outcome |
|---:|---|---:|---:|---|
| 1 | `store` 5 MiB; `renew` it | 5 MiB | 5 MiB | OK (within quota) |
| 2 | `store` 5 MiB; `renew` it | 10 MiB | 10 MiB | OK (at quota) |
| 3 | `store` 1 MiB; `renew` it | — | — | **`PermanentAllowanceExceeded`** |

The per-slot cap holds: at most `bytes_allowance` bytes can be renewed against any one slot.

### Example 2 — single slot lifecycle across one window

`DefaultAuthorizationWindow = RetentionPeriod = 14 days`. PoP authorizes Alice with `bytes_allowance = 10 MiB` at relay block `R0`. Alice stores 10 MiB and renews it at parachain block `1` (relay still ≈ `R0`). The slot is active while `relay_now < R0 + 14 days`; the renewed `Transactions` entry was indexed at parachain block `1`, so its `RetentionPeriod` clock fires at parachain block `1 + 14 days + 1` (the `on_initialize` cleanup once `obsolete` reaches `1`).

| Relay block | Slot state | `slot.bytes_permanent` | Alice's on-chain renewed bytes | `PermanentStorageUsed` |
|---:|---|---:|---:|---:|
| `R0` | active | 0 | 0 | 0 |
| `R0` (Alice acts) | active; `store(10 MiB)` + `renew` | 10 MiB | 10 MiB | 10 MiB |
| `R0` → `R0 + 14 days − 1` | active, idle | 10 MiB | 10 MiB | 10 MiB |
| `R0 + 14 days` | **expired** (`relay_now >= expiration`); next read prunes; further `store` / `renew` reject with `InvalidTransaction::Payment` | (slot pruned) | 10 MiB | 10 MiB |
| `R0 + 14 days + 2` (parachain side: obsolete cleanup fires) | (pruned) | — | 0 | 0 |

From here Alice's path branches:

- **PoP re-authorizes** (`authorize_account`) — the lazy prune has already dropped the old slot, so a fresh single-slot vec is created. Counters start at `0`; she can `store` / `renew` again. Repeating the pattern every window gives steady-state on-chain footprint = `bytes_allowance` per account (= 10 MiB).
- **PoP does not re-authorize** — the storage entry is gone after the first read past expiry. Alice cannot `store` or `renew`. Her renewed data has already aged out.

Two things worth noting:

1. `slot.bytes_permanent` is **not** decremented when the renewed data ages out — that is the chain-wide `PermanentStorageUsed`'s job. The per-slot counter is irrelevant after the slot is pruned. While the slot is active and the byte cap is reached, the renew gate rejects on the per-slot check before considering the chain-wide counter.
2. `Transactions` is the source of truth for on-chain renewed bytes. The chain-wide counter mirrors that same total via increments at renew time and decrements at obsolete-block cleanup; the per-slot counter does not need to.

### Example 3 — end-of-window overlap (worst case, single-slot flow)

Worst case for per-account on-chain footprint when only the default window is used: renew right at the end of one window, re-authorize immediately at the start of the next, renew again. Both renewals overlap on chain until the older one ages out.

| Day | Action | `slot.bytes_permanent` | On-chain renewed bytes |
|---:|---|---:|---:|
| 13 | renew 10 MiB (slot A) | 10 MiB (A) | 10 MiB |
| 14 | slot A expired → pruned on next read; `authorize_account` pushes slot B; renew 10 MiB | 10 MiB (B) | **20 MiB** |
| 15–26 | both batches on chain | 10 MiB (B) | 20 MiB |
| 27 | day 13's batch ages out (chain-wide decrement) | 10 MiB (B) | 10 MiB |
| 28 | day 14's batch ages out; new slot; new renew | … | … |

Peak on-chain bytes per account under this flow: `2 × bytes_allowance`. Generalising, with `RetentionPeriod / DefaultAuthorizationWindow = K`, the bound is `(K + 1) × bytes_allowance`: at any moment up to `K + 1` consecutive single-slot windows can overlap on chain (the current window's renew plus up to `K` earlier windows still inside their `RetentionPeriod`). Aligned periods (Westend / Paseo) give `K = 1`, so peak = `2 × bytes_allowance` during overlap windows.

### Example 4 — concurrent slots via `_window`

PoP grants Bob two slots side-by-side, both with `bytes_allowance = 10 MiB`:

- Slot A: `authorize_account_window(Bob, transactions=3, bytes=10 MiB, starts_at=None, expiration=R0 + 14 days)` — folds into the default window via additive merge if `authorize_account` was called first.
- Slot B: `authorize_account_window(Bob, transactions=3, bytes=10 MiB, starts_at=None, expiration=R0 + 28 days)` — distinct `expiration`, so it pushes a new slot.

After both calls, `AuthorizationSlots[Bob]` is `[slot_A, slot_B]` (sorted by `expiration` asc). At `relay_now = R0`, both are active.

| Action | Picked slot | `slot_A` | `slot_B` |
|---|---|---|---|
| `store(2 MiB)` | A (earliest expiration) | `bytes = 2 MiB` | unchanged |
| `renew(5 MiB)` of a 5 MiB blob | A | `bytes_permanent = 5 MiB` | unchanged |
| `renew(6 MiB)` of a 6 MiB blob | B (A would breach `5 + 6 > 10`) | unchanged | `bytes_permanent = 6 MiB` |

The folded `account_authorization_extent(Bob)` reports `bytes_allowance = 20 MiB`, `bytes_permanent = 11 MiB`, `transactions = 3`. The priority boost reads the folded view and stays in-budget on the soft axes.

### Example 5 — chain-wide cap at scale

`MaxPermanentStorageSize = 1.7 TiB`. Many users renewing concurrently:

| Block | Action | `PermanentStorageUsed` | Outcome |
|---:|---|---:|---|
| n | aggregate renews bring counter to 1.36 TiB | 1.36 TiB | `PermanentStorageNearCap` event fires (≥ 80% of cap) |
| n+k | further renews would exceed 1.7 TiB | 1.7 TiB | `ChainPermanentCapReached` rejects new renews |
| n+k+RetentionPeriod | obsolete cleanup decrements as old renewals age out | < 1.7 TiB | new renews accepted again |

The chain-wide cap is a hard ceiling on `PermanentStorageUsed`; the on-chain renewed bytes equal the counter (modulo a transient lag inside `on_initialize`). The system self-corrects: as soon as the counter falls below the cap, renewals resume.

A single account, even via stacked `_window` grants, cannot push concurrent on-chain renewed bytes past `(K + 1) × MaxAuthorizationSlots × bytes_allowance` (and only gets there if an authorizer separately grants the maximum number of overlapping slots — a deliberate operational choice). A Sybil-like attacker spreading across many accounts is bounded by the chain-wide cap regardless of account count.

## Migration

`STORAGE_VERSION = 5`. Migrations are only relevant for the Paseo / Westend testnets carrying pre-existing on-chain state forward; see the `pallet_bulletin_transaction_storage::migrations::{v1, v2, v3, v4, v5}` modules for the wiring.

`v3 → v4` (`migrations::v4::MigrateV3ToV4`, a `SteppedMigration`) translates each legacy `Authorizations[scope]` entry into a single-slot `Authorizations[scope] = Authorization { slots: [TimedAuthorization { extent, starts_at: relay_now, expiration: relay_now + DefaultAuthorizationWindow }] }` in place (shared storage prefix), dropping zero-allowance or already-expired entries. Consumed counters in the legacy `AuthorizationExtent` are dropped — translated slots start with `bytes = bytes_permanent = transactions = 0` and the legacy caps applied to a fresh window. The migration aborts cleanly if the relay-chain block number is unavailable (`relay_now == 0`); a later block reruns it.

`v4 → v5` (`migrations::v5::MigrateV4ToV5`, a `SteppedMigration`) re-encodes each `AutoRenewals` entry from `{ account }` to `{ account, recurring: true, paid: false }`. All pre-existing entries were written by the old fee-paying `enable_auto_renew`, which is the forever-renewal path and did **not** pre-pay against the owner's authorization — so they migrate as `{ recurring: true, paid: false }` and `do_process_auto_renewals` charges them per-cycle, preserving their on-chain behaviour across the upgrade. New one-shot (`recurring: false`) and new prepaid (`paid: true`) entries are only reachable through the v5 extrinsics, which can't have written any entries before the migration runs.

## Capacity planning operational steps

When `PermanentStorageNearCap` fires, governance can either:

- Pass a referendum to upgrade collator disk capacity and raise `MaxPermanentStorageSize` (via runtime upgrade for `ConstU64`-backed configs, or `system.set_storage` for `parameter_types! { pub storage }`-backed configs).
- Coordinate spawning another bulletin chain.

## Auto-renewal

`AutoRenewals` entries carry `{ account, recurring: bool, paid: bool }`:

- `recurring: false` — one-shot, set by [`renew`](#storage-types). The first cycle consumes the prepayment and removes the registration.
- `recurring: true` — forever, set by `enable_auto_renew`. The first cycle consumes the prepayment; cycle 2 onward charges per-cycle from the owner's authorization.
- `paid: true` — the next cycle has already been charged at registration time (`bytes_permanent` + 1 tx slot picked atomically against an active slot in the extension's `check_signed`, plus the chain-wide `PermanentStorageUsed` increment). The first cycle delivers without re-charging and flips `paid` to `false`.

### Registration

`renew` and `enable_auto_renew` route through `extension::check_signed` with `is_renew = true`, which calls `check_authorization` and atomically picks an active slot with `bytes_permanent + size <= bytes_allowance`. Failure surfaces immediately at pool ingress (`PermanentAllowanceExceeded` per-slot, `ChainPermanentCapReached` chain-wide). Spam is bounded structurally by this up-front charge — a caller cannot over-schedule past `bytes_allowance` or `MaxPermanentStorageSize`. The pallet body then writes the `AutoRenewals` entry with `paid: true`; it does **not** re-invoke `do_renew`, otherwise `bytes_permanent` would be double-charged (once at registration, once on the prepaid cycle).

`disable_auto_renew` is rejected for signed owners while `paid: true` (`CannotDisablePrepaidAutoRenewal`) — the prepayment has already been deducted and cannot be reclaimed before the first cycle fires. Root bypasses the check (governance / cleanup); the prepayment is then forfeit.

### Cycle delivery

The actual renewal flows through `process_auto_renewals` (driven by the `apply_block_inherents` mandatory inherent). It drains `PendingAutoRenewals` (populated by the obsolete-block cleanup in `on_initialize`) entry-by-entry:

1. If `paid: true`: deliver the renewal **without** charging the owner, then flip `paid` to `false` so subsequent cycles charge per-cycle. For `recurring: false`, the registration is removed immediately after the prepaid cycle.
2. If `paid: false`: call the same `check_authorization` → `pick_slot_for_consumption` path as a user `renew`. The per-slot hard cap, chain-wide `PermanentStorageUsed` cap, `kind = Renew` stamp in `Transactions`, and obsolete-cleanup decrement all fire consistently. No separate accounting path.

If a queued auto-renewal can no longer pass the gate (no active slot, per-slot cap, or chain-wide cap), `process_auto_renewals` removes the registration and emits `AutoRenewalFailed`; the data ages out normally on its `RetentionPeriod`. The latest-entry guard in `on_initialize` skips an obsolete entry when `TransactionByContentHash[hash]` points to a later block — a manual `force_renew` may have moved the latest reference forward; the renewal cycle then fires from the new entry's expiry, not the original.

## TODO

- **Reserve block-transaction slots for user txs.** `process_auto_renewals` is mandatory and pushes into the same `BlockTransactions` slot as user `store` / `force_renew`. Cap auto-renewals to a fraction of `MaxBlockTransactions` or partition the slot budget.
- **Per-content dedup of re-renewals (nice-to-have).** On a renew of `X`, look up the previous `(block, idx)` for `X` via `TransactionByContentHash` and cancel its pending decrement — drops the per-content double-count when the same content is renewed in multiple consecutive windows.
