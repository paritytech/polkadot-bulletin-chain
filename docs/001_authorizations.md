# Authorizations design

## Motivation

Two distinct problems shape the allowance design. They map directly onto the two limits in [Allowance Limits](#allowance-limits).

Block-throughput numbers used below:

| Parameter | Value |
|---|---|
| `MaxTransactionSize` | 2 MiB |
| `MaxBlockTransactions` | 512 |
| `MAX_BLOCK_LENGTH` × `NORMAL_DISPATCH_RATIO` (90%) | **~9 MiB / block** (binding constraint for storage data) |
| Block time | 6 s ⇒ 14 400 blocks/day |
| Sustained max throughput | **~127 GiB/day**, **~1.73 TiB / 2 weeks** |

### 1. Wasted block space (soft)

The chain has ~9 MiB of body capacity per block. If `store` rejects every call the moment a user crosses their per-account allowance, blocks frequently sit empty even when authorized users have data ready to send — capacity is left on the table.

Instead of rejecting `store` calls once a user is over their allowance, accept them at a lower priority. In-budget users still go first; over-budget calls fill whatever block space is left. Nothing is wasted, and no one is starved. ⇒ motivates a **soft limit** on temporary storage allowance, enforced by priority rather than rejection.

### 2. Unbounded permanent storage on collators (hard)

`renew` extends a stored item's lifetime: as long as it is renewed before the current retention window expires, it stays on disk for another window.

At sustained-peak block usage, the current window's fresh `store` data alone is ~1.73 TiB. Whatever is renewed from the previous window sits on top of that, then renewals from the window before, and so on. After N windows of full usage where everything keeps getting renewed, disk requirement is roughly **N × 1.73 TiB**.

Note: temp and permanent are **independent** disk pressures. With both saturated (max `store` rate *and* `MaxPermanentStorageSize` ≈ 1.7 TiB of permanent), peak collator disk is ~3.4 TiB, not 1.7 TiB. `MaxPermanentStorageSize` should therefore be set well below collator disk — leaving headroom for temp — not equal to it.

⇒ motivates a **hard limit** on permanent storage allowance (per account and chain-wide).

## Storage Types

Conceptually there will be 2 types of storage:

- **Temporary storage** — happens through the `store` call.
- **Permanent storage** — happens through the `renew` call (this can also be initiated through the auto-renewal flow).

## Allowance Limits

There are 2 limits on allowances:

- A **soft limit** on temporary storage allowance.
- A **hard limit** on permanent storage allowance.

Once the soft limit is crossed, the `store` calls post this for that account will be on lower priority — meant to utilise the block space when available.

PoP grants two numbers per account: `bytes_allowance` (size budget) and `transactions_allowance` (count budget). The soft and hard caps reuse `bytes_allowance` with different semantics; `transactions_allowance` is only relevant on the soft side.

- **Soft (temp)** — `bytes_allowance` and `transactions_allowance` are used **only as priority thresholds**. The boost drops to `0` once *either* axis is at-or-over cap (`bytes >= bytes_allowance` or `transactions >= transactions_allowance`). `store` calls aren't rejected; they just queue behind in-budget signers. Neither axis is a rejection threshold.
- **Hard (permanent)** — `bytes_allowance` is the real cap. `renew` is **rejected** when `bytes_permanent + size > bytes_allowance`. The transaction-count axis does not gate renew.

A signer can therefore consume up to `bytes_allowance` of temp usage *and* up to `bytes_allowance` of permanent usage off the same grant; only the permanent side is rejected at the cap. Splitting into two independent allowances is deferred (see [Hard Limit § Open questions](#open-questions)).

### Allowance storage and `authorize_account`

- One `AuthorizationExtent` per account is kept in the `Authorizations` storage map, keyed by `AuthorizationScope::Account(AccountId)`.
- `AuthorizationExtent` carries `{ transactions, transactions_allowance, bytes, bytes_permanent, bytes_allowance }`: store/renew usage counters and the caps. Both `store` and `renew` bump `transactions += 1`; `store` bumps `bytes += size`; `renew` bumps `bytes_permanent += size`.
- `authorize_account(who, transactions, bytes)` behaves differently per state of the existing entry:
    - **Unexpired**: caps are **additive** — `bytes_allowance += bytes`, `transactions_allowance += transactions`. This matches the PoP `claim_long_term_storage` flow, where each successful claim is expected to extend the caps. Consumed counters (`bytes`, `bytes_permanent`, `transactions`) are preserved. Expiry is left untouched; use `refresh_account_authorization` to push it back.
    - **Expired-but-present**: re-grant the caps (`bytes_allowance = bytes`, `transactions_allowance = transactions`), reset the soft counters to `0` (`bytes = 0`, `transactions = 0`), and **leave `bytes_permanent` as-is**. The lazy ledger drain remains the only path that ever decrements `bytes_permanent`, so in-flight on-chain commitments aren't clobbered by re-authorize. (Pairs with the rule below: `remove_expired_account_authorization` refuses to remove the entry while `bytes_permanent > 0`.)
    - **Missing**: create a fresh entry with all counters at `0`.
- `authorize_preimage(content_hash, max_size)` follows the same shape but with `transactions_allowance = 1` baked in (a preimage grant is a single-shot store right). Unexpired re-authorize **replaces** the cap (preimage grants are point-in-time, not additive).

### Refresh authorization

`refresh_account_authorization(who)` extends an authorization's lifetime:

- Extends `expiration` by another `AuthorizationPeriod`. Caps and **all** consumed counters (`bytes`, `bytes_permanent`, `transactions`) are left untouched. Refresh does not grant additional capacity.
- To grant more capacity within an unexpired window, call `authorize_account` (additive on the unexpired path); to fully reset the soft counters, let the authorization expire and re-authorize.
- `bytes_permanent` survival across refresh is load-bearing: clearing it would let a holder commit unbounded permanent storage by refreshing repeatedly.
- Origin: only `T::Authorizer` (e.g. PoP) can call it. Users cannot self-refresh; PoP controls the cadence at which a user's authorization is renewed.

## Soft Limit

Implemented in [PR #448](https://github.com/paritytech/polkadot-bulletin-chain/pull/448).

- `check_authorization` does **not** reject `store` calls over the soft limit; it saturates `bytes` and `transactions` upward and lets the tx validate.
- The `AllowanceBasedPriority` transaction extension adds a priority boost via a runtime-selected `BoostStrategy`. The boost only applies to **signed account-scoped `store` / `store_with_cid_config`** — `renew` consumes allowance but is excluded (it operates on already-stored data and shouldn't compete for the same priority slots as new submissions); preimage-scoped stores also get `0` (only account-scoped variants consume the caller's per-account allowance).
- The strategy is fed the **post-this-tx** extent (caller pre-applies `bytes += size` and `transactions += 1`), so the boost decision reduces to "would this leave the holder in-budget on both axes?".
- `in_budget(extent)` is `true` iff `bytes_allowance != 0 && bytes <= bytes_allowance && transactions <= transactions_allowance`.
- `FlatBoost` (default in both runtimes): `ALLOWANCE_PRIORITY_BOOST` while in-budget on both axes, `0` once either axis is over cap.
- `ProportionalBoost` (alternative): boost scales with the **tighter** of the byte-budget and tx-budget remainders — `min(BOOST × bytes_rem / bytes_allowance, BOOST × tx_rem / transactions_allowance)`. Fresh grant yields the full boost; at-cap on either axis yields zero.
- Net effect: in-budget `store` txs sort strictly above over-budget ones; over-budget txs ride leftover block space (no rejection, just demotion). Pool nonce/arrival ordering breaks ties among in-budget signers.

### Example

PoP authorizes Alice for 64 MiB and 4 transactions (`bytes_allowance = 64 MiB`, `transactions_allowance = 4`, `bytes = 0`, `transactions = 0`). Bulletin runtime uses `FlatBoost`.

| Step | Call | After | Boost |
|---|---|---|---|
| 1 | `store(30 MiB)` | `bytes = 30 MiB`, `transactions = 1` | `ALLOWANCE_PRIORITY_BOOST` (in-budget on both axes) |
| 2 | `store(30 MiB)` | `bytes = 60 MiB`, `transactions = 2` | `ALLOWANCE_PRIORITY_BOOST` (still in-budget) |
| 3 | `store(10 MiB)` | `bytes = 70 MiB` (over byte cap), `transactions = 3` | `0` (over byte axis) |
| 4 | `store(1 MiB)` | `bytes = 71 MiB`, `transactions = 4` | `0` (still over byte axis; tx axis exactly at cap) |
| 5 | PoP calls `authorize_account(Alice, 4, 64 MiB)` | unexpired → additive: `bytes_allowance = 128 MiB`, `transactions_allowance = 8`; `bytes` / `transactions` preserved | — |
| 6 | `store(20 MiB)` | `bytes = 91 MiB`, `transactions = 5` | `ALLOWANCE_PRIORITY_BOOST` (in-budget on both axes again: `91 ≤ 128` and `5 ≤ 8`) |

Steps 1–2 ride normal high priority. From step 3 onward Alice's `store` calls still validate and consume both axes, but the `AllowanceBasedPriority` extension contributes `0`, so they queue behind every in-budget signer and only land in blocks with leftover space. Step 5 shows PoP topping up both axes via additive re-authorize within the unexpired window — `refresh_account_authorization` would only push expiry back without restoring soft headroom, so it's not the right tool here.

## Hard Limit

Proposed; not yet wired. The hard cap is enforced at two levels and a renewal that would breach **either** is **rejected** (no leftover-space fallback — permanent storage doesn't have one).

### Caps

- **Per-account** (`Error::PermanentAllowanceExceeded`): a `renew` of `size` bytes for account `A` is rejected if
  `A.bytes_permanent + size > A.bytes_allowance`.
  Re-uses the same `bytes_allowance` field that the soft side reads; PoP's byte grant per account bounds both temp and permanent usage. (`transactions_allowance` plays no role on the hard side.)
- **Chain-wide** (`Error::ChainPermanentCapReached`): rejected if `PermanentStorageUsed + size > T::MaxPermanentStorageSize::get()`.
  `MaxPermanentStorageSize` is a `Config` trait constant (e.g. `1.7 TiB`). See [Capacity Planning](#capacity-planning) for how the runtime can make it adjustable at runtime.

`bytes_permanent` may transiently exceed `bytes_allowance` if the cap is lowered below the account's current usage — via the expired re-grant path of `authorize_account` (where `bytes_allowance = bytes` replaces the old cap), or via governance overriding `MaxPermanentStorageSize`. (Within an unexpired window, `authorize_account` is additive and can never lower the cap.) New renews reject; ledger drains will bring it back below the cap eventually. Existing on-chain data is unaffected.

### New storage items

- `PermanentStorageUsed: StorageValue<u64, ValueQuery>` — chain-wide sum of all accounts' `bytes_permanent`. Maintained across renews and lazy expiry-driven decrements.
- `PermanentStorageLedger: StorageMap<BlockNumber, BoundedVec<(AuthorizationScope, u64), MaxBlockTransactions>>` — for each block, the list of `(scope, size)` pairs that came in via `renew` in that block, where `scope` is the `AuthorizationScope::{Account, Preimage}` whose `bytes_permanent` was bumped. Drained lazily by `on_poll` once retention has elapsed; the scope tells the drain which `Authorizations` entry to decrement (account- or preimage-keyed).
- `PermanentStorageLedgerCursor: StorageValue<BlockNumber, ValueQuery>` — oldest block whose ledger entry has not been fully drained yet. Advanced by `on_poll` as drain progresses; bounds how far back the ledger needs to be retained.

### `renew` flow

1. Verify authorization (existing).
2. **Hard-cap check (per-account)**: reject with `Error::PermanentAllowanceExceeded` if `extent.bytes_permanent + size > extent.bytes_allowance`.
3. **Hard-cap check (chain-wide)**: reject with `Error::ChainPermanentCapReached` if `PermanentStorageUsed + size > T::MaxPermanentStorageSize::get()`.
4. Index data, push to `BlockTransactions` (existing).
5. `extent.bytes_permanent += size`.
6. `PermanentStorageUsed += size`.
7. Append `(scope, size)` to `PermanentStorageLedger[current_block]` (`scope` is the `AuthorizationScope` whose `bytes_permanent` was bumped — account or preimage).
8. Emit `Renewed`.

### Expiry flow (`on_poll` — lazy)

The data itself is pruned by the existing `Transactions::<T>::remove(obsolete)` in `on_initialize` (where `obsolete = n - RetentionPeriod - 1`); that hasn't changed. What runs lazily is the **accounting drain**: decrementing per-account `bytes_permanent` and chain-wide `PermanentStorageUsed` for ledger entries whose retention has elapsed.

Why lazy: the drain can touch up to `MaxBlockTransactions` entries per due block (each: read auth + write auth + write counter), which is non-trivial mandatory weight. Deferring it to `on_poll` keeps `on_finalize` light and lets the work happen when there's spare block capacity.

Why safe: lagging drain *over*-counts (still includes bytes already pruned on chain). That makes the cap stricter than necessary, never looser — we never oversubscribe past `MaxPermanentStorageSize`.

**Mechanism:**

- `PermanentStorageLedgerCursor: StorageValue<BlockNumber, ValueQuery>` — the oldest block in the ledger that hasn't been drained yet.
- In `on_poll`, while `PermanentStorageLedgerCursor + RetentionPeriod <= current_block` and weight permits:
    1. Take a bounded batch from `PermanentStorageLedger[PermanentStorageLedgerCursor]`.
    2. For each `(scope, size)`: decrement `Authorizations[scope].extent.bytes_permanent` and `PermanentStorageUsed` by `size` (saturating).
    3. If the entry is now empty, advance the cursor; otherwise leave it for the next `on_poll`.

`store` calls don't add to `PermanentStorageLedger`, so they're a no-op for the drain — temp data is fully handled by the existing `Transactions::<T>::remove(obsolete)` in `on_initialize`.

**Liveness fallback (mandatory):** if the chain runs sustained mandatory/operational load, `on_poll` can be skipped indefinitely. To prevent the cursor from stalling and `PermanentStorageUsed` drifting toward `MaxPermanentStorageSize`, `on_initialize` does a bounded mandatory drain when the cursor falls behind a now-drainable entry:

> If `current_block - PermanentStorageLedgerCursor > RetentionPeriod` (i.e. the cursor's entry is drainable but `on_poll` didn't pick it up), drain a bounded batch in `on_initialize` under mandatory weight. Same algorithm as the `on_poll` step; just with a non-skippable trigger.

Mandatory work is bounded per block, so it doesn't overflow weight.

### Re-renewal accounting

If user A renews item X at block 100 and again at block 200:

- Two `PermanentStorageLedger` entries (one per block).
- `bytes_permanent[A]` and `PermanentStorageUsed` are incremented twice (once per renew).
- At block 100 + retention, the first record expires and decrements once.
- At block 200 + retention, the second record expires and decrements once.

During the overlap (block 200 through block 100 + retention) both copies of X are on disk, so `bytes_permanent[A] = 2 × size` honestly tracks on-chain state — it's not an overcount of actual storage. The counter follows reality: it drops back as the older copy ages out. From a *unique-content* perspective, however, the same data is counted twice against the cap.

> Per-content dedup is a nice-to-have for cap *efficiency* (avoid counting the same content twice when re-renewed) and could be done as part of [PR #313's `TransactionByContentHash`](#align-with-auto-renewal-pr-313) — on `renew(X)`, look up the previous `(block, idx)` for `X` and cancel its ledger entry instead of appending a new one. Not required for per-account correctness (the per-account cap already bounds each user's contribution).

### Authorization expiry / refresh interactions

- `refresh_account_authorization` only extends expiration; all consumed counters (`bytes`, `bytes_permanent`, `transactions`) are preserved (see [Refresh authorization](#refresh-authorization)). No global counter changes — the data is still on chain.
- An authorization expiring (no refresh) does **not** free `bytes_permanent` or decrement `PermanentStorageUsed`. The data still lives on chain until its own renewal record expires; the per-account counter drops only at that point.
- `remove_expired_account_authorization` **refuses** to remove the entry while `bytes_permanent > 0`. Removing it would orphan the lazy ledger drain (it has nowhere to decrement). The entry becomes removable once the ledger has fully drained the account's pending decrements (i.e. once `bytes_permanent` has dropped back to `0` naturally).

### Migration / bootstrap

- **Polkadot deployment**: no migration needed.
- **Paseo deployment**: don't care about counter accuracy — if usage genuinely matters there, it would be set up explicitly.
- **Catch-up if needed**: `PermanentStorageUsed` (and per-account `bytes_permanent`) can be recomputed off-chain by scanning `Transactions[*]` and adjusted on-chain via a root-only call. Treated as an operational tool, not a runtime migration.

### Open questions

- **Split allowance (deferred)**: a single `bytes_allowance` is the v1 design choice (see [Allowance Limits](#allowance-limits)). A future split into `bytes_allowance` (soft / temp) and `bytes_permanent_allowance` (hard / permanent) would let PoP grant generous temp without also granting generous permanent capacity. Add only if policy demands it.
- **Per-content deduplication of re-renewals**: nice-to-have; not blocking.

## Capacity Planning

Track the overall space utilisation for permanent storage and act when it's close to full.

**Signals.** Emit `PermanentStorageUsed` updates as events on each `renew`/expiry, plus a `PermanentStorageNearCap` event when crossing a threshold (e.g. 80% of `MaxPermanentStorageSize`).

**Reactions.** Governance can either:

- Pass a referendum to upgrade collator disk capacity and raise `MaxPermanentStorageSize`. The pallet sees `MaxPermanentStorageSize` as a `Config` trait constant; the runtime picks the backing: `parameter_types! { pub const … }` (runtime-upgrade only) or `parameter_types! { pub storage … }` (storage-backed; mutable at runtime via `system.set_storage`). Pallet code is identical either way.
- Coordinate spawning another bulletin chain.

## Auto Renewal

**Constraint.** Whatever auto-renewal mechanism we land on, it must reuse the manual `renew` code path so all the [Hard Limit](#hard-limit) accounting fires consistently — per-account `bytes_permanent` increment, chain-wide `PermanentStorageUsed` cap check, `PermanentStorageLedger` append, and lazy expiry-driven decrement. No separate accounting path; auto-renewal is just an automated trigger of the same extrinsic semantics.

## TODO

### Align with auto-renewal ([PR #313](https://github.com/paritytech/polkadot-bulletin-chain/pull/313))

PR #313 introduces `TransactionByContentHash`, `AutoRenewals`, `PendingAutoRenewals`, and the `process_auto_renewals` mandatory inherent. The two designs need to be merged at one point — `do_renew` — before either ships in production. Items to resolve:

- **Use `TransactionByContentHash` to fix the re-renewal DoS.** On `renew(X)`, look up the previous `(block, idx)` for `X`; cancel its `PermanentStorageLedger` entry instead of double-counting. Drops the per-content overcount and makes `PermanentStorageUsed` an accurate measure of unique-content bytes on chain.
- **Centralize accounting in `do_renew`.** PR #313 already plumbs `renew`, `renew_content_hash`, and `process_auto_renewals` through a shared `do_renew`. The hard-cap checks (per-account `bytes_permanent + size > bytes_allowance`, chain-wide `PermanentStorageUsed + size > MaxPermanentStorageSize`) and the ledger append must live there — one place, three call sites.
- **Generalize `PermanentStorageLedger` to carry `AuthorizationScope`** (not just `AccountId`), so preimage-scoped renewals participate in the chain-wide cap and lazy decrement. PR #313's `AutoRenewalData { account }` is account-only by intent; that's fine for auto-renewal, but the ledger has to handle both scopes.
- **Specify `process_auto_renewals` behavior on chain-wide cap rejection.** If `do_renew` rejects an auto-renewal because of `MaxPermanentStorageSize`, treat it the same as PR #313's "block full" path: remove the registration, emit `AutoRenewalFailed`, let the data expire normally.
- **Drop the snapshot check in `enable_auto_renew`** (or replace with a real reservation). The current check in PR #313 (`extent.transactions > 0 && extent.bytes >= tx_info.size`) is misleading — it suggests "this will work" while making no guarantees beyond the current block. Either remove it (and let `process_auto_renewals` be the single source of truth on per-cycle eligibility) or back it with a deposit / reservation that survives across cycles.
- **Reserve block-transaction slots for user txs.** `process_auto_renewals` is mandatory and pushes into the same `BlockTransactions` slot as user `store`/`renew` calls. If auto-renewals fill `MaxBlockTransactions`, user txs in the same block fail with `TooManyTransactions`. Either cap auto-renewals to a fraction of `MaxBlockTransactions` or partition the slot budget.
- **Audit the `on_initialize` weight in PR #313.** Per expiring tx: 2 reads + up to 2 writes, all in mandatory weight. Worst case at `MaxBlockTransactions = 512` is ~1500 reads + ~500 writes per `on_initialize`. Bench it; chunk over multiple blocks if it doesn't fit.
